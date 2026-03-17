use bytes::Bytes;
use http::{HeaderMap, Method, Uri, request::Parts};
use http_body_util::BodyExt;
use hyper::body::Incoming;

use std::net::SocketAddr;

use crate::error::RsqError;
use crate::router::PathParams;
use crate::state::AppState;

pub enum RsqRequestBody {
    Streaming(Incoming),
    Buffered(Bytes),
    Consumed,
}

pub struct RequestContext {
    parts: Parts,
    body: RsqRequestBody,
    path_params: PathParams,
    state: AppState,
    peer_addr: Option<SocketAddr>,
    max_body_size: usize,
    dep_cache: crate::state::TypeMap,
}

impl RequestContext {
    pub fn new(parts: Parts, body: RsqRequestBody, path_params: PathParams, state: AppState, peer_addr: Option<SocketAddr>) -> Self {
        Self {
            parts,
            body,
            path_params,
            state,
            peer_addr,
            max_body_size: 1024 * 1024,
            dep_cache: crate::state::TypeMap::new(),
        }
    }

    pub async fn from_request<B>(
        request: http::Request<B>,
        path_params: PathParams,
        state: AppState,
    ) -> Result<Self, RsqError>
    where
        B: http_body::Body<Data = Bytes> + Send + 'static,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        let (parts, body) = request.into_parts();
        let collected = body
            .collect()
            .await
            .map_err(|error| RsqError::internal(format!("failed to read request body: {error}")))?;
        Ok(Self::new(
            parts,
            RsqRequestBody::Buffered(collected.to_bytes()),
            path_params,
            state,
            None,
        ))
    }

    pub fn from_incoming(
        request: http::Request<Incoming>,
        path_params: PathParams,
        state: AppState,
        peer_addr: Option<SocketAddr>,
    ) -> Self {
        let (parts, body) = request.into_parts();
        Self::new(parts, RsqRequestBody::Streaming(body), path_params, state, peer_addr)
    }

    pub fn method(&self) -> &Method {
        &self.parts.method
    }

    pub fn uri(&self) -> &Uri {
        &self.parts.uri
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.parts.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.parts.headers
    }

    pub fn path_param(&self, key: &str) -> Option<&str> {
        self.path_params.get(key).map(String::as_str)
    }

    pub fn path_params(&self) -> &PathParams {
        &self.path_params
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn set_max_body_size(&mut self, max_body_size: usize) {
        self.max_body_size = max_body_size;
    }

    pub fn set_body(&mut self, body: Bytes) {
        self.body = RsqRequestBody::Buffered(body);
    }

    /// Access the per-request dependency cache.
    pub(crate) fn dep_cache_mut(&mut self) -> &mut crate::state::TypeMap {
        &mut self.dep_cache
    }

    pub async fn take_body_bytes(&mut self) -> Result<Bytes, RsqError> {
        let body = std::mem::replace(&mut self.body, RsqRequestBody::Consumed);
        match body {
            RsqRequestBody::Streaming(incoming) => {
                let collected = incoming
                    .collect()
                    .await
                    .map_err(|error| RsqError::internal(format!("failed to read request body: {error}")))?;
                let bytes = collected.to_bytes();
                if bytes.len() > self.max_body_size {
                    return Err(RsqError::new(
                        http::StatusCode::PAYLOAD_TOO_LARGE,
                        format!("payload exceeded {} bytes", self.max_body_size),
                    ));
                }
                Ok(bytes)
            }
            RsqRequestBody::Buffered(bytes) => {
                if bytes.len() > self.max_body_size {
                    return Err(RsqError::new(
                        http::StatusCode::PAYLOAD_TOO_LARGE,
                        format!("payload exceeded {} bytes", self.max_body_size),
                    ));
                }
                Ok(bytes)
            }
            RsqRequestBody::Consumed => Err(RsqError::bad_request("request body already consumed")),
        }
    }
}
