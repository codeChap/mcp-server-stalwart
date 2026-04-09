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

use crate::admin::AdminClient;
use crate::jmap::{EmailAttachment, JmapClient};
use crate::sieve;

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

    #[schemars(description = "Email body as HTML (optional). When provided, the email is sent as multipart with both plain text and HTML parts.")]
    pub html_body: Option<String>,

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
pub struct DownloadAttachmentsParams {
    #[schemars(description = "Email ID to download attachments from")]
    pub email_id: String,

    #[schemars(description = "Directory path to save attachments to")]
    pub download_dir: String,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateDomainParams {
    #[schemars(description = "Domain name to add (e.g. 'postchap.com')")]
    pub domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateAccountParams {
    #[schemars(description = "Primary email address for the account (e.g. 'hello@postchap.com')")]
    pub email: String,

    #[schemars(description = "Account password")]
    pub password: String,

    #[schemars(description = "Display name (e.g. 'Derrick Egersdorfer')")]
    pub description: Option<String>,

    #[schemars(description = "Disk quota in bytes (0 for unlimited)")]
    pub quota: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAccountsParams {
    #[schemars(description = "Account name to get details for. If omitted, lists all accounts.")]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ManageAliasesParams {
    #[schemars(description = "Account name (e.g. 'hello@codechap.com')")]
    pub account: String,

    #[schemars(description = "Action: 'add' or 'remove'")]
    pub action: String,

    #[schemars(description = "Email alias to add or remove (e.g. 'derrick@codechap.com')")]
    pub alias: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetDsnAccountsParams {
    #[schemars(
        description = "Email addresses that should get delivery reports (SUCCESS + FAILURE DSN). Must have at least one address."
    )]
    pub accounts: Vec<String>,
}

#[derive(Clone)]
pub struct StalwartServer {
    client: Arc<JmapClient>,
    admin: Option<Arc<AdminClient>>,
    tool_router: ToolRouter<Self>,
}

impl StalwartServer {
    fn require_admin(&self) -> Result<&AdminClient, McpError> {
        self.admin.as_deref().ok_or_else(|| {
            McpError::internal_error(
                "Admin API not configured. Set STALWART_ADMIN_URL and STALWART_ADMIN_PASSWORD env vars.",
                None,
            )
        })
    }
}

#[tool_router]
impl StalwartServer {
    pub fn new(client: JmapClient, admin: Option<AdminClient>) -> Self {
        Self {
            client: Arc::new(client),
            admin: admin.map(Arc::new),
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

    #[tool(description = "Download all attachments from an email to a local directory. \
                           Returns the list of saved file paths.")]
    async fn download_attachments(
        &self,
        Parameters(p): Parameters<DownloadAttachmentsParams>,
    ) -> Result<CallToolResult, McpError> {
        // Ensure download directory exists
        tokio::fs::create_dir_all(&p.download_dir).await.map_err(|e| {
            McpError::invalid_params(format!("cannot create directory '{}': {}", p.download_dir, e), None)
        })?;

        // Get attachment metadata for the email
        let meta = self
            .client
            .get_email_attachments(&[p.email_id.clone()])
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let email = meta["list"]
            .as_array()
            .and_then(|list| list.first())
            .ok_or_else(|| McpError::invalid_params("email not found", None))?;

        let attachments = match email["attachments"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                return Ok(CallToolResult::success(vec![Content::text(
                    "No attachments found on this email.",
                )]));
            }
        };

        let mut saved: Vec<serde_json::Value> = Vec::new();
        for att in attachments {
            let blob_id = att["blobId"].as_str().unwrap_or_default();
            let name = att["name"]
                .as_str()
                .unwrap_or("attachment");
            let content_type = att["type"]
                .as_str()
                .unwrap_or("application/octet-stream");
            let size = att["size"].as_u64().unwrap_or(0);

            if blob_id.is_empty() {
                continue;
            }

            let data = self
                .client
                .download_blob(blob_id, name, content_type)
                .await
                .map_err(|e| {
                    McpError::internal_error(
                        format!("failed to download '{}': {}", name, e),
                        None,
                    )
                })?;

            let dest = std::path::Path::new(&p.download_dir).join(name);
            tokio::fs::write(&dest, &data).await.map_err(|e| {
                McpError::internal_error(
                    format!("failed to write '{}': {}", dest.display(), e),
                    None,
                )
            })?;

            saved.push(json!({
                "filename": name,
                "path": dest.display().to_string(),
                "content_type": content_type,
                "size": size
            }));
        }

        let text = serde_json::to_string_pretty(&saved).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "List all domains configured on the server. Requires admin API.")]
    async fn list_domains(&self) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        match admin.list_principals("domain").await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Add a domain to the server so it can receive email. Requires admin API.")]
    async fn create_domain(
        &self,
        Parameters(p): Parameters<CreateDomainParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let principal = json!({
            "type": "domain",
            "name": p.domain
        });

        match admin.create_account(principal).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&json!({
                    "status": "created",
                    "domain": p.domain,
                    "result": result
                }))
                .unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Create a new email account on the server. Requires admin API.")]
    async fn create_account(
        &self,
        Parameters(p): Parameters<CreateAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let mut principal = json!({
            "type": "individual",
            "name": p.email,
            "emails": [p.email],
            "secrets": [p.password],
            "quota": p.quota.unwrap_or(0)
        });

        if let Some(desc) = &p.description {
            principal["description"] = json!(desc);
        }

        match admin.create_account(principal).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&json!({
                    "status": "created",
                    "account": p.email,
                    "description": p.description,
                    "result": result
                }))
                .unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "List all email accounts on the server, or get details for a specific account by name. Requires admin API.")]
    async fn list_accounts(
        &self,
        Parameters(p): Parameters<ListAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        if let Some(name) = &p.name {
            match admin.get_account(name).await {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                    Ok(CallToolResult::success(vec![Content::text(text)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
            }
        } else {
            match admin.list_accounts().await {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                    Ok(CallToolResult::success(vec![Content::text(text)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
            }
        }
    }

    #[tool(description = "Add or remove an email alias on an account. Requires admin API.")]
    async fn manage_aliases(
        &self,
        Parameters(p): Parameters<ManageAliasesParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let action = p.action.to_lowercase();
        if action != "add" && action != "remove" {
            return Err(McpError::invalid_params("action must be 'add' or 'remove'", None));
        }

        let op = if action == "add" { "addItem" } else { "removeItem" };

        let changes = vec![json!({
            "action": op,
            "field": "emails",
            "value": p.alias
        })];

        admin
            .update_account(&p.account, changes)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        // Fetch updated account to confirm
        let updated = admin
            .get_account(&p.account)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let emails = &updated["emails"];

        let text = serde_json::to_string_pretty(&json!({
            "status": action,
            "alias": p.alias,
            "account": p.account,
            "emails": emails
        }))
        .unwrap_or_default();

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "List email addresses that have DSN delivery reports enabled (requires admin API)")]
    async fn get_dsn_accounts(&self) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let settings = admin
            .get_settings("sieve.trusted.scripts.dsn-notify")
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let script = settings["contents"].as_str().unwrap_or_default();

        if script.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No DSN notify script configured. No accounts have delivery reports enabled.",
            )]));
        }

        match sieve::parse_dsn_script(script) {
            Ok(accounts) => {
                let text = serde_json::to_string_pretty(&accounts).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Set which email addresses get DSN delivery reports (SUCCESS + FAILURE). \
                           Replaces the full list. Requires admin API.")]
    async fn set_dsn_accounts(
        &self,
        Parameters(p): Parameters<SetDsnAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        if p.accounts.is_empty() {
            return Err(McpError::invalid_params(
                "Provide at least one email address. To disable DSN entirely, remove the script via the admin panel.",
                None,
            ));
        }

        // Check current script for manual customisations before overwriting
        let settings = admin
            .get_settings("sieve.trusted.scripts.dsn-notify")
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let current = settings["contents"].as_str().unwrap_or_default();

        if !current.is_empty() {
            if let Err(e) = sieve::parse_dsn_script(current) {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Current script has custom modifications that would be lost:\n{e}\n\n\
                     Remove or fix the script manually via the admin panel before using this tool."
                ))]));
            }
        }

        // Generate and apply the new script
        let new_script = sieve::generate_dsn_script(&p.accounts)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        admin
            .set_settings(vec![(
                "sieve.trusted.scripts.dsn-notify.contents".to_string(),
                new_script,
            )])
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&json!({
            "status": "updated",
            "accounts": p.accounts
        }))
        .unwrap_or_default();

        Ok(CallToolResult::success(vec![Content::text(text)]))
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

        match self.client.send_email(from, &p.to, &p.subject, &p.body, p.html_body.as_deref(), &cc, &bcc, &uploaded).await {
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
                "Stalwart mail server MCP. Tools: get_mailboxes, create_mailbox, search_emails, get_emails, delete_emails, send_email, download_attachments. \
                 Search returns email IDs; use get_emails to read content, delete_emails to remove them, or download_attachments to save attachments to disk. \
                 Admin tools (require STALWART_ADMIN_URL/PASSWORD): create_account creates a new account, \
                 list_accounts lists all accounts or gets details for one, \
                 manage_aliases adds/removes email aliases on an account, \
                 get_dsn_accounts lists addresses with delivery reports enabled, set_dsn_accounts updates the list."
            )
    }
}
