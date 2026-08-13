mod admin;
mod check_sent;
mod jmap;
mod params;
mod secrets;
mod server;
mod sieve;
mod util;

use anyhow::{Context, Result};
use rmcp::{ServiceExt, transport::stdio};
use std::env;

use admin::AdminClient;
use jmap::JmapClient;
use server::StalwartServer;

#[tokio::main]
async fn main() -> Result<()> {
    let session_url = env::var("JMAP_SESSION_URL").context("JMAP_SESSION_URL is required")?;
    let username = env::var("JMAP_USERNAME").context("JMAP_USERNAME is required")?;
    let password = env::var("JMAP_PASSWORD").context("JMAP_PASSWORD is required")?;

    let client = JmapClient::connect(&session_url, &username, &password).await?;

    let admin = match (
        env::var("STALWART_ADMIN_URL"),
        env::var("STALWART_ADMIN_PASSWORD"),
    ) {
        (Ok(url), Ok(pass)) => {
            let user = env::var("STALWART_ADMIN_USER").unwrap_or_else(|_| "admin".into());
            // A failed admin connection (e.g. the box is briefly unreachable at
            // spawn time) must NOT abort startup — that would drop every tool,
            // including the JMAP ones, and leave the agent with no way to reach
            // the server at all. Degrade gracefully: warn and run without admin.
            match AdminClient::connect(&url, &user, &pass).await {
                Ok(client) => Some(client),
                Err(e) => {
                    eprintln!(
                        "warning: Stalwart admin API unavailable ({e:#}); \
                         admin tools (including check_sent) will be disabled this session"
                    );
                    None
                }
            }
        }
        _ => None,
    };

    let mailbox_passwords = secrets::load_from_env()?;
    if !mailbox_passwords.is_empty() {
        eprintln!(
            "loaded {} extra mailbox password(s) from JMAP_SECRETS_FILE / JMAP_ACCOUNTS",
            mailbox_passwords.len()
        );
    }

    let server = StalwartServer::new(client, admin, mailbox_passwords);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
