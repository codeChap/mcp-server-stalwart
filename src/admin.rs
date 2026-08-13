use anyhow::{Context, Result};
use reqwest::{Client, Method};
use serde_json::{Value, json};

use crate::util::http_client;

#[derive(Clone)]
pub struct AdminClient {
    http: Client,
    /// Base URL including `/api`, e.g. `https://mail.example.com/api`
    api_url: String,
    username: String,
    password: String,
    /// Longer-lived client used only for log scans (can be slow on busy boxes).
    logs_http: Client,
}

impl AdminClient {
    /// Normalize an admin base URL so callers may pass either
    /// `https://host` or `https://host/api` (with or without trailing slash).
    pub fn normalize_api_url(raw: &str) -> String {
        let trimmed = raw.trim().trim_end_matches('/');
        if trimmed.ends_with("/api") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/api")
        }
    }

    /// Connect to the Stalwart Admin API and validate credentials.
    ///
    /// Makes a lightweight settings request on startup to fail fast
    /// if the URL or credentials are wrong.
    pub async fn connect(api_url: &str, username: &str, password: &str) -> Result<Self> {
        let api_url = Self::normalize_api_url(api_url);
        // Admin API calls (settings, principals) are small requests.
        // Without these timeouts a stalled connection to the Stalwart box hangs
        // the tool call — and the whole agent turn — indefinitely.
        let http = http_client(10, 30)?;

        // Log scans can be slow when the host is busy. Keep a separate client
        // with a longer timeout so principal/settings calls stay snappy.
        let logs_http = http_client(10, 90)?;

        let client = Self {
            http,
            api_url,
            username: username.to_string(),
            password: password.to_string(),
            logs_http,
        };

        client
            .get_settings("server.hostname")
            .await
            .context("Stalwart admin API connection failed — check URL and credentials")?;

        Ok(client)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_url, path)
    }

    async fn parse_get_body(
        resp: reqwest::Response,
        url: &str,
        body_ctx: &'static str,
        parse_ctx: &'static str,
    ) -> Result<Value> {
        let status = resp.status();
        let body = resp.text().await.context(body_ctx)?;
        if !status.is_success() {
            anyhow::bail!("admin API returned {status} for GET {url}: {body}");
        }
        serde_json::from_str(&body).context(parse_ctx)
    }

    /// GET that returns the full parsed JSON body (caller extracts fields).
    async fn get_raw(
        &self,
        path: &str,
        query: &[(&str, &str)],
        client: &Client,
    ) -> Result<(String, Value)> {
        let url = self.url(path);
        let resp = client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(query)
            .send()
            .await
            .with_context(|| format!("admin API request failed (GET {url})"))?;
        let parsed = Self::parse_get_body(
            resp,
            &url,
            "failed to read admin API response body",
            "failed to parse admin API response",
        )
        .await?;
        Ok((url, parsed))
    }

    /// Mutation (POST/PATCH) with optional empty body treated as success.
    async fn mutate(
        &self,
        method: Method,
        path: &str,
        body: &Value,
        empty_status: Value,
    ) -> Result<Value> {
        let url = self.url(path);
        let method_label = method.clone();
        let resp = self
            .http
            .request(method, &url)
            .basic_auth(&self.username, Some(&self.password))
            .json(body)
            .send()
            .await
            .with_context(|| {
                format!("admin API request failed ({} {url})", method_label.as_str())
            })?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .context("failed to read admin API response")?;

        if !status.is_success() {
            anyhow::bail!("admin API returned {status}: {text}");
        }

        if text.is_empty() {
            Ok(empty_status)
        } else {
            serde_json::from_str(&text).context("failed to parse admin API response")
        }
    }

    /// Fetch settings matching a key prefix.
    ///
    /// Returns the `items` map from `GET /api/settings/list?prefix=...`.
    /// Keys in the response have the prefix stripped.
    pub async fn get_settings(&self, prefix: &str) -> Result<Value> {
        let (_, resp) = self
            .get_raw("/settings/list", &[("prefix", prefix)], &self.http)
            .await?;
        Ok(resp["data"]["items"].clone())
    }

    /// List principals by type.
    ///
    /// Returns the `data` object from `GET /principal?type={type}&limit=0`.
    pub async fn list_principals(&self, principal_type: &str) -> Result<Value> {
        let (_, resp) = self
            .get_raw(
                "/principal",
                &[("type", principal_type), ("limit", "0")],
                &self.http,
            )
            .await?;
        Ok(resp["data"].clone())
    }

    /// List all individual (user) accounts.
    pub async fn list_accounts(&self) -> Result<Value> {
        self.list_principals("individual").await
    }

    /// Get details for a specific account by name.
    ///
    /// Returns the `data` object from `GET /principal/{name}`.
    pub async fn get_account(&self, name: &str) -> Result<Value> {
        let (_, resp) = self
            .get_raw(&format!("/principal/{name}"), &[], &self.http)
            .await?;
        Ok(resp["data"].clone())
    }

    /// Create a new principal account via POST.
    pub async fn create_account(&self, principal: Value) -> Result<Value> {
        self.mutate(
            Method::POST,
            "/principal",
            &principal,
            json!({"status": "created"}),
        )
        .await
    }

    /// Update a principal account via PATCH.
    ///
    /// Stalwart expects an array of change operations, e.g.:
    /// `[{"action": "addItem", "field": "emails", "value": "alias@example.com"}]`
    pub async fn update_account(&self, name: &str, changes: Vec<Value>) -> Result<Value> {
        self.mutate(
            Method::PATCH,
            &format!("/principal/{name}"),
            &Value::Array(changes),
            json!({"status": "ok"}),
        )
        .await
    }

    /// Fetch server log events, optionally filtered by free-text substring match.
    ///
    /// Stalwart's `GET /api/logs` returns events that include SMTP submission,
    /// queue handling, and outbound delivery — the only authoritative source
    /// for "did this message actually leave the server".
    ///
    /// **Important:** On hosts with multi-GB daily logs, the server-side `filter`
    /// parameter often times out (it appears to scan the full file). Prefer
    /// unfiltered fetches with a `limit` and filter client-side (see `check_sent`).
    pub async fn get_logs(&self, filter: Option<&str>, limit: u32) -> Result<Value> {
        let url = self.url("/logs");
        let limit_str = limit.to_string();
        let mut query: Vec<(&str, &str)> = vec![("limit", &limit_str)];
        if let Some(f) = filter {
            if !f.is_empty() {
                query.push(("filter", f));
            }
        }
        let resp = self
            .logs_http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&query)
            .send()
            .await
            .with_context(|| {
                format!(
                    "admin API log fetch failed (GET {url}, limit={limit}, filter={:?}). \
                     On busy servers the server-side `filter` param can hang — call without \
                     filter and filter client-side instead.",
                    filter
                )
            })?;

        let parsed = Self::parse_get_body(
            resp,
            &url,
            "failed to read admin API log response body",
            "failed to parse admin API log response",
        )
        .await?;
        Ok(parsed["data"].clone())
    }

    /// First password in a principal's `secrets` array (admin API account payload).
    pub fn first_secret(details: &Value) -> Option<String> {
        details["secrets"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    /// Insert or update settings via the admin API.
    ///
    /// Takes key-value pairs and sends them as an insert operation.
    pub async fn set_settings(&self, values: Vec<(String, String)>) -> Result<()> {
        let pairs: Vec<Value> = values.into_iter().map(|(k, v)| json!([k, v])).collect();

        let ops = json!([{
            "type": "insert",
            "values": pairs,
            "assert_empty": false
        }]);

        let url = self.url("/settings");
        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .json(&ops)
            .send()
            .await
            .with_context(|| format!("admin API settings update failed (POST {url})"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("admin API settings update returned {status}: {body}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AdminClient;
    use serde_json::json;

    #[test]
    fn normalize_adds_api_suffix() {
        assert_eq!(
            AdminClient::normalize_api_url("https://mail.example.com"),
            "https://mail.example.com/api"
        );
        assert_eq!(
            AdminClient::normalize_api_url("https://mail.example.com/"),
            "https://mail.example.com/api"
        );
    }

    #[test]
    fn normalize_keeps_existing_api() {
        assert_eq!(
            AdminClient::normalize_api_url("https://mail.example.com/api"),
            "https://mail.example.com/api"
        );
        assert_eq!(
            AdminClient::normalize_api_url("https://mail.example.com/api/"),
            "https://mail.example.com/api"
        );
    }

    #[test]
    fn first_secret_reads_first_entry() {
        let details = json!({"secrets": ["alpha", "beta"]});
        assert_eq!(
            AdminClient::first_secret(&details).as_deref(),
            Some("alpha")
        );
        assert_eq!(AdminClient::first_secret(&json!({})), None);
        assert_eq!(AdminClient::first_secret(&json!({"secrets": []})), None);
    }
}
