//! In-process test client for FastRust applications.
//!
//! Drives [`RsqApp`] directly via [`RsqApp::handle`] — no TCP socket, no port,
//! instant startup. Ideal for unit and integration tests.
//!
//! # Example
//!
//! ```rust,no_run
//! use rust_squared::{RsqApp, RsqError};
//! use rust_squared::testing::TestClient;
//!
//! #[tokio::test]
//! async fn test_hello() {
//!     let app = RsqApp::new()
//!         .get("/hello", || async { Ok::<_, RsqError>("world") })
//!         .unwrap();
//!
//!     let client = TestClient::new(app);
//!     let res = client.get("/hello").send().await;
//!     assert_eq!(res.status(), 200);
//! }
//! ```

use std::collections::HashMap;

use bytes::Bytes;
use http::{Method, StatusCode};
use http_body_util::Full;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::app::RsqApp;

/// In-process test client. Wraps an [`RsqApp`] and dispatches requests directly.
pub struct TestClient {
    app: RsqApp,
}

impl TestClient {
    /// Create a new test client wrapping the given app.
    pub fn new(app: RsqApp) -> Self {
        Self { app }
    }

    /// Start a GET request builder.
    pub fn get(&self, path: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(&self.app, Method::GET, path.into())
    }

    /// Start a POST request builder.
    pub fn post(&self, path: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(&self.app, Method::POST, path.into())
    }

    /// Start a PUT request builder.
    pub fn put(&self, path: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(&self.app, Method::PUT, path.into())
    }

    /// Start a PATCH request builder.
    pub fn patch(&self, path: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(&self.app, Method::PATCH, path.into())
    }

    /// Start a DELETE request builder.
    pub fn delete(&self, path: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(&self.app, Method::DELETE, path.into())
    }
}

/// Fluent request builder returned by [`TestClient`] methods.
pub struct RequestBuilder<'a> {
    app: &'a RsqApp,
    method: Method,
    path: String,
    headers: HashMap<String, String>,
    body: Bytes,
}

impl<'a> RequestBuilder<'a> {
    fn new(app: &'a RsqApp, method: Method, path: String) -> Self {
        Self {
            app,
            method,
            path,
            headers: HashMap::new(),
            body: Bytes::new(),
        }
    }

    /// Add a request header.
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set the request body as raw bytes.
    pub fn body(mut self, bytes: impl Into<Bytes>) -> Self {
        self.body = bytes.into();
        self
    }

    /// Serialize `value` as JSON and set `Content-Type: application/json`.
    pub fn json<S: Serialize>(mut self, value: &S) -> Self {
        self.body = Bytes::from(serde_json::to_vec(value).expect("failed to serialize JSON body"));
        self.headers.insert(
            "content-type".to_string(),
            "application/json".to_string(),
        );
        self
    }

    /// Dispatch the request in-process and return a [`TestResponse`].
    pub async fn send(self) -> TestResponse {
        let mut builder = http::Request::builder()
            .method(self.method)
            .uri(self.path);

        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let request = builder
            .body(Full::new(self.body))
            .expect("failed to build test request");

        let response = self.app.handle(request).await;

        // Buffer the response body
        use http_body_util::BodyExt;
        let (parts, body) = response.into_parts();
        let bytes = body
            .collect()
            .await
            .map(|b| b.to_bytes())
            .unwrap_or_default();

        TestResponse { parts, bytes }
    }
}

/// Buffered HTTP response returned by [`RequestBuilder::send`].
pub struct TestResponse {
    parts: http::response::Parts,
    bytes: Bytes,
}

impl TestResponse {
    /// HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.parts.status
    }

    /// HTTP status code as u16.
    pub fn status_code(&self) -> u16 {
        self.parts.status.as_u16()
    }

    /// Response headers.
    pub fn headers(&self) -> &http::HeaderMap {
        &self.parts.headers
    }

    /// Get a single response header value as a string.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.parts.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Response body as raw bytes.
    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// Response body as UTF-8 text.
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes).unwrap_or("")
    }

    /// Deserialize the response body as JSON.
    ///
    /// # Panics
    ///
    /// Panics if the body is not valid JSON for type `T`.
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.bytes).expect("failed to deserialize response body as JSON")
    }

    /// Assert the response has the expected status code. Panics with a useful
    /// message if the status does not match.
    #[track_caller]
    pub fn assert_status(&self, expected: StatusCode) -> &Self {
        assert_eq!(
            self.parts.status,
            expected,
            "expected status {} but got {} (body: {})",
            expected,
            self.parts.status,
            self.text()
        );
        self
    }

    /// Assert the response status is 200 OK.
    #[track_caller]
    pub fn assert_ok(&self) -> &Self {
        self.assert_status(StatusCode::OK)
    }

    /// Assert the response body, parsed as `serde_json::Value`, contains all
    /// key/value pairs in `expected`. Unknown keys in the actual response are
    /// ignored (partial match), matching FastAPI's test idiom.
    ///
    /// Only works for top-level object keys. Nested matching is not supported.
    #[track_caller]
    pub fn assert_json_contains(&self, expected: serde_json::Value) -> &Self {
        let actual: serde_json::Value =
            serde_json::from_slice(&self.bytes).expect("response body is not JSON");
        if let (Some(expected_obj), Some(actual_obj)) =
            (expected.as_object(), actual.as_object())
        {
            for (key, expected_val) in expected_obj {
                let actual_val = actual_obj.get(key).unwrap_or_else(|| {
                    panic!(
                        "expected key '{}' in response JSON but it was missing. Actual: {}",
                        key, actual
                    )
                });
                assert_eq!(
                    actual_val, expected_val,
                    "key '{}': expected {} but got {}",
                    key, expected_val, actual_val
                );
            }
        } else {
            assert_eq!(&actual, &expected, "response JSON did not match expected");
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RsqApp, RsqError};
    use serde_json::json;

    #[tokio::test]
    async fn test_client_get_returns_200() {
        let app = RsqApp::new()
            .get("/ping", || async { Ok::<_, RsqError>("pong") })
            .unwrap();

        let client = TestClient::new(app);
        let res = client.get("/ping").send().await;
        res.assert_ok();
        assert!(res.text().contains("pong"));
    }

    #[tokio::test]
    async fn test_client_post_with_json_body() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct Input {
            name: String,
        }

        async fn handler(crate::Json(body): crate::Json<Input>) -> Result<String, RsqError> {
            Ok(format!("hello {}", body.name))
        }

        let app = RsqApp::new().post("/greet", handler).unwrap();
        let client = TestClient::new(app);

        let res = client
            .post("/greet")
            .json(&json!({"name": "Alice"}))
            .send()
            .await;

        res.assert_ok();
        assert!(res.text().contains("Alice"));
    }

    #[tokio::test]
    async fn test_client_assert_status_404() {
        let app = RsqApp::new()
            .get("/exists", || async { Ok::<_, RsqError>("ok") })
            .unwrap();

        let client = TestClient::new(app);
        let res = client.get("/missing").send().await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_client_headers_sent_correctly() {
        async fn handler(crate::Headers(map): crate::Headers) -> Result<String, RsqError> {
            let val = map
                .get("x-custom")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            Ok(val)
        }

        let app = RsqApp::new().get("/headers", handler).unwrap();
        let client = TestClient::new(app);

        let res = client
            .get("/headers")
            .header("x-custom", "test-value")
            .send()
            .await;

        res.assert_ok();
        assert!(res.text().contains("test-value"));
    }

    #[tokio::test]
    async fn test_client_json_deserialize() {
        use serde::Serialize;

        #[derive(Serialize)]
        struct Output {
            value: i32,
        }

        async fn handler() -> Result<String, RsqError> {
            let output = Output { value: 42 };
            Ok(serde_json::to_string(&output).expect("serialization should not fail"))
        }

        let app = RsqApp::new().get("/data", handler).unwrap();
        let client = TestClient::new(app);

        let res = client.get("/data").send().await;
        res.assert_ok();
        let body: serde_json::Value = res.json();
        assert_eq!(body["value"], 42);
    }
}
