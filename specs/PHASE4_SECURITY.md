# Phase 4: Security Hardening — Implementation Spec

**Repo:** https://github.com/Purabhh/FastRust
**Branch:** main
**Crate:** rust_squared (path: `rust_squared/`)

---

## Prerequisites

```bash
git pull origin main
cargo check --workspace   # must pass before starting
cargo test --workspace    # must pass before starting
```

---

## TASK 4.1: TLS/HTTPS Support

### Goal
Add a `serve_tls()` method to `RsqApp` that wraps `TcpListener` with a TLS acceptor using `tokio-rustls`.

### Files to modify
- `Cargo.toml` (workspace root, line ~18 `[workspace.dependencies]`)
- `rust_squared/Cargo.toml` (line ~8 `[dependencies]`)
- `rust_squared/src/app.rs`

### Step-by-step

1. **Add dependencies to workspace Cargo.toml** under `[workspace.dependencies]`:
```toml
tokio-rustls = "0.26"
rustls = "0.23"
rustls-pemfile = "2"
```

2. **Add dependencies to rust_squared/Cargo.toml** under `[dependencies]`:
```toml
tokio-rustls.workspace = true
rustls.workspace = true
rustls-pemfile.workspace = true
```

3. **Add to `rust_squared/src/app.rs`** — a new method `serve_tls` on `RsqApp`:

```rust
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc as StdArc;
use tokio_rustls::TlsAcceptor;

impl RsqApp {
    pub async fn serve_tls(
        self,
        addr: SocketAddr,
        cert_path: impl AsRef<std::path::Path>,
        key_path: impl AsRef<std::path::Path>,
    ) -> Result<(), RsqError> {
        // 1. Load certs from PEM file
        let cert_file = File::open(cert_path.as_ref())
            .map_err(|e| RsqError::internal(format!("failed to open cert file: {e}")))?;
        let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
            .collect::<Result<_, _>>()
            .map_err(|e| RsqError::internal(format!("failed to parse certs: {e}")))?;

        // 2. Load private key from PEM file
        let key_file = File::open(key_path.as_ref())
            .map_err(|e| RsqError::internal(format!("failed to open key file: {e}")))?;
        let key = rustls_pemfile::private_key(&mut BufReader::new(key_file))
            .map_err(|e| RsqError::internal(format!("failed to parse private key: {e}")))?
            .ok_or_else(|| RsqError::internal("no private key found in file"))?;

        // 3. Build rustls ServerConfig
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| RsqError::internal(format!("TLS config error: {e}")))?;

        let acceptor = TlsAcceptor::from(StdArc::new(config));

        // 4. Bind TCP listener
        let listener = TcpListener::bind(addr).await
            .map_err(|e| RsqError::internal(format!("failed to bind listener: {e}")))?;

        // 5. Accept loop — same as serve_listener but with TLS wrapping
        let app = Arc::new(self);
        loop {
            let (stream, _) = listener.accept().await
                .map_err(|e| RsqError::internal(format!("failed to accept: {e}")))?;
            let acceptor = acceptor.clone();
            let app = Arc::clone(&app);
            tokio::spawn(async move {
                let tls_stream = match acceptor.accept(stream).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("TLS handshake failed: {e}");
                        return;
                    }
                };
                let io = TokioIo::new(tls_stream);
                let service = service_fn(move |request| {
                    let app = Arc::clone(&app);
                    async move { Ok::<_, std::convert::Infallible>(app.handle_incoming(request).await) }
                });
                if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("connection error: {e}");
                }
            });
        }
    }
}
```

### Existing code to keep working
- `serve()` and `serve_listener()` must remain unchanged and functional.
- All 27+ existing tests must still pass.

### Testing
No automated test needed (TLS requires cert files). But verify `cargo check --workspace` passes.

---

## TASK 4.2: Security Headers Middleware

### Goal
Add `SecurityHeadersMiddleware` that injects security headers into every response. Use a builder pattern so each header is configurable.

### File to modify
- `rust_squared/src/middleware.rs` (append after `CorsMiddleware` impl)
- `rust_squared/src/lib.rs` (add export)

### The middleware pattern to follow

Every middleware in this crate implements this trait using `Box::pin(async move { ... })`:

```rust
pub trait RsqMiddleware: Send + Sync + 'static {
    fn handle<'a>(&'a self, ctx: RequestContext, next: Next) -> BoxFuture<'a, Result<Response, RsqError>>;
}
```

### Implementation

```rust
#[derive(Clone, Debug)]
pub struct SecurityHeadersMiddleware {
    hsts: Option<HeaderValue>,
    content_type_options: Option<HeaderValue>,
    frame_options: Option<HeaderValue>,
    csp: Option<HeaderValue>,
    xss_protection: Option<HeaderValue>,
}

impl SecurityHeadersMiddleware {
    /// Returns a middleware with all recommended security headers enabled.
    pub fn defaults() -> Self {
        Self {
            hsts: Some(HeaderValue::from_static("max-age=63072000; includeSubDomains")),
            content_type_options: Some(HeaderValue::from_static("nosniff")),
            frame_options: Some(HeaderValue::from_static("DENY")),
            csp: Some(HeaderValue::from_static("default-src 'self'")),
            xss_protection: Some(HeaderValue::from_static("0")),
        }
    }

    pub fn hsts(mut self, value: impl Into<Option<HeaderValue>>) -> Self {
        self.hsts = value.into();
        self
    }

    pub fn content_security_policy(mut self, value: impl Into<Option<HeaderValue>>) -> Self {
        self.csp = value.into();
        self
    }

    pub fn frame_options(mut self, value: impl Into<Option<HeaderValue>>) -> Self {
        self.frame_options = value.into();
        self
    }
}
```

The `RsqMiddleware` impl should call `next.run(ctx).await?` then insert each `Some(header)` into the response headers. Header names:
- `strict-transport-security`
- `x-content-type-options`
- `x-frame-options`
- `content-security-policy`
- `x-xss-protection`

### Export
Add to `rust_squared/src/lib.rs` in the middleware re-export line:
```rust
pub use middleware::{..., SecurityHeadersMiddleware};
```

### Tests (add to middleware.rs `#[cfg(test)] mod tests`)

```rust
#[tokio::test]
async fn security_headers_defaults_applied() {
    let app = RsqApp::new()
        .middleware(SecurityHeadersMiddleware::defaults())
        .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
        .unwrap();
    let response = app.handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap()).await;
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["x-frame-options"], "DENY");
    assert_eq!(response.headers()["content-security-policy"], "default-src 'self'");
    assert_eq!(response.headers()["x-xss-protection"], "0");
}

#[tokio::test]
async fn security_headers_can_be_customized() {
    let mw = SecurityHeadersMiddleware::defaults()
        .frame_options(None);
    let app = RsqApp::new()
        .middleware(mw)
        .route(Route::new(Method::GET, "/", |_| async { Ok("ok") }))
        .unwrap();
    let response = app.handle(Request::builder().uri("/").body(Full::new(Bytes::new())).unwrap()).await;
    assert!(!response.headers().contains_key("x-frame-options"));
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
}
```

---

## TASK 4.3: CSRF Protection Middleware

### Goal
Add `CsrfMiddleware` using the double-submit cookie pattern.

### File to modify
- `rust_squared/src/middleware.rs` (append)
- `rust_squared/src/lib.rs` (add export)

### How double-submit cookie works
1. On every response, set a cookie `csrf_token=<random_hex>` (generate 32 random bytes, hex-encode)
2. On unsafe methods (POST, PUT, DELETE, PATCH), read the `X-CSRF-Token` request header
3. Also read the `csrf_token` cookie from the `Cookie` request header
4. If they match, allow the request. If they don't match or either is missing, return 403 Forbidden
5. Skip validation entirely for safe methods (GET, HEAD, OPTIONS)

### Dependencies needed
None — use `rand` or generate random bytes with a simple counter. Actually, to keep deps minimal:
- Use `std::time::SystemTime` + process id as a simple token seed, OR
- Better: add `rand = "0.8"` to workspace and crate Cargo.toml for proper random generation

**Recommended:** Add to workspace Cargo.toml:
```toml
rand = "0.8"
```
Add to rust_squared/Cargo.toml:
```toml
rand.workspace = true
```

### Implementation sketch

```rust
use rand::Rng;

#[derive(Clone, Debug)]
pub struct CsrfMiddleware {
    cookie_name: String,
    header_name: String,
    token_length: usize,
}

impl CsrfMiddleware {
    pub fn new() -> Self {
        Self {
            cookie_name: "csrf_token".to_string(),
            header_name: "x-csrf-token".to_string(),
            token_length: 32,
        }
    }
}
```

In the `handle` impl:
1. Check if method is safe (GET/HEAD/OPTIONS) — if so, skip validation, just call `next.run(ctx)` and set the cookie on response
2. For unsafe methods:
   a. Extract `csrf_token` cookie value from `Cookie` header (parse `key=value; key2=value2` format)
   b. Extract `X-CSRF-Token` header value
   c. If both exist and match, proceed with `next.run(ctx)`
   d. Otherwise return `403 Forbidden`
3. On every response, generate a new token and set `Set-Cookie: csrf_token=<token>; Path=/; SameSite=Strict; HttpOnly`

### Tests

```rust
#[tokio::test]
async fn csrf_allows_get_without_token() { /* GET should pass */ }

#[tokio::test]
async fn csrf_rejects_post_without_token() { /* POST without headers should return 403 */ }

#[tokio::test]
async fn csrf_allows_post_with_matching_token() {
    // Build request with Cookie: csrf_token=abc123 and X-CSRF-Token: abc123
    // Should return 200
}

#[tokio::test]
async fn csrf_rejects_post_with_mismatched_token() {
    // Cookie says "abc" but header says "xyz" -> 403
}
```

---

## TASK 4.4: Input Sanitization Utilities

### Goal
Add utility functions for sanitizing user input.

### File to create
- `rust_squared/src/sanitize.rs` (NEW FILE)

### File to modify
- `rust_squared/src/lib.rs` (add `pub mod sanitize;` and re-exports)

### Functions to implement

```rust
/// Escapes HTML special characters: & < > " '
/// Prevents XSS when user input is rendered in HTML.
pub fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Returns true if the string is safe to use as an HTTP header value.
/// Rejects values containing \r, \n, or \0 (header injection vectors).
pub fn is_safe_header_value(input: &str) -> bool {
    !input.contains('\r') && !input.contains('\n') && !input.contains('\0')
}

/// Removes all null bytes from the input string.
/// Null bytes can cause truncation in C-backed systems.
pub fn strip_null_bytes(input: &str) -> String {
    input.replace('\0', "")
}
```

### Export from lib.rs
```rust
pub mod sanitize;
pub use sanitize::{html_escape, is_safe_header_value, strip_null_bytes};
```

### Tests (in sanitize.rs)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_handles_all_special_chars() {
        assert_eq!(html_escape("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;");
    }

    #[test]
    fn html_escape_passes_safe_string() {
        assert_eq!(html_escape("hello world"), "hello world");
    }

    #[test]
    fn is_safe_header_rejects_newlines() {
        assert!(!is_safe_header_value("value\r\nEvil-Header: injected"));
    }

    #[test]
    fn is_safe_header_accepts_normal_value() {
        assert!(is_safe_header_value("application/json"));
    }

    #[test]
    fn strip_null_bytes_removes_nulls() {
        assert_eq!(strip_null_bytes("hello\0world"), "helloworld");
    }

    #[test]
    fn strip_null_bytes_noop_on_clean() {
        assert_eq!(strip_null_bytes("clean"), "clean");
    }
}
```

---

## Quality Gates (ALL must pass before pushing)

1. `cargo check --workspace` — zero errors
2. `cargo test --workspace` — all existing tests + new tests pass
3. `cargo clippy --workspace` — no new warnings
4. Each task committed separately with descriptive message
5. Push: `git push https://<YOUR_GITHUB_PAT>@github.com/Purabhh/FastRust.git main`

---

## Dependency Summary

| Crate | Version | Where | Why |
|-------|---------|-------|-----|
| tokio-rustls | 0.26 | workspace + rust_squared | TLS acceptor |
| rustls | 0.23 | workspace + rust_squared | TLS config |
| rustls-pemfile | 2 | workspace + rust_squared | Parse PEM certs/keys |
| rand | 0.8 | workspace + rust_squared | CSRF token generation |
