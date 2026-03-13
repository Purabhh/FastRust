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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_handles_all_special_chars() {
        assert_eq!(
            html_escape("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"
        );
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
