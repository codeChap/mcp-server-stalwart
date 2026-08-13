//! JMAP mailbox and email tools.

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, model::*, tool, tool_router,
};
use serde_json::{Value, json};

use crate::jmap::{EmailAttachment, JmapClient, OutgoingEmail};
use crate::params::*;
use crate::server::StalwartServer;
use crate::util::{guess_mime, tool_result, tool_success, tool_text};

#[tool_router(router = router_mail, vis = "pub(crate)")]
impl StalwartServer {
    #[tool(description = "List all mailboxes/folders with message counts. \
                           Optional `account` switches mailbox.")]
    async fn get_mailboxes(
        &self,
        Parameters(p): Parameters<OptionalAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);
        Ok(tool_result(client.get_mailboxes().await))
    }

    #[tool(
        description = "Search emails with filters (query text, from, to, subject, mailbox). \
                       Returns email IDs — use get_emails to read full content. Optional `account` switches mailbox \
                       (JMAP_SECRETS_FILE / JMAP_ACCOUNTS, or admin API). \
                       WARNING: Do NOT use this to verify outbound SMTP sends (invoice mailers, contact forms) — \
                       submissions are NOT auto-saved to Sent. Use check_sent (admin logs) instead."
    )]
    async fn search_emails(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let filter = build_email_filter(&p);
        let position = p.position.unwrap_or(0);
        let limit = p.limit.unwrap_or(10).min(50);

        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);

        Ok(tool_result(
            client.search_emails(filter, None, position, limit).await,
        ))
    }

    #[tool(
        description = "Get full email content by IDs. Returns subject, from, to, date, \
                           body text, and metadata for each email."
    )]
    async fn get_emails(
        &self,
        Parameters(p): Parameters<GetEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.is_empty() {
            return Err(McpError::invalid_params("ids must not be empty", None));
        }

        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);

        Ok(tool_result(client.get_emails(&p.ids).await))
    }

    #[tool(description = "Permanently delete emails by ID. This cannot be undone.")]
    async fn delete_emails(
        &self,
        Parameters(p): Parameters<DeleteEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.is_empty() {
            return Err(McpError::invalid_params("ids must not be empty", None));
        }
        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);
        Ok(tool_result(client.delete_emails(&p.ids).await))
    }

    #[tool(
        description = "Create a new mailbox/folder. Optionally set a role (archive, drafts, junk, sent, trash) \
                           or nest under a parent mailbox."
    )]
    async fn create_mailbox(
        &self,
        Parameters(p): Parameters<CreateMailboxParams>,
    ) -> Result<CallToolResult, McpError> {
        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);
        Ok(tool_result(
            client
                .create_mailbox(&p.name, p.parent_id.as_deref(), p.role.as_deref())
                .await,
        ))
    }

    #[tool(
        description = "Download all attachments from an email to a local directory. \
                           Returns the list of saved file paths."
    )]
    async fn download_attachments(
        &self,
        Parameters(p): Parameters<DownloadAttachmentsParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::fs::create_dir_all(&p.download_dir)
            .await
            .map_err(|e| {
                McpError::invalid_params(
                    format!("cannot create directory '{}': {}", p.download_dir, e),
                    None,
                )
            })?;

        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);

        let meta = client
            .get_email_attachments(std::slice::from_ref(&p.email_id))
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let email = meta["list"]
            .as_array()
            .and_then(|list| list.first())
            .ok_or_else(|| McpError::invalid_params("email not found", None))?;

        let attachments = match email["attachments"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(tool_text("No attachments found on this email.")),
        };

        Ok(tool_success(
            &save_attachments(client, attachments, &p.download_dir).await?,
        ))
    }
}

#[tool_router(router = router_send, vis = "pub(crate)")]
impl StalwartServer {
    #[tool(description = "Send an email with optional file attachments. \
                           Optional `account` sends as that mailbox (e.g. hello@codechap.com) \
                           instead of the default JMAP user.")]
    async fn send_email(
        &self,
        Parameters(p): Parameters<SendEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.to.is_empty() {
            return Err(McpError::invalid_params("to must not be empty", None));
        }
        let resolved = self.resolve_client(&p.account).await?;
        let client = resolved.get(&self.client);
        let from = client.username();
        let cc = p.cc.unwrap_or_default();
        let bcc = p.bcc.unwrap_or_default();
        let uploaded = upload_attachments(client, p.attachments.unwrap_or_default()).await?;

        Ok(tool_result(
            client
                .send_email(&OutgoingEmail {
                    from,
                    to: &p.to,
                    subject: &p.subject,
                    body: &p.body,
                    html_body: p.html_body.as_deref(),
                    cc: &cc,
                    bcc: &bcc,
                    attachments: &uploaded,
                })
                .await,
        ))
    }
}

fn build_email_filter(p: &SearchParams) -> Value {
    let mut conditions: Vec<Value> = Vec::new();

    if let Some(q) = &p.query {
        conditions.push(json!({"text": q}));
    }
    if let Some(from) = &p.from {
        conditions.push(json!({"from": from}));
    }
    if let Some(to) = &p.to {
        conditions.push(json!({"to": to}));
    }
    if let Some(subject) = &p.subject {
        conditions.push(json!({"subject": subject}));
    }
    if let Some(mailbox_id) = &p.mailbox_id {
        conditions.push(json!({"inMailbox": mailbox_id}));
    }

    if conditions.len() == 1 {
        conditions.remove(0)
    } else if conditions.is_empty() {
        json!({})
    } else {
        json!({"operator": "AND", "conditions": conditions})
    }
}

async fn save_attachments(
    client: &JmapClient,
    attachments: &[Value],
    download_dir: &str,
) -> Result<Vec<Value>, McpError> {
    let mut saved = Vec::new();
    for att in attachments {
        let blob_id = att["blobId"].as_str().unwrap_or_default();
        let name = att["name"].as_str().unwrap_or("attachment");
        let content_type = att["type"].as_str().unwrap_or("application/octet-stream");
        let size = att["size"].as_u64().unwrap_or(0);

        if blob_id.is_empty() {
            continue;
        }

        let data = client
            .download_blob(blob_id, name, content_type)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("failed to download '{}': {}", name, e), None)
            })?;

        let dest = std::path::Path::new(download_dir).join(name);
        tokio::fs::write(&dest, &data).await.map_err(|e| {
            McpError::internal_error(format!("failed to write '{}': {}", dest.display(), e), None)
        })?;

        saved.push(json!({
            "filename": name,
            "path": dest.display().to_string(),
            "content_type": content_type,
            "size": size
        }));
    }
    Ok(saved)
}

async fn upload_attachments(
    client: &JmapClient,
    attachments: Vec<AttachmentParam>,
) -> Result<Vec<EmailAttachment>, McpError> {
    let mut uploaded = Vec::new();
    for att in attachments {
        let data = tokio::fs::read(&att.path).await.map_err(|e| {
            McpError::invalid_params(format!("failed to read '{}': {}", att.path, e), None)
        })?;
        let content_type = att
            .content_type
            .unwrap_or_else(|| guess_mime(&att.filename));
        let blob = client.upload_blob(data, &content_type).await.map_err(|e| {
            McpError::internal_error(format!("upload failed for '{}': {}", att.filename, e), None)
        })?;
        uploaded.push(EmailAttachment {
            blob_id: blob.blob_id,
            content_type,
            filename: att.filename,
            size: blob.size,
        });
    }
    Ok(uploaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(from: Option<&str>, to: Option<&str>, query: Option<&str>) -> SearchParams {
        SearchParams {
            query: query.map(str::to_string),
            from: from.map(str::to_string),
            to: to.map(str::to_string),
            subject: None,
            mailbox_id: None,
            position: None,
            limit: None,
            account: None,
        }
    }

    #[test]
    fn empty_filter_is_object() {
        assert_eq!(build_email_filter(&search(None, None, None)), json!({}));
    }

    #[test]
    fn single_condition_unwrapped() {
        assert_eq!(
            build_email_filter(&search(Some("a@b.com"), None, None)),
            json!({"from": "a@b.com"})
        );
    }

    #[test]
    fn multiple_conditions_anded() {
        let filter = build_email_filter(&search(Some("a@b.com"), Some("c@d.com"), Some("hi")));
        assert_eq!(filter["operator"], "AND");
        assert_eq!(filter["conditions"].as_array().unwrap().len(), 3);
    }
}
