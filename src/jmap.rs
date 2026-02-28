use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone)]
pub struct JmapClient {
    http: Client,
    api_url: String,
    username: String,
    password: String,
    account_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    api_url: String,
    accounts: HashMap<String, AccountInfo>,
    primary_accounts: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountInfo {
    #[allow(dead_code)]
    name: String,
}

impl JmapClient {
    pub async fn connect(session_url: &str, username: &str, password: &str) -> Result<Self> {
        let http = Client::builder()
            .user_agent("mcp-server-stalwart/0.1.0")
            .build()?;

        let session: Session = http
            .get(session_url)
            .basic_auth(username, Some(password))
            .send()
            .await
            .context("failed to fetch JMAP session")?
            .error_for_status()
            .context("JMAP session auth failed")?
            .json()
            .await
            .context("failed to parse JMAP session")?;

        let account_id = session
            .primary_accounts
            .get("urn:ietf:params:jmap:mail")
            .cloned()
            .context("no primary mail account found")?;

        if !session.accounts.contains_key(&account_id) {
            bail!("account {account_id} not in session");
        }

        Ok(Self {
            http,
            api_url: session.api_url,
            username: username.to_string(),
            password: password.to_string(),
            account_id,
        })
    }

    async fn call(&self, method: &str, args: Value) -> Result<Value> {
        let request = json!({
            "using": [
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:mail",
                "urn:ietf:params:jmap:submission"
            ],
            "methodCalls": [[method, args, "r0"]]
        });

        let resp: JmapResponse = self
            .http
            .post(&self.api_url)
            .basic_auth(&self.username, Some(&self.password))
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let call = resp
            .method_responses
            .into_iter()
            .next()
            .context("empty JMAP response")?;

        // call is [method_name, result, call_id]
        if call[0].as_str() == Some("error") {
            bail!("JMAP error: {}", call[1]);
        }

        Ok(call[1].clone())
    }

    pub async fn get_mailboxes(&self) -> Result<Value> {
        self.call(
            "Mailbox/get",
            json!({
                "accountId": self.account_id,
                "properties": ["id", "name", "parentId", "role", "totalEmails", "unreadEmails"]
            }),
        )
        .await
    }

    pub async fn search_emails(
        &self,
        filter: Value,
        sort: Option<Value>,
        position: u32,
        limit: u32,
    ) -> Result<Value> {
        let sort = sort.unwrap_or_else(|| json!([{"property": "receivedAt", "isAscending": false}]));

        self.call(
            "Email/query",
            json!({
                "accountId": self.account_id,
                "filter": filter,
                "sort": sort,
                "position": position,
                "limit": limit
            }),
        )
        .await
    }

    pub async fn get_emails(&self, ids: &[String]) -> Result<Value> {
        self.call(
            "Email/get",
            json!({
                "accountId": self.account_id,
                "#ids": { "resultOf": null, "name": null, "path": null },
                "ids": ids,
                "properties": [
                    "id", "threadId", "mailboxIds", "from", "to", "cc", "bcc",
                    "subject", "receivedAt", "sentAt", "size", "keywords",
                    "preview", "textBody", "htmlBody", "bodyValues"
                ],
                "fetchTextBodyValues": true,
                "fetchHTMLBodyValues": true,
                "maxBodyValueBytes": 65536
            }),
        )
        .await
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapResponse {
    method_responses: Vec<Vec<Value>>,
}
