# FastRust Roadmap

## Current State

FastRust (`rust_squared` crate) is a Hyper-based async web framework for Rust,
feature-complete through Phase 4 (security hardening). It ships a trie router,
11 built-in middlewares, typed extractors, SSE, OpenAPI 3.1 generation, and an
in-process `TestClient`. The framework is pre-1.0: the public API is stable
enough for experimentation but not yet pinned for semver guarantees.

## Architecture Overview (for contributors)

| File | Purpose |
|------|---------|
| `app.rs` | `RsqApp` builder, `serve()`, `handle()`, tower::Service impl |
| `router/` | Trie-based router; `Route`, `RouteMeta`, `BoxedHandler` |
| `extract.rs` | `FromRequest` trait + all extractors (`Path`, `Query`, `Json`, `State`, `Depends`, `Pagination`, …) |
| `middleware.rs` | `RsqMiddleware` trait + 11 built-in middlewares (CORS, Auth, CSRF, Rate-limit, Timeout, …) |
| `request.rs` | `RequestContext` — per-request state, body, path params, dep cache |
| `response.rs` | `IntoResponse` trait + `Response`/`Html`/`Redirect`/`NdjsonResponse` |
| `error.rs` | `RsqError` — typed errors with status code; `ValidationErrors` FastAPI-compat format |
| `openapi.rs` | OpenAPI 3.1 spec generation, Swagger UI response |
| `testing.rs` | `TestClient` — in-process test helper (no TCP) |
| `sse.rs` | `Sse` / `SseEvent` — Server-Sent Events response type |
| `ws.rs` | WebSocket upgrade scaffold (feature-gated, incomplete) |
| `state.rs` | `AppState` — type-erased shared state map |
| `multipart.rs` | Multipart form body parser |
| `sanitize.rs` | `html_escape`, `is_safe_header_value`, `strip_null_bytes` |
| `schema.rs` | `RsqSchema` trait + `#[derive(RsqSchema)]` macro |
| `cookie.rs` | `CookieJar` extractor, `set_cookie` helpers |
| `static_files.rs` | `ServeDir` — static file serving with path-traversal guard |

## Week 1–2: Stabilize

- [ ] Fix `with_auth` example — show real `BearerAuthMiddleware` flow with `State<AppConfig>`
- [ ] Fix `hello_world` example — actually call `.serve()`
- [ ] Implement `cargo-rsq` CLI (spec at `specs/PHASE5_DX_POLISH.md`)
- [ ] Publish `rust_squared` 0.1.0 to crates.io
- [ ] **Acceptance**: all examples compile and run end-to-end; `cargo install cargo-rsq` works

## Month 1: Foundation

- [ ] Complete WebSocket `on_upgrade()` with `tokio-tungstenite` (`ws.rs` scaffold exists)
- [ ] Add `request_body = "Type"` attribute to `#[post]`/`#[put]`/`#[patch]` macros
- [ ] Auto-register schemas from `Json<T>` return type in route macros
- [ ] Add `sqlx` + SQLite integration example with `State<SqlitePool>`
- [ ] Add `#[derive(RsqSchema)]` support for enum types
- [ ] Run and publish cross-framework benchmarks vs raw Hyper, Axum, FastAPI
- [ ] Change `TimeoutMiddleware` to return 408 REQUEST_TIMEOUT (currently 504)
- [ ] **Acceptance**: working database example, complete OpenAPI story, benches published

## Month 2–3: Growth

- [ ] Prefix routing / route groups: `Router::new().prefix("/api/v1").middleware(auth)`
- [ ] JWT middleware: `JwtMiddleware<Claims>` that validates and injects `Claims`
- [ ] `cargo rsq dev` hot-reload
- [ ] Error handler customization: `app.on_error(|err| async { … })`
- [ ] Response streaming beyond SSE: `StreamBody` type (design at `docs/STREAMING_BODY_DESIGN.md`)
- [ ] **Acceptance**: version 0.2.0, published benchmark comparison

## Backlog

- `#[middleware]` attribute for handler-level middleware
- Improve Swagger UI (API title, servers block, security scheme)
- LRU eviction for rate limiter token buckets
- `WithBody` error responses for all error types (currently plain text)
- `cargo rsq generate` for route/handler boilerplate

## Getting Started for Contributors

```bash
# Verify everything compiles
cargo check --workspace

# Run all tests (unit + integration + regression)
cargo test --workspace

# Lint
cargo clippy --workspace

# Run a specific test file
cargo test --test integration_test
cargo test --test regression_test
```

### Adding a new middleware

1. Add a `pub struct MyMiddleware { … }` in `middleware.rs`.
2. Implement `RsqMiddleware` using `Box::pin(async move { … })`.
3. Re-export from `lib.rs` under `pub use middleware::MyMiddleware`.
4. Add unit tests inside `middleware.rs` and an integration smoke-test in
   `tests/integration.rs`.

### Adding a new extractor

1. Define `pub struct MyExtractor(pub T)` in `extract.rs`.
2. Implement `FromRequest` with `#[async_trait]`.
3. Re-export from `lib.rs`.
4. Add tests in `extract.rs` and/or `tests/regression_test.rs`.
