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
        let results = self.call_multi(vec![(method, args, "r0")]).await?;
        Ok(results.into_iter().next().context("empty JMAP response")?)
    }

    async fn call_multi(&self, calls: Vec<(&str, Value, &str)>) -> Result<Vec<Value>> {
        let method_calls: Vec<Value> = calls
            .into_iter()
            .map(|(method, args, id)| json!([method, args, id]))
            .collect();

        let request = json!({
            "using": [
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:mail",
                "urn:ietf:params:jmap:submission"
            ],
            "methodCalls": method_calls
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

        let mut results = Vec::new();
        for call in resp.method_responses {
            if call[0].as_str() == Some("error") {
                bail!("JMAP error: {}", call[1]);
            }
            results.push(call[1].clone());
        }

        Ok(results)
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
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    async fn get_drafts_mailbox_id(&self) -> Result<String> {
        let result = self.get_mailboxes().await?;
        result["list"]
            .as_array()
            .and_then(|list| {
                list.iter().find(|m| m["role"].as_str() == Some("drafts"))
            })
            .and_then(|m| m["id"].as_str())
            .map(|s| s.to_string())
            .context("no drafts mailbox found")
    }

    async fn get_identity_id(&self) -> Result<String> {
        let result = self.call("Identity/get", json!({"accountId": self.account_id})).await?;
        result["list"]
            .as_array()
            .and_then(|list| list.first())
            .and_then(|id| id["id"].as_str())
            .map(|s| s.to_string())
            .context("no identity found for this account")
    }

    pub async fn send_email(
        &self,
        from: &str,
        to: &[String],
        subject: &str,
        body: &str,
        cc: &[String],
        bcc: &[String],
    ) -> Result<Value> {
        let identity_id = self.get_identity_id().await?;

        let to_addrs: Vec<Value> = to.iter().map(|a| json!({"email": a})).collect();
        let cc_addrs: Vec<Value> = cc.iter().map(|a| json!({"email": a})).collect();
        let bcc_addrs: Vec<Value> = bcc.iter().map(|a| json!({"email": a})).collect();

        let drafts_id = self.get_drafts_mailbox_id().await?;

        let mut email = json!({
            "from": [{"email": from}],
            "to": to_addrs,
            "subject": subject,
            "bodyValues": {
                "body": {
                    "value": body,
                    "charset": "utf-8"
                }
            },
            "textBody": [{"partId": "body", "type": "text/plain"}],
            "mailboxIds": {drafts_id: true}
        });

        if !cc_addrs.is_empty() {
            email["cc"] = json!(cc_addrs);
        }
        if !bcc_addrs.is_empty() {
            email["bcc"] = json!(bcc_addrs);
        }

        let results = self.call_multi(vec![
            (
                "Email/set",
                json!({
                    "accountId": self.account_id,
                    "create": {
                        "draft": email
                    }
                }),
                "r0",
            ),
            (
                "EmailSubmission/set",
                json!({
                    "accountId": self.account_id,
                    "create": {
                        "send": {
                            "emailId": "#draft",
                            "identityId": identity_id
                        }
                    },
                    "onSuccessDestroyEmail": ["#send"]
                }),
                "r1",
            ),
        ]).await?;

        // Return the submission result
        results.into_iter().last().context("no submission response")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapResponse {
    method_responses: Vec<Vec<Value>>,
}
