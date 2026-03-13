use std::path::PathBuf;

use crate::{Handler, Method, RequestContext, Response, RsqBody, RsqError};

use tokio::fs::File;
use tokio::io::AsyncReadExt;

pub struct ServeDir {
    dir: PathBuf,
}

impl ServeDir {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

#[async_trait::async_trait]
impl Handler<()> for ServeDir {
    async fn call(&self, ctx: RequestContext) -> Result<Response, RsqError> {
        let path = ctx.params.get("file").ok_or(RsqError::NotFound)?;
        let full_path = self.dir.join(path);
        if !full_path.starts_with(&self.dir) {
            return Err(RsqError::NotFound);
        }

        let mut file = File::open(&full_path).await.map_err(|_| RsqError::NotFound)?;

        let mut contents = Vec::new();
        file.read_to_end(&mut contents).await.map_err(|e| RsqError::internal(e.to_string()))?;

        let mime = mime_from_extension(full_path.extension().and_then(|s| s.to_str()).unwrap_or(""));

        Response::builder()
            .header("Content-Type", mime)
            .body(RsqBody::new_full(contents.into()))
            .map_err(|e| RsqError::internal(e.to_string()))
    }
}

fn mime_from_extension(ext: &str) -> &str {
    match ext {
        "html" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}