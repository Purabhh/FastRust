//! Server-Sent Events (SSE) support.
//!
//! ```ignore
//! use rust_squared::sse::{Sse, SseEvent};
//! use tokio_stream::wrappers::ReceiverStream;
//!
//! async fn events() -> Result<Sse, RsqError> {
//!     let (tx, rx) = tokio::sync::mpsc::channel(16);
//!     tokio::spawn(async move {
//!         tx.send(SseEvent::new().data("hello")).await.ok();
//!     });
//!     Ok(Sse::new(ReceiverStream::new(rx)))
//! }
//! ```

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;
use http::StatusCode;
use http_body::Frame;
use http_body_util::BodyExt;

use crate::response::{IntoResponse, Response};

/// A single SSE event.
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
    event: Option<String>,
    data: Option<String>,
    id: Option<String>,
    retry: Option<u64>,
}

impl SseEvent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }

    pub fn data(mut self, data: impl Into<String>) -> Self {
        self.data = Some(data.into());
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn retry(mut self, ms: u64) -> Self {
        self.retry = Some(ms);
        self
    }

    /// Format this event as an SSE text block.
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        if let Some(ref event) = self.event {
            out.push_str(&format!("event: {event}\n"));
        }
        if let Some(ref data) = self.data {
            for line in data.lines() {
                out.push_str(&format!("data: {line}\n"));
            }
        }
        if let Some(ref id) = self.id {
            out.push_str(&format!("id: {id}\n"));
        }
        if let Some(retry) = self.retry {
            out.push_str(&format!("retry: {retry}\n"));
        }
        out.push('\n');
        out
    }
}

/// Body adapter that converts a stream of `SseEvent` into an HTTP body.
pub struct SseBody<S> {
    stream: Pin<Box<S>>,
}

impl<S> http_body::Body for SseBody<S>
where
    S: Stream<Item = SseEvent> + Send + 'static,
{
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(event)) => {
                let bytes = Bytes::from(event.to_string());
                Poll::Ready(Some(Ok(Frame::data(bytes))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// SSE response wrapper. Wraps a stream of `SseEvent` into a proper
/// `text/event-stream` response.
pub struct Sse {
    body: crate::response::RsqBody,
}

impl Sse {
    pub fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = SseEvent> + Send + 'static,
    {
        let sse_body = SseBody {
            stream: Box::pin(stream),
        };
        Self {
            body: sse_body.boxed(),
        }
    }
}

impl IntoResponse for Sse {
    fn into_response(self) -> Response {
        let mut response = http::Response::builder()
            .status(StatusCode::OK)
            .body(self.body)
            .expect("sse response builder should be infallible");
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/event-stream"),
        );
        response.headers_mut().insert(
            http::header::CACHE_CONTROL,
            http::HeaderValue::from_static("no-cache"),
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_formats_data_only() {
        let event = SseEvent::new().data("hello");
        assert_eq!(event.to_string(), "data: hello\n\n");
    }

    #[test]
    fn event_formats_full() {
        let event = SseEvent::new()
            .event("update")
            .data("payload")
            .id("42")
            .retry(5000);
        let out = event.to_string();
        assert!(out.contains("event: update\n"));
        assert!(out.contains("data: payload\n"));
        assert!(out.contains("id: 42\n"));
        assert!(out.contains("retry: 5000\n"));
    }

    #[test]
    fn event_multiline_data() {
        let event = SseEvent::new().data("line1\nline2");
        let out = event.to_string();
        assert!(out.contains("data: line1\n"));
        assert!(out.contains("data: line2\n"));
    }

    #[tokio::test]
    async fn sse_into_response_sets_headers() {
        let stream = futures_util::stream::iter(vec![
            SseEvent::new().data("hello"),
        ]);
        let sse = Sse::new(stream);
        let response = sse.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[http::header::CONTENT_TYPE],
            "text/event-stream"
        );
        assert_eq!(
            response.headers()[http::header::CACHE_CONTROL],
            "no-cache"
        );
    }
}
