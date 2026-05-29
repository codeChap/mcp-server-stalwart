use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
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

    #[schemars(description = "Email account to search (e.g. 'portal@excellerate.site'). Omit to use the default account. Requires admin API.")]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetEmailsParams {
    #[schemars(description = "List of email IDs to retrieve")]
    pub ids: Vec<String>,

    #[schemars(description = "Email account that owns these emails (e.g. 'portal@excellerate.site'). Omit to use the default account. Requires admin API.")]
    pub account: Option<String>,
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

    #[schemars(description = "Permissions to grant at creation (e.g. ['email-send', 'authenticate', 'imap-authenticate']). Newly-created principals start with no permissions and cannot authenticate, send, or receive mail until permissions are granted. Use update_account_permissions afterwards if omitted here.")]
    pub permissions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateAccountPermissionsParams {
    #[schemars(description = "Account name (e.g. 'hello@codechap.com')")]
    pub account: String,

    #[schemars(description = "Action: 'set' replaces the full enabledPermissions list; 'add' grants the listed permissions; 'remove' revokes them. Default 'set'.")]
    pub action: Option<String>,

    #[schemars(description = "Permission names to set, add, or remove (e.g. ['email-send', 'authenticate', 'imap-authenticate', 'imap-append']).")]
    pub permissions: Vec<String>,
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
pub struct ResetPasswordParams {
    #[schemars(description = "Account name (e.g. 'hello@codechap.com')")]
    pub account: String,

    #[schemars(description = "New password. If omitted, a strong 24-character random password is generated and returned.")]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetDsnAccountsParams {
    #[schemars(
        description = "Email addresses that should get delivery reports (SUCCESS + FAILURE DSN). Must have at least one address."
    )]
    pub accounts: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckSentParams {
    #[schemars(
        description = "Recipient email or domain to look up (substring match, e.g. 'alice@example.com' or 'example.com')"
    )]
    pub to: Option<String>,

    #[schemars(description = "Sender email or domain to look up (substring match)")]
    pub from: Option<String>,

    #[schemars(
        description = "Free-text filter passed straight to the log search — overrides 'to' and 'from' when set"
    )]
    pub filter: Option<String>,

    #[schemars(
        description = "Only show log events at or after this RFC3339 timestamp (e.g. '2026-05-08T12:00:00Z')"
    )]
    pub since: Option<String>,

    #[schemars(
        description = "Number of raw log rows to scan from the server (default 200, max 1000). Filtering and de-duplication happen client-side after the fetch."
    )]
    pub scan_limit: Option<u32>,
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

    /// Create a temporary JMAP client authenticated as a different account.
    /// Looks up the account's password via the admin API.
    async fn client_for_account(&self, account: &str) -> Result<JmapClient, McpError> {
        let admin = self.require_admin()?;
        let details = admin
            .get_account(account)
            .await
            .map_err(|e| McpError::internal_error(format!("failed to look up account '{}': {}", account, e), None))?;

        let password = details["secrets"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::internal_error(format!("no password found for account '{}'", account), None))?;

        JmapClient::connect(self.client.session_url(), account, password)
            .await
            .map_err(|e| McpError::internal_error(format!("failed to connect as '{}': {}", account, e), None))
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

        let client: &JmapClient;
        let temp_client;
        if let Some(acct) = &p.account {
            temp_client = self.client_for_account(acct).await?;
            client = &temp_client;
        } else {
            client = &self.client;
        }

        match client.search_emails(filter, None, position, limit).await {
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

        let client: &JmapClient;
        let temp_client;
        if let Some(acct) = &p.account {
            temp_client = self.client_for_account(acct).await?;
            client = &temp_client;
        } else {
            client = &self.client;
        }

        match client.get_emails(&p.ids).await {
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

        if let Some(perms) = &p.permissions {
            principal["enabledPermissions"] = json!(perms);
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

    #[tool(description = "Update an account's enabledPermissions. Newly-created principals start with no permissions \
                           and cannot authenticate, send, or receive mail until permissions are granted. \
                           Actions: 'set' replaces the list, 'add' grants, 'remove' revokes. Requires admin API.")]
    async fn update_account_permissions(
        &self,
        Parameters(p): Parameters<UpdateAccountPermissionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let action = p.action.as_deref().unwrap_or("set").to_lowercase();

        let changes: Vec<serde_json::Value> = match action.as_str() {
            "set" => vec![json!({
                "action": "set",
                "field": "enabledPermissions",
                "value": p.permissions,
            })],
            "add" => p.permissions.iter().map(|perm| json!({
                "action": "addItem",
                "field": "enabledPermissions",
                "value": perm,
            })).collect(),
            "remove" => p.permissions.iter().map(|perm| json!({
                "action": "removeItem",
                "field": "enabledPermissions",
                "value": perm,
            })).collect(),
            _ => return Err(McpError::invalid_params(
                "action must be 'set', 'add', or 'remove'",
                None,
            )),
        };

        if changes.is_empty() {
            return Err(McpError::invalid_params(
                "permissions must not be empty for add/remove actions",
                None,
            ));
        }

        admin
            .update_account(&p.account, changes)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let updated = admin
            .get_account(&p.account)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&json!({
            "status": "ok",
            "action": action,
            "account": p.account,
            "enabledPermissions": updated["enabledPermissions"],
        }))
        .unwrap_or_default();

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Reset an account's password. If 'password' is omitted, a strong random password is generated. \
                           The new password is returned in plaintext so it can be delivered to the user — handle the response carefully. \
                           Requires admin API.")]
    async fn reset_password(
        &self,
        Parameters(p): Parameters<ResetPasswordParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let new_password = match p.password {
            Some(pw) if !pw.is_empty() => pw,
            _ => generate_password(24).map_err(|e| {
                McpError::internal_error(format!("failed to generate password: {e}"), None)
            })?,
        };

        let changes = vec![json!({
            "action": "set",
            "field": "secrets",
            "value": [&new_password],
        })];

        admin
            .update_account(&p.account, changes)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let text = serde_json::to_string_pretty(&json!({
            "status": "reset",
            "account": p.account,
            "password": new_password,
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

    #[tool(
        description = "Verify whether an email was sent or delivered by inspecting Stalwart's server logs. \
                       This is the authoritative answer — searching mailboxes alone misses outbound traffic \
                       (SMTP submissions are not auto-saved to the sender's Sent folder). Returns delivery \
                       status events (delivery.delivered, delivery.completed, delivery.failed, etc.) plus \
                       SMTP submission events (smtp.rcpt-to, queue.queue-message). Filter by recipient/sender \
                       substring and optional cutoff timestamp."
    )]
    async fn check_sent(
        &self,
        Parameters(p): Parameters<CheckSentParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        // Pick a single substring filter for the server-side query. If multiple are provided,
        // we use the most specific (filter > to > from) for the fetch and apply the others
        // client-side so the user gets the intersection.
        let primary_filter: Option<String> = p
            .filter
            .clone()
            .or_else(|| p.to.clone())
            .or_else(|| p.from.clone());

        let scan_limit = p.scan_limit.unwrap_or(200).clamp(1, 1000);

        let raw = admin
            .get_logs(primary_filter.as_deref(), scan_limit)
            .await
            .map_err(|e| McpError::internal_error(format!("log fetch failed: {e}"), None))?;

        let items = raw["items"].as_array().cloned().unwrap_or_default();

        let to_needle = p.to.as_deref().map(|s| s.to_lowercase());
        let from_needle = p.from.as_deref().map(|s| s.to_lowercase());
        let since_cutoff = p.since.as_deref();

        // Subset of event_ids that prove submission attempt or delivery outcome.
        let relevant_event_prefixes = [
            "delivery.",        // attempt-end, completed, delivered, failed, dsn-success, dsn-failure
            "queue.",           // queue-message, message-locked, etc.
            "smtp.rcpt-to",     // submission RCPT acceptance
            "smtp.mail-from",   // submission MAIL FROM acceptance
            "smtp.message-",    // smtp.message-accepted, smtp.message-rejected
            "smtp.data",        // smtp.data-end-of-message etc.
            "outgoing-report.", // outbound DSN reports
            "tls-rpt.",
            "mta-sts.",
        ];

        let mut matched: Vec<&Value> = Vec::new();
        for item in &items {
            let event_id = item["event_id"].as_str().unwrap_or("").to_lowercase();
            let details = item["details"].as_str().unwrap_or("").to_lowercase();
            let timestamp = item["timestamp"].as_str().unwrap_or("");

            // Time filter
            if let Some(cutoff) = since_cutoff {
                if !timestamp.is_empty() && timestamp < cutoff {
                    continue;
                }
            }

            // Only events relevant to outbound submission/delivery
            if !relevant_event_prefixes
                .iter()
                .any(|prefix| event_id.starts_with(prefix))
            {
                continue;
            }

            // Apply secondary client-side substring filters
            if let Some(needle) = &to_needle {
                if !details.contains(needle) {
                    continue;
                }
            }
            if let Some(needle) = &from_needle {
                if !details.contains(needle) {
                    continue;
                }
            }

            matched.push(item);
        }

        // Roll up by queueId so a single send shows as one record with its event timeline
        use std::collections::BTreeMap;
        let queue_id_re = regex::Regex::new(r#"queueId = (\d+)"#).unwrap();
        let from_re = regex::Regex::new(r#"from = "([^"]+)""#).unwrap();
        let to_re = regex::Regex::new(r#"to = (?:\[([^\]]+)\]|"([^"]+)")"#).unwrap();
        let code_re = regex::Regex::new(r#"code = (\d+)"#).unwrap();
        let host_re = regex::Regex::new(r#"hostname = "([^"]+)""#).unwrap();

        // queue_id -> aggregated record
        let mut groups: BTreeMap<String, serde_json::Map<String, Value>> = BTreeMap::new();
        let mut orphans: Vec<Value> = Vec::new();

        for item in &matched {
            let details = item["details"].as_str().unwrap_or("");
            let timestamp = item["timestamp"].as_str().unwrap_or("");
            let event = item["event"].as_str().unwrap_or("");
            let event_id = item["event_id"].as_str().unwrap_or("");

            let qid = queue_id_re
                .captures(details)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string());

            let summary_event = json!({
                "timestamp": timestamp,
                "event_id": event_id,
                "event": event,
                "code": code_re.captures(details).and_then(|c| c.get(1)).map(|m| m.as_str()),
                "hostname": host_re.captures(details).and_then(|c| c.get(1)).map(|m| m.as_str()),
            });

            let Some(qid) = qid else {
                orphans.push(json!({
                    "timestamp": timestamp,
                    "event_id": event_id,
                    "event": event,
                    "details": details,
                }));
                continue;
            };

            let group = groups.entry(qid.clone()).or_insert_with(|| {
                let mut m = serde_json::Map::new();
                m.insert("queue_id".into(), json!(qid));
                m.insert(
                    "from".into(),
                    json!(from_re
                        .captures(details)
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str())),
                );
                m.insert(
                    "to".into(),
                    json!(to_re
                        .captures(details)
                        .and_then(|c| c.get(1).or(c.get(2)))
                        .map(|m| m.as_str().trim().trim_matches('"'))),
                );
                m.insert("events".into(), json!([]));
                m.insert("first_seen".into(), json!(timestamp));
                m.insert("last_seen".into(), json!(timestamp));
                m.insert("delivered".into(), json!(false));
                m.insert("failed".into(), json!(false));
                m
            });

            // Update first/last seen (logs come newest-first; we widen the range)
            let first = group
                .get("first_seen")
                .and_then(|v: &Value| v.as_str())
                .unwrap_or("")
                .to_string();
            let last = group
                .get("last_seen")
                .and_then(|v: &Value| v.as_str())
                .unwrap_or("")
                .to_string();
            if timestamp < first.as_str() || first.is_empty() {
                group.insert("first_seen".into(), json!(timestamp));
            }
            if timestamp > last.as_str() {
                group.insert("last_seen".into(), json!(timestamp));
            }

            if event_id == "delivery.delivered" || event_id == "delivery.completed" {
                group.insert("delivered".into(), json!(true));
            }
            if event_id == "delivery.failed" || event_id == "delivery.bounce" {
                group.insert("failed".into(), json!(true));
            }

            if let Some(events_arr) = group
                .get_mut("events")
                .and_then(|v: &mut Value| v.as_array_mut())
            {
                events_arr.push(summary_event);
            }
        }

        let messages: Vec<Value> = groups.into_values().map(Value::Object).collect();

        let total_delivered = messages.iter().filter(|m| m["delivered"].as_bool().unwrap_or(false)).count();
        let total_failed = messages.iter().filter(|m| m["failed"].as_bool().unwrap_or(false)).count();

        let summary = json!({
            "scanned_log_rows": items.len(),
            "primary_filter": primary_filter,
            "to_filter": p.to,
            "from_filter": p.from,
            "since": p.since,
            "messages_found": messages.len(),
            "delivered_count": total_delivered,
            "failed_count": total_failed,
            "messages": messages,
            "orphan_events": orphans,
        });

        let text = serde_json::to_string_pretty(&summary).unwrap_or_default();
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

/// Generate a random password from /dev/urandom.
///
/// Uses a 64-character alphanumeric charset so modulo distribution is uniform.
fn generate_password(len: usize) -> std::io::Result<String> {
    use std::io::Read;
    const CHARS: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    let mut bytes = vec![0u8; len];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes
        .iter()
        .map(|&b| CHARS[(b as usize) % CHARS.len()] as char)
        .collect())
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
                 Admin tools (require STALWART_ADMIN_URL/PASSWORD): create_account creates a new account \
                 (optionally with a permissions list — without permissions, new accounts cannot authenticate or submit mail), \
                 list_accounts lists all accounts or gets details for one, \
                 manage_aliases adds/removes email aliases on an account, \
                 update_account_permissions manages the enabledPermissions list on an account (set/add/remove), \
                 reset_password sets a new account password (auto-generates one if omitted), \
                 get_dsn_accounts lists addresses with delivery reports enabled, set_dsn_accounts updates the list."
            )
    }
}
