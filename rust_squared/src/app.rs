use std::net::SocketAddr;
use std::sync::Arc;
use std::collections::HashMap;

use bytes::Bytes;
use http::{Method, Request};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::error::RsqError;
use crate::extract::Handler;
use crate::middleware::{Next, RsqMiddleware};
use crate::openapi::{openapi_response, swagger_ui_response};
use crate::request::RequestContext;
use crate::response::{IntoResponse, Response};
use crate::router::{MethodNotAllowed, Route, Router};
use crate::schema::RsqSchema;
use crate::state::AppState;

#[derive(Clone, Default)]
pub struct RsqApp {
    router: Router,
    state: AppState,
    middlewares: Arc<Vec<Arc<dyn RsqMiddleware>>>,
    docs_enabled: bool,
    schemas: Arc<HashMap<String, serde_json::Value>>,
}

impl RsqApp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route(mut self, route: Route) -> Result<Self, RsqError> {
        self.router.insert(route)?;
        Ok(self)
    }

    pub fn middleware<M>(mut self, middleware: M) -> Self
    where
        M: RsqMiddleware,
    {
        Arc::make_mut(&mut self.middlewares).push(Arc::new(middleware));
        self
    }

    pub fn with_docs(mut self) -> Self {
        self.docs_enabled = true;
        self
    }

    pub fn schema<T>(mut self) -> Self
    where
        T: RsqSchema,
    {
        Arc::make_mut(&mut self.schemas).insert(T::schema_name().to_string(), T::schema());
        self
    }

    pub fn route_handler<H, Args>(
        self,
        method: Method,
        pattern: impl Into<String>,
        handler: H,
    ) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route(handler.into_route(method, pattern.into()))
    }

    pub fn get<H, Args>(self, pattern: impl Into<String>, handler: H) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route_handler(Method::GET, pattern, handler)
    }

    pub fn post<H, Args>(self, pattern: impl Into<String>, handler: H) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route_handler(Method::POST, pattern, handler)
    }

    pub fn put<H, Args>(self, pattern: impl Into<String>, handler: H) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route_handler(Method::PUT, pattern, handler)
    }

    pub fn delete<H, Args>(self, pattern: impl Into<String>, handler: H) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route_handler(Method::DELETE, pattern, handler)
    }

    pub fn patch<H, Args>(self, pattern: impl Into<String>, handler: H) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route_handler(Method::PATCH, pattern, handler)
    }

    pub fn head<H, Args>(self, pattern: impl Into<String>, handler: H) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route_handler(Method::HEAD, pattern, handler)
    }

    pub fn options<H, Args>(self, pattern: impl Into<String>, handler: H) -> Result<Self, RsqError>
    where
        H: Handler<Args>,
    {
        self.route_handler(Method::OPTIONS, pattern, handler)
    }

    pub fn state<T>(mut self, value: T) -> Result<Self, RsqError>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.state.insert(value)?;
        Ok(self)
    }

    pub async fn handle<B>(&self, request: Request<B>) -> Response
    where
        B: http_body::Body<Data = Bytes> + Send + 'static,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        let method = request.method().clone();
        let path = request.uri().path().to_string();

        if let Some(response) = self.docs_response(&method, &path) {
            return response;
        }

        match self.router.find(&method, &path) {
            Ok((route, params)) => match RequestContext::from_request(request, params, self.state.clone()).await {
                Ok(ctx) => match Next::new(Arc::clone(&self.middlewares), route.clone()).run(ctx).await {
                    Ok(response) => response,
                    Err(error) => error.into_response(),
                },
                Err(error) => error.into_response(),
            },
            Err(MethodNotAllowed { allowed }) if !allowed.is_empty() => method_not_allowed_response(allowed),
            Err(_) => RsqError::not_found("route not found").into_response(),
        }
    }

    pub async fn handle_incoming(&self, request: Request<Incoming>) -> Response {
        let method = request.method().clone();
        let path = request.uri().path().to_string();

        if let Some(response) = self.docs_response(&method, &path) {
            return response;
        }

        match self.router.find(&method, &path) {
            Ok((route, params)) => {
                let ctx = RequestContext::from_incoming(request, params, self.state.clone());
                match Next::new(Arc::clone(&self.middlewares), route.clone()).run(ctx).await {
                    Ok(response) => response,
                    Err(error) => error.into_response(),
                }
            }
            Err(MethodNotAllowed { allowed }) if !allowed.is_empty() => method_not_allowed_response(allowed),
            Err(_) => RsqError::not_found("route not found").into_response(),
        }
    }

    pub async fn serve(self, addr: SocketAddr) -> Result<(), RsqError> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|error| RsqError::internal(format!("failed to bind listener: {error}")))?;
        self.serve_listener(listener).await
    }

    pub async fn serve_listener(self, listener: TcpListener) -> Result<(), RsqError> {
        let app = Arc::new(self);
        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|error| RsqError::internal(format!("failed to accept connection: {error}")))?;
            let app = Arc::clone(&app);
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |request| {
                    let app = Arc::clone(&app);
                    async move { Ok::<_, std::convert::Infallible>(app.handle_incoming(request).await) }
                });
                if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("connection error: {error}");
                }
            });
        }
    }

    fn docs_response(&self, method: &Method, path: &str) -> Option<Response> {
        if !self.docs_enabled {
            return None;
        }

        match (method, path) {
            (&Method::GET, "/openapi.json") => Some(openapi_response(self.router.routes(), &self.schemas).into_response()),
            (&Method::GET, "/docs") => Some(swagger_ui_response("/openapi.json").into_response()),
            _ => None,
        }
    }
}

fn method_not_allowed_response(allowed: Vec<Method>) -> Response {
    let allow = allowed
        .iter()
        .map(Method::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let mut response = RsqError::method_not_allowed("method not allowed").into_response();
    response.headers_mut().insert(
        http::header::ALLOW,
        allow.parse().expect("allow header should be valid"),
    );
    response
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::{Method, Request, StatusCode};
    use http_body_util::Full;
    use serde::{Deserialize, Serialize};

    use super::RsqApp;
    use crate::extract::{Path, Query};
    use crate::router::Route;

    #[tokio::test]
    async fn handles_get_route() {
        let app = RsqApp::new()
            .route(Route::new(Method::GET, "/", |_| async { Ok("hello") }))
            .unwrap();

        let response = app
            .handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn returns_not_found_for_unknown_path() {
        let app = RsqApp::new();

        let response = app
            .handle(Request::builder().uri("/missing").body(Full::new(Bytes::new())).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn returns_method_not_allowed_and_allow_header() {
        let app = RsqApp::new()
            .route(Route::new(Method::POST, "/items", |_| async { Ok(StatusCode::CREATED) }))
            .unwrap();

        let response = app
            .handle(Request::builder().method(Method::GET).uri("/items").body(Full::new(Bytes::new())).unwrap())
            .await;

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()["allow"], "POST");
    }

    #[test]
    fn state_rejects_duplicate_types() {
        let err = RsqApp::new().state(1_u64).unwrap().state(2_u64).err().unwrap();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[derive(Deserialize)]
    struct UserPath {
        id: u64,
    }

    #[derive(Deserialize)]
    struct UserQuery {
        verbose: bool,
    }

    #[tokio::test]
    async fn get_handler_supports_extractors() {
        async fn show_user(
            Path(path): Path<UserPath>,
            Query(query): Query<UserQuery>,
        ) -> Result<String, crate::RsqError> {
            Ok(format!("{}:{}", path.id, query.verbose))
        }

        let app = RsqApp::new().get("/users/{id}", show_user).unwrap();
        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/users/5?verbose=true")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn serves_openapi_json_when_docs_enabled() {
        let app = RsqApp::new()
            .with_docs()
            .route(Route::new(Method::GET, "/users/{id}", |_| async { Ok("ok") }))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/openapi.json")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[http::header::CONTENT_TYPE], "application/json");
    }

    #[tokio::test]
    async fn serves_docs_html_when_enabled() {
        let app = RsqApp::new().with_docs();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/docs")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[http::header::CONTENT_TYPE], "text/html; charset=utf-8");
    }

    #[derive(Serialize, crate::RsqSchema)]
    struct UserDoc {
        id: u64,
    }

    #[tokio::test]
    async fn serves_registered_schemas_in_openapi() {
        let mut meta = crate::RouteMeta::default();
        meta.set_response_schema("UserDoc");
        let app = RsqApp::new()
            .with_docs()
            .schema::<UserDoc>()
            .route(Route::new(Method::GET, "/users/{id}", |_| async { Ok("ok") }).with_meta(meta))
            .unwrap();

        let response = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/openapi.json")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }
}
