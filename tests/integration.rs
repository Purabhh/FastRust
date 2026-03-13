// tests/integration.rs

use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use rust_squared::{RsqApp, Route, Sse, SseEvent};

#[tokio::test]
async fn test_sse_streaming() {
    let app = RsqApp::new()
        .get("/sse", |_| async {
            let stream = futures::stream::iter(vec![
                SseEvent::data("one"),
                SseEvent::new("custom").data("two"),
                SseEvent::data("three"),
            ]);
            Sse::new(stream).into_response()
        })?;

    let req = Request::builder()
        .method(Method::GET)
        .uri("/sse")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let response = app.handle(req).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body);

    assert!(body_str.contains("data: one\n\n"));
    assert!(body_str.contains("event: custom\ndata: two\n\n"));
    assert!(body_str.contains("data: three\n\n"));
}
