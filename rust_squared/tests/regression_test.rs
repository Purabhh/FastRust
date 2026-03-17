//! Regression tests for bugs fixed in the audit review cycle.
//!
//! Each test is labelled with the original bug ID so failures are easy to
//! trace back to the relevant fix.

use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use serde::Deserialize;
use validator::Validate;

use rust_squared::{
    Json, MaxBodySizeMiddleware, RsqApp, RsqError, ValidatedJson,
    CsrfMiddleware, testing::TestClient,
    router::Route,
    request::RequestContext,
};

// ── C-1: body size limit enforced by MaxBodySizeMiddleware ────────────────────
//
// Bug: the configured limit was applied only to streaming bodies; buffered
// bodies coming through `from_request` bypassed the check entirely.
// Fix: `take_body_bytes()` now enforces the limit for both body variants.

#[tokio::test]
async fn regression_c1_body_size_limit_enforced() {
    let limit: usize = 64;

    let app = RsqApp::new()
        .middleware(MaxBodySizeMiddleware::new(limit))
        .route(Route::new(
            Method::POST,
            "/upload",
            |mut ctx: RequestContext| async move {
                ctx.take_body_bytes().await?;
                Ok::<_, RsqError>("ok")
            },
        ))
        .unwrap();

    let client = TestClient::new(app);

    // Body at limit — must succeed.
    let at_limit = vec![b'a'; limit];
    let res = client
        .post("/upload")
        .body(Bytes::from(at_limit))
        .send()
        .await;
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "body at limit must be accepted; got: {}",
        res.text()
    );

    // Body over limit — must be rejected with 413.
    let over_limit = vec![b'b'; limit + 1];
    let res = client
        .post("/upload")
        .body(Bytes::from(over_limit))
        .send()
        .await;
    assert_eq!(
        res.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "body exceeding limit must return 413; got: {}",
        res.text()
    );
}

// ── M-4: CSRF token always rotated on safe methods, even when cookie exists ───
//
// Bug: when a request already carried a valid `csrf_token` cookie,
// `CsrfMiddleware` skipped setting a new `Set-Cookie` header on GET responses,
// leaving clients with a stale token indefinitely.
// Fix: the middleware now always emits `Set-Cookie` on safe methods.

#[tokio::test]
async fn regression_m4_csrf_token_rotated_when_cookie_already_exists() {
    let app = RsqApp::new()
        .middleware(CsrfMiddleware::new())
        .route(Route::new(Method::GET, "/page", |_| async { Ok("page") }))
        .unwrap();

    // First GET — no existing cookie.
    let first_res = app
        .handle(
            Request::builder()
                .method(Method::GET)
                .uri("/page")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(first_res.status(), StatusCode::OK);
    assert!(
        first_res.headers().contains_key("set-cookie"),
        "first GET must set a new CSRF cookie"
    );

    // Second GET — send back the cookie we received.
    // The middleware must still issue a fresh Set-Cookie to rotate the token.
    let second_res = app
        .handle(
            Request::builder()
                .method(Method::GET)
                .uri("/page")
                .header("cookie", "csrf_token=someexistingtoken")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(second_res.status(), StatusCode::OK);
    assert!(
        second_res.headers().contains_key("set-cookie"),
        "subsequent GET must still rotate and set a fresh CSRF cookie (M-4)"
    );
}

// ── H-6: method_not_allowed response does not panic on custom/unusual methods ─
//
// Bug: `method_not_allowed_response` called `.parse().expect(...)` on the
// joined `Allow` string without validating that all method names were valid
// HTTP header values, which could panic on custom methods with unusual chars.
// Fix: the Allow header is now built via `HeaderValue::from_bytes` with a
// fallback, so it never panics.
//
// We test the safe path with standard methods (which must always work) and
// verify the 405 + Allow response is structurally correct.

#[tokio::test]
async fn regression_h6_method_not_allowed_does_not_panic_on_standard_methods() {
    let app = RsqApp::new()
        .get("/resource", || async { Ok::<_, RsqError>("ok") })
        .unwrap()
        .post("/resource", || async { Ok::<_, RsqError>("created") })
        .unwrap();

    // PUT is not registered — should return 405 with Allow listing GET, POST.
    let response = app
        .handle(
            Request::builder()
                .method(Method::PUT)
                .uri("/resource")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "unregistered method must return 405"
    );

    let allow = response
        .headers()
        .get("allow")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        allow.contains("GET"),
        "Allow header must include GET; got: {allow}"
    );
    assert!(
        allow.contains("POST"),
        "Allow header must include POST; got: {allow}"
    );
}

// ── H-5: ValidatedJson checks Content-Type before reading body ────────────────
//
// Bug: `ValidatedJson` read the full request body before checking
// `Content-Type`, wasting memory and bypassing content negotiation.
// Fix: content-type is now checked first; the body is only read when the
// media type is `application/json`.

#[tokio::test]
async fn regression_h5_validated_json_checks_content_type_before_reading_body() {
    #[derive(Deserialize, Validate)]
    struct Payload {
        #[validate(length(min = 1))]
        name: String,
    }

    async fn handler(
        ValidatedJson(p): ValidatedJson<Payload>,
    ) -> Result<String, RsqError> {
        Ok(p.name)
    }

    let app = RsqApp::new().post("/create", handler).unwrap();

    // Wrong Content-Type — must return 415 even if the body is valid JSON.
    let response = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/create")
                .header("content-type", "text/plain")
                .body(Full::new(Bytes::from_static(br#"{"name":"alice"}"#)))
                .unwrap(),
        )
        .await;

    assert_eq!(
        response.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "ValidatedJson must reject non-JSON Content-Type with 415 (H-5)"
    );

    // Missing Content-Type — also must return 415.
    let response = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/create")
                .body(Full::new(Bytes::from_static(br#"{"name":"alice"}"#)))
                .unwrap(),
        )
        .await;

    assert_eq!(
        response.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "ValidatedJson must reject missing Content-Type with 415 (H-5)"
    );

    // Correct Content-Type + valid body — must succeed.
    let response = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/create")
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from_static(br#"{"name":"alice"}"#)))
                .unwrap(),
        )
        .await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "ValidatedJson must accept valid JSON with correct Content-Type"
    );
}

// ── Additional: Json extractor returns 422 on malformed JSON body ─────────────

#[tokio::test]
async fn json_extractor_returns_422_on_malformed_json() {
    #[derive(Deserialize)]
    struct Payload {
        name: String,
    }

    async fn handler(Json(p): Json<Payload>) -> Result<String, RsqError> {
        Ok(p.name)
    }

    let app = RsqApp::new().post("/submit", handler).unwrap();

    let response = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/submit")
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from_static(b"not-valid-json{")))
                .unwrap(),
        )
        .await;

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "malformed JSON body must return 422"
    );
}

// ── Additional: Query extractor returns 422 on type mismatch ──────────────────

#[tokio::test]
async fn query_extractor_returns_422_on_type_mismatch() {
    use rust_squared::Query;

    #[derive(Deserialize)]
    struct Params {
        count: u32,
    }

    async fn handler(Query(p): Query<Params>) -> Result<String, RsqError> {
        Ok(format!("{}", p.count))
    }

    let app = RsqApp::new().get("/items", handler).unwrap();

    // "count=abc" cannot parse into u32 → 422
    let response = app
        .handle(
            Request::builder()
                .method(Method::GET)
                .uri("/items?count=abc")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "type mismatch in query string must return 422"
    );
}

// ── Additional: Path extractor returns 422 when param cannot be deserialized ──

#[tokio::test]
async fn path_extractor_returns_422_on_bad_param_type() {
    use rust_squared::Path;

    #[derive(Deserialize)]
    struct Params {
        id: u64,
    }

    async fn handler(Path(p): Path<Params>) -> Result<String, RsqError> {
        Ok(format!("{}", p.id))
    }

    let app = RsqApp::new().get("/items/{id}", handler).unwrap();

    // "notanumber" cannot parse into u64 → 422
    let response = app
        .handle(
            Request::builder()
                .method(Method::GET)
                .uri("/items/notanumber")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;

    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "type mismatch in path param must return 422"
    );
}
