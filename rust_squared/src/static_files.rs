use std::path::PathBuf;
use std::sync::Arc;

use http_body_util::BodyExt;
use tokio::fs;

use crate::error::RsqError;
use crate::request::RequestContext;
use crate::router::Route;

/// Serves static files from a directory.
pub struct StaticFiles {
    dir: PathBuf,
}

/// Alias for backwards compatibility.
pub type ServeDir = StaticFiles;

impl StaticFiles {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Create a GET route that serves files under the given pattern.
    ///
    /// The pattern should end with a `{file}` parameter, e.g. `/static/{file}`.
    pub fn into_route(self, pattern: impl Into<String>) -> Route {
        let dir = Arc::new(self.dir);
        Route::new(http::Method::GET, pattern, move |ctx: RequestContext| {
            let dir = Arc::clone(&dir);
            async move {
                let file_path = ctx
                    .path_param("file")
                    .unwrap_or_default()
                    .to_string();

                if file_path.contains("..") {
                    return Err(RsqError::not_found("file not found"));
                }

                let full_path = dir.join(&file_path);

                // Path traversal protection
                if !full_path.starts_with(dir.as_ref()) {
                    return Err(RsqError::not_found("file not found"));
                }

                let contents = fs::read(&full_path)
                    .await
                    .map_err(|_| RsqError::not_found("file not found"))?;

                let mime = mime_from_extension(
                    full_path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or(""),
                );

                let body = http_body_util::Full::new(bytes::Bytes::from(contents)).boxed();
                http::Response::builder()
                    .header("Content-Type", mime)
                    .body(body)
                    .map_err(|e| RsqError::internal(e.to_string()))
            }
        })
    }
}

fn mime_from_extension(ext: &str) -> &str {
    match ext {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}
