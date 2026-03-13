//! Static file serving for FastRust.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use http::StatusCode;
use http_body_util::Full;

use crate::error::RsqError;
use crate::response::{IntoResponse, Response, RsqBody};

/// Serves static files from a directory on disk.
pub struct StaticFiles {
    base_dir: PathBuf,
}

impl StaticFiles {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Serve a file relative to the base directory.
    /// Returns 404 if the file doesn't exist, 403 if path traversal is detected.
    pub async fn serve(&self, file_path: &str) -> Result<Response, RsqError> {
        // Prevent path traversal
        if file_path.contains("..") || file_path.contains('\0') {
            return Err(RsqError::new(StatusCode::FORBIDDEN, "forbidden path"));
        }

        let clean = file_path.trim_start_matches('/');
        let full_path = self.base_dir.join(clean);

        // Ensure resolved path is under base_dir
        let canonical_base = self
            .base_dir
            .canonicalize()
            .map_err(|_| RsqError::not_found("base directory not found"))?;
        let canonical_file = full_path
            .canonicalize()
            .map_err(|_| RsqError::not_found("file not found"))?;
        if !canonical_file.starts_with(&canonical_base) {
            return Err(RsqError::new(StatusCode::FORBIDDEN, "forbidden path"));
        }

        let content = tokio::fs::read(&canonical_file)
            .await
            .map_err(|_| RsqError::not_found("file not found"))?;

        let content_type = guess_content_type(&canonical_file);
        let body: RsqBody = http_body_util::BodyExt::boxed(Full::new(Bytes::from(content)));
        let mut resp = http::Response::builder()
            .status(StatusCode::OK)
            .body(body)
            .expect("static file response should be infallible");
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_str(content_type)
                .unwrap_or_else(|_| http::HeaderValue::from_static("application/octet-stream")),
        );
        Ok(resp)
    }
}

/// Guess MIME type from file extension.
pub fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        Some("js" | "mjs") => "application/javascript",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("wasm") => "application/wasm",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn content_type_detection() {
        assert_eq!(guess_content_type(Path::new("style.css")), "text/css");
        assert_eq!(
            guess_content_type(Path::new("app.js")),
            "application/javascript"
        );
        assert_eq!(
            guess_content_type(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(guess_content_type(Path::new("data.json")), "application/json");
        assert_eq!(guess_content_type(Path::new("photo.png")), "image/png");
        assert_eq!(guess_content_type(Path::new("photo.jpg")), "image/jpeg");
        assert_eq!(guess_content_type(Path::new("icon.svg")), "image/svg+xml");
        assert_eq!(
            guess_content_type(Path::new("unknown.xyz")),
            "application/octet-stream"
        );
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let sf = StaticFiles::new(PathBuf::from("/tmp"));
        let result = sf.serve("../etc/passwd").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_null_bytes() {
        let sf = StaticFiles::new(PathBuf::from("/tmp"));
        let result = sf.serve("file\0.txt").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn not_found_for_missing_file() {
        let sf = StaticFiles::new(PathBuf::from("/tmp"));
        let result = sf.serve("nonexistent_file_12345.txt").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().status(), StatusCode::NOT_FOUND);
    }
}
