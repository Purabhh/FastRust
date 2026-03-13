use std::sync::Arc;

use futures::stream;
use rust_squared::{BearerAuthMiddleware, CorsMiddleware, Json, LoggingMiddleware, Path, Query, RsqApp, RsqSchema, State};

use http::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::time::{interval, Duration};

#[derive(Deserialize)]
struct UserId {
    id: u64,
}

#[derive(Deserialize)]
struct Search {
    q: String,
}

#[derive(Deserialize)]
struct UpdateUser {
    name: String,
}

#[derive(Serialize, RsqSchema)]
struct User {
    id: u64,
    name: String,
}

async fn get_user(
    Path(path): Path<UserId>,
    Query(query): Query<Search>,
    State(prefix): State<Arc<String>>,
) -> Result<Json<User>, rust_squared::RsqError> {
    Ok(Json(User {
        id: path.id,
        name: format!("{} - {}", prefix.as_ref(), query.q),
    }))
}

async fn update_user(
    Path(path): Path<UserId>,
    Json(payload): Json<UpdateUser>,
) -> Result<StatusCode, rust_squared::RsqError> {
    // Update logic
    Ok(StatusCode::OK)
}

async fn sse_stream() -> Result<rust_squared::Response, rust_squared::RsqError> {
    let stream = stream::unfold(0, |i| async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Some((rust_squared::response::sse::Event::default().data(format!("tick {i}")), i + 1))
    });

    Ok(rust_squared::response::sse(stream))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = RsqApp::new()
        .with_docs()
        .middleware(LoggingMiddleware::new())
        .middleware(CorsMiddleware::permissive())
        .middleware(BearerAuthMiddleware::new("secret"))
        .state(Arc::new("prefix".to_string()))
        .unwrap()
        .schema::<User>()
        .get("/users/{id}", get_user)
        .unwrap()
        .post("/users/{id}", update_user)
        .unwrap()
        .get("/sse", sse_stream)
        .unwrap();

    let layered = tower_http::compression::CompressionLayer::new().layer(app);
layered.serve("0.0.0.0:3000".parse()?).await?;

    Ok(())
}