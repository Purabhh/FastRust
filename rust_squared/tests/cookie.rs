use rust_squared::{Cookie, Method, RequestContext, Route, RsqApp, set_cookie};
use http::Request;
use http_body_util::Full;
use bytes::Bytes;

#[tokio::test]
async fn cookie_extractor_parses_header() {
    let app = RsqApp::new()
        .route(Route::new(Method::GET, "/", |cookie: Cookie| async move {
            assert_eq!(cookie.0.get("session").map(|s| s.as_str()), Some("abc123"));
            assert_eq!(cookie.0.get("theme").map(|s| s.as_str()), Some("dark"));
            Ok("ok")
        }))?;

    let req = Request::builder()
        .header("cookie", "session=abc123; theme=dark")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let mut ctx = RequestContext::from_request(req, Default::default(), Default::default()).await.unwrap();
    let response = app.routes()[0].call(ctx).await.unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
}

#[test]
fn set_cookie_adds_header() {
    let mut res = http::Response::new(());
    set_cookie(&mut res, "session", "new123");
    assert_eq!(res.headers()["set-cookie"], "session=new123");
}