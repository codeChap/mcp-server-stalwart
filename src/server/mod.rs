//! MCP tool surface for Stalwart.
//!
//! Thin handlers: params live in `params`, shared helpers in `util`,
//! outbound-log analysis in `check_sent`. Domain routers live in this module.

mod admin_tools;
mod mail;
mod outbound;

use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::tool::ToolRouter, model::*, tool_handler,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::admin::AdminClient;
use crate::jmap::JmapClient;
use crate::secrets;

#[derive(Clone)]
pub struct StalwartServer {
    pub(crate) client: Arc<JmapClient>,
    pub(crate) admin: Option<Arc<AdminClient>>,
    mailbox_passwords: Arc<HashMap<String, String>>,
    /// Held for `#[tool_handler]` routing; not read directly by hand-written code.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl StalwartServer {
    pub fn new(
        client: JmapClient,
        admin: Option<AdminClient>,
        mailbox_passwords: HashMap<String, String>,
    ) -> Self {
        Self {
            client: Arc::new(client),
            admin: admin.map(Arc::new),
            mailbox_passwords: Arc::new(mailbox_passwords),
            tool_router: Self::tool_router(),
        }
    }

    fn tool_router() -> ToolRouter<Self> {
        Self::router_mail() + Self::router_admin() + Self::router_outbound() + Self::router_send()
    }

    pub(crate) fn map_admin(e: impl ToString) -> McpError {
        McpError::internal_error(e.to_string(), None)
    }

    pub(crate) fn require_admin(&self) -> Result<&AdminClient, McpError> {
        self.admin.as_deref().ok_or_else(|| {
            McpError::internal_error(
                "Admin API not configured. Set STALWART_ADMIN_URL and STALWART_ADMIN_PASSWORD env vars.",
                None,
            )
        })
    }

    pub(crate) async fn password_for_account(&self, account: &str) -> Result<String, McpError> {
        if let Some(p) = secrets::lookup(&self.mailbox_passwords, account) {
            return Ok(p.to_string());
        }
        let admin = self.admin.as_deref().ok_or_else(|| {
            McpError::internal_error(
                format!(
                    "no password for account '{account}'. Set JMAP_SECRETS_FILE \
                     (mailman4 secrets.toml) or JMAP_ACCOUNTS, or configure the admin API."
                ),
                None,
            )
        })?;
        let details = admin.get_account(account).await.map_err(|e| {
            McpError::internal_error(format!("failed to look up account '{account}': {e}"), None)
        })?;
        AdminClient::first_secret(&details).ok_or_else(|| {
            McpError::internal_error(format!("no password found for account '{account}'"), None)
        })
    }

    /// Create a temporary JMAP client authenticated as a different account.
    /// Password order: JMAP_SECRETS_FILE / JMAP_ACCOUNTS, then admin API.
    pub(crate) async fn client_for_account(&self, account: &str) -> Result<JmapClient, McpError> {
        let password = self.password_for_account(account).await?;
        JmapClient::connect(self.client.session_url(), account, &password)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("failed to connect as '{account}': {e}"), None)
            })
    }

    /// Resolve optional `account` to either a temporary client or the default.
    pub(crate) async fn resolve_client(
        &self,
        account: &Option<String>,
    ) -> Result<ResolvedClient, McpError> {
        match account.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(ResolvedClient::Default),
            Some(acct) if acct.eq_ignore_ascii_case(self.client.username()) => {
                Ok(ResolvedClient::Default)
            }
            Some(acct) => {
                let temp = self.client_for_account(acct).await?;
                Ok(ResolvedClient::Temporary(temp))
            }
        }
    }

    pub(crate) async fn patch_account(
        &self,
        name: &str,
        changes: Vec<Value>,
    ) -> Result<Value, McpError> {
        let admin = self.require_admin()?;
        admin
            .update_account(name, changes)
            .await
            .map_err(Self::map_admin)?;
        admin.get_account(name).await.map_err(Self::map_admin)
    }

    pub(crate) async fn dsn_script(&self) -> Result<String, McpError> {
        let settings = self
            .require_admin()?
            .get_settings("sieve.trusted.scripts.dsn-notify")
            .await
            .map_err(Self::map_admin)?;
        Ok(settings["contents"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}

/// Owns a temporary JMAP client when switching accounts, or borrows the default.
pub(crate) enum ResolvedClient {
    Default,
    Temporary(JmapClient),
}

impl ResolvedClient {
    pub(crate) fn get<'a>(&'a self, default: &'a JmapClient) -> &'a JmapClient {
        match self {
            ResolvedClient::Default => default,
            ResolvedClient::Temporary(c) => c,
        }
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
