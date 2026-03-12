# FastRust

FastRust is a Rust web framework built directly on Hyper to deliver a FastAPI-style developer
experience with Rust-native performance and control.

It is designed for people who want:

- cleaner endpoint code than raw Hyper
- more direct control than stacking on top of another full framework
- automatic docs and typed request handling
- a foundation that stays close to the metal

## Why FastRust

Most Rust web development today tends to fall into one of three buckets:

- raw HTTP work with Hyper, which is powerful but low-level
- established frameworks like Axum and Actix Web, which are productive but come with their own design choices
- Python-style DX expectations from frameworks like FastAPI, which many Rust frameworks only partially mirror

FastRust sits in the middle:

- built directly on Hyper
- typed extractors for request data
- route macros for cleaner endpoint definitions
- middleware for cross-cutting concerns
- automatic `/openapi.json` and `/docs`

## Positioning

| Approach | What it feels like | Tradeoff |
| --- | --- | --- |
| Raw Hyper | Maximum control, protocol-level building blocks | More boilerplate, manual routing and parsing |
| Axum / Actix Web | Mature ecosystem, productive, batteries included | You adopt their framework model |
| FastRust | FastAPI-style ergonomics on a Hyper-native base | Still early, MVP stage, features still growing |

## Comparison

### Raw Hyper

With raw Hyper, you usually match on method and path manually and build responses yourself:

```rust
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;

async fn handle(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    match (req.method().as_str(), req.uri().path()) {
        ("GET", "/users/1") => Ok(Response::new(Full::new(Bytes::from("user 1")))),
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("not found")))
            .unwrap()),
    }
}
```

That gives you full control, but you carry the routing and request parsing burden yourself.

### Typical Rust Framework Style

In many frameworks, endpoint definitions get more ergonomic, but you are still working inside that
framework's conventions:

```rust
async fn get_user(Path(id): Path<u64>) -> Json<User> {
    Json(User { id })
}
```

That is productive, but the framework owns the abstractions, runtime decisions, and middleware model.

### FastRust

FastRust aims for clean endpoint code while staying directly on Hyper:

```rust
use rust_squared::{Json, Path, RsqApp, get};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct UserPath {
    id: u64,
}

#[derive(Serialize)]
struct User {
    id: u64,
}

#[get(
    "/users/{id}",
    summary = "Fetch a user",
    description = "Returns one user from the API by id.",
    operation_id = "getUser",
    tag = "Users"
)]
async fn get_user(Path(path): Path<UserPath>) -> Result<Json<User>, rust_squared::RsqError> {
    Ok(Json(User { id: path.id }))
}

#[tokio::main]
async fn main() {
    let _app = RsqApp::new()
        .with_docs()
        .route(get_user_route())
        .expect("example route should register");
}
```

The goal is to make endpoint code feel simple while keeping the underlying stack predictable.

## Current Features

FastRust currently includes:

- trie-based router with static-over-parameter precedence
- `RsqApp` builder with `get`, `post`, and direct route registration
- typed extractors: `Path<T>`, `Query<T>`, `Json<T>`, `State<T>`
- JSON request parsing and JSON response serialization
- route macros: `#[get(...)]`, `#[post(...)]`
- route metadata: `summary`, `description`, `operation_id`, `tag`
- middleware system with onion-style chaining
- built-in middleware:
  - `CorsMiddleware`
  - `BearerAuthMiddleware`
  - `LoggingMiddleware`
- generated docs endpoints:
  - `/openapi.json`
  - `/docs`
- benchmark harness for router lookup and in-process request dispatch

## Example

```rust
use rust_squared::{
    BearerAuthMiddleware, CorsMiddleware, Json, Path, Query, RsqApp, State, get,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct AppConfig {
    prefix: String,
}

#[derive(Deserialize)]
struct UserPath {
    id: u64,
}

#[derive(Deserialize)]
struct UserQuery {
    verbose: bool,
}

#[derive(Serialize)]
struct User {
    id: u64,
    label: String,
    verbose: bool,
}

#[get("/users/{id}", summary = "Fetch a user", tag = "Users")]
async fn get_user(
    Path(path): Path<UserPath>,
    Query(query): Query<UserQuery>,
    State(config): State<AppConfig>,
) -> Result<Json<User>, rust_squared::RsqError> {
    Ok(Json(User {
        id: path.id,
        label: format!("{}-{}", config.prefix, path.id),
        verbose: query.verbose,
    }))
}

#[tokio::main]
async fn main() {
    let _app = RsqApp::new()
        .state(AppConfig {
            prefix: "user".into(),
        })
        .unwrap()
        .middleware(CorsMiddleware::permissive())
        .middleware(BearerAuthMiddleware::new("secret-token"))
        .with_docs()
        .route(get_user_route())
        .expect("route should register");
}
```

## Documentation Endpoints

When docs are enabled with `.with_docs()`, FastRust exposes:

- `/openapi.json` for the generated OpenAPI 3.1 document
- `/docs` for the interactive Swagger UI page

This gives the framework a practical FastAPI-style feedback loop early in development.

## Benchmarks

FastRust includes a benchmark harness built with Criterion.

Available benchmarks:

- `cargo bench -p rust_squared --bench routing_bench`
- `cargo bench -p rust_squared --bench throughput_bench`

These cover:

- router lookup speed
- in-process request dispatch cost

See [BENCHMARKS.md](./BENCHMARKS.md) for the benchmarking notes and intended comparison strategy.

## Project Structure

```text
Purabh/rsq/
  rust_squared/          # main framework crate
  rust_squared_macros/   # proc macros
  cargo-rsq/             # CLI crate
  examples/              # example applications
```

## Current Status

FastRust is currently in MVP development.

Implemented:

- routing foundation
- typed extractors
- route macros
- middleware
- basic auth/logging/CORS
- OpenAPI endpoint and Swagger docs page
- benchmark scaffolding

Still evolving:

- richer schema generation for request/response models
- more HTTP method macros (`put`, `patch`, `delete`)
- more polished CLI support
- stronger docs/export workflows

## Why Build Directly on Hyper

FastRust is built directly on Hyper because Hyper is the low-level HTTP engine, not a full opinionated
web framework. That means FastRust can:

- inherit a proven HTTP foundation
- stay close to the transport layer
- avoid unnecessary framework stacking
- focus development effort on developer experience, routing, docs, and middleware

## Vision

The long-term vision is straightforward:

FastAPI-level developer experience, Rust-level speed, and Hyper-level control.
