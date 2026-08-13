//! Outbound delivery verification: log scan + credential check.

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, model::*, tool, tool_router,
};
use serde_json::json;

use crate::check_sent::{LogFilters, ScanMeta, analyze_logs};
use crate::jmap::JmapClient;
use crate::params::*;
use crate::server::StalwartServer;
use crate::util::tool_success;

#[tool_router(router = router_outbound, vis = "pub(crate)")]
impl StalwartServer {
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
        let (use_server_filter, server_filter, scan_limit) = scan_window(&p);

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
}

fn scan_window(p: &CheckSentParams) -> (bool, Option<String>, u32) {
    // Default: unfiltered fetch + client-side filter. Server-side `filter=` is
    // opt-in only — on production hosts with multi-GB log files it frequently
    // times out (observed >15–30s with 0 bytes returned).
    let use_server_filter = p.use_server_filter.unwrap_or(false);
    let server_filter = if use_server_filter {
        p.filter
            .clone()
            .or_else(|| p.to.clone())
            .or_else(|| p.from.clone())
    } else {
        None
    };
    let scan_limit = p.scan_limit.unwrap_or(500).clamp(1, 5000);
    (use_server_filter, server_filter, scan_limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(
        use_server_filter: Option<bool>,
        filter: Option<&str>,
        to: Option<&str>,
        from: Option<&str>,
        scan_limit: Option<u32>,
    ) -> CheckSentParams {
        CheckSentParams {
            to: to.map(str::to_string),
            from: from.map(str::to_string),
            filter: filter.map(str::to_string),
            since: None,
            scan_limit,
            use_server_filter,
        }
    }

    #[test]
    fn default_window_is_unfiltered() {
        let (use_server, filter, limit) =
            scan_window(&params(None, Some("q"), Some("to"), None, None));
        assert!(!use_server);
        assert_eq!(filter, None);
        assert_eq!(limit, 500);
    }

    #[test]
    fn server_filter_prefers_filter_then_to_then_from() {
        let (_, filter, _) = scan_window(&params(
            Some(true),
            Some("qid"),
            Some("to"),
            Some("from"),
            Some(10),
        ));
        assert_eq!(filter.as_deref(), Some("qid"));

        let (_, filter, limit) = scan_window(&params(
            Some(true),
            None,
            Some("to"),
            Some("from"),
            Some(9000),
        ));
        assert_eq!(filter.as_deref(), Some("to"));
        assert_eq!(limit, 5000);

        let (_, filter, limit) =
            scan_window(&params(Some(true), None, None, Some("from"), Some(0)));
        assert_eq!(filter.as_deref(), Some("from"));
        assert_eq!(limit, 1);
    }
}
