extern crate self as rust_squared;

pub mod app;
pub mod cookie;
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
pub mod static_files;
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
pub use cookie::{CookieJar, set_cookie, set_cookie_with};
pub use sanitize::{html_escape, is_safe_header_value, strip_null_bytes};
pub use static_files::StaticFiles;
pub use openapi::{build_spec, openapi_response, swagger_ui_response};
pub use request::{RequestContext, RsqRequestBody};
pub use multipart::Multipart;
pub use response::{Html, IntoResponse, Redirect, Response, RsqBody};
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

/// Serve any Tower `Service` as an HTTP server (supports HTTP/1 and HTTP/2).
pub async fn serve<S>(service: S, addr: std::net::SocketAddr) -> Result<(), RsqError>
where
    S: tower::Service<http::Request<hyper::body::Incoming>, Response = Response, Error = RsqError>
        + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| RsqError::internal(format!("failed to bind listener: {error}")))?;

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|error| RsqError::internal(format!("failed to accept connection: {error}")))?;
        let service = service.clone();
        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            if let Err(error) = hyper_util::server::conn::auto::Builder::new(
                hyper_util::rt::TokioExecutor::new(),
            )
            .serve_connection(io, service)
            .await
            {
                tracing::error!("connection error: {error}");
            }
        });
    }
}
