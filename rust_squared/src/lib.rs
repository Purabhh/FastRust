extern crate self as rust_squared;

pub mod app;
pub mod error;
pub mod extract;
pub mod middleware;
pub mod openapi;
pub mod request;
pub mod response;
pub mod router;
pub mod multipart;
pub mod sanitize;
pub mod schema;
pub mod sse;
pub mod ws;
pub mod state;

pub use http::Method;
pub use serde_json;
pub use app::RsqApp;
pub use error::RsqError;
pub use extract::{FromRequest, Handler, Json, Path, Query, State};
pub use middleware::{
    BearerAuthMiddleware, CompressionMiddleware, CorsMiddleware, CsrfMiddleware,
    LoggingMiddleware, MaxBodySizeMiddleware, Next, RateLimitMiddleware,
    RequestIdMiddleware, RequestValidationMiddleware, RsqMiddleware,
    SecurityHeadersMiddleware, TimeoutMiddleware,
};
pub use sanitize::{html_escape, is_safe_header_value, strip_null_bytes};
pub use openapi::{build_spec, openapi_response, swagger_ui_response};
pub use request::{RequestContext, RsqRequestBody};
pub use response::{Html, IntoResponse, Response, RsqBody};
pub use router::{MethodNotAllowed, Route, RouteMeta, Router};
pub use schema::RsqSchema;
pub use state::AppState;
pub use rust_squared_macros::{RsqSchema, delete, get, head, options, patch, post, put};

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
