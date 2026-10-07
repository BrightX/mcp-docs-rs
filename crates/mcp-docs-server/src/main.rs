//! mcp-docs 的 MCP 服务入口（stdio）。

mod config;
mod project;
mod server;

use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = config::Args::parse();
    let cfg = config::resolve_config(&args)?;

    let service = server::DocsServer::from_projects(cfg.projects, cfg.default_project, cfg.store)?
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
