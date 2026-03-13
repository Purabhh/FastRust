use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use futures_util::future::BoxFuture;
use http_body_util::BodyExt;
use http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE,
};
use http::{HeaderValue, Method, StatusCode};

use crate::error::RsqError;
use crate::request::RequestContext;
use crate::response::{IntoResponse, Response};
use crate::router::Route;

/// Middleware trait for the onion-model request pipeline.
///
/// Uses explicit `BoxFuture` instead of `async_trait` to keep dyn-safety
/// (`Arc<dyn RsqMiddleware>`) while avoiding the proc-macro dependency.
///
/// Implement with `Box::pin(async move { ... })`:
/// ```ignore
/// impl RsqMiddleware for MyMiddleware {
///     fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
///         Box::pin(async move {
///             // pre-processing
///             let response = next.run(ctx).await?;
///             // post-processing
///             Ok(response)
///         })
///     }
/// }
/// ```
pub trait RsqMiddleware: Send + Sync + 'static {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>>;
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

impl RsqMiddleware for BearerAuthMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            let authorized = ctx
                .headers()
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(|value| {
                    value.starts_with("Bearer ")
                        && value[7..] == self.expected_token
                })
                .unwrap_or(false);

            if !authorized {
                return Ok((StatusCode::UNAUTHORIZED, "unauthorized").into_response());
            }

            next.run(ctx).await
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct LoggingMiddleware;

impl LoggingMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl RsqMiddleware for LoggingMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            tracing::info!(method = %ctx.method(), path = %ctx.uri().path(), "request received");
            let mut response = next.run(ctx).await?;
            response.headers_mut().insert(
                http::header::HeaderName::from_static("x-fastrust-logged"),
                HeaderValue::from_static("true"),
            );
            Ok(response)
        })
    }
}

impl RsqMiddleware for CorsMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
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
        })
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

// ── Task 2.1: TimeoutMiddleware ──────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct TimeoutMiddleware {
    duration: Duration,
}

impl TimeoutMiddleware {
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }
}

impl RsqMiddleware for TimeoutMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            match tokio::time::timeout(self.duration, next.run(ctx)).await {
                Ok(result) => result,
                Err(_) => Ok((StatusCode::GATEWAY_TIMEOUT, "request timed out").into_response()),
            }
        })
    }
}

// ── Task 2.2: RequestIdMiddleware ────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct RequestIdMiddleware;

impl RequestIdMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl RsqMiddleware for RequestIdMiddleware {
    fn handle<'a>(&'a self, mut ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            let request_id = ctx
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let header_value = HeaderValue::from_str(&request_id)
                .unwrap_or_else(|_| HeaderValue::from_static("invalid"));

            ctx.headers_mut().insert(
                http::header::HeaderName::from_static("x-request-id"),
                header_value.clone(),
            );

            let mut response = next.run(ctx).await?;
            response.headers_mut().insert(
                http::header::HeaderName::from_static("x-request-id"),
                header_value,
            );
            Ok(response)
        })
    }
}

// ── Task 2.3: RateLimitMiddleware ────────────────────────────────────────────

struct TokenBucket {
    tokens: f64,
    last_refill: std::time::Instant,
    rate: f64,
    burst: f64,
}

impl TokenBucket {
    fn new(rate: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            last_refill: std::time::Instant::now(),
            rate,
            burst,
        }
    }

    fn try_consume(&mut self) -> bool {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub struct RateLimitMiddleware {
    requests_per_second: f64,
    burst_size: f64,
    buckets: Arc<std::sync::Mutex<HashMap<String, TokenBucket>>>,
}

impl RateLimitMiddleware {
    pub fn new(requests_per_second: f64, burst_size: usize) -> Self {
        Self {
            requests_per_second,
            burst_size: burst_size as f64,
            buckets: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }
}

impl std::fmt::Debug for RateLimitMiddleware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitMiddleware")
            .field("requests_per_second", &self.requests_per_second)
            .field("burst_size", &self.burst_size)
            .finish()
    }
}

impl RsqMiddleware for RateLimitMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            let key = ctx
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
                .unwrap_or_else(|| "unknown".to_string());

            let allowed = {
                let mut buckets = self.buckets.lock().expect("rate limit lock poisoned");
                let bucket = buckets
                    .entry(key)
                    .or_insert_with(|| TokenBucket::new(self.requests_per_second, self.burst_size));
                bucket.try_consume()
            };

            if !allowed {
                let mut response = (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
                response.headers_mut().insert(
                    http::header::HeaderName::from_static("retry-after"),
                    HeaderValue::from_static("1"),
                );
                return Ok(response);
            }

            next.run(ctx).await
        })
    }
}

// ── Task 2.4: CompressionMiddleware ──────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CompressionMiddleware {
    min_size: usize,
}

impl CompressionMiddleware {
    pub fn new() -> Self {
        Self { min_size: 256 }
    }

    pub fn min_size(mut self, min_size: usize) -> Self {
        self.min_size = min_size;
        self
    }
}

impl Default for CompressionMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl RsqMiddleware for CompressionMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            let accepts_gzip = ctx
                .headers()
                .get(http::header::ACCEPT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("gzip"))
                .unwrap_or(false);

            let response = next.run(ctx).await?;

            if !accepts_gzip {
                return Ok(response);
            }

            // Collect the response body to check size
            let (parts, body) = response.into_parts();
            let body_bytes = http_body_util::BodyExt::collect(body)
                .await
                .map_err(|e| RsqError::internal(format!("failed to collect response body: {e}")))?
                .to_bytes();

            if body_bytes.len() < self.min_size {
                let rebuilt = http::Response::from_parts(
                    parts,
                    http_body_util::Full::new(body_bytes).boxed(),
                );
                return Ok(rebuilt);
            }

            // Compress with gzip
            let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
            encoder
                .write_all(&body_bytes)
                .map_err(|e| RsqError::internal(format!("gzip compression failed: {e}")))?;
            let compressed = encoder
                .finish()
                .map_err(|e| RsqError::internal(format!("gzip finalize failed: {e}")))?;

            let mut rebuilt = http::Response::from_parts(
                parts,
                http_body_util::Full::new(bytes::Bytes::from(compressed)).boxed(),
            );
            rebuilt.headers_mut().insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            rebuilt.headers_mut().remove(CONTENT_LENGTH);
            Ok(rebuilt)
        })
    }
}

// ── Task 2.5: MaxBodySizeMiddleware ──────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct MaxBodySizeMiddleware {
    max_bytes: usize,
}

impl MaxBodySizeMiddleware {
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }
}

impl RsqMiddleware for MaxBodySizeMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            if let Some(content_length) = ctx.headers().get(CONTENT_LENGTH) {
                if let Ok(len_str) = content_length.to_str() {
                    if let Ok(len) = len_str.parse::<usize>() {
                        if len > self.max_bytes {
                            return Ok((
                                StatusCode::PAYLOAD_TOO_LARGE,
                                format!("payload exceeds {} byte limit", self.max_bytes),
                            )
                                .into_response());
                        }
                    }
                }
            }
            next.run(ctx).await
        })
    }
}

// ── Task 2.6: RequestValidationMiddleware ────────────────────────────────────

#[derive(Clone, Debug)]
pub struct RequestValidationMiddleware {
    allowed_content_types: Vec<String>,
}

impl RequestValidationMiddleware {
    pub fn new(allowed_content_types: Vec<String>) -> Self {
        Self {
            allowed_content_types,
        }
    }

    pub fn json_only() -> Self {
        Self {
            allowed_content_types: vec!["application/json".to_string()],
        }
    }
}

impl RsqMiddleware for RequestValidationMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            let needs_content_type = matches!(
                *ctx.method(),
                Method::POST | Method::PUT | Method::PATCH
            );

            if needs_content_type {
                let content_type = ctx
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok());

                match content_type {
                    None => {
                        return Ok((
                            StatusCode::UNSUPPORTED_MEDIA_TYPE,
                            "Content-Type header is required",
                        )
                            .into_response());
                    }
                    Some(ct) => {
                        let ct_lower = ct.to_ascii_lowercase();
                        let matched = self.allowed_content_types.iter().any(|allowed| {
                            ct_lower.starts_with(allowed)
                        });
                        if !matched {
                            return Ok((
                                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                                format!("Content-Type '{}' is not allowed", ct),
                            )
                                .into_response());
                        }
                    }
                }
            }

            next.run(ctx).await
        })
    }
}

// ── Task 4.2: SecurityHeadersMiddleware ──────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SecurityHeadersMiddleware {
    hsts: Option<HeaderValue>,
    content_type_options: Option<HeaderValue>,
    frame_options: Option<HeaderValue>,
    csp: Option<HeaderValue>,
    xss_protection: Option<HeaderValue>,
}

impl SecurityHeadersMiddleware {
    pub fn defaults() -> Self {
        Self {
            hsts: Some(HeaderValue::from_static("max-age=63072000; includeSubDomains")),
            content_type_options: Some(HeaderValue::from_static("nosniff")),
            frame_options: Some(HeaderValue::from_static("DENY")),
            csp: Some(HeaderValue::from_static("default-src 'self'")),
            xss_protection: Some(HeaderValue::from_static("0")),
        }
    }

    pub fn hsts(mut self, value: impl Into<Option<HeaderValue>>) -> Self {
        self.hsts = value.into();
        self
    }

    pub fn content_security_policy(mut self, value: impl Into<Option<HeaderValue>>) -> Self {
        self.csp = value.into();
        self
    }

    pub fn frame_options(mut self, value: impl Into<Option<HeaderValue>>) -> Self {
        self.frame_options = value.into();
        self
    }
}

impl RsqMiddleware for SecurityHeadersMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            let mut response = next.run(ctx).await?;
            let headers = response.headers_mut();
            if let Some(ref v) = self.hsts {
                headers.insert(
                    http::header::HeaderName::from_static("strict-transport-security"),
                    v.clone(),
                );
            }
            if let Some(ref v) = self.content_type_options {
                headers.insert(
                    http::header::HeaderName::from_static("x-content-type-options"),
                    v.clone(),
                );
            }
            if let Some(ref v) = self.frame_options {
                headers.insert(
                    http::header::HeaderName::from_static("x-frame-options"),
                    v.clone(),
                );
            }
            if let Some(ref v) = self.csp {
                headers.insert(
                    http::header::HeaderName::from_static("content-security-policy"),
                    v.clone(),
                );
            }
            if let Some(ref v) = self.xss_protection {
                headers.insert(
                    http::header::HeaderName::from_static("x-xss-protection"),
                    v.clone(),
                );
            }
            Ok(response)
        })
    }
}

// ── Task 4.3: CsrfMiddleware ────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CsrfMiddleware {
    cookie_name: String,
    header_name: String,
    token_length: usize,
}

impl CsrfMiddleware {
    pub fn new() -> Self {
        Self {
            cookie_name: "csrf_token".to_string(),
            header_name: "x-csrf-token".to_string(),
            token_length: 32,
        }
    }

    fn generate_token(&self) -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let bytes: Vec<u8> = (0..self.token_length).map(|_| rng.r#gen::<u8>()).collect();
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn extract_cookie_value<'a>(&self, cookie_header: &'a str) -> Option<&'a str> {
        for pair in cookie_header.split(';') {
            let pair = pair.trim();
            if let Some(value) = pair.strip_prefix(&self.cookie_name) {
                let value = value.trim_start();
                if let Some(value) = value.strip_prefix('=') {
                    return Some(value.trim());
                }
            }
        }
        None
    }
}

impl Default for CsrfMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl RsqMiddleware for CsrfMiddleware {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
            let is_safe = matches!(
                *ctx.method(),
                Method::GET | Method::HEAD | Method::OPTIONS
            );

            if !is_safe {
                let cookie_token = ctx
                    .headers()
                    .get(http::header::COOKIE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|cookie_str| self.extract_cookie_value(cookie_str))
                    .map(String::from);

                let header_token = ctx
                    .headers()
                    .get(&self.header_name)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);

                match (cookie_token, header_token) {
                    (Some(cookie), Some(header)) if cookie == header => {}
                    _ => {
                        return Ok((StatusCode::FORBIDDEN, "CSRF token mismatch").into_response());
                    }
                }
            }

            let mut response = next.run(ctx).await?;
            let token = self.generate_token();
            let cookie_value = format!(
                "{}={}; Path=/; SameSite=Strict; HttpOnly",
                self.cookie_name, token
            );
            if let Ok(v) = HeaderValue::from_str(&cookie_value) {
                response.headers_mut().insert(http::header::SET_COOKIE, v);
            }
            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use futures_util::future::BoxFuture;
    use http::{Method, Request, StatusCode};
    use http::header::AUTHORIZATION;
    use http_body_util::Full;
    use tokio::sync::Mutex;

    use std::time::Duration;

    use super::*;
    use crate::RsqApp;
    use crate::request::{RequestContext, RsqRequestBody};
    use crate::router::Route;
    use crate::state::AppState;

    #[derive(Clone)]
    struct RecordingMiddleware {
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RsqMiddleware for RecordingMiddleware {
        fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<crate::Response, crate::RsqError>> {
            Box::pin(async move {
                self.events.lock().await.push("before");
                let response = next.run(ctx).await?;
                self.events.lock().await.push("after");
                Ok(response)
            })
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

    // ── Timeout tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn timeout_allows_fast_requests() {
        let app = RsqApp::new()
            .middleware(TimeoutMiddleware::new(Duration::from_secs(5)))
            .route(Route::new(Method::GET, "/", |_| async { Ok("fast") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn timeout_returns_504_on_slow_handler() {
        let app = RsqApp::new()
            .middleware(TimeoutMiddleware::new(Duration::from_millis(10)))
            .route(Route::new(Method::GET, "/slow", |_| async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                Ok::<_, crate::RsqError>("slow")
            }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/slow").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    // ── RequestId tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn request_id_generates_uuid() {
        let app = RsqApp::new()
            .middleware(RequestIdMiddleware::new())
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        let id = response.headers().get("x-request-id").expect("should have x-request-id");
        assert!(!id.is_empty());
        // UUID v4 format: 8-4-4-4-12 hex chars
        assert_eq!(id.to_str().unwrap().len(), 36);
    }

    #[tokio::test]
    async fn request_id_preserves_existing() {
        let app = RsqApp::new()
            .middleware(RequestIdMiddleware::new())
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .uri("/")
                    .header("x-request-id", "my-custom-id")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.headers()["x-request-id"], "my-custom-id");
    }

    // ── RateLimit tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn rate_limit_allows_within_burst() {
        let app = RsqApp::new()
            .middleware(RateLimitMiddleware::new(10.0, 5))
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        for _ in 0..5 {
            let response = app
                .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
                .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn rate_limit_returns_429_when_exceeded() {
        let app = RsqApp::new()
            .middleware(RateLimitMiddleware::new(1.0, 2))
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        // Use up the burst
        let _ = app.handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap()).await;
        let _ = app.handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap()).await;

        // Third request should be rate-limited
        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().contains_key("retry-after"));
    }

    // ── Compression tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn compression_skips_small_responses() {
        let app = RsqApp::new()
            .middleware(CompressionMiddleware::new())
            .route(Route::new(Method::GET, "/", |_| async { Ok("small") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .uri("/")
                    .header("accept-encoding", "gzip")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert!(!response.headers().contains_key("content-encoding"));
    }

    #[tokio::test]
    async fn compression_gzips_large_responses() {
        let large_body = "x".repeat(1000);
        let app = RsqApp::new()
            .middleware(CompressionMiddleware::new())
            .route(Route::new(Method::GET, "/big", move |_| {
                let body = large_body.clone();
                async move { Ok::<_, crate::RsqError>(body) }
            }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .uri("/big")
                    .header("accept-encoding", "gzip")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.headers()["content-encoding"], "gzip");
    }

    #[tokio::test]
    async fn compression_skips_without_accept_encoding() {
        let large_body = "x".repeat(1000);
        let app = RsqApp::new()
            .middleware(CompressionMiddleware::new())
            .route(Route::new(Method::GET, "/big", move |_| {
                let body = large_body.clone();
                async move { Ok::<_, crate::RsqError>(body) }
            }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/big").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert!(!response.headers().contains_key("content-encoding"));
    }

    // ── MaxBodySize tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn max_body_size_allows_small_payload() {
        let app = RsqApp::new()
            .middleware(MaxBodySizeMiddleware::new(1024))
            .route(Route::new(Method::POST, "/upload", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/upload")
                    .header("content-length", "100")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn max_body_size_rejects_large_payload() {
        let app = RsqApp::new()
            .middleware(MaxBodySizeMiddleware::new(1024))
            .route(Route::new(Method::POST, "/upload", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/upload")
                    .header("content-length", "999999")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    // ── RequestValidation tests ──────────────────────────────────────────

    #[tokio::test]
    async fn validation_allows_get_without_content_type() {
        let app = RsqApp::new()
            .middleware(RequestValidationMiddleware::json_only())
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn validation_rejects_post_without_content_type() {
        let app = RsqApp::new()
            .middleware(RequestValidationMiddleware::json_only())
            .route(Route::new(Method::POST, "/data", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/data")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn validation_allows_post_with_valid_content_type() {
        let app = RsqApp::new()
            .middleware(RequestValidationMiddleware::json_only())
            .route(Route::new(Method::POST, "/data", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/data")
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn validation_rejects_post_with_wrong_content_type() {
        let app = RsqApp::new()
            .middleware(RequestValidationMiddleware::json_only())
            .route(Route::new(Method::POST, "/data", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/data")
                    .header("content-type", "text/plain")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    // ── SecurityHeaders tests ───────────────────────────────────────────

    #[tokio::test]
    async fn security_headers_defaults_applied() {
        let app = RsqApp::new()
            .middleware(SecurityHeadersMiddleware::defaults())
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();
        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        assert_eq!(response.headers()["content-security-policy"], "default-src 'self'");
        assert_eq!(response.headers()["x-xss-protection"], "0");
    }

    #[tokio::test]
    async fn security_headers_can_be_customized() {
        let mw = SecurityHeadersMiddleware::defaults()
            .frame_options(None);
        let app = RsqApp::new()
            .middleware(mw)
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();
        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert!(!response.headers().contains_key("x-frame-options"));
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }

    // ── CSRF tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn csrf_allows_get_without_token() {
        let app = RsqApp::new()
            .middleware(CsrfMiddleware::new())
            .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
            .unwrap();
        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("set-cookie"));
    }

    #[tokio::test]
    async fn csrf_rejects_post_without_token() {
        let app = RsqApp::new()
            .middleware(CsrfMiddleware::new())
            .route(Route::new(Method::POST, "/submit", |_| async { Ok("ok") }))
            .unwrap();
        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/submit")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn csrf_allows_post_with_matching_token() {
        let app = RsqApp::new()
            .middleware(CsrfMiddleware::new())
            .route(Route::new(Method::POST, "/submit", |_| async { Ok("ok") }))
            .unwrap();
        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/submit")
                    .header("cookie", "csrf_token=abc123")
                    .header("x-csrf-token", "abc123")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn csrf_rejects_post_with_mismatched_token() {
        let app = RsqApp::new()
            .middleware(CsrfMiddleware::new())
            .route(Route::new(Method::POST, "/submit", |_| async { Ok("ok") }))
            .unwrap();
        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/submit")
                    .header("cookie", "csrf_token=abc")
                    .header("x-csrf-token", "xyz")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

use tower::Service as TowerService;
use hyper::body::Incoming;
use hyper::Request as HyperRequest;

#[derive(Clone)]
struct TowerMiddleware<T> {
inner: T,
}

impl<T> RsqMiddleware for TowerMiddleware<T>
where
    T: TowerService<HyperRequest<Incoming>, Response = Response, Error = Box<dyn std::error::Error + Send + Sync>> + Clone + Send + Sync + 'static,
    T::Future: Send + 'static,
{
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>> {
        Box::pin(async move {
let mut request = ctx.into_hyper_request();
let response = self.inner.clone().call(request).await.map_err(|e| RsqError::internal(e.to_string()))?;
next.run(ctx).await // Wait, no—tower middlewares wrap the service, so for chaining, need to call next inside, but since Rsq is the inner, this adapter should call the tower on the request, then pass to next if ok.
// Actually, to wrap, the adapter should call tower with a service that calls next.
// But since RsqMiddleware is not Tower, it's better to have RsqApp impl Tower Service, then users can layer Tower on top.
// For using Tower in Rsq middleware chain, the adapter can convert to hyper, call tower, convert back to Rsq Response.
// But if tower is a layer wrapping a service, it's more complex.
// To fully support, make RsqApp impl Tower Service, and provide a way to add Tower Layers to RsqApp.
// Let's do that.
        })
    }
}

use tower::Service as TowerService;

#[derive(Clone)]
struct TowerMiddleware<T> {
    inner: T,
}

#[async_trait]
impl<T> RsqMiddleware for TowerMiddleware<T>
where
    T: TowerService<Request<Incoming>, Response = Response, Error = Box<dyn std::error::Error + Send + Sync>> + Clone + Send + Sync + 'static,
    T::Future: Send + 'static,
{
    async fn handle(&self, ctx: RequestContext, next: Next) -> Result<Response, RsqError> {
        let mut request = ctx.into_hyper_request();
        let response = self.inner.clone().call(request).await.map_err(|e| RsqError::internal(e.to_string()))?;
        next.run(ctx).await // Wait, no—tower middlewares wrap the service, so for chaining, need to call next inside, but since Rsq is the inner, this adapter should call the tower on the request, then pass to next if ok.
        // Actually, to wrap, the adapter should call tower with a service that calls next.
        // But since RsqMiddleware is not Tower, it's better to have RsqApp impl Tower Service, then users can layer Tower on top.
        // For using Tower in Rsq middleware chain, the adapter can convert to hyper, call tower, convert back to Rsq Response.
        // But if tower is a layer wrapping a service, it's more complex.
        // To fully support, make RsqApp impl Tower Service, and provide a way to add Tower Layers to RsqApp.
        // Let's do that.
    }
}
