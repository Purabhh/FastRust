//! Basic multipart/form-data parsing.
//!
//! Extracts form fields and file uploads from multipart request bodies.

use async_trait::async_trait;
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

#[async_trait]
impl FromRequest for Multipart {
    async fn from_request(ctx: &mut RequestContext) -> Result<Self, RsqError> {
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

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Maximum number of parts accepted from a single multipart body.
/// fix(M-1): prevents unbounded memory use from maliciously crafted requests.
const MAX_PARTS: usize = 100;

fn parse_multipart(body: &[u8], boundary: &str) -> Result<Multipart, RsqError> {
    let delimiter = format!("--{boundary}");
    let end_delimiter = format!("--{boundary}--");
    let delim_bytes = delimiter.as_bytes();
    let end_delim_bytes = end_delimiter.as_bytes();

    let mut parts = Vec::new();
    let mut search_start = 0usize;

    // Find all delimiter positions by byte searching
    // fix(M-1): loop now breaks when find_subsequence returns None (boundary not found)
    // instead of looping forever; also enforces MAX_PARTS limit.
    while let Some(p) = find_subsequence(&body[search_start..], delim_bytes) {
        // fix(M-1): enforce max parts limit to prevent resource exhaustion
        if parts.len() >= MAX_PARTS {
            break;
        }
        let delim_pos = search_start + p;

        // Check if this is the closing delimiter
        let after_delim = delim_pos + delim_bytes.len();
        if body.get(after_delim..after_delim + 2) == Some(b"--") {
            break;
        }

        // Skip past the delimiter and the following CRLF or LF
        let section_start = if body.get(after_delim..after_delim + 2) == Some(b"\r\n") {
            after_delim + 2
        } else if body.get(after_delim..after_delim + 1) == Some(b"\n") {
            after_delim + 1
        } else {
            after_delim
        };

        // Find where this section ends: next occurrence of delimiter
        let section_end = match find_subsequence(&body[section_start..], delim_bytes) {
            Some(p) => section_start + p,
            None => body.len(),
        };

        // Trim trailing CRLF before the next delimiter
        let section = &body[section_start..section_end];
        let section = if section.ends_with(b"\r\n") {
            &section[..section.len() - 2]
        } else if section.ends_with(b"\n") {
            &section[..section.len() - 1]
        } else {
            section
        };

        // Check if the section is just the end delimiter suffix
        if section == b"--" || section.starts_with(end_delim_bytes) {
            search_start = section_end;
            continue;
        }

        // Split headers from body at double CRLF (headers are ASCII)
        let (headers_bytes, data_bytes) =
            if let Some(pos) = find_subsequence(section, b"\r\n\r\n") {
                (&section[..pos], &section[pos + 4..])
            } else if let Some(pos) = find_subsequence(section, b"\n\n") {
                (&section[..pos], &section[pos + 2..])
            } else {
                search_start = section_end;
                continue;
            };

        // Parse headers as ASCII strings
        let headers_str = match std::str::from_utf8(headers_bytes) {
            Ok(s) => s,
            Err(_) => {
                search_start = section_end;
                continue;
            }
        };

        let mut name = None;
        let mut filename = None;
        let mut content_type = None;

        for line in headers_str.lines() {
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
            // Keep part data as raw bytes — never convert through String
            parts.push(Part {
                name,
                filename,
                content_type,
                data: Bytes::copy_from_slice(data_bytes),
            });
        }

        // fix(M-1): if section_end didn't advance search_start, force advancement
        // to prevent an infinite loop on malformed input.
        if section_end <= search_start {
            break;
        }
        search_start = section_end;
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

    #[test]
    fn parse_binary_file_upload_preserves_bytes() {
        // Binary data that would be corrupted by UTF-8 round-tripping
        let binary_data: &[u8] = &[0xFF, 0xFE, 0x00, 0x01];
        let mut body = Vec::new();
        body.extend_from_slice(b"------boundary\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"bin\"; filename=\"data.bin\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n");
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(binary_data);
        body.extend_from_slice(b"\r\n------boundary--\r\n");

        let result = parse_multipart(&body, "----boundary").unwrap();
        assert_eq!(result.parts().len(), 1);
        let part = &result.parts()[0];
        assert_eq!(part.name, "bin");
        assert_eq!(part.filename.as_deref(), Some("data.bin"));
        // Verify no byte was silently replaced — exact match required
        assert_eq!(&part.data[..], binary_data);
    }
}
