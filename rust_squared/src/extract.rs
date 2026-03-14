use std::future::Future;

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use http::header::CONTENT_TYPE;
use serde::Serialize;
use serde::de::DeserializeOwned;

use http_body_util::BodyExt;

use validator::Validate;

use crate::error::RsqError;
use crate::request::RequestContext;
use crate::response::{IntoResponse, Response};

#[async_trait]
pub trait FromRequest: Sized + Send {
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError>;
}

pub struct Path<T>(pub T);

impl<T> Path<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[async_trait]
impl<T> FromRequest for Path<T>
where
    T: DeserializeOwned + Send,
{
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let encoded = ctx
            .path_params()
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    percent_encoding::utf8_percent_encode(key, percent_encoding::NON_ALPHANUMERIC),
                    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC),
                )
            })
            .collect::<Vec<_>>()
            .join("&");

        let value = serde_urlencoded::from_str(&encoded)
            .map_err(|error| RsqError::unprocessable_entity(format!("invalid path parameters: {error}")))?;
        Ok(Self(value))
    }
}

pub struct Query<T>(pub T);

impl<T> Query<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[async_trait]
impl<T> FromRequest for Query<T>
where
    T: DeserializeOwned + Send,
{
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let raw = ctx.uri().query().unwrap_or_default();
        let value = serde_urlencoded::from_str(raw)
            .map_err(|error| RsqError::unprocessable_entity(format!("invalid query string: {error}")))?;
        Ok(Self(value))
    }
}

pub struct Json<T>(pub T);

impl<T> Json<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[async_trait]
impl<T> FromRequest for Json<T>
where
    T: DeserializeOwned + Send,
{
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let is_json = ctx
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value.eq_ignore_ascii_case("application/json")
                    || value
                        .to_ascii_lowercase()
                        .starts_with("application/json;")
            })
            .unwrap_or(false);

        if !is_json {
            return Err(RsqError::unsupported_media_type(
                "expected `Content-Type: application/json`",
            ));
        }

        let bytes = ctx.take_body_bytes().await?;
        let value = serde_json::from_slice(&bytes)
            .map_err(|error| RsqError::unprocessable_entity(format!("invalid json body: {error}")))?;
        Ok(Self(value))
    }
}

impl<T> IntoResponse for Json<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response {
        let body = serde_json::to_vec(&self.0).expect("json serialization should succeed");
        let mut response = http::Response::builder()
            .status(http::StatusCode::OK)
            .body(http_body_util::Full::new(bytes::Bytes::from(body)).boxed())
            .expect("response builder should be infallible");
        response.headers_mut().insert(
            CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        response
    }
}

pub struct State<T>(pub T);

impl<T> State<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[async_trait]
impl<T> FromRequest for State<T>
where
    T: Clone + Send + Sync + 'static,
{
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let value = ctx.state().get::<T>().ok_or_else(|| {
            RsqError::internal(format!(
                "state for `{}` is not registered",
                std::any::type_name::<T>()
            ))
        })?;
        Ok(Self(value))
    }
}

pub trait Handler<Args>: Clone + Send + Sync + 'static {
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route;
}

impl<F, Fut, Res> Handler<()> for F
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::new(method, pattern, move |_| {
            let handler = self.clone();
            async move { handler().await }
        })
    }
}

// Workaround for rust#100013: return BoxFuture directly instead of async fn
// so older Rust (1.85) can prove the 'static bound on Box::pin.
fn extract_1<F, Fut, Res, A>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        handler(a).await.map(IntoResponse::into_response)
    })
}

fn extract_2<F, Fut, Res, A, B>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A, B) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        let b = B::from_request(&mut ctx).await?;
        handler(a, b).await.map(IntoResponse::into_response)
    })
}

fn extract_3<F, Fut, Res, A, B, C>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A, B, C) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        let b = B::from_request(&mut ctx).await?;
        let c = C::from_request(&mut ctx).await?;
        handler(a, b, c).await.map(IntoResponse::into_response)
    })
}

fn extract_4<F, Fut, Res, A, B, C, D>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A, B, C, D) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        let b = B::from_request(&mut ctx).await?;
        let c = C::from_request(&mut ctx).await?;
        let d = D::from_request(&mut ctx).await?;
        handler(a, b, c, d).await.map(IntoResponse::into_response)
    })
}

fn extract_5<F, Fut, Res, A, B, C, D, E>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A, B, C, D, E) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        let b = B::from_request(&mut ctx).await?;
        let c = C::from_request(&mut ctx).await?;
        let d = D::from_request(&mut ctx).await?;
        let e = E::from_request(&mut ctx).await?;
        handler(a, b, c, d, e).await.map(IntoResponse::into_response)
    })
}

fn extract_6<F, Fut, Res, A, B, C, D, E, G>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A, B, C, D, E, G) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
    G: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        let b = B::from_request(&mut ctx).await?;
        let c = C::from_request(&mut ctx).await?;
        let d = D::from_request(&mut ctx).await?;
        let e = E::from_request(&mut ctx).await?;
        let g = G::from_request(&mut ctx).await?;
        handler(a, b, c, d, e, g).await.map(IntoResponse::into_response)
    })
}

fn extract_7<F, Fut, Res, A, B, C, D, E, G, H>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A, B, C, D, E, G, H) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
    G: FromRequest + Send + 'static,
    H: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        let b = B::from_request(&mut ctx).await?;
        let c = C::from_request(&mut ctx).await?;
        let d = D::from_request(&mut ctx).await?;
        let e = E::from_request(&mut ctx).await?;
        let g = G::from_request(&mut ctx).await?;
        let h = H::from_request(&mut ctx).await?;
        handler(a, b, c, d, e, g, h).await.map(IntoResponse::into_response)
    })
}

fn extract_8<F, Fut, Res, A, B, C, D, E, G, H, I>(handler: F, mut ctx: RequestContext) -> BoxFuture<'static, Result<Response, RsqError>>
where
    F: Fn(A, B, C, D, E, G, H, I) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
    G: FromRequest + Send + 'static,
    H: FromRequest + Send + 'static,
    I: FromRequest + Send + 'static,
{
    Box::pin(async move {
        let a = A::from_request(&mut ctx).await?;
        let b = B::from_request(&mut ctx).await?;
        let c = C::from_request(&mut ctx).await?;
        let d = D::from_request(&mut ctx).await?;
        let e = E::from_request(&mut ctx).await?;
        let g = G::from_request(&mut ctx).await?;
        let h = H::from_request(&mut ctx).await?;
        let i = I::from_request(&mut ctx).await?;
        handler(a, b, c, d, e, g, h, i).await.map(IntoResponse::into_response)
    })
}

impl<F, Fut, Res, A> Handler<(A,)> for F
where
    F: Fn(A) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_1(self.clone(), ctx)
        }))
    }
}

impl<F, Fut, Res, A, B> Handler<(A, B)> for F
where
    F: Fn(A, B) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_2(self.clone(), ctx)
        }))
    }
}

impl<F, Fut, Res, A, B, C> Handler<(A, B, C)> for F
where
    F: Fn(A, B, C) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_3(self.clone(), ctx)
        }))
    }
}

impl<F, Fut, Res, A, B, C, D> Handler<(A, B, C, D)> for F
where
    F: Fn(A, B, C, D) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_4(self.clone(), ctx)
        }))
    }
}

impl<F, Fut, Res, A, B, C, D, E> Handler<(A, B, C, D, E)> for F
where
    F: Fn(A, B, C, D, E) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_5(self.clone(), ctx)
        }))
    }
}

impl<F, Fut, Res, A, B, C, D, E, G> Handler<(A, B, C, D, E, G)> for F
where
    F: Fn(A, B, C, D, E, G) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
    G: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_6(self.clone(), ctx)
        }))
    }
}

impl<F, Fut, Res, A, B, C, D, E, G, H> Handler<(A, B, C, D, E, G, H)> for F
where
    F: Fn(A, B, C, D, E, G, H) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
    G: FromRequest + Send + 'static,
    H: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_7(self.clone(), ctx)
        }))
    }
}

impl<F, Fut, Res, A, B, C, D, E, G, H, I> Handler<(A, B, C, D, E, G, H, I)> for F
where
    F: Fn(A, B, C, D, E, G, H, I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, RsqError>> + Send + 'static,
    Res: IntoResponse + 'static,
    A: FromRequest + Send + 'static,
    B: FromRequest + Send + 'static,
    C: FromRequest + Send + 'static,
    D: FromRequest + Send + 'static,
    E: FromRequest + Send + 'static,
    G: FromRequest + Send + 'static,
    H: FromRequest + Send + 'static,
    I: FromRequest + Send + 'static,
{
    fn into_route(self, method: http::Method, pattern: String) -> crate::router::Route {
        crate::router::Route::from_boxed(method, pattern, std::sync::Arc::new(move |ctx| {
            extract_8(self.clone(), ctx)
        }))
    }
}

// ── New extractors ────────────────────────────────────────────────────────────

pub struct Headers(pub http::HeaderMap);

#[async_trait]
impl FromRequest for Headers {
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        Ok(Headers(ctx.headers().clone()))
    }
}

pub struct RawBody(pub bytes::Bytes);

#[async_trait]
impl FromRequest for RawBody {
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        Ok(RawBody(ctx.take_body_bytes().await?))
    }
}

pub struct ClientIp(pub std::net::IpAddr);

#[async_trait]
impl FromRequest for ClientIp {
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        if let Some(addr) = ctx.peer_addr() {
            return Ok(ClientIp(addr.ip()));
        }
        if let Some(val) = ctx.headers().get("x-real-ip") {
            if let Ok(s) = val.to_str() {
                if let Ok(ip) = s.trim().parse::<std::net::IpAddr>() {
                    return Ok(ClientIp(ip));
                }
            }
        }
        if let Some(val) = ctx.headers().get("x-forwarded-for") {
            if let Ok(s) = val.to_str() {
                if let Some(first) = s.split(',').next() {
                    if let Ok(ip) = first.trim().parse::<std::net::IpAddr>() {
                        return Ok(ClientIp(ip));
                    }
                }
            }
        }
        Err(RsqError::bad_request("could not determine client IP"))
    }
}

// ── ValidatedJson<T> ─────────────────────────────────────────────────────────

/// Like `Json<T>` but runs `validator::Validate` after deserialization.
/// Returns 422 with FastAPI-compatible error body on validation failure.
pub struct ValidatedJson<T>(pub T);

impl<T> ValidatedJson<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[async_trait]
impl<T> FromRequest for ValidatedJson<T>
where
    T: DeserializeOwned + Validate + Send,
{
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let bytes = ctx.take_body_bytes().await?;

        // Check content-type
        let content_type = ctx
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if !content_type.starts_with("application/json") {
            return Err(RsqError::unsupported_media_type(
                "Content-Type must be application/json",
            ));
        }

        let value: T = serde_json::from_slice(&bytes).map_err(|e| {
            RsqError::unprocessable_entity(format!("invalid JSON: {e}"))
        })?;

        value.validate().map_err(|e| {
            let ve = crate::error::ValidationErrors::from_validator(&e, "body");
            RsqError::new(
                http::StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::to_string(&ve).unwrap_or_default(),
            )
        })?;

        Ok(Self(value))
    }
}

// ── ValidatedQuery<T> ────────────────────────────────────────────────────────

/// Like `Query<T>` but runs `validator::Validate` after deserialization.
/// Returns 422 with FastAPI-compatible error body on validation failure.
pub struct ValidatedQuery<T>(pub T);

impl<T> ValidatedQuery<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[async_trait]
impl<T> FromRequest for ValidatedQuery<T>
where
    T: DeserializeOwned + Validate + Send,
{
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let raw = ctx.uri().query().unwrap_or_default();
        let value: T = serde_urlencoded::from_str(raw).map_err(|e| {
            RsqError::unprocessable_entity(format!("invalid query string: {e}"))
        })?;

        value.validate().map_err(|e| {
            let ve = crate::error::ValidationErrors::from_validator(&e, "query");
            RsqError::new(
                http::StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::to_string(&ve).unwrap_or_default(),
            )
        })?;

        Ok(Self(value))
    }
}

// ── Depends<T> — per-request dependency injection ────────────────────────────

/// Trait implemented by types that can be resolved as request-scoped dependencies.
///
/// Implement this for any type you want to inject into handlers via [`Depends<T>`].
/// The result is cached in the request context — `build()` is called at most once
/// per request per type.
pub trait FromDependency: Sized + Send + Sync + Clone + 'static {
    fn build(ctx: &mut RequestContext) -> impl std::future::Future<Output = Result<Self, RsqError>> + Send;
}

/// Dependency injection extractor. Resolves `T` via [`FromDependency::build`],
/// caching the result in the request context so multiple handler parameters of
/// the same type share the same instance.
///
/// # Example
///
/// ```rust,ignore
/// async fn handler(Depends(pool): Depends<MyPool>) -> Result<String, RsqError> {
///     Ok("ok".to_string())
/// }
/// ```
pub struct Depends<T>(pub T);

impl<T> Depends<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[async_trait]
impl<T> FromRequest for Depends<T>
where
    T: FromDependency + Send + Sync,
{
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        // Return cached instance if already resolved this request
        if let Some(cached) = ctx.dep_cache_mut().get::<T>().cloned() {
            return Ok(Depends(cached));
        }
        // Build, cache, return
        let value = T::build(ctx).await?;
        ctx.dep_cache_mut().insert(value.clone());
        Ok(Depends(value))
    }
}

// ── Pagination ────────────────────────────────────────────────────────────────

/// Query-string pagination extractor. Reads `?page=&per_page=` with defaults.
///
/// Defaults: `page = 1`, `per_page = 20`, max `per_page` capped at `100`.
///
/// # Example
///
/// ```rust,ignore
/// async fn list_items(Pagination { page, per_page }: Pagination) -> Result<String, RsqError> {
///     let offset = (page - 1) * per_page;
///     Ok(format!("page={page} per_page={per_page} offset={offset}"))
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Pagination {
    /// Current page number, 1-indexed.
    pub page: u64,
    /// Items per page (capped at 100).
    pub per_page: u64,
}

impl Pagination {
    /// Zero-based row offset: `(page - 1) * per_page`.
    pub fn offset(self) -> u64 {
        (self.page.saturating_sub(1)) * self.per_page
    }

    /// Alias for `per_page` — how many rows to fetch.
    pub fn limit(self) -> u64 {
        self.per_page
    }
}

#[derive(serde::Deserialize)]
struct PaginationParams {
    #[serde(default = "default_page")]
    page: u64,
    #[serde(default = "default_per_page")]
    per_page: u64,
}

fn default_page() -> u64 { 1 }
fn default_per_page() -> u64 { 20 }

#[async_trait]
impl FromRequest for Pagination {
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let raw = ctx.uri().query().unwrap_or_default();
        let params: PaginationParams = serde_urlencoded::from_str(raw)
            .map_err(|e| RsqError::unprocessable_entity(format!("invalid pagination params: {e}")))?;

        Ok(Pagination {
            page: params.page.max(1),
            per_page: params.per_page.clamp(1, 100),
        })
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::{Method, Request, StatusCode};
    use http_body_util::Full;
    use serde::{Deserialize, Serialize};

    use super::{ClientIp, Depends, Headers, Json, Pagination, Path, Query, RawBody, State, ValidatedJson};
    use crate::RsqApp;
    use crate::request::RequestContext;

    #[derive(Debug, Deserialize)]
    struct UserPath {
        id: u64,
    }

    #[derive(Debug, Deserialize)]
    struct SearchQuery {
        q: String,
    }

    #[derive(Debug, Deserialize)]
    struct CreateUser {
        name: String,
    }

    #[derive(Debug, Serialize)]
    struct UserResponse {
        id: u64,
        name: String,
        prefix: String,
    }

    #[tokio::test]
    async fn typed_extractors_work_in_handler_adapter() {
        async fn create_user(
            Path(path): Path<UserPath>,
            Query(query): Query<SearchQuery>,
            Json(payload): Json<CreateUser>,
            State(prefix): State<String>,
        ) -> Result<Json<UserResponse>, crate::RsqError> {
            Ok(Json(UserResponse {
                id: path.id,
                name: payload.name,
                prefix: format!("{prefix}-{}", query.q),
            }))
        }

        let app = RsqApp::new()
            .state(String::from("state"))
            .unwrap()
            .post("/users/{id}", create_user)
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/users/7?q=search")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(Full::new(Bytes::from_static(br#"{"name":"Ada"}"#)))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[http::header::CONTENT_TYPE], "application/json");
    }

    #[tokio::test]
    async fn json_extractor_rejects_wrong_content_type() {
        async fn create_user(Json(_payload): Json<CreateUser>) -> Result<StatusCode, crate::RsqError> {
            Ok(StatusCode::CREATED)
        }

        let app = RsqApp::new().post("/users", create_user).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/users")
                    .body(Full::new(Bytes::from_static(br#"{"name":"Ada"}"#)))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn headers_extractor_returns_request_headers() {
        async fn handler(Headers(map): Headers) -> Result<StatusCode, crate::RsqError> {
            assert!(map.contains_key("x-test-header"));
            Ok(StatusCode::OK)
        }

        let app = RsqApp::new().get("/headers", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/headers")
                    .header("x-test-header", "hello")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn raw_body_extractor_returns_body_bytes() {
        async fn handler(RawBody(body): RawBody) -> Result<StatusCode, crate::RsqError> {
            assert_eq!(body.as_ref(), b"hello world");
            Ok(StatusCode::OK)
        }

        let app = RsqApp::new().post("/body", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/body")
                    .body(Full::new(Bytes::from_static(b"hello world")))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn client_ip_extractor_falls_back_to_x_forwarded_for() {
        async fn handler(ClientIp(ip): ClientIp) -> Result<StatusCode, crate::RsqError> {
            assert_eq!(ip.to_string(), "1.2.3.4");
            Ok(StatusCode::OK)
        }

        let app = RsqApp::new().get("/ip", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/ip")
                    .header("x-forwarded-for", "1.2.3.4, 5.6.7.8")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn client_ip_extractor_returns_error_when_no_ip_available() {
        async fn handler(ClientIp(_ip): ClientIp) -> Result<StatusCode, crate::RsqError> {
            Ok(StatusCode::OK)
        }

        let app = RsqApp::new().get("/ip", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/ip")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn validated_json_rejects_invalid_field() {
        use validator::Validate;

        #[derive(serde::Deserialize, Validate)]
        struct CreateUser {
            #[validate(length(min = 1, max = 32))]
            username: String,
        }

        async fn handler(ValidatedJson(body): ValidatedJson<CreateUser>) -> Result<StatusCode, crate::RsqError> {
            let _ = body.username;
            Ok(StatusCode::OK)
        }

        let app = RsqApp::new().post("/users", handler).unwrap();

        // Empty username should fail validation
        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/users")
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from_static(br#"{"username":""}"#)))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn validated_json_passes_valid_input() {
        use validator::Validate;

        #[derive(serde::Deserialize, Validate)]
        struct CreateUser {
            #[validate(length(min = 1, max = 32))]
            username: String,
        }

        async fn handler(ValidatedJson(body): ValidatedJson<CreateUser>) -> Result<StatusCode, crate::RsqError> {
            assert_eq!(body.username, "alice");
            Ok(StatusCode::OK)
        }

        let app = RsqApp::new().post("/users", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::POST)
                    .uri("/users")
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from_static(br#"{"username":"alice"}"#)))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn depends_extractor_resolves_dependency() {
        #[derive(Clone)]
        struct RequestId(String);

        impl crate::extract::FromDependency for RequestId {
            async fn build(_ctx: &mut RequestContext) -> Result<Self, crate::RsqError> {
                Ok(RequestId("req-123".to_string()))
            }
        }

        async fn handler(
            Depends(rid): Depends<RequestId>,
        ) -> Result<StatusCode, crate::RsqError> {
            assert_eq!(rid.0, "req-123");
            Ok(StatusCode::OK)
        }

        let app = RsqApp::new().get("/test", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/test")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn depends_extractor_caches_per_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

        #[derive(Clone)]
        struct Counter(u32);

        impl crate::extract::FromDependency for Counter {
            async fn build(_ctx: &mut RequestContext) -> Result<Self, crate::RsqError> {
                CALL_COUNT.fetch_add(1, Ordering::SeqCst);
                Ok(Counter(42))
            }
        }

        async fn handler(
            Depends(c1): Depends<Counter>,
            Depends(c2): Depends<Counter>,
        ) -> Result<StatusCode, crate::RsqError> {
            assert_eq!(c1.0, 42);
            assert_eq!(c2.0, 42);
            Ok(StatusCode::OK)
        }

        CALL_COUNT.store(0, Ordering::SeqCst);
        let app = RsqApp::new().get("/test", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/test")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        // build() should only be called once despite two Depends<Counter> params
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);
    }

    async fn response_body_text(response: crate::response::Response) -> String {
        use http_body_util::BodyExt;
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[tokio::test]
    async fn pagination_defaults() {
        async fn handler(p: Pagination) -> Result<String, crate::RsqError> {
            Ok(format!("page={} per_page={} offset={}", p.page, p.per_page, p.offset()))
        }

        let app = RsqApp::new().get("/items", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/items")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body_text(response).await;
        assert!(body.contains("page=1"), "got: {body}");
        assert!(body.contains("per_page=20"), "got: {body}");
        assert!(body.contains("offset=0"), "got: {body}");
    }

    #[tokio::test]
    async fn pagination_custom_values() {
        async fn handler(p: Pagination) -> Result<String, crate::RsqError> {
            Ok(format!("page={} per_page={} offset={}", p.page, p.per_page, p.offset()))
        }

        let app = RsqApp::new().get("/items", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/items?page=3&per_page=10")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body_text(response).await;
        assert!(body.contains("page=3"), "got: {body}");
        assert!(body.contains("per_page=10"), "got: {body}");
        assert!(body.contains("offset=20"), "got: {body}");
    }

    #[tokio::test]
    async fn pagination_caps_per_page_at_100() {
        async fn handler(p: Pagination) -> Result<String, crate::RsqError> {
            Ok(format!("{}", p.per_page))
        }

        let app = RsqApp::new().get("/items", handler).unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/items?per_page=9999")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body_text(response).await;
        assert_eq!(body.trim(), "100", "got: {body}");
    }
}
