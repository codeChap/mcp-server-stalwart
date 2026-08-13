//! MCP tool parameter types (JSON Schema via schemars).

use schemars::JsonSchema;
use serde::Deserialize;

const ACCOUNT_PARAM: &str = "Mailbox to act as (e.g. 'hello@codechap.com'). \
    Omit for the default JMAP account. Password comes from JMAP_SECRETS_FILE / JMAP_ACCOUNTS, \
    then the admin API. Does not require admin if the password is in the secrets file.";

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

    #[schemars(description = ACCOUNT_PARAM)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetEmailsParams {
    #[schemars(description = "List of email IDs to retrieve")]
    pub ids: Vec<String>,

    #[schemars(description = ACCOUNT_PARAM)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteEmailsParams {
    #[schemars(description = "List of email IDs to delete")]
    pub ids: Vec<String>,

    #[schemars(description = ACCOUNT_PARAM)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendEmailParams {
    #[schemars(description = "Recipient email addresses")]
    pub to: Vec<String>,

    #[schemars(description = "Email subject")]
    pub subject: String,

    #[schemars(description = "Email body (plain text)")]
    pub body: String,

    #[schemars(
        description = "Email body as HTML (optional). When provided, the email is sent as multipart with both plain text and HTML parts."
    )]
    pub html_body: Option<String>,

    #[schemars(description = "CC recipients (optional)")]
    pub cc: Option<Vec<String>>,

    #[schemars(description = "BCC recipients (optional)")]
    pub bcc: Option<Vec<String>>,

    #[schemars(
        description = "File attachments (optional). Each attachment needs a file path and filename."
    )]
    pub attachments: Option<Vec<AttachmentParam>>,

    #[schemars(description = ACCOUNT_PARAM)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachmentParam {
    #[schemars(description = "Absolute path to the file on disk")]
    pub path: String,

    #[schemars(description = "Filename for the attachment (e.g., 'report.pdf')")]
    pub filename: String,

    #[schemars(
        description = "MIME type (e.g., 'application/pdf', 'image/png'). Auto-detected from extension if omitted."
    )]
    pub content_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadAttachmentsParams {
    #[schemars(description = "Email ID to download attachments from")]
    pub email_id: String,

    #[schemars(description = "Directory path to save attachments to")]
    pub download_dir: String,

    #[schemars(description = ACCOUNT_PARAM)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateMailboxParams {
    #[schemars(description = "Name of the mailbox to create")]
    pub name: String,

    #[schemars(description = "Parent mailbox ID for nesting (optional, top-level if omitted)")]
    pub parent_id: Option<String>,

    #[schemars(
        description = "Mailbox role (optional). Standard roles: archive, drafts, inbox, junk, sent, trash"
    )]
    pub role: Option<String>,

    #[schemars(description = ACCOUNT_PARAM)]
    pub account: Option<String>,
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

    #[schemars(
        description = "Permissions to grant at creation (e.g. ['email-send', 'authenticate', 'imap-authenticate']). Newly-created principals start with no permissions and cannot authenticate, send, or receive mail until permissions are granted. Use update_account_permissions afterwards if omitted here."
    )]
    pub permissions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateAccountPermissionsParams {
    #[schemars(description = "Account name (e.g. 'hello@codechap.com')")]
    pub account: String,

    #[schemars(
        description = "Action: 'set' replaces the full enabledPermissions list; 'add' grants the listed permissions; 'remove' revokes them. Default 'set'."
    )]
    pub action: Option<String>,

    #[schemars(
        description = "Permission names to set, add, or remove (e.g. ['email-send', 'authenticate', 'imap-authenticate', 'imap-append'])."
    )]
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

    #[schemars(
        description = "New password. If omitted, a strong 24-character random password is generated and returned."
    )]
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
        description = "Recipient email or domain to look up (substring match, e.g. 'info@excellerateservices.co.za' or 'excellerateservices.co.za'). \
                       This is usually the most specific filter — prefer it over `from` when verifying a contact-form send. \
                       Applied CLIENT-SIDE (server-side log filter often hangs on busy hosts)."
    )]
    pub to: Option<String>,

    #[schemars(
        description = "Sender email or domain (substring match, e.g. 'no-reply-za@excellerate.site'). \
                       Useful when you know the From address but not the recipient. Applied CLIENT-SIDE."
    )]
    pub from: Option<String>,

    #[schemars(
        description = "Optional free-text filter applied CLIENT-SIDE to the details field (e.g. a queueId). \
                       NOT passed to the server by default — Stalwart's server-side `filter` param can hang \
                       for >30s on multi-GB daily logs. Set `use_server_filter=true` only if you know you need it."
    )]
    pub filter: Option<String>,

    #[schemars(
        description = "Only show log events at or after this RFC3339 timestamp (e.g. '2026-07-06T08:00:00Z'). \
                       IMPORTANT: set this to roughly when the send should have happened to avoid scanning thousands of old log rows. \
                       Stalwart logs are newest-first, so a tight `since` makes the tool much faster and more accurate."
    )]
    pub since: Option<String>,

    #[schemars(
        description = "Number of raw log rows to fetch from the server (default 500, max 5000). \
                       Fetches newest-first WITHOUT server-side text filter, then filters client-side. \
                       Raise this when the send is older than what the default window covers."
    )]
    pub scan_limit: Option<u32>,

    #[schemars(
        description = "If true, pass `filter`/`to`/`from` to Stalwart's server-side log search. \
                       DEFAULT false — server-side filter frequently times out on busy production hosts. \
                       Only enable for short-lived servers or when you know the filter is selective (e.g. a queueId)."
    )]
    pub use_server_filter: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OptionalAccountParams {
    #[schemars(description = ACCOUNT_PARAM)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VerifyAccountAuthParams {
    #[schemars(
        description = "Account email / username to test (e.g. 'hello@codechap.com'). \
                       This is the SMTP submission username apps put in their mailer DSN."
    )]
    pub username: String,

    #[schemars(
        description = "Password to test. Same secret apps use for SMTP on port 587 / JMAP. \
                       Do NOT pass the Stalwart admin password here — that is a different principal."
    )]
    pub password: String,
}
