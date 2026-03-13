use std::sync::Arc;

use futures_util::future::BoxFuture;
use http::Method;

use crate::error::RsqError;
use crate::request::RequestContext;
use crate::response::{IntoResponse, Response};

pub type BoxedHandler =
    Arc<dyn Fn(RequestContext) -> BoxFuture<'static, Result<Response, RsqError>> + Send + Sync>;

#[derive(Clone, Debug, Default)]
pub struct RouteMeta {
    summary: Option<String>,
    description: Option<String>,
    operation_id: Option<String>,
    tags: Vec<String>,
    request_body_schema: Option<String>,
    response_schema: Option<String>,
}

impl RouteMeta {
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn operation_id(&self) -> Option<&str> {
        self.operation_id.as_deref()
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn request_body_schema(&self) -> Option<&str> {
        self.request_body_schema.as_deref()
    }

    pub fn response_schema(&self) -> Option<&str> {
        self.response_schema.as_deref()
    }

    pub fn set_summary(&mut self, summary: impl Into<String>) {
        self.summary = Some(summary.into());
    }

    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = Some(description.into());
    }

    pub fn set_operation_id(&mut self, operation_id: impl Into<String>) {
        self.operation_id = Some(operation_id.into());
    }

    pub fn add_tag(&mut self, tag: impl Into<String>) {
        self.tags.push(tag.into());
    }

    pub fn set_request_body_schema(&mut self, schema_name: impl Into<String>) {
        self.request_body_schema = Some(schema_name.into());
    }

    pub fn set_response_schema(&mut self, schema_name: impl Into<String>) {
        self.response_schema = Some(schema_name.into());
    }
}

#[derive(Clone)]
pub struct Route {
    method: Method,
    pattern: String,
    handler: BoxedHandler,
    meta: RouteMeta,
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
            meta: RouteMeta::default(),
        }
    }

    pub fn with_meta(mut self, meta: RouteMeta) -> Self {
        self.meta = meta;
        self
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn meta(&self) -> &RouteMeta {
        &self.meta
    }

    pub async fn call(&self, ctx: RequestContext) -> Result<Response, RsqError> {
        (self.handler)(ctx).await
    }
}
