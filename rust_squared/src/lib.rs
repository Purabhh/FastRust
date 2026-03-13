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
pub use multipart::{Multipart, Part};
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

use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use tokio::net::TcpListener;

pub async fn serve<S>(service: S, addr: SocketAddr) -> Result<(), RsqError>
where
    S: tower::Service<hyper::Request<Incoming>, Response = Response, Error = RsqError> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    let listener = TcpListener::bind(addr).await
        .map_err(|e| RsqError::internal(format!("failed to bind: {e}")))?;

    loop {
        let (stream, _) = listener.accept().await
            .map_err(|e| RsqError::internal(format!("failed to accept: {e}")))?;
        let service = service.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            if let Err(e) = AutoBuilder::new(TokioExecutor::new()).serve_connection(io, service).await {
                tracing::error!("connection error: {e}");
            }
        });
    }
}

use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use tokio::net::TcpListener;

pub async fn serve<S>(service: S, addr: SocketAddr) -> Result<(), RsqError>
where
    S: Service<Request<Incoming>, Response = Response, Error = RsqError> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    let listener = TcpListener::bind(addr).await
        .map_err(|e| RsqError::internal(format!("failed to bind: {e}")))?;

    loop {
        let (stream, _) = listener.accept().await
            .map_err(|e| RsqError::internal(format!("failed to accept: {e}")))?;
        let service = service.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            if let Err(e) = AutoBuilder::new(TokioExecutor::new()).serve_connection(io, service).await {
                tracing::error!("connection error: {e}");
            }
        });
    }
}
