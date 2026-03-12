use rust_squared::RsqApp;

#[tokio::main]
async fn main() {
    let _app = RsqApp::new()
        .get("/", || async { Ok("hello world") })
        .expect("example route should register");
}
