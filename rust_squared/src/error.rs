use std::fmt::{Display, Formatter};

use http::StatusCode;
use serde::Serialize;

use crate::response::{IntoResponse, Response};

#[derive(Debug)]
pub struct RsqError {
    status: StatusCode,
    message: String,
    context: Option<String>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Clone for RsqError {
    fn clone(&self) -> Self {
        Self {
            status: self.status,
            message: self.message.clone(),
            context: self.context.clone(),
            source: None,
        }
    }
}

impl RsqError {
    #[track_caller]
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            context: None,
            source: None,
        }
    }

    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    #[track_caller]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    #[track_caller]
    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        Self::new(StatusCode::METHOD_NOT_ALLOWED, message)
    }

    #[track_caller]
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    #[track_caller]
    pub fn unsupported_media_type(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, message)
    }

    #[track_caller]
    pub fn unprocessable_entity(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, message)
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn context(mut self, msg: impl Into<String>) -> Self {
        self.context = Some(msg.into());
        self
    }

    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
}

impl Display for RsqError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.context {
            Some(ctx) => write!(f, "{}: {} ({})", self.status, self.message, ctx),
            None => write!(f, "{}: {}", self.status, self.message),
        }
    }
}

impl std::error::Error for RsqError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl IntoResponse for RsqError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

// ── FastAPI-compatible 422 validation error ───────────────────────────────────

/// A single validation failure — matches FastAPI/Pydantic v2 wire format exactly.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub loc: Vec<serde_json::Value>,
    pub msg: String,
    pub input: serde_json::Value,
}

/// 422 Unprocessable Entity body — matches FastAPI's `{"detail": [...]}` format.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationErrors {
    pub detail: Vec<ValidationDetail>,
}

impl ValidationErrors {
    pub fn new(detail: Vec<ValidationDetail>) -> Self {
        Self { detail }
    }

    /// Convert a `validator::ValidationErrors` map into FastAPI wire format.
    pub fn from_validator(errors: &validator::ValidationErrors, location: &str) -> Self {
        let mut detail = Vec::new();
        for (field, field_errors) in errors.field_errors() {
            for e in field_errors {
                detail.push(ValidationDetail {
                    error_type: e.code.to_string(),
                    loc: vec![
                        serde_json::Value::String(location.to_string()),
                        serde_json::Value::String(field.to_string()),
                    ],
                    msg: e.message.as_ref()
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| e.code.to_string()),
                    input: serde_json::Value::Null,
                });
            }
        }
        Self { detail }
    }
}

impl IntoResponse for ValidationErrors {
    fn into_response(self) -> Response {
        use http_body_util::BodyExt;
        let body = serde_json::to_vec(&self).unwrap_or_default();
        http::Response::builder()
            .status(http::StatusCode::UNPROCESSABLE_ENTITY)
            .header("content-type", "application/json")
            .body(
                http_body_util::Full::new(bytes::Bytes::from(body)).boxed()
            )
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::StatusCode;
    use std::error::Error;

    #[test]
    fn factory_methods_return_correct_status() {
        assert_eq!(RsqError::internal("err").status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(RsqError::not_found("err").status(), StatusCode::NOT_FOUND);
        assert_eq!(RsqError::bad_request("err").status(), StatusCode::BAD_REQUEST);
        assert_eq!(RsqError::method_not_allowed("err").status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(RsqError::unsupported_media_type("err").status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(RsqError::unprocessable_entity("err").status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn context_appended_to_display() {
        let err = RsqError::internal("something failed").context("while parsing config");
        assert!(err.to_string().contains("while parsing config"));
    }

    #[test]
    fn with_source_chains_errors() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err = RsqError::internal("read failed").with_source(io_err);
        assert!(err.source().is_some());
    }

    #[test]
    fn clone_drops_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::Other, "oops");
        let err = RsqError::internal("fail").with_source(io_err);
        let cloned = err.clone();
        assert!(cloned.source().is_none());
        assert_eq!(cloned.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
