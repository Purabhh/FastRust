use std::sync::Arc;

use async_trait::async_trait;
use http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    AUTHORIZATION,
};
use http::{HeaderValue, Method, StatusCode};

use crate::error::RsqError;
use crate::request::RequestContext;
use crate::response::{IntoResponse, Response};
use crate::router::Route;

#[async_trait]
pub trait RsqMiddleware: Send + Sync + 'static {
    async fn handle(&self, ctx: RequestContext, next: Next) -> Result<Response, RsqError>;
}

#[derive(Clone)]
pub struct Next {
    middlewares: Arc<Vec<Arc<dyn RsqMiddleware>>>,
    index: usize,
    route: Route,
}

impl Next {
    pub(crate) fn new(middlewares: Arc<Vec<Arc<dyn RsqMiddleware>>>, route: Route) -> Self {
        Self {
            middlewares,
            index: 0,
            route,
        }
    }

    pub async fn run(self, ctx: RequestContext) -> Result<Response, RsqError> {
        if let Some(middleware) = self.middlewares.get(self.index) {
            let next = Self {
                middlewares: Arc::clone(&self.middlewares),
                index: self.index + 1,
                route: self.route.clone(),
            };
            middleware.handle(ctx, next).await
        } else {
            self.route.call(ctx).await
        }
    }
}

#[derive(Clone, Debug)]
pub struct CorsMiddleware {
    allow_origin: HeaderValue,
    allow_methods: HeaderValue,
    allow_headers: HeaderValue,
}

impl CorsMiddleware {
    pub fn permissive() -> Self {
        Self {
            allow_origin: HeaderValue::from_static("*"),
            allow_methods: HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
            allow_headers: HeaderValue::from_static("*"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BearerAuthMiddleware {
    expected_token: String,
}

impl BearerAuthMiddleware {
    pub fn new(expected_token: impl Into<String>) -> Self {
        Self {
            expected_token: expected_token.into(),
        }
    }
}

#[async_trait]
impl RsqMiddleware for BearerAuthMiddleware {
    async fn handle(&self, ctx: RequestContext, next: Next) -> Result<Response, RsqError> {
        let authorized = ctx
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(|value| value == format!("Bearer {}", self.expected_token))
            .unwrap_or(false);

        if !authorized {
            return Ok((StatusCode::UNAUTHORIZED, "unauthorized").into_response());
        }

        next.run(ctx).await
    }
}

#[derive(Clone, Debug, Default)]
pub struct LoggingMiddleware;

impl LoggingMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RsqMiddleware for LoggingMiddleware {
    async fn handle(&self, ctx: RequestContext, next: Next) -> Result<Response, RsqError> {
        tracing::info!(method = %ctx.method(), path = %ctx.uri().path(), "request received");
        let mut response = next.run(ctx).await?;
        response.headers_mut().insert(
            http::header::HeaderName::from_static("x-fastrust-logged"),
            HeaderValue::from_static("true"),
        );
        Ok(response)
    }
}

#[async_trait]
impl RsqMiddleware for CorsMiddleware {
    async fn handle(&self, ctx: RequestContext, next: Next) -> Result<Response, RsqError> {
        if ctx.method() == Method::OPTIONS {
            let mut response = StatusCode::NO_CONTENT.into_response();
            apply_cors_headers(
                &mut response,
                &self.allow_origin,
                &self.allow_methods,
                &self.allow_headers,
            );
            return Ok(response);
        }

        let mut response = next.run(ctx).await?;
        apply_cors_headers(
            &mut response,
            &self.allow_origin,
            &self.allow_methods,
            &self.allow_headers,
        );
        Ok(response)
    }
}

fn apply_cors_headers(
    response: &mut Response,
    allow_origin: &HeaderValue,
    allow_methods: &HeaderValue,
    allow_headers: &HeaderValue,
) {
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, allow_origin.clone());
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_METHODS, allow_methods.clone());
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_HEADERS, allow_headers.clone());
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use bytes::Bytes;
    use http::{Method, Request, StatusCode};
    use http::header::AUTHORIZATION;
    use http_body_util::Full;
    use tokio::sync::Mutex;

    use super::{BearerAuthMiddleware, CorsMiddleware, LoggingMiddleware, Next, RsqMiddleware};
    use crate::RsqApp;
    use crate::request::{RequestContext, RsqRequestBody};
    use crate::router::Route;
    use crate::state::AppState;

    #[derive(Clone)]
    struct RecordingMiddleware {
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl RsqMiddleware for RecordingMiddleware {
        async fn handle(&self, ctx: RequestContext, next: Next) -> Result<crate::Response, crate::RsqError> {
            self.events.lock().await.push("before");
            let response = next.run(ctx).await?;
            self.events.lock().await.push("after");
            Ok(response)
        }
    }

    #[tokio::test]
    async fn middleware_runs_around_handler() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let middleware = RecordingMiddleware {
            events: Arc::clone(&events),
        };

        let app = RsqApp::new()
            .middleware(middleware)
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(*events.lock().await, vec!["before", "after"]);
    }

    #[tokio::test]
    async fn cors_middleware_adds_headers() {
        let app = RsqApp::new()
            .middleware(CorsMiddleware::permissive())
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;

        assert_eq!(response.headers()["access-control-allow-origin"], "*");
    }

    #[tokio::test]
    async fn cors_preflight_short_circuits() {
        let middleware = CorsMiddleware::permissive();
        let route = Route::new(Method::GET, "/", |_| async { Ok("ok") });
        let next = Next::new(Arc::new(Vec::new()), route);
        let request = Request::builder()
            .method(Method::OPTIONS)
            .uri("/")
            .body(())
            .unwrap();
        let ctx = RequestContext::new(
            request.into_parts().0,
            RsqRequestBody::Buffered(Bytes::new()),
            Default::default(),
            AppState::new(),
        );

        let response = middleware.handle(ctx, next).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn bearer_auth_rejects_missing_token() {
        let app = RsqApp::new()
            .middleware(BearerAuthMiddleware::new("secret"))
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn bearer_auth_allows_valid_token() {
        let app = RsqApp::new()
            .middleware(BearerAuthMiddleware::new("secret"))
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .uri("/")
                    .header(AUTHORIZATION, "Bearer secret")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn logging_middleware_marks_response() {
        let app = RsqApp::new()
            .middleware(LoggingMiddleware::new())
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;

        assert_eq!(response.headers()["x-fastrust-logged"], "true");
    }
}
