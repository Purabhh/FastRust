use bytes::Bytes;
use http::StatusCode;
use http::header::CONTENT_TYPE;
use http_body_util::{BodyExt, Full, combinators::BoxBody};

/// Flexible body type: supports both buffered (Full) and streaming responses.
pub type RsqBody = BoxBody<Bytes, std::convert::Infallible>;
pub type Response = http::Response<RsqBody>;

/// Helper to wrap a Full body into BoxBody.
fn boxed_full(body: Bytes) -> RsqBody {
    Full::new(body).boxed()
}

pub trait IntoResponse {
    fn into_response(self) -> Response;
}

pub struct Html(pub String);

fn build_response(status: StatusCode, body: Bytes) -> Response {
    http::Response::builder()
        .status(status)
        .body(boxed_full(body))
        .expect("response builder should be infallible")
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for StatusCode {
    fn into_response(self) -> Response {
        build_response(self, Bytes::new())
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response {
        build_response(StatusCode::OK, Bytes::from_static(self.as_bytes()))
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        build_response(StatusCode::OK, Bytes::from(self))
    }
}

impl IntoResponse for Bytes {
    fn into_response(self) -> Response {
        build_response(StatusCode::OK, self)
    }
}

impl IntoResponse for (StatusCode, &'static str) {
    fn into_response(self) -> Response {
        build_response(self.0, Bytes::from_static(self.1.as_bytes()))
    }
}

impl IntoResponse for (StatusCode, String) {
    fn into_response(self) -> Response {
        build_response(self.0, Bytes::from(self.1))
    }
}

impl IntoResponse for Html {
    fn into_response(self) -> Response {
        let mut response = build_response(StatusCode::OK, Bytes::from(self.0));
        response
            .headers_mut()
            .insert(CONTENT_TYPE, http::HeaderValue::from_static("text/html; charset=utf-8"));
        response
    }
}

/// HTTP redirect response.
pub struct Redirect {
    status: StatusCode,
    location: String,
}

impl Redirect {
    /// 302 Found (temporary redirect).
    pub fn to(url: impl Into<String>) -> Self {
        Self { status: StatusCode::FOUND, location: url.into() }
    }

    /// 301 Moved Permanently.
    pub fn permanent(url: impl Into<String>) -> Self {
        Self { status: StatusCode::MOVED_PERMANENTLY, location: url.into() }
    }

    /// 303 See Other (redirect after POST).
    pub fn see_other(url: impl Into<String>) -> Self {
        Self { status: StatusCode::SEE_OTHER, location: url.into() }
    }
}

impl IntoResponse for Redirect {
    fn into_response(self) -> Response {
        let mut response = build_response(self.status, Bytes::new());
        // fix(H-3): non-ASCII redirect URL caused panic; now returns 500 instead
        match http::HeaderValue::from_str(&self.location) {
            Ok(v) => {
                response.headers_mut().insert(http::header::LOCATION, v);
                response
            }
            Err(_) => build_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                Bytes::from_static(b"invalid redirect URL"),
            ),
        }
    }
}

// ── NdjsonResponse<S> — streaming NDJSON ─────────────────────────────────────

use futures_util::stream::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Streaming newline-delimited JSON response.
///
/// Each item yielded by `S` is serialized as a JSON object followed by `\n`.
/// Sets `Content-Type: application/x-ndjson` and `X-Accel-Buffering: no`
/// (disables nginx/proxy response buffering).
///
/// # Example
///
/// ```rust,ignore
/// async fn stream_items() -> NdjsonResponse<impl Stream<Item = Item>> {
///     let stream = futures_util::stream::iter(vec![Item { id: 1 }, Item { id: 2 }]);
///     NdjsonResponse::new(stream)
/// }
/// ```
pub struct NdjsonResponse<S> {
    stream: S,
}

impl<S> NdjsonResponse<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

/// Internal streaming body that serializes each item as NDJSON.
struct NdjsonBody<S> {
    stream: Pin<Box<S>>,
}

impl<S, T> http_body::Body for NdjsonBody<S>
where
    S: Stream<Item = T>,
    T: serde::Serialize,
{
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(item)) => {
                let mut json = serde_json::to_vec(&item).unwrap_or_else(|e| {
                    format!("{{\"error\":\"{e}\"}}").into_bytes()
                });
                json.push(b'\n');
                Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(json)))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S, T> IntoResponse for NdjsonResponse<S>
where
    S: Stream<Item = T> + Send + Sync + 'static,
    T: serde::Serialize,
{
    fn into_response(self) -> Response {
        let body = NdjsonBody { stream: Box::pin(self.stream) };
        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/x-ndjson")
            .header("x-accel-buffering", "no")
            .header("cache-control", "no-cache")
            .body(body.boxed())
            .expect("response builder infallible")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_to_302() {
        let r = Redirect::to("/login").into_response();
        assert_eq!(r.status(), StatusCode::FOUND);
        assert_eq!(r.headers()["location"], "/login");
    }

    #[test]
    fn redirect_permanent_301() {
        let r = Redirect::permanent("/new-url").into_response();
        assert_eq!(r.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(r.headers()["location"], "/new-url");
    }

    #[test]
    fn redirect_see_other_303() {
        let r = Redirect::see_other("/result").into_response();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        assert_eq!(r.headers()["location"], "/result");
    }
}
