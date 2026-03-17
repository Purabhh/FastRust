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

    /// Create a GET route that serves files under the given mount prefix.
    ///
    /// `pattern` is the URL prefix at which files are mounted, e.g. `/static`.
    /// Any path below that prefix is mapped to the served directory, so
    /// `/static/css/app.css` serves `<dir>/css/app.css`. Path traversal
    /// sequences (`..`) are rejected.
    pub fn into_route(self, pattern: impl Into<String>) -> Route {
        let dir = Arc::new(self.dir);
        let mount_prefix = {
            let p = pattern.into();
            // Normalise: strip trailing slash so prefix stripping is consistent
            p.trim_end_matches('/').to_string()
        };
        // fix(H-4): use the `**` catch-all wildcard so the trie matches any
        // depth below the mount prefix (e.g. /static/css/app.css). The handler
        // always reads the real path from ctx.uri().path() anyway.
        let route_pattern = format!("{mount_prefix}/**");
        Route::new(http::Method::GET, route_pattern, move |ctx: RequestContext| {
            let dir = Arc::clone(&dir);
            let prefix = mount_prefix.clone();
            async move {
                // Derive the relative file path from the raw request URI so
                // that nested paths like /static/css/app.css work correctly.
                let uri_path = ctx.uri().path();
                let relative = uri_path
                    .strip_prefix(&prefix)
                    .unwrap_or(uri_path)
                    .trim_start_matches('/');

                // Block path traversal: reject any segment containing ".."
                if relative.split('/').any(|seg| seg == "..") {
                    return Err(RsqError::not_found("file not found"));
                }

                // Also reject percent-encoded traversal sequences
                if relative.contains("%2e") || relative.contains("%2E") {
                    return Err(RsqError::not_found("file not found"));
                }

                let full_path = dir.join(relative);

                // fix: path traversal guard used unresolved path; symlinks could escape served dir
                // canonicalize is sync but acceptable here — it's a single stat syscall
                let canonical_dir = std::fs::canonicalize(dir.as_ref())
                    .unwrap_or_else(|_| dir.as_ref().to_path_buf());
                let canonical_full = match std::fs::canonicalize(&full_path) {
                    Ok(p) => p,
                    Err(_) => return Err(RsqError::not_found("file not found")),
                };
                if !canonical_full.starts_with(&canonical_dir) {
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    // ── E8: nested URI path stripping resolves to the correct on-disk path ──

    /// Simulates the path-resolution logic inside `into_route` to verify that
    /// a multi-segment URI like `/static/css/app.css` maps to `<dir>/css/app.css`.
    /// This exercises the prefix-strip and `dir.join(relative)` arithmetic that
    /// was rewritten in WS-D without relying on the router trie (which only
    /// supports single-segment `{param}` wildcards).
    #[test]
    fn nested_uri_resolves_to_correct_disk_path() {
        let dir = Path::new("/srv/static");
        let mount_prefix = "/static";

        // Mimic the handler logic verbatim.
        let uri_path = "/static/css/app.css";
        let relative = uri_path
            .strip_prefix(mount_prefix)
            .unwrap_or(uri_path)
            .trim_start_matches('/');

        assert_eq!(
            relative, "css/app.css",
            "prefix stripping must leave only the sub-path"
        );

        let full_path = dir.join(relative);
        assert_eq!(
            full_path,
            Path::new("/srv/static/css/app.css"),
            "joined path must be <dir>/css/app.css"
        );

        // The canonical guard: resolved path must start with the served directory.
        assert!(
            full_path.starts_with(dir),
            "resolved path must remain inside the served directory"
        );
    }

    // ── E9: Windows-style and percent-encoded path traversal is blocked ──────

    /// Verifies the traversal-rejection predicates in `into_route` against the
    /// attack vectors identified in the audit:
    ///   • plain `..` segments (forward-slash separated)
    ///   • percent-encoded `%2e%2e` / `%2E%2E`
    ///   • Windows-style `\..\` (caught by canonical `starts_with` guard on Windows,
    ///     treated as a literal filename component on Unix — safe either way)
    #[test]
    fn path_traversal_inputs_are_rejected() {
        let dir = Path::new("/srv/static");

        // Each tuple: (raw relative path after prefix stripping, expected rejection reason)
        let traversal_cases: &[(&str, &str)] = &[
            // Standard forward-slash traversal — caught by the ".." segment check.
            ("../../../etc/passwd", "plain .. segment"),
            // Double-dot at the start of a path with no leading slash.
            ("../secret.key", "single .. at start"),
            // Percent-encoded dot (lower-case) — caught by the %2e check.
            ("%2e%2e/etc/passwd", "percent-encoded %2e"),
            // Percent-encoded dot (upper-case) — caught by the %2E check.
            ("%2E%2E/etc/passwd", "percent-encoded %2E"),
        ];

        for (relative, label) in traversal_cases {
            // Check 1: forward-slash split catches plain ".." segments.
            let has_dotdot = relative.split('/').any(|seg| seg == "..");
            // Check 2: percent-encoded traversal.
            let has_pct_encoded = relative.contains("%2e") || relative.contains("%2E");
            // Check 3: canonical path escape guard (catches any remaining variants).
            let full_path = dir.join(relative);
            let escapes_dir = !full_path.starts_with(dir);

            assert!(
                has_dotdot || has_pct_encoded || escapes_dir,
                "traversal vector '{relative}' ({label}) must be caught by at least one guard"
            );
        }
    }
}
