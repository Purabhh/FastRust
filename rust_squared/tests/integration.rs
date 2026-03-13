//! Full-stack integration tests — exercises the entire request pipeline
//! from HTTP request through router, middleware, extractors, and response.

use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use serde::{Deserialize, Serialize};

use rust_squared::{
    BearerAuthMiddleware, CorsMiddleware, CompressionMiddleware, CsrfMiddleware,
    LoggingMiddleware, MaxBodySizeMiddleware, RateLimitMiddleware, RequestIdMiddleware,
    RequestValidationMiddleware, SecurityHeadersMiddleware, TimeoutMiddleware,
    Json, Path, Query, RsqApp, RsqError, RsqSchema,
};
use rust_squared::router::Route;

// ── Helpers ─────────────────────────────────────────────────────────

fn get(uri: &str) -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Full::new(Bytes::new()))
        .unwrap()
}

fn post_json(uri: &str, body: &str) -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

// ── Full middleware stack ────────────────────────────────────────────

#[tokio::test]
async fn full_middleware_stack() {
    let app = RsqApp::new()
        .middleware(LoggingMiddleware::new())
        .middleware(RequestIdMiddleware::new())
        .middleware(SecurityHeadersMiddleware::defaults())
        .middleware(CorsMiddleware::permissive())
        .middleware(TimeoutMiddleware::new(std::time::Duration::from_secs(5)))
        .middleware(MaxBodySizeMiddleware::new(1024 * 1024))
        .middleware(RequestValidationMiddleware::json_only())
        .get("/", || async { Ok("hello") })
        .unwrap();

    let response = app.handle(get("/")).await;
    assert_eq!(response.status(), StatusCode::OK);
    // Check all middleware headers present
    assert!(response.headers().contains_key("x-request-id"));
    assert!(response.headers().contains_key("x-fastrust-logged"));
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["x-frame-options"], "DENY");
}

// ── Extractor pipeline ──────────────────────────────────────────────

#[derive(Deserialize)]
struct ItemPath {
    id: u64,
}

#[derive(Deserialize)]
struct ItemQuery {
    format: String,
}

#[derive(Deserialize, Serialize, RsqSchema)]
struct CreateItem {
    name: String,
}

#[derive(Serialize)]
struct ItemResponse {
    id: u64,
    name: String,
    format: String,
}

#[tokio::test]
async fn extractors_path_query_json_combined() {
    async fn create_item(
        Path(path): Path<ItemPath>,
        Query(query): Query<ItemQuery>,
        Json(body): Json<CreateItem>,
    ) -> Result<Json<ItemResponse>, RsqError> {
        Ok(Json(ItemResponse {
            id: path.id,
            name: body.name,
            format: query.format,
        }))
    }

    let app = RsqApp::new()
        .post("/items/{id}", create_item)
        .unwrap();

    let response = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/items/42?format=json")
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from(r#"{"name":"Widget"}"#)))
                .unwrap(),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/json");
}

// ── Auth + rate limiting ────────────────────────────────────────────

#[tokio::test]
async fn auth_then_rate_limit() {
    let app = RsqApp::new()
        .middleware(BearerAuthMiddleware::new("token123"))
        .middleware(RateLimitMiddleware::new(100.0, 5))
        .get("/protected", || async { Ok("secret") })
        .unwrap();

    // No auth -> 401
    let r = app.handle(get("/protected")).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

    // Valid auth -> 200
    let r = app
        .handle(
            Request::builder()
                .uri("/protected")
                .header("authorization", "Bearer token123")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
}

// ── CSRF on mutating methods only ───────────────────────────────────

#[tokio::test]
async fn csrf_skips_get_blocks_post() {
    let app = RsqApp::new()
        .middleware(CsrfMiddleware::new())
        .get("/page", || async { Ok("page") })
        .unwrap()
        .post("/submit", |_ctx: rust_squared::request::RequestContext| async {
            Ok::<_, RsqError>("submitted")
        })
        .unwrap();

    // GET works without CSRF
    let r = app.handle(get("/page")).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.headers().contains_key("set-cookie"));

    // POST without token -> 403
    let r = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/submit")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // POST with matching tokens -> 200
    let r = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/submit")
                .header("cookie", "csrf_token=mytoken")
                .header("x-csrf-token", "mytoken")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
}

// ── OpenAPI docs integration ────────────────────────────────────────

#[tokio::test]
async fn openapi_includes_all_routes() {
    let mut meta = rust_squared::RouteMeta::default();
    meta.set_summary("List items");
    meta.set_response_schema("CreateItem");

    let app = RsqApp::new()
        .with_docs()
        .schema::<CreateItem>()
        .get("/items", || async { Ok("[]") })
        .unwrap()
        .route(
            Route::new(Method::POST, "/items", |_| async { Ok("created") })
                .with_meta(meta),
        )
        .unwrap();

    let r = app.handle(get("/openapi.json")).await;
    assert_eq!(r.status(), StatusCode::OK);

    let r = app.handle(get("/docs")).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "text/html; charset=utf-8");
}

// ── Compression integration ─────────────────────────────────────────

#[tokio::test]
async fn compression_end_to_end() {
    let large = "x".repeat(1000);
    let app = RsqApp::new()
        .middleware(CompressionMiddleware::new())
        .get("/big", move || {
            let body = large.clone();
            async move { Ok::<_, RsqError>(body) }
        })
        .unwrap();

    // With accept-encoding: gzip
    let r = app
        .handle(
            Request::builder()
                .uri("/big")
                .header("accept-encoding", "gzip")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(r.headers()["content-encoding"], "gzip");

    // Without accept-encoding
    let r = app.handle(get("/big")).await;
    assert!(!r.headers().contains_key("content-encoding"));
}

// ── Multiple HTTP methods on same path ──────────────────────────────

#[tokio::test]
async fn multiple_methods_same_path() {
    let app = RsqApp::new()
        .get("/resource", || async { Ok("GET") })
        .unwrap()
        .post("/resource", |_ctx: rust_squared::request::RequestContext| async {
            Ok::<_, RsqError>("POST")
        })
        .unwrap()
        .put("/resource", |_ctx: rust_squared::request::RequestContext| async {
            Ok::<_, RsqError>("PUT")
        })
        .unwrap()
        .delete("/resource", |_ctx: rust_squared::request::RequestContext| async {
            Ok::<_, RsqError>("DELETE")
        })
        .unwrap();

    assert_eq!(app.handle(get("/resource")).await.status(), StatusCode::OK);

    let r = app
        .handle(
            Request::builder()
                .method(Method::POST)
                .uri("/resource")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);

    // PATCH not registered -> 405
    let r = app
        .handle(
            Request::builder()
                .method(Method::PATCH)
                .uri("/resource")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// ── Sanitization utils ──────────────────────────────────────────────

#[test]
fn sanitization_integration() {
    assert_eq!(
        rust_squared::html_escape("<img onerror=alert(1)>"),
        "&lt;img onerror=alert(1)&gt;"
    );
    assert!(!rust_squared::is_safe_header_value("value\r\nX-Injected: true"));
    assert!(rust_squared::is_safe_header_value("normal-value"));
    assert_eq!(rust_squared::strip_null_bytes("a\0b\0c"), "abc");
}
