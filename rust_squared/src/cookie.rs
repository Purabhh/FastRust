//! Cookie parsing and `Set-Cookie` response helpers.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::error::RsqError;
use crate::extract::FromRequest;
use crate::request::RequestContext;

/// A parsed cookie jar extracted from the `Cookie` request header.
#[derive(Debug, Clone, Default)]
pub struct CookieJar(HashMap<String, String>);

impl CookieJar {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(|s| s.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[async_trait]
impl FromRequest for CookieJar {
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let header = match ctx.headers().get(http::header::COOKIE) {
            Some(h) => h,
            None => return Ok(Self(HashMap::new())),
        };
        let value = header
            .to_str()
            .map_err(|e| RsqError::bad_request(format!("invalid cookie header: {e}")))?;
        let mut map = HashMap::new();
        for pair in value.split(';') {
            let pair = pair.trim();
            if let Some((key, val)) = pair.split_once('=') {
                map.insert(key.trim().to_string(), val.trim().to_string());
            }
        }
        Ok(Self(map))
    }
}

/// Build a `Set-Cookie` header value.
///
/// Returns `Err` if the cookie name or value contains characters that are
/// invalid in an HTTP header value (e.g. control characters).
/// fix(H-2): was `expect()`; now returns `Result` so callers can handle errors.
pub fn set_cookie(name: &str, value: &str) -> Result<http::HeaderValue, RsqError> {
    http::HeaderValue::from_str(&format!("{name}={value}"))
        .map_err(|_| RsqError::internal("cookie name/value contains invalid header characters"))
}

/// Build a `Set-Cookie` header value with attributes.
///
/// Returns `Err` if the cookie name, value, or attrs contain characters that
/// are invalid in an HTTP header value.
/// fix(H-2): was `expect()`; now returns `Result` so callers can handle errors.
pub fn set_cookie_with(name: &str, value: &str, attrs: &str) -> Result<http::HeaderValue, RsqError> {
    http::HeaderValue::from_str(&format!("{name}={value}; {attrs}"))
        .map_err(|_| RsqError::internal("cookie value/attrs contain invalid header characters"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http::{Method, Request, StatusCode};
    use http_body_util::Full;

    use crate::RsqApp;

    #[tokio::test]
    async fn cookie_extractor_parses_header() {
        async fn handler(jar: CookieJar) -> Result<String, RsqError> {
            Ok(format!(
                "{}-{}",
                jar.get("session").unwrap_or("none"),
                jar.get("theme").unwrap_or("none"),
            ))
        }

        let app = RsqApp::new().get("/test", handler).unwrap();
        let resp = app
            .handle(
                Request::builder()
                    .method(Method::GET)
                    .uri("/test")
                    .header("cookie", "session=abc123; theme=dark")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn cookie_extractor_empty_when_no_header() {
        async fn handler(jar: CookieJar) -> Result<String, RsqError> {
            Ok(format!("{}", jar.len()))
        }

        let app = RsqApp::new().get("/test", handler).unwrap();
        let resp = app
            .handle(
                Request::builder()
                    .uri("/test")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn set_cookie_formats_correctly() {
        let hv = set_cookie("session", "abc123").unwrap();
        assert_eq!(hv.to_str().unwrap(), "session=abc123");
    }

    #[test]
    fn set_cookie_with_attrs() {
        let hv = set_cookie_with("session", "abc", "HttpOnly; Secure; Path=/").unwrap();
        assert_eq!(
            hv.to_str().unwrap(),
            "session=abc; HttpOnly; Secure; Path=/"
        );
    }

    #[test]
    fn cookie_jar_operations() {
        let mut map = HashMap::new();
        map.insert("a".into(), "1".into());
        map.insert("b".into(), "2".into());
        let jar = CookieJar(map);
        assert_eq!(jar.get("a"), Some("1"));
        assert!(jar.contains("b"));
        assert!(!jar.contains("c"));
        assert_eq!(jar.len(), 2);
        assert!(!jar.is_empty());
    }
}
