use std::future::Future;

use http::header::CONTENT_TYPE;
use serde::Serialize;
use serde::de::DeserializeOwned;

use http_body_util::BodyExt;

use crate::error::RsqError;
use crate::request::RequestContext;
use crate::response::{IntoResponse, Response};

pub trait FromRequest: Sized + Send {
    fn from_request<'a>(ctx: &'a mut RequestContext) -> impl Future<Output = Result<Self, RsqError>> + Send + 'a
    where
        Self: 'a;
}

pub struct Path<T>(pub T);

impl<T> Path<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> FromRequest for Path<T>
where
    T: DeserializeOwned + Send,
{
    async fn from_request<'a>(ctx: &'a mut RequestContext) -> Result<Self, RsqError>
    where
        Self: 'a,
    {
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

impl<T> FromRequest for Query<T>
where
    T: DeserializeOwned + Send,
{
    async fn from_request<'a>(ctx: &'a mut RequestContext) -> Result<Self, RsqError>
    where
        Self: 'a,
    {
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

impl<T> FromRequest for Json<T>
where
    T: DeserializeOwned + Send,
{
    async fn from_request<'a>(ctx: &'a mut RequestContext) -> Result<Self, RsqError>
    where
        Self: 'a,
    {
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

impl<T> FromRequest for State<T>
where
    T: Clone + Send + Sync + 'static,
{
    async fn from_request<'a>(ctx: &'a mut RequestContext) -> Result<Self, RsqError>
    where
        Self: 'a,
    {
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

// Named async fn helpers — workaround for rust#100013.
// Async fns own their parameters, so the compiler can prove 'static.
// Return Response directly so callers can build a BoxedHandler without Route::new.
async fn extract_1<F, Fut, Res, A>(handler: F, mut ctx: RequestContext) -> Result<Response, RsqError>
where
    F: Fn(A) -> Fut,
    Fut: Future<Output = Result<Res, RsqError>>,
    Res: IntoResponse,
    A: FromRequest,
{
    let a = A::from_request(&mut ctx).await?;
    handler(a).await.map(IntoResponse::into_response)
}

async fn extract_2<F, Fut, Res, A, B>(handler: F, mut ctx: RequestContext) -> Result<Response, RsqError>
where
    F: Fn(A, B) -> Fut,
    Fut: Future<Output = Result<Res, RsqError>>,
    Res: IntoResponse,
    A: FromRequest,
    B: FromRequest,
{
    let a = A::from_request(&mut ctx).await?;
    let b = B::from_request(&mut ctx).await?;
    handler(a, b).await.map(IntoResponse::into_response)
}

async fn extract_3<F, Fut, Res, A, B, C>(handler: F, mut ctx: RequestContext) -> Result<Response, RsqError>
where
    F: Fn(A, B, C) -> Fut,
    Fut: Future<Output = Result<Res, RsqError>>,
    Res: IntoResponse,
    A: FromRequest,
    B: FromRequest,
    C: FromRequest,
{
    let a = A::from_request(&mut ctx).await?;
    let b = B::from_request(&mut ctx).await?;
    let c = C::from_request(&mut ctx).await?;
    handler(a, b, c).await.map(IntoResponse::into_response)
}

async fn extract_4<F, Fut, Res, A, B, C, D>(handler: F, mut ctx: RequestContext) -> Result<Response, RsqError>
where
    F: Fn(A, B, C, D) -> Fut,
    Fut: Future<Output = Result<Res, RsqError>>,
    Res: IntoResponse,
    A: FromRequest,
    B: FromRequest,
    C: FromRequest,
    D: FromRequest,
{
    let a = A::from_request(&mut ctx).await?;
    let b = B::from_request(&mut ctx).await?;
    let c = C::from_request(&mut ctx).await?;
    let d = D::from_request(&mut ctx).await?;
    handler(a, b, c, d).await.map(IntoResponse::into_response)
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
            Box::pin(extract_1(self.clone(), ctx))
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
            Box::pin(extract_2(self.clone(), ctx))
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
            Box::pin(extract_3(self.clone(), ctx))
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
            Box::pin(extract_4(self.clone(), ctx))
        }))
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::{Method, Request, StatusCode};
    use http_body_util::Full;
    use serde::{Deserialize, Serialize};

    use super::{Json, Path, Query, State};
    use crate::RsqApp;

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
}
