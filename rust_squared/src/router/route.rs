use std::sync::Arc;

use futures_util::future::BoxFuture;
use http::Method;

use crate::error::RsqError;
use crate::request::RequestContext;
use crate::response::{IntoResponse, Response};

pub type BoxedHandler =
    Arc<dyn Fn(RequestContext) -> BoxFuture<'static, Result<Response, RsqError>> + Send + Sync>;

#[derive(Clone)]
pub struct Route {
    method: Method,
    pattern: String,
    handler: BoxedHandler,
}

impl Route {
    pub fn new<F, Fut, R>(method: Method, pattern: impl Into<String>, handler: F) -> Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<R, RsqError>> + Send + 'static,
        R: IntoResponse + 'static,
    {
        Self {
            method,
            pattern: pattern.into(),
            handler: Arc::new(move |ctx| {
                let fut = handler(ctx);
                Box::pin(async move { fut.await.map(IntoResponse::into_response) })
            }),
        }
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub async fn call(&self, ctx: RequestContext) -> Result<Response, RsqError> {
        (self.handler)(ctx).await
    }
}
