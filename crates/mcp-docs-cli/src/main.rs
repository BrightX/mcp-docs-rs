//! mcp-docs 命令行工具。

use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// 检索本地 rustdoc 文档的命令行工具。
#[derive(Debug, Parser)]
#[command(name = "mcp-docs", version, about = "检索本地 rustdoc 文档")]
struct Cli {
    /// rustdoc 产物目录（`cargo doc` 的输出目录）。
    #[arg(long, global = true, default_value = "target/doc")]
    doc_dir: std::path::PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 打印条目树（调试用）。
    Tree,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("错误：{err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    match &cli.command {
        Command::Tree => print_tree(&cli.doc_dir),
    }
}

/// 打印 `doc_dir` 下所有已发现的条目。
fn print_tree(doc_dir: &Path) -> anyhow::Result<()> {
    let items = mcp_docs_core::discover_all(doc_dir)?;
    println!("共 {} 个条目：", items.len());
    for item in &items {
        println!(
            "{:<10} {:<36} {}",
            item.kind.file_prefix(),
            item.id,
            item.html_path.display()
        );
    }
    Ok(())
}
