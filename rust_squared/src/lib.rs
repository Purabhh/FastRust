pub mod app;
pub mod error;
pub mod extract;
pub mod middleware;
pub mod openapi;
pub mod request;
pub mod response;
pub mod router;
pub mod state;

pub use http::Method;
pub use app::RsqApp;
pub use error::RsqError;
pub use extract::{FromRequest, Handler, Json, Path, Query, State};
pub use middleware::{BearerAuthMiddleware, CorsMiddleware, LoggingMiddleware, Next, RsqMiddleware};
pub use openapi::{build_spec, openapi_response, swagger_ui_response};
pub use request::{RequestContext, RsqRequestBody};
pub use response::{Html, IntoResponse, Response, RsqBody};
pub use router::{MethodNotAllowed, Route, RouteMeta, Router};
pub use state::AppState;
pub use rust_squared_macros::{get, post, RsqSchema};

pub fn route<H, Args>(method: Method, pattern: impl Into<String>, handler: H) -> Route
where
    H: Handler<Args>,
{
    handler.into_route(method, pattern.into())
}

pub fn route_with_meta<H, Args>(
    method: Method,
    pattern: impl Into<String>,
    handler: H,
    meta: RouteMeta,
) -> Route
where
    H: Handler<Args>,
{
    handler.into_route(method, pattern.into()).with_meta(meta)
}
