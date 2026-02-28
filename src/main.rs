mod jmap;
mod server;

use anyhow::{Context, Result};
use rmcp::{ServiceExt, transport::stdio};

use jmap::JmapClient;
use server::StalwartServer;

#[tokio::main]
async fn main() -> Result<()> {
    let session_url =
        std::env::var("JMAP_SESSION_URL").context("JMAP_SESSION_URL is required")?;
    let username = std::env::var("JMAP_USERNAME").context("JMAP_USERNAME is required")?;
    let password = std::env::var("JMAP_PASSWORD").context("JMAP_PASSWORD is required")?;

    let client = JmapClient::connect(&session_url, &username, &password).await?;
    let server = StalwartServer::new(client);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
