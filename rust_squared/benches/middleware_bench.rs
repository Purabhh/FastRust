use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use rust_squared::{
    BearerAuthMiddleware, CompressionMiddleware, CorsMiddleware, LoggingMiddleware,
    RateLimitMiddleware, Route, RsqApp,
};
use tokio::runtime::Runtime;

fn build_baseline_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
}

fn build_logging_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
        .middleware(LoggingMiddleware::new())
}

fn build_cors_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
        .middleware(CorsMiddleware::permissive())
}

fn build_bearer_auth_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
        .middleware(BearerAuthMiddleware::new("bench-token"))
}

fn build_rate_limit_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
        .middleware(RateLimitMiddleware::new(1_000_000.0, 1_000_000))
}

fn build_compression_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
        .middleware(CompressionMiddleware::new())
}

fn build_all_middleware_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/health", |_| async {
            Ok(StatusCode::OK)
        }))
        .unwrap()
        .middleware(LoggingMiddleware::new())
        .middleware(CorsMiddleware::permissive())
        .middleware(BearerAuthMiddleware::new("bench-token"))
        .middleware(RateLimitMiddleware::new(1_000_000.0, 1_000_000))
        .middleware(CompressionMiddleware::new())
}

fn make_get_health() -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header("authorization", "Bearer bench-token")
        .body(Full::new(Bytes::new()))
        .unwrap()
}

fn middleware_overhead_benchmark(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();

    let baseline = build_baseline_app();
    c.bench_function("middleware_baseline_no_middleware", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = make_get_health();
            let resp = baseline.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    let logging_app = build_logging_app();
    c.bench_function("middleware_logging_only", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = make_get_health();
            let resp = logging_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    let cors_app = build_cors_app();
    c.bench_function("middleware_cors_only", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = make_get_health();
            let resp = cors_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    let auth_app = build_bearer_auth_app();
    c.bench_function("middleware_bearer_auth_only", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = make_get_health();
            let resp = auth_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    let rate_app = build_rate_limit_app();
    c.bench_function("middleware_rate_limit_only", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = make_get_health();
            let resp = rate_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    let compression_app = build_compression_app();
    c.bench_function("middleware_compression_only", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = make_get_health();
            let resp = compression_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    let all_app = build_all_middleware_app();
    c.bench_function("middleware_all_combined", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = make_get_health();
            let resp = all_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });
}

criterion_group!(benches, middleware_overhead_benchmark);
criterion_main!(benches);
