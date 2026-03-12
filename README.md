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

## Simplicity Comparison

The most important question for FastRust is not just "is it fast?" but:

How little code should it take to do something common and real?

Below is the same idea in four styles: fetch one user from a database and return JSON from
`GET /users/{id}`.

### 1. Raw Rust on Hyper

This is powerful, but you manually handle routing, path parsing, response building, and glue code.

```rust
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use serde::Serialize;

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

async fn load_user(id: u64) -> User {
    User { id, name: "Ada".into() }
}

async fn handle(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    match (req.method().as_str(), req.uri().path()) {
        ("GET", path) if path.starts_with("/users/") => {
            let id = path.trim_start_matches("/users/").parse::<u64>().unwrap();
            let user = load_user(id).await;
            let body = serde_json::to_vec(&user).unwrap();

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(Full::new(Bytes::from(body)))
                .unwrap())
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("not found")))
            .unwrap()),
    }
}
```

### 2. Axum

Axum removes a lot of the plumbing and gives you typed extractors.

```rust
use axum::{extract::Path, response::Json, routing::get, Router};
use serde::Serialize;

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

async fn load_user(id: u64) -> User {
    User { id, name: "Ada".into() }
}

async fn get_user(Path(id): Path<u64>) -> Json<User> {
    Json(load_user(id).await)
}

let app = Router::new().route("/users/:id", get(get_user));
```

### 3. FastAPI

FastAPI is the ergonomics bar many developers expect now: very little boilerplate and automatic docs.

```python
from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI()

class User(BaseModel):
    id: int
    name: str

def load_user(user_id: int) -> User:
    return User(id=user_id, name="Ada")

@app.get("/users/{user_id}")
async def get_user(user_id: int) -> User:
    return load_user(user_id)
```

### 4. FastRust

FastRust is aiming for that same clarity while staying directly on Hyper.

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
    name: String,
}

async fn load_user(id: u64) -> User {
    User { id, name: "Ada".into() }
}

#[get(
    "/users/{id}",
    summary = "Fetch a user",
    description = "Returns one user by id.",
    operation_id = "getUser",
    tag = "Users"
)]
async fn get_user(Path(path): Path<UserPath>) -> Result<Json<User>, rust_squared::RsqError> {
    Ok(Json(load_user(path.id).await))
}

#[tokio::main]
async fn main() {
    let _app = RsqApp::new()
        .with_docs()
        .route(get_user_route())
        .expect("example route should register");
}
```

### What This Comparison Shows

Raw Hyper gives you maximum control, but the application code pays for it.

Axum is much cleaner, but you are still building inside another framework's conventions and route
registration model.

FastAPI is extremely concise and sets the standard for automatic docs and easy handler definitions.

FastRust's goal is to get as close as possible to that clarity while remaining native Rust and
directly Hyper-based.

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
