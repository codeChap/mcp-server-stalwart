use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::util::http_client;

#[derive(Clone)]
pub struct JmapClient {
    http: Client,
    session_url: String,
    api_url: String,
    upload_url: String,
    download_url: String,
    username: String,
    password: String,
    account_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    api_url: String,
    upload_url: String,
    download_url: String,
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
        // A tight connect timeout stops a dead/slow server from hanging the tool
        // call forever; the overall timeout is generous so large attachment
        // uploads/downloads still complete.
        let http = http_client(10, 300)?;

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
            session_url: session_url.to_string(),
            api_url: session.api_url,
            upload_url: session.upload_url,
            download_url: session.download_url,
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
        let sort =
            sort.unwrap_or_else(|| json!([{"property": "receivedAt", "isAscending": false}]));

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

    pub async fn create_mailbox(
        &self,
        name: &str,
        parent_id: Option<&str>,
        role: Option<&str>,
    ) -> Result<Value> {
        let mut mailbox = json!({
            "name": name
        });
        if let Some(pid) = parent_id {
            mailbox["parentId"] = json!(pid);
        }
        if let Some(r) = role {
            mailbox["role"] = json!(r);
        }

        self.call(
            "Mailbox/set",
            json!({
                "accountId": self.account_id,
                "create": {
                    "mb0": mailbox
                }
            }),
        )
        .await
    }

    pub async fn delete_emails(&self, ids: &[String]) -> Result<Value> {
        self.call(
            "Email/set",
            json!({
                "accountId": self.account_id,
                "destroy": ids
            }),
        )
        .await
    }

    pub async fn upload_blob(&self, data: Vec<u8>, content_type: &str) -> Result<UploadedBlob> {
        let url = self.upload_url.replace("{accountId}", &self.account_id);
        let size = data.len() as u64;
        let resp: Value = self
            .http
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", content_type)
            .body(data)
            .send()
            .await
            .context("blob upload request failed")?
            .error_for_status()
            .context("blob upload returned error status")?
            .json()
            .await
            .context("failed to parse blob upload response")?;

        let blob_id = resp["blobId"]
            .as_str()
            .context("no blobId in upload response")?
            .to_string();

        Ok(UploadedBlob { blob_id, size })
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn session_url(&self) -> &str {
        &self.session_url
    }

    pub async fn get_email_attachments(&self, ids: &[String]) -> Result<Value> {
        self.call(
            "Email/get",
            json!({
                "accountId": self.account_id,
                "ids": ids,
                "properties": ["id", "subject", "attachments"]
            }),
        )
        .await
    }

    pub async fn download_blob(
        &self,
        blob_id: &str,
        name: &str,
        content_type: &str,
    ) -> Result<Vec<u8>> {
        let url = self
            .download_url
            .replace("{accountId}", &self.account_id)
            .replace("{blobId}", blob_id)
            .replace("{name}", name)
            .replace("{type}", content_type);

        let bytes = self
            .http
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .context("blob download request failed")?
            .error_for_status()
            .context("blob download returned error status")?
            .bytes()
            .await
            .context("failed to read blob bytes")?;

        Ok(bytes.to_vec())
    }

    async fn get_mailbox_id_by_role(&self, role: &str) -> Result<Option<String>> {
        let result = self.get_mailboxes().await?;
        Ok(result["list"]
            .as_array()
            .and_then(|list| list.iter().find(|m| m["role"].as_str() == Some(role)))
            .and_then(|m| m["id"].as_str())
            .map(|s| s.to_string()))
    }

    async fn ensure_mailbox_with_role(&self, role: &str, name: &str) -> Result<String> {
        if let Some(id) = self.get_mailbox_id_by_role(role).await? {
            return Ok(id);
        }
        let result = self.create_mailbox(name, None, Some(role)).await?;
        result["created"]["mb0"]["id"]
            .as_str()
            .map(|s| s.to_string())
            .with_context(|| format!("failed to create {role} mailbox"))
    }

    async fn get_identity_id(&self) -> Result<String> {
        let result = self
            .call("Identity/get", json!({"accountId": self.account_id}))
            .await?;
        result["list"]
            .as_array()
            .and_then(|list| list.first())
            .and_then(|id| id["id"].as_str())
            .map(|s| s.to_string())
            .context("no identity found for this account")
    }

    pub async fn send_email(&self, msg: &OutgoingEmail<'_>) -> Result<Value> {
        let identity_id = self.get_identity_id().await?;
        let drafts_id = self
            .get_mailbox_id_by_role("drafts")
            .await?
            .context("no drafts mailbox found")?;
        let sent_id = self.ensure_mailbox_with_role("sent", "Sent").await?;
        let email = build_outgoing_email(msg, &drafts_id);

        let results = self
            .call_multi(vec![
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
                        "onSuccessUpdateEmail": {
                            "#send": {
                                format!("mailboxIds/{drafts_id}"): Value::Null,
                                format!("mailboxIds/{sent_id}"): true,
                                "keywords/$seen": true,
                            }
                        }
                    }),
                    "r1",
                ),
            ])
            .await?;

        // Return the submission result
        results.into_iter().last().context("no submission response")
    }
}

pub struct UploadedBlob {
    pub blob_id: String,
    pub size: u64,
}

pub struct EmailAttachment {
    pub blob_id: String,
    pub content_type: String,
    pub filename: String,
    pub size: u64,
}

pub struct OutgoingEmail<'a> {
    pub from: &'a str,
    pub to: &'a [String],
    pub subject: &'a str,
    pub body: &'a str,
    pub html_body: Option<&'a str>,
    pub cc: &'a [String],
    pub bcc: &'a [String],
    pub attachments: &'a [EmailAttachment],
}

fn address_list(addrs: &[String]) -> Vec<Value> {
    addrs.iter().map(|a| json!({"email": a})).collect()
}

fn build_outgoing_email(msg: &OutgoingEmail<'_>, drafts_id: &str) -> Value {
    let to_addrs = address_list(msg.to);
    let cc_addrs = address_list(msg.cc);
    let bcc_addrs = address_list(msg.bcc);

    let mut email = json!({
        "from": [{"email": msg.from}],
        "to": to_addrs,
        "subject": msg.subject,
        "bodyValues": {
            "body": {
                "value": msg.body,
                "charset": "utf-8"
            }
        },
        "textBody": [{"partId": "body", "type": "text/plain"}],
        "mailboxIds": {drafts_id: true}
    });

    if let Some(html) = msg.html_body {
        email["bodyValues"]["html"] = json!({
            "value": html,
            "charset": "utf-8"
        });
        email["htmlBody"] = json!([{"partId": "html", "type": "text/html"}]);
    }

    if !cc_addrs.is_empty() {
        email["cc"] = json!(cc_addrs);
    }
    if !bcc_addrs.is_empty() {
        email["bcc"] = json!(bcc_addrs);
    }
    if !msg.attachments.is_empty() {
        let att_list: Vec<Value> = msg
            .attachments
            .iter()
            .map(|a| {
                json!({
                    "blobId": a.blob_id,
                    "type": a.content_type,
                    "name": a.filename,
                    "size": a.size,
                    "disposition": "attachment"
                })
            })
            .collect();
        email["attachments"] = json!(att_list);
    }

    email
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapResponse {
    method_responses: Vec<Vec<Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<'a>(
        to: &'a [String],
        html: Option<&'a str>,
        cc: &'a [String],
        bcc: &'a [String],
        attachments: &'a [EmailAttachment],
    ) -> OutgoingEmail<'a> {
        OutgoingEmail {
            from: "from@x.com",
            to,
            subject: "Hi",
            body: "plain",
            html_body: html,
            cc,
            bcc,
            attachments,
        }
    }

    #[test]
    fn outgoing_email_plain_html_and_cc() {
        let to = vec!["to@y.com".into()];
        let cc = vec!["cc@z.com".into()];
        let email = build_outgoing_email(
            &sample(&to, Some("<b>html</b>"), &cc, &[], &[]),
            "drafts-id",
        );
        assert_eq!(email["from"][0]["email"], "from@x.com");
        assert_eq!(email["to"][0]["email"], "to@y.com");
        assert_eq!(email["cc"][0]["email"], "cc@z.com");
        assert!(email.get("bcc").is_none());
        assert_eq!(email["bodyValues"]["body"]["value"], "plain");
        assert_eq!(email["bodyValues"]["html"]["value"], "<b>html</b>");
        assert_eq!(email["htmlBody"][0]["partId"], "html");
        assert_eq!(email["mailboxIds"]["drafts-id"], true);
        assert!(email.get("attachments").is_none());
    }

    #[test]
    fn outgoing_email_with_attachment() {
        let to = vec!["to@y.com".into()];
        let bcc = vec!["bcc@z.com".into()];
        let attachments = [EmailAttachment {
            blob_id: "b1".into(),
            content_type: "application/pdf".into(),
            filename: "a.pdf".into(),
            size: 12,
        }];
        let email = build_outgoing_email(&sample(&to, None, &[], &bcc, &attachments), "d0");
        assert_eq!(email["bcc"][0]["email"], "bcc@z.com");
        assert_eq!(email["attachments"][0]["blobId"], "b1");
        assert_eq!(email["attachments"][0]["disposition"], "attachment");
        assert!(email.get("htmlBody").is_none());
    }
}
