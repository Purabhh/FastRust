use std::sync::Arc;

use futures::stream;
use rust_squared::{BearerAuthMiddleware, CorsMiddleware, Json, LoggingMiddleware, Path, Query, RsqApp, RsqSchema, State, Sse};

use http::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::time::{interval, Duration};

#[derive(Serialize, Deserialize, RsqSchema)]
struct User {
    id: u64,
    name: String,
}

async fn hello() -> &'static str {
    "Hello, world!"
}

async fn create_user(Json(user): Json<User>) -> Result<(StatusCode, Json<User>), RsqError> {
    Ok((StatusCode::CREATED, Json(user)))
}

async fn sse_handler(State(state): State<Arc<String>>) -> Sse {
    let stream = interval(Duration::from_secs(1)).map(move |_| {
        let msg = format!("{}: {}", *state, chrono::Utc::now().timestamp());
        SseEvent::default().data(msg)
    });
    Sse::new(stream)
}

#[tokio::main]
async fn main() -> Result<(), RsqError> {
    let app = RsqApp::new()
        .with_docs()
        .middleware(LoggingMiddleware)
        .middleware(CorsMiddleware)
        .middleware(BearerAuthMiddleware)
        .state(Arc::new("prefix".to_string()))?
        .schema::<User>()
        .get("/", hello)
        .post("/users", create_user)
        .get("/sse", sse_handler)?;

    app.serve("0.0.0.0:3000".parse()?).await
}