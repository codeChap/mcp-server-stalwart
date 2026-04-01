use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::jmap::{EmailAttachment, JmapClient};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    #[schemars(description = "Text to search for in email subject, body, from, to fields")]
    pub query: Option<String>,

    #[schemars(description = "Filter by sender email address")]
    pub from: Option<String>,

    #[schemars(description = "Filter by recipient email address")]
    pub to: Option<String>,

    #[schemars(description = "Filter by subject text")]
    pub subject: Option<String>,

    #[schemars(description = "Mailbox ID to search within")]
    pub mailbox_id: Option<String>,

    #[schemars(description = "Start position for pagination (default 0)")]
    pub position: Option<u32>,

    #[schemars(description = "Maximum results to return (default 10, max 50)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetEmailsParams {
    #[schemars(description = "List of email IDs to retrieve")]
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteEmailsParams {
    #[schemars(description = "List of email IDs to delete")]
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendEmailParams {
    #[schemars(description = "Recipient email addresses")]
    pub to: Vec<String>,

    #[schemars(description = "Email subject")]
    pub subject: String,

    #[schemars(description = "Email body (plain text)")]
    pub body: String,

    #[schemars(description = "CC recipients (optional)")]
    pub cc: Option<Vec<String>>,

    #[schemars(description = "BCC recipients (optional)")]
    pub bcc: Option<Vec<String>>,

    #[schemars(description = "File attachments (optional). Each attachment needs a file path and filename.")]
    pub attachments: Option<Vec<AttachmentParam>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachmentParam {
    #[schemars(description = "Absolute path to the file on disk")]
    pub path: String,

    #[schemars(description = "Filename for the attachment (e.g., 'report.pdf')")]
    pub filename: String,

    #[schemars(description = "MIME type (e.g., 'application/pdf', 'image/png'). Auto-detected from extension if omitted.")]
    pub content_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateMailboxParams {
    #[schemars(description = "Name of the mailbox to create")]
    pub name: String,

    #[schemars(description = "Parent mailbox ID for nesting (optional, top-level if omitted)")]
    pub parent_id: Option<String>,

    #[schemars(description = "Mailbox role (optional). Standard roles: archive, drafts, inbox, junk, sent, trash")]
    pub role: Option<String>,
}

#[derive(Clone)]
pub struct StalwartServer {
    client: Arc<JmapClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl StalwartServer {
    pub fn new(client: JmapClient) -> Self {
        Self {
            client: Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List all mailboxes/folders with message counts")]
    async fn get_mailboxes(&self) -> Result<CallToolResult, McpError> {
        match self.client.get_mailboxes().await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Search emails with filters (query text, from, to, subject, mailbox). \
                           Returns email IDs — use get_emails to read full content.")]
    async fn search_emails(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conditions: Vec<serde_json::Value> = Vec::new();

        if let Some(q) = &p.query {
            conditions.push(json!({"text": q}));
        }
        if let Some(from) = &p.from {
            conditions.push(json!({"from": from}));
        }
        if let Some(to) = &p.to {
            conditions.push(json!({"to": to}));
        }
        if let Some(subject) = &p.subject {
            conditions.push(json!({"subject": subject}));
        }
        if let Some(mailbox_id) = &p.mailbox_id {
            conditions.push(json!({"inMailbox": mailbox_id}));
        }

        let filter = if conditions.len() == 1 {
            conditions.remove(0)
        } else if conditions.is_empty() {
            json!({})
        } else {
            json!({"operator": "AND", "conditions": conditions})
        };

        let position = p.position.unwrap_or(0);
        let limit = p.limit.unwrap_or(10).min(50);

        match self.client.search_emails(filter, None, position, limit).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Get full email content by IDs. Returns subject, from, to, date, \
                           body text, and metadata for each email.")]
    async fn get_emails(
        &self,
        Parameters(p): Parameters<GetEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.is_empty() {
            return Err(McpError::invalid_params("ids must not be empty", None));
        }
        match self.client.get_emails(&p.ids).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Permanently delete emails by ID. This cannot be undone.")]
    async fn delete_emails(
        &self,
        Parameters(p): Parameters<DeleteEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.is_empty() {
            return Err(McpError::invalid_params("ids must not be empty", None));
        }
        match self.client.delete_emails(&p.ids).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Create a new mailbox/folder. Optionally set a role (archive, drafts, junk, sent, trash) \
                           or nest under a parent mailbox.")]
    async fn create_mailbox(
        &self,
        Parameters(p): Parameters<CreateMailboxParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .client
            .create_mailbox(&p.name, p.parent_id.as_deref(), p.role.as_deref())
            .await
        {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Send an email with optional file attachments")]
    async fn send_email(
        &self,
        Parameters(p): Parameters<SendEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.to.is_empty() {
            return Err(McpError::invalid_params("to must not be empty", None));
        }
        let from = self.client.username();
        let cc = p.cc.unwrap_or_default();
        let bcc = p.bcc.unwrap_or_default();

        let mut uploaded: Vec<EmailAttachment> = Vec::new();
        for att in p.attachments.unwrap_or_default() {
            let data = tokio::fs::read(&att.path).await.map_err(|e| {
                McpError::invalid_params(format!("failed to read '{}': {}", att.path, e), None)
            })?;
            let content_type = att
                .content_type
                .unwrap_or_else(|| guess_mime(&att.filename));
            let blob = self.client.upload_blob(data, &content_type).await.map_err(|e| {
                McpError::internal_error(format!("upload failed for '{}': {}", att.filename, e), None)
            })?;
            uploaded.push(EmailAttachment {
                blob_id: blob.blob_id,
                content_type,
                filename: att.filename,
                size: blob.size,
            });
        }

        match self.client.send_email(from, &p.to, &p.subject, &p.body, &cc, &bcc, &uploaded).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }
}

fn guess_mime(filename: &str) -> String {
    match filename.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
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

#[tool_handler]
impl ServerHandler for StalwartServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("stalwart", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Stalwart mail server MCP. Tools: get_mailboxes, create_mailbox, search_emails, get_emails, delete_emails, send_email. \
                 Search returns email IDs; use get_emails to read content or delete_emails to remove them."
            )
    }
}
