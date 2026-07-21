//! MCP tool surface for Stalwart.
//!
//! Thin handlers: params live in `params`, shared helpers in `util`,
//! outbound-log analysis in `check_sent`.

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
};
use serde_json::json;
use std::sync::Arc;

use crate::admin::AdminClient;
use crate::check_sent::{LogFilters, ScanMeta, analyze_logs};
use crate::jmap::{EmailAttachment, JmapClient};
use crate::params::*;
use crate::sieve;
use crate::util::{generate_password, guess_mime, tool_error, tool_result, tool_success, tool_text};

#[derive(Clone)]
pub struct StalwartServer {
    client: Arc<JmapClient>,
    admin: Option<Arc<AdminClient>>,
    /// Held for `#[tool_handler]` routing; not read directly by hand-written code.
    #[allow(dead_code)]
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
        let details = admin.get_account(account).await.map_err(|e| {
            McpError::internal_error(
                format!("failed to look up account '{}': {}", account, e),
                None,
            )
        })?;

        let password = details["secrets"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::internal_error(format!("no password found for account '{}'", account), None)
            })?;

        JmapClient::connect(self.client.session_url(), account, password)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("failed to connect as '{}': {}", account, e), None)
            })
    }

    /// Resolve optional `account` to either a temporary client or the default.
    async fn resolve_client(
        &self,
        account: &Option<String>,
    ) -> Result<ResolvedClient, McpError> {
        if let Some(acct) = account {
            let temp = self.client_for_account(acct).await?;
            Ok(ResolvedClient::Temporary(temp))
        } else {
            Ok(ResolvedClient::Default)
        }
    }
}

/// Owns a temporary JMAP client when switching accounts, or borrows the default.
enum ResolvedClient {
    Default,
    Temporary(JmapClient),
}

impl ResolvedClient {
    fn get<'a>(&'a self, default: &'a JmapClient) -> &'a JmapClient {
        match self {
            ResolvedClient::Default => default,
            ResolvedClient::Temporary(c) => c,
        }
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
        Ok(tool_result(self.client.get_mailboxes().await))
    }

    #[tool(
        description = "Search emails with filters (query text, from, to, subject, mailbox). \
                       Returns email IDs — use get_emails to read full content. Optional `account` switches mailbox (admin API). \
                       WARNING: Do NOT use this to verify outbound SMTP sends (invoice mailers, contact forms) — \
                       submissions are NOT auto-saved to Sent. Use check_sent (admin logs) instead."
    )]
    async fn search_emails(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let filter = build_email_filter(&p);
        let position = p.position.unwrap_or(0);
        let limit = p.limit.unwrap_or(10).min(50);

        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);

        Ok(tool_result(
            client.search_emails(filter, None, position, limit).await,
        ))
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

        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);

        Ok(tool_result(client.get_emails(&p.ids).await))
    }

    #[tool(description = "Permanently delete emails by ID. This cannot be undone.")]
    async fn delete_emails(
        &self,
        Parameters(p): Parameters<DeleteEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.is_empty() {
            return Err(McpError::invalid_params("ids must not be empty", None));
        }
        Ok(tool_result(self.client.delete_emails(&p.ids).await))
    }

    #[tool(description = "Create a new mailbox/folder. Optionally set a role (archive, drafts, junk, sent, trash) \
                           or nest under a parent mailbox.")]
    async fn create_mailbox(
        &self,
        Parameters(p): Parameters<CreateMailboxParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(tool_result(
            self.client
                .create_mailbox(&p.name, p.parent_id.as_deref(), p.role.as_deref())
                .await,
        ))
    }

    #[tool(description = "Download all attachments from an email to a local directory. \
                           Returns the list of saved file paths.")]
    async fn download_attachments(
        &self,
        Parameters(p): Parameters<DownloadAttachmentsParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::fs::create_dir_all(&p.download_dir).await.map_err(|e| {
            McpError::invalid_params(
                format!("cannot create directory '{}': {}", p.download_dir, e),
                None,
            )
        })?;

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
            _ => return Ok(tool_text("No attachments found on this email.")),
        };

        let mut saved: Vec<serde_json::Value> = Vec::new();
        for att in attachments {
            let blob_id = att["blobId"].as_str().unwrap_or_default();
            let name = att["name"].as_str().unwrap_or("attachment");
            let content_type = att["type"].as_str().unwrap_or("application/octet-stream");
            let size = att["size"].as_u64().unwrap_or(0);

            if blob_id.is_empty() {
                continue;
            }

            let data = self
                .client
                .download_blob(blob_id, name, content_type)
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("failed to download '{}': {}", name, e), None)
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

        Ok(tool_success(&saved))
    }

    #[tool(description = "List all domains configured on the server. Requires admin API.")]
    async fn list_domains(&self) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;
        Ok(tool_result(admin.list_principals("domain").await))
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
            Ok(result) => Ok(tool_success(&json!({
                "status": "created",
                "domain": p.domain,
                "result": result
            }))),
            Err(e) => Ok(tool_error(e.to_string())),
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
            Ok(result) => Ok(tool_success(&json!({
                "status": "created",
                "account": p.email,
                "description": p.description,
                "result": result
            }))),
            Err(e) => Ok(tool_error(e.to_string())),
        }
    }

    #[tool(description = "List all email accounts on the server, or get details for a specific account by name. Requires admin API.")]
    async fn list_accounts(
        &self,
        Parameters(p): Parameters<ListAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        if let Some(name) = &p.name {
            Ok(tool_result(admin.get_account(name).await))
        } else {
            Ok(tool_result(admin.list_accounts().await))
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
            return Err(McpError::invalid_params(
                "action must be 'add' or 'remove'",
                None,
            ));
        }

        let op = if action == "add" {
            "addItem"
        } else {
            "removeItem"
        };

        let changes = vec![json!({
            "action": op,
            "field": "emails",
            "value": p.alias
        })];

        admin
            .update_account(&p.account, changes)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let updated = admin
            .get_account(&p.account)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(tool_success(&json!({
            "status": action,
            "alias": p.alias,
            "account": p.account,
            "emails": updated["emails"]
        })))
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
        let changes = permission_changes(&action, &p.permissions)?;

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

        Ok(tool_success(&json!({
            "status": "ok",
            "action": action,
            "account": p.account,
            "enabledPermissions": updated["enabledPermissions"],
        })))
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

        Ok(tool_success(&json!({
            "status": "reset",
            "account": p.account,
            "password": new_password,
        })))
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
            return Ok(tool_text(
                "No DSN notify script configured. No accounts have delivery reports enabled.",
            ));
        }

        Ok(tool_result(sieve::parse_dsn_script(script)))
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

        let settings = admin
            .get_settings("sieve.trusted.scripts.dsn-notify")
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let current = settings["contents"].as_str().unwrap_or_default();

        if !current.is_empty() {
            if let Err(e) = sieve::parse_dsn_script(current) {
                return Ok(tool_error(format!(
                    "Current script has custom modifications that would be lost:\n{e}\n\n\
                     Remove or fix the script manually via the admin panel before using this tool."
                )));
            }
        }

        let new_script = sieve::generate_dsn_script(&p.accounts)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        admin
            .set_settings(vec![(
                "sieve.trusted.scripts.dsn-notify.contents".to_string(),
                new_script,
            )])
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(tool_success(&json!({
            "status": "updated",
            "accounts": p.accounts
        })))
    }

    #[tool(
        description = "THE FIRST TOOL TO REACH FOR when verifying any outbound email — contact form submissions, \
                       WordPress wp_mail() sends, invoice/statement mailers, password resets, transactional mail, \
                       or anything where you need to answer 'did this message actually leave the server?'. \
                       \
                       Reads Stalwart's /api/logs (the only authoritative source) and groups events by queueId \
                       so each send becomes one record showing: auth/submission -> delivery attempt -> final \
                       status (delivery.delivered / delivery.dsn-success / delivery.failed) plus the upstream \
                       MX response code and hostname. \
                       \
                       Do NOT search mailboxes first — SMTP submissions are NOT auto-saved to the sender's Sent \
                       folder, so a mailbox search will miss outbound traffic and waste your time. Start here. \
                       \
                       IMPORTANT implementation notes learned in production: \
                       (1) Fetches newest log rows WITHOUT the server-side `filter` param by default — that param \
                       hangs on multi-GB daily logs; to/from/filter are applied client-side. \
                       (2) If nothing was submitted at all (wrong SMTP password in the app), there will be no \
                       queue events — use `verify_account_auth` to test the app's SMTP credentials. \
                       (3) Admin password ≠ mailbox password; apps must use the account password. \
                       \
                       Common use cases: (1) 'Did the contact form / invoice mailer send to info@Y?' \
                       (2) 'Was a transactional email delivered?' \
                       (3) 'Why did an outbound email bounce — what SMTP code did the remote return?'"
    )]
    async fn check_sent(
        &self,
        Parameters(p): Parameters<CheckSentParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        // Default: unfiltered fetch + client-side filter. Server-side `filter=` is
        // opt-in only — on production hosts with multi-GB log files it frequently
        // times out (observed >15–30s with 0 bytes returned).
        let use_server_filter = p.use_server_filter.unwrap_or(false);
        let server_filter: Option<String> = if use_server_filter {
            p.filter
                .clone()
                .or_else(|| p.to.clone())
                .or_else(|| p.from.clone())
        } else {
            None
        };

        let scan_limit = p.scan_limit.unwrap_or(500).clamp(1, 5000);

        let raw = admin
            .get_logs(server_filter.as_deref(), scan_limit)
            .await
            .map_err(|e| McpError::internal_error(format!("log fetch failed: {e:#}"), None))?;

        let items = raw["items"].as_array().cloned().unwrap_or_default();

        let summary = analyze_logs(
            &items,
            LogFilters {
                to: p.to.as_deref(),
                from: p.from.as_deref(),
                filter: p.filter.as_deref(),
                since: p.since.as_deref(),
            },
            ScanMeta {
                use_server_filter,
                server_filter,
                to_filter: p.to.clone(),
                from_filter: p.from.clone(),
                since: p.since.clone(),
            },
        );

        Ok(tool_success(&summary))
    }

    #[tool(
        description = "Verify that a username/password can authenticate to Stalwart. \
                       Use this when outbound mail never appears in check_sent — the #1 cause is an app \
                       configured with the wrong password (often the admin password instead of the mailbox \
                       password). Stalwart uses the same account secret for JMAP and SMTP submission (port 587), \
                       so a successful JMAP login here means the SMTP DSN credentials are good. \
                       Admin password (`STALWART_ADMIN_PASSWORD`) is a DIFFERENT principal and will fail for \
                       hello@… accounts."
    )]
    async fn verify_account_auth(
        &self,
        Parameters(p): Parameters<VerifyAccountAuthParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.username.trim().is_empty() || p.password.is_empty() {
            return Err(McpError::invalid_params(
                "username and password are required",
                None,
            ));
        }

        let session_url = self.client.session_url().to_string();
        match JmapClient::connect(&session_url, &p.username, &p.password).await {
            Ok(_) => Ok(tool_success(&json!({
                "status": "ok",
                "authenticated": true,
                "username": p.username,
                "session_url": session_url,
                "note": "Credentials accepted. This same password is valid for SMTP submission \
                         (smtp://user:pass@host:587). Safe to put in app mailer DSNs."
            }))),
            Err(e) => Ok(tool_success(&json!({
                "status": "auth_failed",
                "authenticated": false,
                "username": p.username,
                "session_url": session_url,
                "error": format!("{e:#}"),
                "note": "Credentials rejected. Common mistakes: (1) using STALWART_ADMIN_PASSWORD \
                         instead of the mailbox password, (2) stale password after a reset, \
                         (3) wrong account email. Check mailman/secrets or list_accounts secrets field \
                         for the live mailbox password. SMTP apps will get 535 Authentication credentials invalid."
            }))),
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
            let blob = self
                .client
                .upload_blob(data, &content_type)
                .await
                .map_err(|e| {
                    McpError::internal_error(
                        format!("upload failed for '{}': {}", att.filename, e),
                        None,
                    )
                })?;
            uploaded.push(EmailAttachment {
                blob_id: blob.blob_id,
                content_type,
                filename: att.filename,
                size: blob.size,
            });
        }

        Ok(tool_result(
            self.client
                .send_email(
                    from,
                    &p.to,
                    &p.subject,
                    &p.body,
                    p.html_body.as_deref(),
                    &cc,
                    &bcc,
                    &uploaded,
                )
                .await,
        ))
    }
}

fn build_email_filter(p: &SearchParams) -> serde_json::Value {
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

    if conditions.len() == 1 {
        conditions.remove(0)
    } else if conditions.is_empty() {
        json!({})
    } else {
        json!({"operator": "AND", "conditions": conditions})
    }
}

fn permission_changes(
    action: &str,
    permissions: &[String],
) -> Result<Vec<serde_json::Value>, McpError> {
    match action {
        "set" => Ok(vec![json!({
            "action": "set",
            "field": "enabledPermissions",
            "value": permissions,
        })]),
        "add" => Ok(permissions
            .iter()
            .map(|perm| {
                json!({
                    "action": "addItem",
                    "field": "enabledPermissions",
                    "value": perm,
                })
            })
            .collect()),
        "remove" => Ok(permissions
            .iter()
            .map(|perm| {
                json!({
                    "action": "removeItem",
                    "field": "enabledPermissions",
                    "value": perm,
                })
            })
            .collect()),
        _ => Err(McpError::invalid_params(
            "action must be 'set', 'add', or 'remove'",
            None,
        )),
    }
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
