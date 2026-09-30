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
    #[arg(long, env = "MCP_DOCS_DIR", default_value = "target/doc")]
    doc_dir: PathBuf,

    /// 输出目录（`index.json` / `meta.json` 的落盘根）。
    #[arg(long, env = "MCP_DOCS_OUT", default_value = "target/doc-search")]
    out_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // 规范化为绝对路径：`get_item_source` 返回的 `path` 才算「可直接打开」，
    // 且不受客户端工作目录影响。
    let doc_dir = std::path::absolute(&args.doc_dir).unwrap_or(args.doc_dir);
    let out_dir = std::path::absolute(&args.out_dir).unwrap_or(args.out_dir);

    let service = server::DocsServer::new(doc_dir, out_dir)?
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
