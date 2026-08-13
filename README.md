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

### Optional (other mailboxes)

Switch tools onto another mailbox with `account` (e.g. `hello@codechap.com`) without needing the admin API.

| Variable | Description |
|----------|-------------|
| `JMAP_SECRETS_FILE` | Path to a mailman4 `secrets.toml` (`[passwords]` table of `"email" = "password"`) |
| `JMAP_ACCOUNTS` | Inline `email=password;other@host=password` list (overrides file on clash) |

### Optional (admin API)

| Variable | Description |
|----------|-------------|
| `STALWART_ADMIN_URL` | Admin API base URL — `https://mail.example.com` **or** `https://mail.example.com/api` (both accepted; `/api` is normalized) |
| `STALWART_ADMIN_USER` | Admin username (default: `admin`) |
| `STALWART_ADMIN_PASSWORD` | **Admin** password (different principal from mailbox passwords) |

### Password gotcha (learned the hard way)

| Secret | Used for | NOT used for |
|--------|----------|--------------|
| `JMAP_PASSWORD` / mailbox password | SMTP submission (`smtp://user:pass@host:587`), JMAP, IMAP for that account | Admin API |
| `STALWART_ADMIN_PASSWORD` | Admin API (`/api/principal`, `/api/logs`, …) | App mailer DSNs |

If an app (invoice mailer, WordPress, etc.) is configured with the admin password as the SMTP secret, Stalwart returns **`535 Authentication credentials invalid`** and **nothing is queued**. `check_sent` will correctly show zero submissions. Use `verify_account_auth` to test credentials before chasing delivery.

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
        "JMAP_SECRETS_FILE": "/home/you/.local/share/mailman4/secrets.toml",
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

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `account` | string | no | Mailbox to list (e.g. `hello@codechap.com`) |

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
| `account` | string | no | Mailbox to search (e.g. `hello@codechap.com`) |

### get_emails

Get full email content by IDs. Returns subject, from, to, date, body text, and metadata.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `ids` | string[] | yes | List of email IDs to retrieve |
| `account` | string | no | Mailbox that owns these emails |

### delete_emails

Permanently delete emails by ID. Cannot be undone.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `ids` | string[] | yes | List of email IDs to delete |
| `account` | string | no | Mailbox to delete from |

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
| `account` | string | no | Send as this mailbox (e.g. `hello@codechap.com`) instead of the default JMAP user |

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

**The first tool to reach for when verifying any outbound email** — contact forms, WordPress `wp_mail()`, invoice/statement mailers, password resets, transactional mail — anything needing *"did this leave the server?"*.

Reads Stalwart's `/api/logs` (authoritative) and groups by `queueId`: submission → delivery attempt → final status (`delivery.delivered` / `delivery.dsn-success` / `delivery.failed`) plus upstream MX `code`/`hostname`.

**Do NOT search mailboxes first** — SMTP submissions are not auto-saved to Sent. Start here.

**How the log fetch works (production lesson):**  
Stalwart's server-side `filter=` query often **hangs** on multi-GB daily log files. By default this tool fetches the newest `scan_limit` rows **unfiltered** and applies `to`/`from`/`filter` **client-side** (fast: ~300ms for 1000 rows). Pass `use_server_filter=true` only if you know you need it (e.g. a unique queueId on a quiet host).

Common use cases:
- "Did the invoice mailer / contact form send to `ap@client.com`?"
- "Was a transactional email delivered — what did Gmail return?"
- "Why bounce — remote SMTP code?"

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `to` | string | no | Recipient email or domain (client-side substring). Prefer this for contact-form checks. |
| `from` | string | no | Sender email or domain (client-side substring). |
| `filter` | string | no | Extra client-side substring (e.g. queueId). |
| `since` | string | no | RFC3339 lower bound on event timestamps. |
| `scan_limit` | number | no | Newest log rows to fetch (default 500, max 5000). Raise if the send is older than the window. |
| `use_server_filter` | bool | no | Default `false`. If `true`, pass filter to Stalwart (can timeout on busy hosts). |

Returns `messages_found`, `delivered_count`, `failed_count`, per-message timelines (`mx_code` / `mx_hostname`), plus `auth_events` (submission auth success/failure) and a `log_window`.

### verify_account_auth

Test whether a username/password is accepted by Stalwart (same secret as SMTP port 587). Use when `check_sent` shows **no submission** — usually the app has the wrong password.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `username` | string | yes | Account email (e.g. `hello@codechap.com`) |
| `password` | string | yes | Candidate password (mailbox secret, **not** admin) |
