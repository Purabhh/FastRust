use rust_squared::{CookieJar, Method, RsqApp, set_cookie};
use http::Request;
use http_body_util::Full;
use bytes::Bytes;

#[tokio::test]
async fn cookie_extractor_parses_header() {
    async fn handler(cookie: CookieJar) -> Result<String, rust_squared::RsqError> {
        Ok(format!(
            "{}-{}",
            cookie.get("session").unwrap_or("none"),
            cookie.get("theme").unwrap_or("none"),
        ))
    }

    let app = RsqApp::new().get("/", handler).unwrap();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/")
        .header("cookie", "session=abc123; theme=dark")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let response = app.handle(req).await;
    assert_eq!(response.status(), http::StatusCode::OK);
}

#[test]
fn set_cookie_formats_correctly() {
    let hv = set_cookie("session", "new123").unwrap();
    assert_eq!(hv.to_str().unwrap(), "session=new123");
}
