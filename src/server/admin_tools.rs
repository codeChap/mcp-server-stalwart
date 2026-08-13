//! Admin principal, domain, alias, permission, password, and DSN tools.

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, model::*, tool, tool_router,
};
use serde_json::{Value, json};

use crate::params::*;
use crate::server::StalwartServer;
use crate::sieve;
use crate::util::{generate_password, tool_error, tool_result, tool_success, tool_text};

#[tool_router(router = router_admin, vis = "pub(crate)")]
impl StalwartServer {
    #[tool(description = "List all domains configured on the server. Requires admin API.")]
    async fn list_domains(&self) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;
        Ok(tool_result(admin.list_principals("domain").await))
    }

    #[tool(description = "Add a domain to the server so it can receive email. Requires admin API.")]
    async fn create_domain(
        &self,
        Parameters(p): Parameters<CreateDomainParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let principal = json!({
            "type": "domain",
            "name": p.domain
        });

        match admin.create_account(principal).await {
            Ok(result) => Ok(tool_success(&json!({
                "status": "created",
                "domain": p.domain,
                "result": result
            }))),
            Err(e) => Ok(tool_error(e.to_string())),
        }
    }

    #[tool(description = "Create a new email account on the server. Requires admin API.")]
    async fn create_account(
        &self,
        Parameters(p): Parameters<CreateAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let mut principal = json!({
            "type": "individual",
            "name": p.email,
            "emails": [p.email],
            "secrets": [p.password],
            "quota": p.quota.unwrap_or(0)
        });

        if let Some(desc) = &p.description {
            principal["description"] = json!(desc);
        }

        if let Some(perms) = &p.permissions {
            principal["enabledPermissions"] = json!(perms);
        }

        match admin.create_account(principal).await {
            Ok(result) => Ok(tool_success(&json!({
                "status": "created",
                "account": p.email,
                "description": p.description,
                "result": result
            }))),
            Err(e) => Ok(tool_error(e.to_string())),
        }
    }

    #[tool(
        description = "List all email accounts on the server, or get details for a specific account by name. Requires admin API."
    )]
    async fn list_accounts(
        &self,
        Parameters(p): Parameters<ListAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        if let Some(name) = &p.name {
            Ok(tool_result(admin.get_account(name).await))
        } else {
            Ok(tool_result(admin.list_accounts().await))
        }
    }

    #[tool(description = "Add or remove an email alias on an account. Requires admin API.")]
    async fn manage_aliases(
        &self,
        Parameters(p): Parameters<ManageAliasesParams>,
    ) -> Result<CallToolResult, McpError> {
        let action = p.action.to_lowercase();
        if action != "add" && action != "remove" {
            return Err(McpError::invalid_params(
                "action must be 'add' or 'remove'",
                None,
            ));
        }

        let op = if action == "add" {
            "addItem"
        } else {
            "removeItem"
        };

        let updated = self
            .patch_account(
                &p.account,
                vec![json!({
                    "action": op,
                    "field": "emails",
                    "value": p.alias
                })],
            )
            .await?;

        Ok(tool_success(&json!({
            "status": action,
            "alias": p.alias,
            "account": p.account,
            "emails": updated["emails"]
        })))
    }

    #[tool(
        description = "Update an account's enabledPermissions. Newly-created principals start with no permissions \
                           and cannot authenticate, send, or receive mail until permissions are granted. \
                           Actions: 'set' replaces the list, 'add' grants, 'remove' revokes. Requires admin API."
    )]
    async fn update_account_permissions(
        &self,
        Parameters(p): Parameters<UpdateAccountPermissionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let action = p.action.as_deref().unwrap_or("set").to_lowercase();
        let changes = permission_changes(&action, &p.permissions)?;

        if changes.is_empty() {
            return Err(McpError::invalid_params(
                "permissions must not be empty for add/remove actions",
                None,
            ));
        }

        let updated = self.patch_account(&p.account, changes).await?;

        Ok(tool_success(&json!({
            "status": "ok",
            "action": action,
            "account": p.account,
            "enabledPermissions": updated["enabledPermissions"],
        })))
    }

    #[tool(
        description = "Reset an account's password. If 'password' is omitted, a strong random password is generated. \
                           The new password is returned in plaintext so it can be delivered to the user — handle the response carefully. \
                           Requires admin API."
    )]
    async fn reset_password(
        &self,
        Parameters(p): Parameters<ResetPasswordParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        let new_password = match p.password {
            Some(pw) if !pw.is_empty() => pw,
            _ => generate_password(24).map_err(|e| {
                McpError::internal_error(format!("failed to generate password: {e}"), None)
            })?,
        };

        let changes = vec![json!({
            "action": "set",
            "field": "secrets",
            "value": [&new_password],
        })];

        admin
            .update_account(&p.account, changes)
            .await
            .map_err(Self::map_admin)?;

        Ok(tool_success(&json!({
            "status": "reset",
            "account": p.account,
            "password": new_password,
        })))
    }

    #[tool(
        description = "List email addresses that have DSN delivery reports enabled (requires admin API)"
    )]
    async fn get_dsn_accounts(&self) -> Result<CallToolResult, McpError> {
        let script = self.dsn_script().await?;

        if script.is_empty() {
            return Ok(tool_text(
                "No DSN notify script configured. No accounts have delivery reports enabled.",
            ));
        }

        Ok(tool_result(sieve::parse_dsn_script(&script)))
    }

    #[tool(
        description = "Set which email addresses get DSN delivery reports (SUCCESS + FAILURE). \
                           Replaces the full list. Requires admin API."
    )]
    async fn set_dsn_accounts(
        &self,
        Parameters(p): Parameters<SetDsnAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let admin = self.require_admin()?;

        if p.accounts.is_empty() {
            return Err(McpError::invalid_params(
                "Provide at least one email address. To disable DSN entirely, remove the script via the admin panel.",
                None,
            ));
        }

        let current = self.dsn_script().await?;

        if !current.is_empty()
            && let Err(e) = sieve::parse_dsn_script(&current)
        {
            return Ok(tool_error(format!(
                "Current script has custom modifications that would be lost:\n{e}\n\n\
                 Remove or fix the script manually via the admin panel before using this tool."
            )));
        }

        let new_script = sieve::generate_dsn_script(&p.accounts)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        admin
            .set_settings(vec![(
                "sieve.trusted.scripts.dsn-notify.contents".to_string(),
                new_script,
            )])
            .await
            .map_err(Self::map_admin)?;

        Ok(tool_success(&json!({
            "status": "updated",
            "accounts": p.accounts
        })))
    }
}

fn permission_changes(action: &str, permissions: &[String]) -> Result<Vec<Value>, McpError> {
    match action {
        "set" => Ok(vec![json!({
            "action": "set",
            "field": "enabledPermissions",
            "value": permissions,
        })]),
        "add" => Ok(permissions
            .iter()
            .map(|perm| {
                json!({
                    "action": "addItem",
                    "field": "enabledPermissions",
                    "value": perm,
                })
            })
            .collect()),
        "remove" => Ok(permissions
            .iter()
            .map(|perm| {
                json!({
                    "action": "removeItem",
                    "field": "enabledPermissions",
                    "value": perm,
                })
            })
            .collect()),
        _ => Err(McpError::invalid_params(
            "action must be 'set', 'add', or 'remove'",
            None,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_set_replaces_list() {
        let changes = permission_changes("set", &["email-send".into()]).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["action"], "set");
        assert_eq!(changes[0]["value"], json!(["email-send"]));
    }

    #[test]
    fn permission_add_and_remove_are_per_item() {
        let add = permission_changes("add", &["a".into(), "b".into()]).unwrap();
        assert_eq!(add.len(), 2);
        assert_eq!(add[0]["action"], "addItem");
        assert_eq!(add[1]["value"], "b");

        let remove = permission_changes("remove", &["a".into()]).unwrap();
        assert_eq!(remove[0]["action"], "removeItem");
    }

    #[test]
    fn permission_rejects_unknown_action() {
        assert!(permission_changes("nope", &["a".into()]).is_err());
    }
}
