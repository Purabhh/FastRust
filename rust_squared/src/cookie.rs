//! Cookie parsing and `Set-Cookie` response helpers.

use std::collections::HashMap;

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

impl FromRequest for CookieJar {
    async fn from_request<'a>(ctx: &'a mut RequestContext) -> Result<Self, RsqError>
    where
        Self: 'a,
    {
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
pub fn set_cookie(name: &str, value: &str) -> http::HeaderValue {
    http::HeaderValue::from_str(&format!("{name}={value}"))
        .expect("cookie name/value should be valid header chars")
}

/// Build a `Set-Cookie` header value with attributes.
pub fn set_cookie_with(name: &str, value: &str, attrs: &str) -> http::HeaderValue {
    http::HeaderValue::from_str(&format!("{name}={value}; {attrs}"))
        .expect("cookie value should be valid header chars")
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
        let hv = set_cookie("session", "abc123");
        assert_eq!(hv.to_str().unwrap(), "session=abc123");
    }

    #[test]
    fn set_cookie_with_attrs() {
        let hv = set_cookie_with("session", "abc", "HttpOnly; Secure; Path=/");
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
