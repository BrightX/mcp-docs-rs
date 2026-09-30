//! mcp-docs 的 MCP 服务入口（stdio）。

mod server;

use std::path::PathBuf;

use clap::Parser;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

/// 以 MCP 服务形式检索本地 rustdoc 文档。
#[derive(Debug, Parser)]
#[command(
    name = "mcp-docs-server",
    version,
    about = "以 MCP 服务形式检索本地 rustdoc 文档"
)]
struct Args {
    /// rustdoc 产物目录（`cargo doc` 的输出目录）。
    #[arg(long, default_value = "target/doc")]
    doc_dir: PathBuf,

    /// 输出目录（`index.json` / `meta.json` 的落盘根）。
    #[arg(long, default_value = "target/doc-search")]
    out_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let service = server::DocsServer::new(args.doc_dir, args.out_dir)?
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
