use http::{Method, StatusCode};
use rust_squared::{Route, RsqApp};

#[tokio::main]
async fn main() {
    let _app = RsqApp::new()
        .route(Route::new(Method::GET, "/items", |_| async { Ok(StatusCode::OK) }))
        .expect("example route should register");
}
