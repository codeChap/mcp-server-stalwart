# mcp-server-stalwart

MCP server for [Stalwart Mail Server](https://stalw.art). Provides email operations (search, read, send, delete) via JMAP and optional admin API access for server management.

## Requirements

- Rust (2024 edition)
- A Stalwart mail server with JMAP enabled

## Build

```bash
cargo build --release
```

Binary is output to `target/release/mcp-server-stalwart`.

## Configuration

The server connects via stdio and is configured through environment variables.

### Required

| Variable | Description |
|----------|-------------|
| `JMAP_SESSION_URL` | JMAP session endpoint (e.g. `https://mail.example.com/jmap/session`) |
| `JMAP_USERNAME` | JMAP account email address |
| `JMAP_PASSWORD` | JMAP account password |

### Optional (admin API)

| Variable | Description |
|----------|-------------|
| `STALWART_ADMIN_URL` | Admin API base URL (e.g. `https://mail.example.com`) |
| `STALWART_ADMIN_USER` | Admin username (default: `admin`) |
| `STALWART_ADMIN_PASSWORD` | Admin password |

## Claude Code MCP config

```json
{
  "mcpServers": {
    "stalwart": {
      "command": "/path/to/mcp-server-stalwart",
      "env": {
        "JMAP_SESSION_URL": "https://mail.example.com/jmap/session",
        "JMAP_USERNAME": "you@example.com",
        "JMAP_PASSWORD": "your-password",
        "STALWART_ADMIN_URL": "https://mail.example.com",
        "STALWART_ADMIN_PASSWORD": "admin-password"
      }
    }
  }
}
```

## Tools

### get_mailboxes

List all mailboxes/folders with message counts.

### create_mailbox

Create a new mailbox/folder.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | yes | Mailbox name |
| `parent_id` | string | no | Parent mailbox ID for nesting (top-level if omitted) |
| `role` | string | no | Standard role: `archive`, `drafts`, `inbox`, `junk`, `sent`, `trash` |

### search_emails

Search emails with filters. Returns email IDs -- use `get_emails` to read full content.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | no | Text to search across subject, body, from, to |
| `from` | string | no | Filter by sender address |
| `to` | string | no | Filter by recipient address |
| `subject` | string | no | Filter by subject text |
| `mailbox_id` | string | no | Restrict to a specific mailbox |
| `position` | number | no | Pagination offset (default 0) |
| `limit` | number | no | Max results (default 10, max 50) |

### get_emails

Get full email content by IDs. Returns subject, from, to, date, body text, and metadata.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `ids` | string[] | yes | List of email IDs to retrieve |

### delete_emails

Permanently delete emails by ID. Cannot be undone.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `ids` | string[] | yes | List of email IDs to delete |

### send_email

Send an email with optional HTML body and file attachments. When `html_body` is provided, the email is sent as multipart with both plain text and HTML parts -- the recipient's email client will choose which to display.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `to` | string[] | yes | Recipient email addresses |
| `subject` | string | yes | Email subject |
| `body` | string | yes | Plain text body |
| `html_body` | string | no | HTML body. When provided, email is sent as multipart (text/plain + text/html) |
| `cc` | string[] | no | CC recipients |
| `bcc` | string[] | no | BCC recipients |
| `attachments` | object[] | no | File attachments (see below) |

**Attachment object:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | yes | Absolute path to the file on disk |
| `filename` | string | yes | Filename for the attachment |
| `content_type` | string | no | MIME type (auto-detected from extension if omitted) |

### download_attachments

Download all attachments from an email to a local directory.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `email_id` | string | yes | Email ID to download attachments from |
| `download_dir` | string | yes | Directory path to save attachments to |

### create_account (admin)

Create a new email account on the server. Requires admin API configuration.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `email` | string | yes | Primary email address |
| `password` | string | yes | Account password |
| `description` | string | no | Display name |
| `quota` | number | no | Disk quota in bytes (0 for unlimited) |
| `permissions` | string[] | no | Permissions to grant at creation (e.g. `email-send`, `authenticate`, `imap-authenticate`). **Without permissions, the account cannot authenticate or submit mail** — either supply them here or call `update_account_permissions` afterwards. |

### list_accounts (admin)

List all accounts, or get details for one. Requires admin API configuration.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | no | Account name for details. If omitted, lists all accounts. |

### manage_aliases (admin)

Add or remove an email alias on an account. Requires admin API configuration.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `account` | string | yes | Account name |
| `action` | string | yes | `add` or `remove` |
| `alias` | string | yes | Alias email to add/remove |

### update_account_permissions (admin)

Update an account's `enabledPermissions`. Newly-created principals start with no permissions and cannot authenticate, send, or receive mail until permissions are granted. Requires admin API configuration.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `account` | string | yes | Target account name |
| `action` | string | no | `set` (replace list, default), `add` (grant), or `remove` (revoke) |
| `permissions` | string[] | yes | Permission names (e.g. `email-send`, `authenticate`, `imap-authenticate`, `imap-append`) |

### reset_password (admin)

Reset an account's password. If `password` is omitted, a strong 24-character random password is generated. The new password is returned in plaintext in the response so it can be delivered to the user. Requires admin API configuration.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `account` | string | yes | Target account name |
| `password` | string | no | New password. Auto-generated if omitted. |

### get_dsn_accounts (admin)

List email addresses that have DSN (Delivery Status Notification) delivery reports enabled. Requires admin API configuration.

### set_dsn_accounts (admin)

Set which email addresses receive DSN delivery reports (SUCCESS + FAILURE). Replaces the full list. Requires admin API configuration.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `accounts` | string[] | yes | Email addresses to enable delivery reports for |

### check_sent (admin)

Verify whether an email actually left the server. Reads Stalwart's `/api/logs` (the only authoritative source) and groups events by `queueId` so each send becomes one record showing submission → delivery attempt → final status (`delivery.delivered`, `delivery.failed`, etc.). Mailbox searches alone miss outbound traffic — SMTP submissions are not auto-saved to the sender's Sent folder.

Pass `to` and/or `from` substrings to filter; the tool will pick the most specific server-side filter and apply the rest client-side. Use `since` (RFC3339) to limit how far back to look.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `to` | string | no | Recipient email or domain (substring match) |
| `from` | string | no | Sender email or domain (substring match) |
| `filter` | string | no | Free-text filter passed straight to the log search; overrides `to`/`from` for the fetch |
| `since` | string | no | Only show events at or after this RFC3339 timestamp |
| `scan_limit` | number | no | Raw log rows to scan from the server (default 200, max 1000) |

Returns a JSON object with `messages_found`, `delivered_count`, `failed_count`, and per-message timelines including the upstream MX `code`/`hostname` so you can confirm Gmail/etc. accepted it.
