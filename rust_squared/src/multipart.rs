//! Basic multipart/form-data parsing.
//!
//! Extracts form fields and file uploads from multipart request bodies.

use bytes::Bytes;

use crate::error::RsqError;
use crate::extract::FromRequest;
use crate::request::RequestContext;

/// A single part of a multipart form submission.
#[derive(Debug, Clone)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Bytes,
}

/// Parsed multipart form data.
#[derive(Debug, Clone, Default)]
pub struct Multipart {
    parts: Vec<Part>,
}

impl Multipart {
    /// Get all parts.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// Get a field by name (first match).
    pub fn field(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.name == name)
    }

    /// Get field value as string.
    pub fn field_str(&self, name: &str) -> Option<&str> {
        self.field(name)
            .and_then(|p| std::str::from_utf8(&p.data).ok())
    }

    /// Get all file uploads (parts with a filename).
    pub fn files(&self) -> Vec<&Part> {
        self.parts.iter().filter(|p| p.filename.is_some()).collect()
    }

    /// Parse multipart body from request context.
    pub async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
        let boundary = ctx
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|ct| extract_boundary(ct))
            .ok_or_else(|| RsqError::bad_request("missing or invalid multipart boundary"))?
            .to_string();

        let body = ctx.take_body_bytes().await?;
        parse_multipart(&body, &boundary)
    }
}

impl FromRequest for Multipart {
    async fn from_request<'a>(ctx: &'a mut RequestContext) -> Result<Self, RsqError>
    where
        Self: 'a,
    {
        Multipart::from_request(ctx).await
    }
}

fn extract_boundary(content_type: &str) -> Option<&str> {
    if !content_type.to_ascii_lowercase().contains("multipart/form-data") {
        return None;
    }
    content_type
        .split(';')
        .find_map(|param| {
            let param = param.trim();
            param.strip_prefix("boundary=")
                .map(|b| b.trim_matches('"'))
        })
}

fn parse_multipart(body: &[u8], boundary: &str) -> Result<Multipart, RsqError> {
    let delimiter = format!("--{boundary}");
    let end_delimiter = format!("--{boundary}--");
    let body_str = String::from_utf8_lossy(body);

    let mut parts = Vec::new();

    for section in body_str.split(&delimiter) {
        let section = section.trim_start_matches("\r\n").trim_end_matches("\r\n");
        if section.is_empty() || section == "--" || section.starts_with("--") {
            continue;
        }

        // Split headers from body at double CRLF
        let (headers_part, data_part) = if let Some(pos) = section.find("\r\n\r\n") {
            (&section[..pos], &section[pos + 4..])
        } else if let Some(pos) = section.find("\n\n") {
            (&section[..pos], &section[pos + 2..])
        } else {
            continue;
        };

        // Remove trailing end delimiter from data
        let data_part = data_part
            .trim_end_matches("\r\n")
            .trim_end_matches(&end_delimiter)
            .trim_end_matches("\r\n");

        let mut name = None;
        let mut filename = None;
        let mut content_type = None;

        for line in headers_part.lines() {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-disposition:") {
                for param in line.split(';') {
                    let param = param.trim();
                    if let Some(n) = param.strip_prefix("name=") {
                        name = Some(n.trim_matches('"').to_string());
                    } else if let Some(f) = param.strip_prefix("filename=") {
                        filename = Some(f.trim_matches('"').to_string());
                    }
                }
            } else if lower.starts_with("content-type:") {
                content_type = Some(line["content-type:".len()..].trim().to_string());
            }
        }

        if let Some(name) = name {
            parts.push(Part {
                name,
                filename,
                content_type,
                data: Bytes::from(data_part.to_string()),
            });
        }
    }

    Ok(Multipart { parts })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_boundary_from_content_type() {
        let ct = "multipart/form-data; boundary=----WebKitFormBoundary7MA4YWxk";
        assert_eq!(extract_boundary(ct), Some("----WebKitFormBoundary7MA4YWxk"));
    }

    #[test]
    fn extract_boundary_quoted() {
        let ct = r#"multipart/form-data; boundary="abc123""#;
        assert_eq!(extract_boundary(ct), Some("abc123"));
    }

    #[test]
    fn extract_boundary_wrong_type() {
        assert_eq!(extract_boundary("application/json"), None);
    }

    #[test]
    fn parse_simple_multipart() {
        let body = "------boundary\r\nContent-Disposition: form-data; name=\"field1\"\r\n\r\nvalue1\r\n------boundary\r\nContent-Disposition: form-data; name=\"field2\"\r\n\r\nvalue2\r\n------boundary--\r\n";
        let result = parse_multipart(body.as_bytes(), "----boundary").unwrap();
        assert_eq!(result.parts().len(), 2);
        assert_eq!(result.field_str("field1"), Some("value1"));
        assert_eq!(result.field_str("field2"), Some("value2"));
    }

    #[test]
    fn parse_file_upload() {
        let body = "------boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: text/plain\r\n\r\nfile content here\r\n------boundary--\r\n";
        let result = parse_multipart(body.as_bytes(), "----boundary").unwrap();
        let files = result.files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename.as_deref(), Some("test.txt"));
        assert_eq!(files[0].content_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn empty_multipart() {
        let body = "------boundary--\r\n";
        let result = parse_multipart(body.as_bytes(), "----boundary").unwrap();
        assert!(result.parts().is_empty());
    }
}

use crate::extract::FromRequest;
use async_trait::async_trait;

#[async_trait]
impl FromRequest for Multipart {
    type Error = RsqError;

    async fn from_request(ctx: &mut RequestContext) -> Result<Self, Self::Error> {
        Multipart::from_request(ctx).await
    }
}

use crate::extract::FromRequest;
use async_trait::async_trait;

#[async_trait]
impl FromRequest for Multipart {
    type Error = RsqError;

    async fn from_request(ctx: &mut RequestContext) -> Result<Self, Self::Error> {
        Multipart::from_request(ctx).await
    }
}
