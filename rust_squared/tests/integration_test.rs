//! Integration tests using TestClient — exercises the full request pipeline
//! in-process with no TCP socket overhead.

use serde::{Deserialize, Serialize};
use http::StatusCode;

use rust_squared::{
    Json, Path, Query, RsqApp, RsqError,
    LoggingMiddleware, RequestIdMiddleware,
    testing::TestClient,
};

// ── Shared types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct ItemPath {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct ItemQuery {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
struct ItemResponse {
    id: u64,
    label: String,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Full happy path: GET with path param + query param → JSON response.
#[tokio::test]
async fn full_app_happy_path() {
    async fn get_item(
        Path(p): Path<ItemPath>,
        Query(q): Query<ItemQuery>,
    ) -> Result<Json<ItemResponse>, RsqError> {
        Ok(Json(ItemResponse {
            id: p.id,
            label: q.label.unwrap_or_else(|| "default".to_string()),
        }))
    }

    let app = RsqApp::new()
        .get("/items/{id}", get_item)
        .unwrap();

    let client = TestClient::new(app);

    let res = client.get("/items/99?label=widget").send().await;
    res.assert_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["id"], 99);
    assert_eq!(body["label"], "widget");
}

/// GET with path param but no query param uses handler-default value.
#[tokio::test]
async fn full_app_happy_path_no_query_uses_default() {
    async fn get_item(
        Path(p): Path<ItemPath>,
        Query(q): Query<ItemQuery>,
    ) -> Result<Json<ItemResponse>, RsqError> {
        Ok(Json(ItemResponse {
            id: p.id,
            label: q.label.unwrap_or_else(|| "default".to_string()),
        }))
    }

    let app = RsqApp::new()
        .get("/items/{id}", get_item)
        .unwrap();

    let client = TestClient::new(app);
    let res = client.get("/items/7").send().await;
    res.assert_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["id"], 7);
    assert_eq!(body["label"], "default");
}

/// Middleware chain: LoggingMiddleware + RequestIdMiddleware attach expected
/// response headers.
#[tokio::test]
async fn middleware_chain_attaches_request_id() {
    let app = RsqApp::new()
        .middleware(LoggingMiddleware::new())
        .middleware(RequestIdMiddleware::new())
        .get("/ping", || async { Ok::<_, RsqError>("pong") })
        .unwrap();

    let client = TestClient::new(app);
    let res = client.get("/ping").send().await;

    res.assert_ok();

    // RequestIdMiddleware must set x-request-id
    let req_id = res.header("x-request-id").expect("x-request-id header missing");
    assert!(!req_id.is_empty(), "x-request-id must not be empty");
    assert_eq!(req_id.len(), 36, "x-request-id must be UUID v4 (36 chars)");

    // LoggingMiddleware must set x-fastrust-logged
    assert_eq!(
        res.header("x-fastrust-logged").unwrap_or(""),
        "true",
        "LoggingMiddleware must set x-fastrust-logged: true"
    );
}

/// A caller-supplied X-Request-Id is preserved by RequestIdMiddleware when it
/// is a valid UUID. (Non-UUID values are intentionally rejected and replaced
/// with a fresh UUID — a log-injection hardening covered by the unit tests in
/// `middleware.rs`.)
#[tokio::test]
async fn middleware_chain_preserves_caller_request_id() {
    let app = RsqApp::new()
        .middleware(RequestIdMiddleware::new())
        .get("/ping", || async { Ok::<_, RsqError>("pong") })
        .unwrap();

    let client = TestClient::new(app);
    let caller_id = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
    let res = client
        .get("/ping")
        .header("x-request-id", caller_id)
        .send()
        .await;

    res.assert_ok();
    assert_eq!(res.header("x-request-id").unwrap_or(""), caller_id);
}

/// Unknown route returns 404.
#[tokio::test]
async fn unknown_route_returns_404() {
    let app = RsqApp::new()
        .get("/known", || async { Ok::<_, RsqError>("ok") })
        .unwrap();

    let client = TestClient::new(app);
    let res = client.get("/unknown").send().await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

/// Wrong method on a registered path returns 405 with an Allow header.
#[tokio::test]
async fn wrong_method_returns_405_with_allow_header() {
    let app = RsqApp::new()
        .get("/resource", || async { Ok::<_, RsqError>("ok") })
        .unwrap();

    let client = TestClient::new(app);
    // POST is not registered; only GET is.
    let res = client.post("/resource").send().await;
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);

    let allow = res.header("allow").expect("405 response must include Allow header");
    assert!(
        allow.contains("GET"),
        "Allow header must list GET, got: {allow}"
    );
}

/// App with multiple routes and state returns correct response per route.
#[tokio::test]
async fn multi_route_app_with_state() {
    #[derive(Clone)]
    struct Config {
        env: String,
    }

    async fn health(
        rust_squared::State(cfg): rust_squared::State<Config>,
    ) -> Result<String, RsqError> {
        Ok(format!("ok-{}", cfg.env))
    }

    let app = RsqApp::new()
        .state(Config { env: "test".to_string() })
        .unwrap()
        .get("/health", health)
        .unwrap()
        .get("/ready", || async { Ok::<_, RsqError>("ready") })
        .unwrap();

    let client = TestClient::new(app);

    let res = client.get("/health").send().await;
    res.assert_ok();
    assert!(res.text().contains("ok-test"));

    let res = client.get("/ready").send().await;
    res.assert_ok();
    assert!(res.text().contains("ready"));

    let res = client.get("/missing").send().await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

/// POST with JSON body is deserialized correctly.
#[tokio::test]
async fn post_json_body_round_trip() {
    #[derive(Deserialize)]
    struct Input {
        name: String,
    }

    async fn create(Json(body): Json<Input>) -> Result<String, RsqError> {
        Ok(format!("created-{}", body.name))
    }

    let app = RsqApp::new().post("/things", create).unwrap();
    let client = TestClient::new(app);

    let res = client
        .post("/things")
        .json(&serde_json::json!({"name": "widget"}))
        .send()
        .await;

    res.assert_ok();
    assert!(res.text().contains("created-widget"));
}
