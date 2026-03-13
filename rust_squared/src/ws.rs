//! WebSocket upgrade support for FastRust.
//!
//! Provides WebSocket upgrade detection and the HTTP 101 handshake response.
//! This module handles **only the handshake phase**: it validates the upgrade
//! request and returns the `101 Switching Protocols` response required by
//! RFC 6455. Ongoing WebSocket session management (reading/writing frames)
//! must be handled at the transport layer after the HTTP response is sent.
//!
//! Note: `on_upgrade()` is **not** provided by this module. To accept a
//! WebSocket connection, call `WebSocketUpgrade::accept()` which returns
//! the 101 response with the correct `Sec-WebSocket-Accept` header.
//!
//! ```text
//! // Pseudocode — actual WebSocket session handling is transport-layer work:
//! //
//! // async fn ws_handler(ctx: RequestContext) -> Result<Response, RsqError> {
//! //     let ws = WebSocketUpgrade::from_headers(ctx.headers())?;
//! //     // ws.accept() returns the 101 Switching Protocols HTTP response.
//! //     // After this response is sent, upgrade the transport to WebSocket
//! //     // frames outside of the HTTP handler.
//! //     Ok(ws.accept())
//! // }
//! ```

use http::{HeaderValue, StatusCode};

use crate::error::RsqError;
use crate::response::{IntoResponse, Response};

/// Checks if a request is a WebSocket upgrade request.
pub fn is_upgrade_request(headers: &http::HeaderMap) -> bool {
    let upgrade = headers
        .get(http::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    let connection = headers
        .get(http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase().contains("upgrade"))
        .unwrap_or(false);

    upgrade && connection
}

/// Represents a WebSocket upgrade request.
/// Call `on_upgrade` with an async handler to accept the connection.
pub struct WebSocketUpgrade {
    sec_key: String,
}

impl WebSocketUpgrade {
    /// Create from request headers. Returns None if not a valid upgrade request.
    pub fn from_headers(headers: &http::HeaderMap) -> Result<Self, RsqError> {
        if !is_upgrade_request(headers) {
            return Err(RsqError::bad_request("not a WebSocket upgrade request"));
        }

        let sec_key = headers
            .get("sec-websocket-key")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| RsqError::bad_request("missing Sec-WebSocket-Key header"))?
            .to_string();

        Ok(Self { sec_key })
    }

    /// Build the 101 Switching Protocols response.
    /// The actual WebSocket handling happens at the transport layer,
    /// not in the HTTP response body.
    pub fn accept(self) -> Response {
        let accept_key = compute_accept_key(&self.sec_key);
        let mut response = StatusCode::SWITCHING_PROTOCOLS.into_response();
        let headers = response.headers_mut();
        headers.insert(
            http::header::UPGRADE,
            HeaderValue::from_static("websocket"),
        );
        headers.insert(
            http::header::CONNECTION,
            HeaderValue::from_static("Upgrade"),
        );
        headers.insert(
            http::header::HeaderName::from_static("sec-websocket-accept"),
            HeaderValue::from_str(&accept_key).expect("accept key should be valid"),
        );
        response
    }
}

/// Compute the Sec-WebSocket-Accept value per RFC 6455.
fn compute_accept_key(key: &str) -> String {
    use std::io::Write;
    let magic = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut hasher = Sha1::new();
    write!(hasher, "{key}{magic}").unwrap();
    base64_encode(&hasher.finalize())
}

// Minimal SHA-1 implementation for WebSocket accept key computation.
// Only used for the 20-byte hash needed by RFC 6455.
struct Sha1 {
    data: Vec<u8>,
}

impl Sha1 {
    fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn finalize(self) -> [u8; 20] {
        // SHA-1 implementation
        let mut h0: u32 = 0x67452301;
        let mut h1: u32 = 0xEFCDAB89;
        let mut h2: u32 = 0x98BADCFE;
        let mut h3: u32 = 0x10325476;
        let mut h4: u32 = 0xC3D2E1F0;

        let bit_len = (self.data.len() as u64) * 8;
        let mut msg = self.data;
        msg.push(0x80);
        while (msg.len() % 64) != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in msg.chunks(64) {
            let mut w = [0u32; 80];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }

            let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);

            #[allow(clippy::needless_range_loop)]
            for i in 0..80 {
                let (f, k) = match i {
                    0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                    20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                    _ => (b ^ c ^ d, 0xCA62C1D6u32),
                };
                let temp = a
                    .rotate_left(5)
                    .wrapping_add(f)
                    .wrapping_add(e)
                    .wrapping_add(k)
                    .wrapping_add(w[i]);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = temp;
            }

            h0 = h0.wrapping_add(a);
            h1 = h1.wrapping_add(b);
            h2 = h2.wrapping_add(c);
            h3 = h3.wrapping_add(d);
            h4 = h4.wrapping_add(e);
        }

        let mut result = [0u8; 20];
        result[0..4].copy_from_slice(&h0.to_be_bytes());
        result[4..8].copy_from_slice(&h1.to_be_bytes());
        result[8..12].copy_from_slice(&h2.to_be_bytes());
        result[12..16].copy_from_slice(&h3.to_be_bytes());
        result[16..20].copy_from_slice(&h4.to_be_bytes());
        result
    }
}

impl std::io::Write for Sha1 {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_upgrade_detects_websocket() {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::UPGRADE, "websocket".parse().unwrap());
        headers.insert(http::header::CONNECTION, "Upgrade".parse().unwrap());
        assert!(is_upgrade_request(&headers));
    }

    #[test]
    fn is_upgrade_rejects_non_websocket() {
        let headers = http::HeaderMap::new();
        assert!(!is_upgrade_request(&headers));
    }

    #[test]
    fn accept_key_rfc6455_test_vector() {
        // RFC 6455 Section 4.2.2 example
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let accept = compute_accept_key(key);
        assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn upgrade_from_headers() {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::UPGRADE, "websocket".parse().unwrap());
        headers.insert(http::header::CONNECTION, "Upgrade".parse().unwrap());
        headers.insert(
            http::header::HeaderName::from_static("sec-websocket-key"),
            "dGhlIHNhbXBsZSBub25jZQ==".parse().unwrap(),
        );
        let ws = WebSocketUpgrade::from_headers(&headers).unwrap();
        let response = ws.accept();
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(response.headers()["upgrade"], "websocket");
        assert_eq!(response.headers()["sec-websocket-accept"], "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn upgrade_rejects_non_ws() {
        let headers = http::HeaderMap::new();
        assert!(WebSocketUpgrade::from_headers(&headers).is_err());
    }

    #[test]
    fn base64_encodes_correctly() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }
}
