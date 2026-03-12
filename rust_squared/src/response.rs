use bytes::Bytes;
use http::StatusCode;
use http_body_util::Full;

pub type RsqBody = Full<Bytes>;
pub type Response = http::Response<RsqBody>;

pub trait IntoResponse {
    fn into_response(self) -> Response;
}

fn build_response(status: StatusCode, body: Bytes) -> Response {
    http::Response::builder()
        .status(status)
        .body(Full::new(body))
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
