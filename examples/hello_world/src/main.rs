use http::Method;
use rust_squared::{Route, RsqApp};

#[tokio::main]
async fn main() {
    let _app = RsqApp::new()
        .route(Route::new(Method::GET, "/", |_| async { Ok("hello world") }))
        .expect("example route should register");
}
