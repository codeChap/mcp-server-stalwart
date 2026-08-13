//! Small shared helpers used by MCP tool handlers.

use rmcp::model::{CallToolResult, Content};
use serde::Serialize;
use std::time::Duration;

pub const USER_AGENT: &str = concat!("mcp-server-stalwart/", env!("CARGO_PKG_VERSION"));

pub fn http_client(connect_secs: u64, timeout_secs: u64) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(connect_secs))
        .timeout(Duration::from_secs(timeout_secs))
        .build()
}

/// Generate a random password from `/dev/urandom`.
///
/// Uses a 64-character alphanumeric charset so modulo distribution is uniform.
pub fn generate_password(len: usize) -> std::io::Result<String> {
    use std::io::Read;
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    let mut bytes = vec![0u8; len];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes
        .iter()
        .map(|&b| CHARS[(b as usize) % CHARS.len()] as char)
        .collect())
}

/// Guess a MIME type from a filename extension.
pub fn guess_mime(filename: &str) -> String {
    match filename
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("txt") => "text/plain",
        Some("html" | "htm") => "text/html",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("zip") => "application/zip",
        Some("gz" | "gzip") => "application/gzip",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Pretty-print a serializable value as a successful MCP tool result.
pub fn tool_success(value: &impl Serialize) -> CallToolResult {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    CallToolResult::success(vec![Content::text(text)])
}

/// Convert a `Result` into a success or error MCP tool result.
pub fn tool_result(result: Result<impl Serialize, impl ToString>) -> CallToolResult {
    match result {
        Ok(value) => tool_success(&value),
        Err(e) => CallToolResult::error(vec![Content::text(e.to_string())]),
    }
}

/// Plain-text success result (no JSON wrapping).
pub fn tool_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text.into())])
}

/// Plain-text error result.
pub fn tool_error(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![Content::text(text.into())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guess_mime_common_types() {
        assert_eq!(guess_mime("report.PDF"), "application/pdf");
        assert_eq!(guess_mime("photo.jpeg"), "image/jpeg");
        assert_eq!(guess_mime("unknown.xyz"), "application/octet-stream");
        assert_eq!(guess_mime("noext"), "application/octet-stream");
    }

    #[test]
    fn generate_password_length() {
        let pw = generate_password(24).expect("urandom available");
        assert_eq!(pw.len(), 24);
        assert!(
            pw.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }
}
