use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct AdminClient {
    http: Client,
    api_url: String,
    username: String,
    password: String,
}

impl AdminClient {
    /// Connect to the Stalwart Admin API and validate credentials.
    ///
    /// Makes a lightweight settings request on startup to fail fast
    /// if the URL or credentials are wrong.
    pub async fn connect(api_url: &str, username: &str, password: &str) -> Result<Self> {
        let api_url = api_url.trim_end_matches('/').to_string();
        let http = Client::builder()
            .user_agent("mcp-server-stalwart/0.1.0")
            .build()?;

        let client = Self {
            http,
            api_url,
            username: username.to_string(),
            password: password.to_string(),
        };

        // Validate credentials with a lightweight request
        client
            .get_settings("server.hostname")
            .await
            .context("Stalwart admin API connection failed — check URL and credentials")?;

        Ok(client)
    }

    /// Fetch settings matching a key prefix.
    ///
    /// Returns the `items` map from `GET /api/settings/list?prefix=...`.
    /// Keys in the response have the prefix stripped.
    pub async fn get_settings(&self, prefix: &str) -> Result<Value> {
        let url = format!("{}/settings/list", self.api_url);
        let resp: Value = self
            .http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("prefix", prefix)])
            .send()
            .await
            .context("admin API request failed")?
            .error_for_status()
            .context("admin API returned error status")?
            .json()
            .await
            .context("failed to parse admin API response")?;

        Ok(resp["data"]["items"].clone())
    }

    /// List principals by type.
    ///
    /// Returns the `data` object from `GET /principal?type={type}&limit=0`.
    pub async fn list_principals(&self, principal_type: &str) -> Result<Value> {
        let url = format!("{}/principal", self.api_url);
        let resp: Value = self
            .http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("type", principal_type), ("limit", "0")])
            .send()
            .await
            .context("admin API request failed")?
            .error_for_status()
            .context("admin API returned error status")?
            .json()
            .await
            .context("failed to parse admin API response")?;
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
        let url = format!("{}/principal/{}", self.api_url, name);
        let resp: Value = self
            .http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .context("admin API request failed")?
            .error_for_status()
            .context("admin API returned error status")?
            .json()
            .await
            .context("failed to parse admin API response")?;
        Ok(resp["data"].clone())
    }

    /// Create a new principal account via POST.
    pub async fn create_account(&self, principal: Value) -> Result<Value> {
        let url = format!("{}/principal", self.api_url);
        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .json(&principal)
            .send()
            .await
            .context("admin API request failed")?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .context("failed to read admin API response")?;

        if !status.is_success() {
            anyhow::bail!("admin API returned {status}: {body}");
        }

        let parsed: Value = if body.is_empty() {
            json!({"status": "created"})
        } else {
            serde_json::from_str(&body).context("failed to parse admin API response")?
        };

        Ok(parsed)
    }

    /// Update a principal account via PATCH.
    ///
    /// Stalwart expects an array of change operations, e.g.:
    /// `[{"action": "addItem", "field": "emails", "value": "alias@example.com"}]`
    pub async fn update_account(&self, name: &str, changes: Vec<Value>) -> Result<Value> {
        let url = format!("{}/principal/{}", self.api_url, name);
        let resp = self
            .http
            .patch(&url)
            .basic_auth(&self.username, Some(&self.password))
            .json(&changes)
            .send()
            .await
            .context("admin API request failed")?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .context("failed to read admin API response")?;

        if !status.is_success() {
            anyhow::bail!("admin API returned {status}: {body}");
        }

        let parsed: Value = if body.is_empty() {
            json!({"status": "ok"})
        } else {
            serde_json::from_str(&body).context("failed to parse admin API response")?
        };

        Ok(parsed)
    }

    /// Fetch server log events, optionally filtered by free-text substring match.
    ///
    /// Stalwart's `GET /api/logs` returns events that include SMTP submission,
    /// queue handling, and outbound delivery — the only authoritative source
    /// for "did this message actually leave the server".
    pub async fn get_logs(&self, filter: Option<&str>, limit: u32) -> Result<Value> {
        let url = format!("{}/logs", self.api_url);
        let limit_str = limit.to_string();
        let mut query: Vec<(&str, &str)> = vec![("limit", &limit_str)];
        if let Some(f) = filter {
            if !f.is_empty() {
                query.push(("filter", f));
            }
        }
        let resp: Value = self
            .http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&query)
            .send()
            .await
            .context("admin API request failed")?
            .error_for_status()
            .context("admin API returned error status")?
            .json()
            .await
            .context("failed to parse admin API response")?;

        Ok(resp["data"].clone())
    }

    /// Insert or update settings via the admin API.
    ///
    /// Takes key-value pairs and sends them as an insert operation.
    pub async fn set_settings(&self, values: Vec<(String, String)>) -> Result<()> {
        let pairs: Vec<Value> = values
            .into_iter()
            .map(|(k, v)| json!([k, v]))
            .collect();

        let ops = json!([{
            "type": "insert",
            "values": pairs,
            "assert_empty": false
        }]);

        let url = format!("{}/settings", self.api_url);
        self.http
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .json(&ops)
            .send()
            .await
            .context("admin API settings update failed")?
            .error_for_status()
            .context("admin API settings update returned error status")?;

        Ok(())
    }
}
