use rust_squared::{Method, Route, Router};

#[test]
fn trailing_slash_matches_without() {
    let mut router = Router::new();
    router.insert(Route::new(Method::GET, "/test/", |_| async { Ok("ok") })).unwrap();
    assert!(router.find(&Method::GET, "/test").is_ok());
}

#[test]
fn no_trailing_slash_matches_with_trailing() {
    // Router normalizes trailing slashes, so /test/ matches /test
    let mut router = Router::new();
    router.insert(Route::new(Method::GET, "/test", |_| async { Ok("ok") })).unwrap();
    assert!(router.find(&Method::GET, "/test/").is_ok());
}

#[test]
fn special_characters() {
    let mut router = Router::new();
    router.insert(Route::new(Method::GET, "/test/@#$%", |_| async { Ok("ok") })).unwrap();
    assert!(router.find(&Method::GET, "/test/@#$%").is_ok());
}

#[test]
fn unicode_segments() {
    let mut router = Router::new();
    router.insert(Route::new(Method::GET, "/test/😊/ cafés", |_| async { Ok("ok") })).unwrap();
    assert!(router.find(&Method::GET, "/test/😊/ cafés").is_ok());
}

#[test]
fn empty_path_segments() {
    let mut router = Router::new();
    router.insert(Route::new(Method::GET, "/test//double//slash", |_| async { Ok("ok") })).unwrap();
    assert!(router.find(&Method::GET, "/test//double//slash").is_ok()); // since split('/') skips empty
}