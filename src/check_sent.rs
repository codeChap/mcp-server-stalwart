//! Client-side analysis of Stalwart `/api/logs` for outbound delivery verification.
//!
//! Extracted from the `check_sent` MCP tool so the heavy filtering/grouping logic
//! is pure, unit-testable, and free of MCP plumbing.

use regex::Regex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Client-side filters applied after fetching raw log rows.
pub struct LogFilters<'a> {
    pub to: Option<&'a str>,
    pub from: Option<&'a str>,
    pub filter: Option<&'a str>,
    pub since: Option<&'a str>,
}

/// Metadata about how the log window was fetched (echoed in the summary).
pub struct ScanMeta {
    pub use_server_filter: bool,
    pub server_filter: Option<String>,
    pub to_filter: Option<String>,
    pub from_filter: Option<String>,
    pub since: Option<String>,
}

// Events that prove submission attempt, auth, or delivery outcome.
const RELEVANT_EVENT_PREFIXES: &[&str] = &[
    "delivery.",        // attempt, completed, delivered, failed, dsn-success, dsn-perm-fail
    "queue.",           // queue-message-authenticated, rescheduled, etc.
    "smtp.rcpt-to",     // submission RCPT
    "smtp.mail-from",   // submission MAIL FROM
    "smtp.message-",    // accepted / rejected
    "smtp.data",
    "auth.success",     // SMTP submission auth (proves the app's password worked)
    "auth.failure",     // wrong password / bad credentials
    "auth.error",
    "outgoing-report.",
    "tls-rpt.",
    "mta-sts.",
];

static QUEUE_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"queueId = (\d+)"#).expect("valid regex"));
static FROM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"from = "([^"]+)""#).expect("valid regex"));
static TO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"to = (?:\[([^\]]+)\]|"([^"]+)")"#).expect("valid regex"));
static CODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"code = (\d+)"#).expect("valid regex"));
static HOST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"hostname = "([^"]+)""#).expect("valid regex"));
static ACCOUNT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"accountName = "([^"]+)""#).expect("valid regex"));

/// Filter raw log items and group them into a `check_sent` summary JSON object.
pub fn analyze_logs(items: &[Value], filters: LogFilters<'_>, meta: ScanMeta) -> Value {
    let matched = filter_items(items, &filters);
    let (messages, auth_events, orphans) = group_matched(&matched);

    let total_delivered = messages
        .iter()
        .filter(|m| m["delivered"].as_bool().unwrap_or(false))
        .count();
    let total_failed = messages
        .iter()
        .filter(|m| m["failed"].as_bool().unwrap_or(false))
        .count();

    let oldest = items
        .last()
        .and_then(|i| i["timestamp"].as_str())
        .unwrap_or("");
    let newest = items
        .first()
        .and_then(|i| i["timestamp"].as_str())
        .unwrap_or("");

    let filter_label = if meta.use_server_filter {
        "ON"
    } else if messages.is_empty() {
        "OFF (client-side only — preferred)"
    } else {
        "OFF (client-side only)"
    };

    let note = if messages.is_empty() {
        format!(
            "Reached the Stalwart admin API and scanned {} log row(s) (window {} .. {}), but found no \
             submission/delivery events matching this filter. This is NOT an access error — the message may \
             be outside the scanned window, never submitted (wrong SMTP password — try verify_account_auth), \
             or the filter may not match. Raise `scan_limit` (default 500, max 5000) or loosen `to`/`from`. \
             Server-side filter was {}.",
            items.len(),
            oldest,
            newest,
            filter_label
        )
    } else {
        format!(
            "Reached the Stalwart admin API and scanned {} log row(s) (window {} .. {}); grouped into {} send(s). \
             Server-side filter: {}.",
            items.len(),
            oldest,
            newest,
            messages.len(),
            filter_label
        )
    };

    json!({
        "status": "ok",
        "note": note,
        "scanned_log_rows": items.len(),
        "log_window": { "newest": newest, "oldest": oldest },
        "server_filter_used": meta.use_server_filter,
        "server_filter": meta.server_filter,
        "to_filter": meta.to_filter,
        "from_filter": meta.from_filter,
        "since": meta.since,
        "messages_found": messages.len(),
        "delivered_count": total_delivered,
        "failed_count": total_failed,
        "messages": messages,
        "auth_events": auth_events,
        "orphan_events": orphans,
    })
}

fn filter_items<'a>(items: &'a [Value], filters: &LogFilters<'_>) -> Vec<&'a Value> {
    let to_needle = filters.to.map(|s| s.to_lowercase());
    let from_needle = filters.from.map(|s| s.to_lowercase());
    let extra_needle = filters.filter.map(|s| s.to_lowercase());
    let since_cutoff = filters.since;

    let mut matched: Vec<&Value> = Vec::new();
    for item in items {
        let event_id = item["event_id"].as_str().unwrap_or("").to_lowercase();
        let details = item["details"].as_str().unwrap_or("").to_lowercase();
        let timestamp = item["timestamp"].as_str().unwrap_or("");

        if let Some(cutoff) = since_cutoff {
            if !timestamp.is_empty() && timestamp < cutoff {
                continue;
            }
        }

        if !RELEVANT_EVENT_PREFIXES
            .iter()
            .any(|prefix| event_id.starts_with(prefix) || event_id == *prefix)
        {
            continue;
        }

        if let Some(needle) = &to_needle {
            if !details.contains(needle.as_str()) {
                continue;
            }
        }
        if let Some(needle) = &from_needle {
            if !details.contains(needle.as_str()) {
                // auth.success lines carry accountName= not from= — still match account
                // (preserved as in the original tool; both branches check the same needle)
                if !(event_id.starts_with("auth.") && details.contains(needle.as_str())) {
                    continue;
                }
            }
        }
        if let Some(needle) = &extra_needle {
            if !details.contains(needle.as_str()) && !event_id.contains(needle.as_str()) {
                continue;
            }
        }

        matched.push(item);
    }
    matched
}

fn group_matched(matched: &[&Value]) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut groups: BTreeMap<String, serde_json::Map<String, Value>> = BTreeMap::new();
    let mut orphans: Vec<Value> = Vec::new();
    let mut auth_events: Vec<Value> = Vec::new();

    for item in matched {
        let details = item["details"].as_str().unwrap_or("");
        let timestamp = item["timestamp"].as_str().unwrap_or("");
        let event = item["event"].as_str().unwrap_or("");
        let event_id = item["event_id"].as_str().unwrap_or("");

        if event_id.starts_with("auth.") {
            auth_events.push(json!({
                "timestamp": timestamp,
                "event_id": event_id,
                "event": event,
                "account": ACCOUNT_RE.captures(details).and_then(|c| c.get(1)).map(|m| m.as_str()),
                "details": details,
            }));
            continue;
        }

        let qid = QUEUE_ID_RE
            .captures(details)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());

        let summary_event = json!({
            "timestamp": timestamp,
            "event_id": event_id,
            "event": event,
            "code": CODE_RE.captures(details).and_then(|c| c.get(1)).map(|m| m.as_str()),
            "hostname": HOST_RE.captures(details).and_then(|c| c.get(1)).map(|m| m.as_str()),
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

        let group = groups.entry(qid.clone()).or_insert_with(|| new_group(&qid, details, timestamp));
        update_group(group, details, timestamp, event_id, summary_event);
    }

    let messages: Vec<Value> = groups.into_values().map(Value::Object).collect();
    (messages, auth_events, orphans)
}

fn new_group(qid: &str, details: &str, timestamp: &str) -> serde_json::Map<String, Value> {
    let to_val = TO_RE
        .captures(details)
        .and_then(|c| c.get(1).or(c.get(2)))
        .map(|m| m.as_str().trim().trim_matches('"').to_string());

    let mut m = serde_json::Map::new();
    m.insert("queue_id".into(), json!(qid));
    m.insert(
        "from".into(),
        json!(FROM_RE
            .captures(details)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())),
    );
    m.insert("to".into(), json!(to_val));
    m.insert("events".into(), json!([]));
    m.insert("first_seen".into(), json!(timestamp));
    m.insert("last_seen".into(), json!(timestamp));
    m.insert("delivered".into(), json!(false));
    m.insert("failed".into(), json!(false));
    m.insert("mx_code".into(), Value::Null);
    m.insert("mx_hostname".into(), Value::Null);
    m
}

fn update_group(
    group: &mut serde_json::Map<String, Value>,
    details: &str,
    timestamp: &str,
    event_id: &str,
    summary_event: Value,
) {
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

    if group
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
    {
        if let Some(f) = FROM_RE
            .captures(details)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())
        {
            group.insert("from".into(), json!(f));
        }
    }
    if let Some(t) = TO_RE
        .captures(details)
        .and_then(|c| c.get(1).or(c.get(2)))
        .map(|m| m.as_str().trim().trim_matches('"'))
    {
        let existing = group
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if t.len() > existing.len() {
            group.insert("to".into(), json!(t));
        }
    }

    let is_delivered = matches!(
        event_id,
        "delivery.delivered" | "delivery.completed" | "delivery.dsn-success"
    );
    let is_failed = matches!(
        event_id,
        "delivery.failed"
            | "delivery.bounce"
            | "delivery.dsn-perm-fail"
            | "delivery.rcpt-to-rejected"
    );

    if is_delivered {
        group.insert("delivered".into(), json!(true));
        apply_mx_fields(group, details);
    }
    if is_failed {
        group.insert("failed".into(), json!(true));
        apply_mx_fields(group, details);
    }

    if let Some(events_arr) = group
        .get_mut("events")
        .and_then(|v: &mut Value| v.as_array_mut())
    {
        events_arr.push(summary_event);
    }
}

fn apply_mx_fields(group: &mut serde_json::Map<String, Value>, details: &str) {
    if let Some(code) = CODE_RE
        .captures(details)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
    {
        group.insert("mx_code".into(), json!(code));
    }
    if let Some(host) = HOST_RE
        .captures(details)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
    {
        group.insert("mx_hostname".into(), json!(host));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_item(event_id: &str, details: &str, timestamp: &str) -> Value {
        json!({
            "event_id": event_id,
            "event": event_id,
            "details": details,
            "timestamp": timestamp,
        })
    }

    #[test]
    fn groups_by_queue_id_and_marks_delivered() {
        let items = vec![
            log_item(
                "delivery.dsn-success",
                r#"queueId = 42, from = "a@x.com", to = "b@y.com", code = 250, hostname = "mx.y.com""#,
                "2026-07-06T10:00:02Z",
            ),
            log_item(
                "queue.message-authenticated",
                r#"queueId = 42, from = "a@x.com", to = ["b@y.com"]"#,
                "2026-07-06T10:00:01Z",
            ),
            log_item(
                "smtp.data",
                r#"unrelated noise without queue"#,
                "2026-07-06T09:59:00Z",
            ),
        ];

        let summary = analyze_logs(
            &items,
            LogFilters {
                to: Some("b@y.com"),
                from: None,
                filter: None,
                since: None,
            },
            ScanMeta {
                use_server_filter: false,
                server_filter: None,
                to_filter: Some("b@y.com".into()),
                from_filter: None,
                since: None,
            },
        );

        assert_eq!(summary["messages_found"], 1);
        assert_eq!(summary["delivered_count"], 1);
        assert_eq!(summary["failed_count"], 0);
        assert_eq!(summary["messages"][0]["queue_id"], "42");
        assert_eq!(summary["messages"][0]["mx_code"], "250");
        assert_eq!(summary["messages"][0]["mx_hostname"], "mx.y.com");
        assert_eq!(summary["server_filter_used"], false);
    }

    #[test]
    fn collects_auth_events_separately() {
        let items = vec![log_item(
            "auth.success",
            r#"accountName = "hello@codechap.com""#,
            "2026-07-06T10:00:00Z",
        )];

        let summary = analyze_logs(
            &items,
            LogFilters {
                to: None,
                from: Some("hello@codechap.com"),
                filter: None,
                since: None,
            },
            ScanMeta {
                use_server_filter: false,
                server_filter: None,
                to_filter: None,
                from_filter: Some("hello@codechap.com".into()),
                since: None,
            },
        );

        assert_eq!(summary["messages_found"], 0);
        assert_eq!(summary["auth_events"].as_array().unwrap().len(), 1);
        assert_eq!(
            summary["auth_events"][0]["account"],
            "hello@codechap.com"
        );
    }

    #[test]
    fn since_filter_drops_old_rows() {
        let items = vec![
            log_item(
                "delivery.delivered",
                r#"queueId = 1, from = "a@x.com", to = "b@y.com""#,
                "2026-07-06T08:00:00Z",
            ),
            log_item(
                "delivery.delivered",
                r#"queueId = 2, from = "a@x.com", to = "b@y.com""#,
                "2026-07-06T12:00:00Z",
            ),
        ];

        let summary = analyze_logs(
            &items,
            LogFilters {
                to: None,
                from: None,
                filter: None,
                since: Some("2026-07-06T10:00:00Z"),
            },
            ScanMeta {
                use_server_filter: false,
                server_filter: None,
                to_filter: None,
                from_filter: None,
                since: Some("2026-07-06T10:00:00Z".into()),
            },
        );

        assert_eq!(summary["messages_found"], 1);
        assert_eq!(summary["messages"][0]["queue_id"], "2");
    }
}
