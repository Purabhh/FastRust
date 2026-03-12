use criterion::{Criterion, black_box, criterion_group, criterion_main};
use http::Method;
use rust_squared::{Route, Router};

fn build_router() -> Router {
    let mut router = Router::new();
    for id in 0..500 {
        router
            .insert(Route::new(Method::GET, format!("/users/{id}"), |_| async { Ok("ok") }))
            .unwrap();
    }
    router
        .insert(Route::new(Method::GET, "/users/{id}/posts/{post_id}", |_| async { Ok("ok") }))
        .unwrap();
    router
        .insert(Route::new(Method::GET, "/health", |_| async { Ok("ok") }))
        .unwrap();
    router
}

fn routing_lookup_benchmark(c: &mut Criterion) {
    let router = build_router();

    c.bench_function("router_lookup_static", |b| {
        b.iter(|| {
            let found = router.find(&Method::GET, black_box("/health")).unwrap();
            black_box(found.0.pattern());
        })
    });

    c.bench_function("router_lookup_param", |b| {
        b.iter(|| {
            let found = router
                .find(&Method::GET, black_box("/users/123/posts/456"))
                .unwrap();
            black_box(found.1);
        })
    });
}

criterion_group!(benches, routing_lookup_benchmark);
criterion_main!(benches);
