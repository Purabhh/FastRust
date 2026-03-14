use std::sync::Arc;

use dashmap::DashMap;
use std::time::Duration;

#[cfg(feature = "compression")]
use std::io::Write;
#[cfg(feature = "compression")]
use flate2::Compression;
#[cfg(feature = "compression")]
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
///     fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>>;
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
    fn handle(&self, mut ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
    buckets: Arc<DashMap<String, TokenBucket>>,
}

impl RateLimitMiddleware {
    pub fn new(requests_per_second: f64, burst_size: usize) -> Self {
        Self {
            requests_per_second,
            burst_size: burst_size as f64,
            buckets: Arc::new(DashMap::new()),
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
        Box::pin(async move {
            let key = ctx
                .peer_addr()
                .map(|addr| addr.ip().to_string())
                .or_else(|| {
                    ctx.headers()
                        .get("x-forwarded-for")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from)
                })
                .unwrap_or_else(|| "0.0.0.0".to_string());

            let allowed = {
                let mut bucket = self.buckets
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

#[cfg(feature = "compression")]
#[derive(Clone, Debug)]
pub struct CompressionMiddleware {
    min_size: usize,
}

#[cfg(feature = "compression")]
impl CompressionMiddleware {
    pub fn new() -> Self {
        Self { min_size: 256 }
    }

    pub fn min_size(mut self, min_size: usize) -> Self {
        self.min_size = min_size;
        self
    }
}

#[cfg(feature = "compression")]
impl Default for CompressionMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "compression")]
impl RsqMiddleware for CompressionMiddleware {
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
        let max_bytes = self.max_bytes;
        Box::pin(async move {
            if let Some(content_length) = ctx.headers().get(CONTENT_LENGTH) {
                if let Ok(len_str) = content_length.to_str() {
                    if let Ok(len) = len_str.parse::<usize>() {
                        if len > max_bytes {
                            return Ok((
                                StatusCode::PAYLOAD_TOO_LARGE,
                                format!("payload exceeds {} byte limit", max_bytes),
                            )
                                .into_response());
                        }
                    }
                }
            }
            let mut ctx = ctx;
            ctx.set_max_body_size(max_bytes);
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
                            ct_lower == *allowed || ct_lower.starts_with(&format!("{allowed};"))
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        // Stack-allocate up to 64 bytes; fall back to heap for unusual lengths.
        // Default token_length is 32, so this is always stack-resident (F7).
        let mut buf = smallvec::SmallVec::<[u8; 64]>::from_elem(0u8, self.token_length);
        rng.fill_bytes(&mut buf);
        // Pre-allocate the exact output capacity — one allocation total.
        let mut out = String::with_capacity(self.token_length * 2);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for &b in buf.iter() {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
        out
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
    fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<Response, RsqError>> {
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
            if is_safe {
                let token = self.generate_token();
                let cookie_value = format!(
                    "{}={}; Path=/; SameSite=Strict; Secure",
                    self.cookie_name, token
                );
                if let Ok(v) = HeaderValue::from_str(&cookie_value) {
                    response.headers_mut().insert(http::header::SET_COOKIE, v);
                }
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
        fn handle(&self, ctx: RequestContext, next: Next) -> BoxFuture<'_, Result<crate::Response, crate::RsqError>> {
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
            None,
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

    // ── A1 regression: chunked/no-Content-Length body exceeding limit ────

    #[tokio::test]
    async fn max_body_size_rejects_large_body_without_content_length() {
        // Handler calls take_body_bytes() to trigger the size check.
        let app = RsqApp::new()
            .middleware(MaxBodySizeMiddleware::new(10))
            .route(Route::new(Method::POST, "/upload", |mut ctx: RequestContext| async move {
                ctx.take_body_bytes().await?;
                Ok("ok")
            }))
            .unwrap();

        // Send 11 bytes with no Content-Length header — only streaming check enforces limit.
        let big_body = Bytes::from(vec![b'x'; 11]);
        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/upload")
                    .body(Full::new(big_body))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    // ── A2 regression: CSRF cookie flags ────────────────────────────────

    #[tokio::test]
    async fn csrf_cookie_has_secure_not_httponly_and_only_on_get() {
        let app = RsqApp::new()
            .middleware(CsrfMiddleware::new())
            .route(Route::new(Method::GET, "/", |_| async { Ok::<_, crate::RsqError>("ok") }))
            .unwrap()
            .route(Route::new(Method::POST, "/submit", |_| async { Ok::<_, crate::RsqError>("ok") }))
            .unwrap();

        // GET: cookie should be set, contain Secure, not contain HttpOnly.
        let get_response: Response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;
        let cookie = get_response
            .headers()
            .get("set-cookie")
            .expect("cookie must be set on GET")
            .to_str()
            .unwrap();
        assert!(cookie.contains("Secure"), "cookie must contain Secure");
        assert!(!cookie.contains("HttpOnly"), "cookie must NOT contain HttpOnly");

        // POST: cookie must NOT be set (double-submit flow; JS already holds it).
        let post_response: Response = app
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
        assert!(
            !post_response.headers().contains_key("set-cookie"),
            "cookie must NOT be set on POST"
        );
    }

    // ── A3 regression: rate limiter key prefers peer_addr ───────────────

    #[tokio::test]
    async fn rate_limit_uses_peer_addr_not_x_forwarded_for() {
        use std::net::SocketAddr;

        let mw = RateLimitMiddleware::new(10.0, 2);
        let route = Route::new(Method::GET, "/", |_| async { Ok::<_, crate::RsqError>("ok") });

        let make_ctx = |peer: Option<SocketAddr>, xff: &'static str| {
            let request = Request::builder()
                .method(Method::GET)
                .uri("/")
                .header("x-forwarded-for", xff)
                .body(())
                .unwrap();
            let (parts, _) = request.into_parts();
            RequestContext::new(
                parts,
                RsqRequestBody::Buffered(Bytes::new()),
                Default::default(),
                AppState::new(),
                peer,
            )
        };

        let peer_a: SocketAddr = "1.2.3.4:1234".parse().unwrap();
        let peer_b: SocketAddr = "5.6.7.8:5678".parse().unwrap();

        // Exhaust peer_a's burst (size 2).
        let r1 = mw.handle(make_ctx(Some(peer_a), "9.9.9.9"), Next::new(Arc::new(Vec::new()), route.clone())).await.unwrap();
        let r2 = mw.handle(make_ctx(Some(peer_a), "9.9.9.9"), Next::new(Arc::new(Vec::new()), route.clone())).await.unwrap();
        let r3 = mw.handle(make_ctx(Some(peer_a), "9.9.9.9"), Next::new(Arc::new(Vec::new()), route.clone())).await.unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        assert_eq!(r2.status(), StatusCode::OK);
        // Third request from peer_a must be rate-limited.
        assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);

        // peer_b (different IP, same XFF) should still be allowed.
        let r4 = mw.handle(make_ctx(Some(peer_b), "9.9.9.9"), Next::new(Arc::new(Vec::new()), route.clone())).await.unwrap();
        assert_eq!(r4.status(), StatusCode::OK);
    }

    // ── G3: concurrent stress test — no panics/deadlocks under load ─────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn rate_limit_concurrent_no_panic_or_deadlock() {
        use std::net::SocketAddr;

        // High rate/burst so requests are unlikely to be rate-limited; the goal is to
        // verify that 20 concurrent tasks driving the same DashMap instance neither
        // panic nor deadlock.
        let mw = Arc::new(RateLimitMiddleware::new(1000.0, 1000));
        let route = Route::new(Method::GET, "/", |_| async { Ok::<_, crate::RsqError>("ok") });

        let handles: Vec<_> = (0..20u8)
            .map(|i| {
                let mw = Arc::clone(&mw);
                let route = route.clone();
                tokio::spawn(async move {
                    let peer: SocketAddr = format!("10.0.0.{}:8080", i % 5).parse().unwrap();
                    let request = Request::builder()
                        .method(Method::GET)
                        .uri("/")
                        .body(())
                        .unwrap();
                    let (parts, _) = request.into_parts();
                    let ctx = RequestContext::new(
                        parts,
                        RsqRequestBody::Buffered(Bytes::new()),
                        Default::default(),
                        AppState::new(),
                        Some(peer),
                    );
                    mw.handle(ctx, Next::new(Arc::new(Vec::new()), route))
                        .await
                        .expect("handle must not return an error")
                        .status()
                })
            })
            .collect();

        for handle in handles {
            // join() implicitly checks for panics — a panic in any task propagates here.
            let status = handle.await.expect("task must not panic");
            // All requests should be allowed given the generous burst.
            assert_eq!(status, StatusCode::OK);
        }
    }

    // ── A4 regression: application/jsonp must be rejected ───────────────

    #[tokio::test]
    async fn validation_rejects_application_jsonp() {
        let app = RsqApp::new()
            .middleware(RequestValidationMiddleware::json_only())
            .route(Route::new(Method::POST, "/data", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/data")
                    .header("content-type", "application/jsonp")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn validation_allows_application_json_with_charset_param() {
        let app = RsqApp::new()
            .middleware(RequestValidationMiddleware::json_only())
            .route(Route::new(Method::POST, "/data", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/data")
                    .header("content-type", "application/json; charset=utf-8")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    // ── E2 regression: configured limit below 1 MiB honored during Json extraction ──

    #[tokio::test]
    async fn max_body_size_below_default_enforced_for_json_extraction() {

        // Allow only 32 bytes — well below the 1 MiB default.
        let app = RsqApp::new()
            .middleware(MaxBodySizeMiddleware::new(32))
            .route(Route::new(
                Method::POST,
                "/json",
                |mut ctx: RequestContext| async move {
                    // Trigger body reading via the Json extractor path.
                    ctx.take_body_bytes().await?;
                    Ok("ok")
                },
            ))
            .unwrap();

        // 33-byte body exceeds the 32-byte configured limit.
        let over_limit = Bytes::from(vec![b'a'; 33]);
        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/json")
                    .header("content-type", "application/json")
                    .body(Full::new(over_limit))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "body exceeding configured sub-1MiB limit must be rejected"
        );

        // A body within the limit must succeed.
        let within_limit = Bytes::from(vec![b'b'; 10]);
        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/json")
                    .header("content-type", "application/json")
                    .body(Full::new(within_limit))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "body within configured limit must be accepted"
        );
    }
}


