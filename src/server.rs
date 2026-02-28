use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::jmap::JmapClient;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    #[schemars(description = "Text to search for in email subject, body, from, to fields")]
    pub query: Option<String>,

    #[schemars(description = "Filter by sender email address")]
    pub from: Option<String>,

    #[schemars(description = "Filter by recipient email address")]
    pub to: Option<String>,

    #[schemars(description = "Filter by subject text")]
    pub subject: Option<String>,

    #[schemars(description = "Mailbox ID to search within")]
    pub mailbox_id: Option<String>,

    #[schemars(description = "Start position for pagination (default 0)")]
    pub position: Option<u32>,

    #[schemars(description = "Maximum results to return (default 10, max 50)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetEmailsParams {
    #[schemars(description = "List of email IDs to retrieve")]
    pub ids: Vec<String>,
}

#[derive(Clone)]
pub struct StalwartServer {
    client: Arc<JmapClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl StalwartServer {
    pub fn new(client: JmapClient) -> Self {
        Self {
            client: Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List all mailboxes/folders with message counts")]
    async fn get_mailboxes(&self) -> Result<CallToolResult, McpError> {
        match self.client.get_mailboxes().await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Search emails with filters (query text, from, to, subject, mailbox). \
                           Returns email IDs — use get_emails to read full content.")]
    async fn search_emails(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut conditions: Vec<serde_json::Value> = Vec::new();

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

        let filter = if conditions.len() == 1 {
            conditions.remove(0)
        } else if conditions.is_empty() {
            json!({})
        } else {
            json!({"operator": "AND", "conditions": conditions})
        };

        let position = p.position.unwrap_or(0);
        let limit = p.limit.unwrap_or(10).min(50);

        match self.client.search_emails(filter, None, position, limit).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Get full email content by IDs. Returns subject, from, to, date, \
                           body text, and metadata for each email.")]
    async fn get_emails(
        &self,
        Parameters(p): Parameters<GetEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.is_empty() {
            return Err(McpError::invalid_params("ids must not be empty", None));
        }
        match self.client.get_emails(&p.ids).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }
}

#[tool_handler]
impl ServerHandler for StalwartServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "stalwart".into(),
                title: None,
                version: env!("CARGO_PKG_VERSION").into(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Stalwart mail server MCP. Tools: get_mailboxes, search_emails, get_emails. \
                 Search returns email IDs; use get_emails to read content."
                    .into(),
            ),
        }
    }
}
