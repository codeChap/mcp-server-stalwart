mod admin;
mod jmap;
mod server;
mod sieve;

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

    let admin = match (env::var("STALWART_ADMIN_URL"), env::var("STALWART_ADMIN_PASSWORD")) {
        (Ok(url), Ok(pass)) => {
            let user = env::var("STALWART_ADMIN_USER").unwrap_or_else(|_| "admin".into());
            Some(AdminClient::connect(&url, &user, &pass).await?)
        }
        _ => None,
    };

    let server = StalwartServer::new(client, admin);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
