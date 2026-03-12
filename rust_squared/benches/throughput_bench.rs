use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use rust_squared::{Route, RsqApp};
use tokio::runtime::Runtime;

fn build_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async { Ok(StatusCode::OK) }))
        .unwrap()
        .route(Route::new(Method::GET, "/users/{id}", |ctx| async move {
            let id = ctx.path_param("id").unwrap_or("unknown").to_string();
            Ok(format!("user:{id}"))
        }))
        .unwrap()
}

fn throughput_benchmark(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let app = build_app();

    c.bench_function("app_handle_static_route", |b| {
        b.to_async(&runtime).iter(|| async {
            let request = Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Full::new(Bytes::new()))
                .unwrap();
            let response = app.handle(request).await;
            assert_eq!(response.status(), StatusCode::OK);
        })
    });

    c.bench_function("app_handle_param_route", |b| {
        b.to_async(&runtime).iter(|| async {
            let request = Request::builder()
                .method(Method::GET)
                .uri("/users/42")
                .body(Full::new(Bytes::new()))
                .unwrap();
            let response = app.handle(request).await;
            assert_eq!(response.status(), StatusCode::OK);
        })
    });
}

criterion_group!(benches, throughput_benchmark);
criterion_main!(benches);
