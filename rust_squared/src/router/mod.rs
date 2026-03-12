mod route;
mod trie;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use http::Method;

use crate::error::RsqError;

pub use route::Route;
use trie::{Match, TrieNode};

pub type PathParams = HashMap<String, String>;

#[derive(Debug, Clone)]
pub struct MethodNotAllowed {
    pub allowed: Vec<Method>,
}

#[derive(Default, Clone)]
pub struct Router {
    routes: Vec<Route>,
    trie: Arc<Mutex<TrieNode>>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, route: Route) -> Result<(), RsqError> {
        let index = self.routes.len();
        self.trie
            .lock()
            .expect("trie lock poisoned")
            .insert(route.pattern(), route.method().clone(), index)?;
        self.routes.push(route);
        Ok(())
    }

    pub fn find(&self, method: &Method, path: &str) -> Result<(&Route, PathParams), MethodNotAllowed> {
        match self
            .trie
            .lock()
            .expect("trie lock poisoned")
            .find(path, method)
        {
            Match::Found(index, params) => Ok((&self.routes[index], params)),
            Match::MethodNotAllowed(allowed) => Err(MethodNotAllowed { allowed }),
            Match::NotFound => Err(MethodNotAllowed {
                allowed: Vec::new(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use http::Method;

    use super::{Route, Router};
    use crate::error::RsqError;
    use crate::request::{RequestContext, RsqRequestBody};
    use crate::state::AppState;

    fn route(method: Method, pattern: &str) -> Route {
        Route::new(method, pattern, |_| async { Ok("ok") })
    }

    #[test]
    fn prefers_static_segments_over_params() {
        let mut router = Router::new();
        router.insert(route(Method::GET, "/users/me")).unwrap();
        router.insert(route(Method::GET, "/users/{id}")).unwrap();

        let (matched, params) = router.find(&Method::GET, "/users/me").unwrap();
        assert_eq!(matched.pattern(), "/users/me");
        assert!(params.is_empty());
    }

    #[test]
    fn decodes_percent_encoded_params() {
        let mut router = Router::new();
        router.insert(route(Method::GET, "/files/{name}")).unwrap();

        let (_, params) = router.find(&Method::GET, "/files/hello%20world").unwrap();
        assert_eq!(params.get("name").map(String::as_str), Some("hello world"));
    }

    #[test]
    fn rejects_duplicate_method_and_pattern() {
        let mut router = Router::new();
        router.insert(route(Method::GET, "/items/{id}")).unwrap();

        let err = router.insert(route(Method::GET, "/items/{id}")).err().unwrap();
        assert_eq!(err.status(), http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_conflicting_param_names() {
        let mut router = Router::new();
        router.insert(route(Method::GET, "/items/{id}")).unwrap();

        let err = router.insert(route(Method::GET, "/items/{slug}")).err().unwrap();
        assert_eq!(err.status(), http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn trailing_slash_normalizes() {
        let mut router = Router::new();
        router.insert(route(Method::GET, "/items/{id}/")).unwrap();

        let (matched, params) = router.find(&Method::GET, "/items/123").unwrap();
        assert_eq!(matched.pattern(), "/items/{id}/");
        assert_eq!(params.get("id").map(String::as_str), Some("123"));
    }

    #[test]
    fn reports_method_not_allowed() {
        let mut router = Router::new();
        router.insert(route(Method::POST, "/items")).unwrap();

        let err = router.find(&Method::GET, "/items").err().unwrap();
        assert!(err.allowed.contains(&Method::POST));
    }

    #[test]
    fn not_found_has_empty_allow_set() {
        let mut router = Router::new();
        router.insert(route(Method::GET, "/items")).unwrap();

        let err = router.find(&Method::GET, "/missing").err().unwrap();
        assert!(err.allowed.is_empty());
    }

    #[test]
    fn route_handler_returns_response() {
        let route = Route::new(Method::GET, "/", |_| async {
            Ok::<_, RsqError>(http::StatusCode::CREATED)
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime
            .block_on(route.call(RequestContext::new(
                http::Request::builder()
                    .uri("/")
                    .body(())
                    .unwrap()
                    .into_parts()
                    .0,
                RsqRequestBody::Buffered(bytes::Bytes::new()),
                Default::default(),
                AppState::new(),
            )))
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::CREATED);
    }
}

