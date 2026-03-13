use rust_squared::{Json, Path, RsqApp, RsqSchema, get};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, RsqSchema)]
struct UserPath {
    id: u64,
}

#[derive(Serialize, RsqSchema)]
struct User {
    id: u64,
}

#[get(
    "/users/{id}",
    summary = "Fetch a user",
    description = "Returns one user from the API by id.",
    operation_id = "getUser",
    tag = "Users",
    response = "User"
)]
async fn get_user(Path(path): Path<UserPath>) -> Result<Json<User>, rust_squared::RsqError> {
    Ok(Json(User { id: path.id }))
}

#[tokio::main]
async fn main() {
    let _app = RsqApp::new()
        .with_docs()
        .schema::<User>()
        .schema::<UserPath>()
        .route(get_user_route())
        .expect("example route should register");
}
