use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use rust_squared::{RateLimitMiddleware, Route, RsqApp};
use tokio::runtime::Runtime;

fn build_plain_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/ping", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
}

fn build_rate_limited_app() -> RsqApp {
    // Very high limit so no requests are rejected during the benchmark; we only
    // measure the overhead introduced by the middleware itself.
    RsqApp::new()
        .route(Route::new(Method::GET, "/ping", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
        .middleware(RateLimitMiddleware::new(1_000_000.0, 1_000_000))
}

fn make_ping_request() -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::GET)
        .uri("/ping")
        .body(Full::new(Bytes::new()))
        .unwrap()
}

fn concurrent_benchmark(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();

    // Baseline: sequential requests, no rate-limit middleware
    let plain_app = build_plain_app();
    c.bench_function("concurrent_sequential_no_rate_limit", |b| {
        b.to_async(&runtime).iter(|| async {
            let resp = plain_app.handle(make_ping_request()).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    // Sequential requests WITH rate-limit middleware (measures per-request overhead)
    let rate_app = build_rate_limited_app();
    c.bench_function("concurrent_sequential_with_rate_limit", |b| {
        b.to_async(&runtime).iter(|| async {
            let resp = rate_app.handle(make_ping_request()).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    // Simulated parallel: 10 concurrent requests sharing the same RateLimitMiddleware
    // instance (shared Arc<DashMap>), measuring contention on the token bucket.
    let parallel_app = build_rate_limited_app();
    c.bench_function("concurrent_parallel_10_with_rate_limit", |b| {
        b.to_async(&runtime).iter(|| async {
            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let app = parallel_app.clone();
                    tokio::spawn(async move { app.handle(make_ping_request()).await })
                })
                .collect();

            for handle in handles {
                let resp = handle.await.expect("task panicked");
                assert_eq!(resp.status(), StatusCode::OK);
            }
        })
    });

    // Simulated parallel: 10 concurrent requests on the plain app (no middleware)
    // Provides a contention-free baseline to compare against the rate-limit variant.
    let parallel_plain_app = build_plain_app();
    c.bench_function("concurrent_parallel_10_no_rate_limit", |b| {
        b.to_async(&runtime).iter(|| async {
            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let app = parallel_plain_app.clone();
                    tokio::spawn(async move { app.handle(make_ping_request()).await })
                })
                .collect();

            for handle in handles {
                let resp = handle.await.expect("task panicked");
                assert_eq!(resp.status(), StatusCode::OK);
            }
        })
    });
}

criterion_group!(benches, concurrent_benchmark);
criterion_main!(benches);
