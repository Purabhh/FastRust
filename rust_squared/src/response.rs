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
        response.headers_mut().insert(
            http::header::LOCATION,
            http::HeaderValue::from_str(&self.location)
                .expect("redirect URL should be a valid header value"),
        );
        response
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
