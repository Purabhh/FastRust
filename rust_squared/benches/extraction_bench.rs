use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use rust_squared::{Route, RsqApp};
use tokio::runtime::Runtime;

// ── JSON body parsing helpers ─────────────────────────────────────────────────

/// Build a JSON payload of approximately the requested byte size.
fn json_payload(target_bytes: usize) -> Bytes {
    // Each entry is ~19 bytes: `{"k":"XXXXXXXXXX"},`
    let entries = (target_bytes / 20).max(1);
    let mut s = String::from("[");
    for i in 0..entries {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(r#"{{"k":"{:010}"}}"#, i));
    }
    s.push(']');
    Bytes::from(s)
}

fn build_json_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::POST, "/data", |mut ctx| async move {
            let _bytes = ctx.take_body_bytes().await?;
            Ok(StatusCode::OK)
        }))
        .unwrap()
}

// ── Path parameter extraction helpers ────────────────────────────────────────

fn build_single_param_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/users/{id}", |ctx| async move {
            let _id = ctx.path_param("id").unwrap_or("").to_string();
            Ok(StatusCode::OK)
        }))
        .unwrap()
}

fn build_multi_param_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(
            Method::GET,
            "/orgs/{org}/repos/{repo}/issues/{issue_id}",
            |ctx| async move {
                let _org = ctx.path_param("org").unwrap_or("").to_string();
                let _repo = ctx.path_param("repo").unwrap_or("").to_string();
                let _issue = ctx.path_param("issue_id").unwrap_or("").to_string();
                Ok(StatusCode::OK)
            },
        ))
        .unwrap()
}

// ── Query string parsing helpers ──────────────────────────────────────────────

fn build_query_app() -> RsqApp {
    RsqApp::new()
        .route(Route::new(Method::GET, "/search", |ctx| async move {
            let _q = ctx.uri().query().unwrap_or("").to_string();
            Ok(StatusCode::OK)
        }))
        .unwrap()
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

fn extraction_benchmark(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();

    // JSON body parsing — small / medium / large
    let json_app = build_json_app();
    let small_payload = json_payload(100);
    let medium_payload = json_payload(1024);
    let large_payload = json_payload(10 * 1024);

    let mut group = c.benchmark_group("json_body_parsing");
    for (label, payload) in [
        ("small_100B", small_payload),
        ("medium_1KB", medium_payload),
        ("large_10KB", large_payload),
    ] {
        group.bench_with_input(BenchmarkId::new("parse", label), &payload, |b, payload| {
            b.to_async(&runtime).iter(|| async {
                let req = Request::builder()
                    .method(Method::POST)
                    .uri("/data")
                    .header("content-type", "application/json")
                    .body(Full::new(payload.clone()))
                    .unwrap();
                let resp = json_app.handle(req).await;
                assert_eq!(resp.status(), StatusCode::OK);
            })
        });
    }
    group.finish();

    // Path parameter extraction — single param
    let single_app = build_single_param_app();
    c.bench_function("path_param_single", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = Request::builder()
                .method(Method::GET)
                .uri("/users/42")
                .body(Full::new(Bytes::new()))
                .unwrap();
            let resp = single_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    // Path parameter extraction — multiple params
    let multi_app = build_multi_param_app();
    c.bench_function("path_param_multiple", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = Request::builder()
                .method(Method::GET)
                .uri("/orgs/acme/repos/widget/issues/99")
                .body(Full::new(Bytes::new()))
                .unwrap();
            let resp = multi_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });

    // Query string parsing
    let query_app = build_query_app();
    c.bench_function("query_string_parsing", |b| {
        b.to_async(&runtime).iter(|| async {
            let req = Request::builder()
                .method(Method::GET)
                .uri("/search?q=rust+web+framework&page=2&per_page=50&sort=stars&order=desc")
                .body(Full::new(Bytes::new()))
                .unwrap();
            let resp = query_app.handle(req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        })
    });
}

criterion_group!(benches, extraction_benchmark);
criterion_main!(benches);
