//! mcp-docs 命令行工具。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};

/// 检索本地 rustdoc 文档的命令行工具。
#[derive(Debug, Parser)]
#[command(name = "mcp-docs", version, about = "检索本地 rustdoc 文档")]
struct Cli {
    /// rustdoc 产物目录（`cargo doc` 的输出目录）。
    #[arg(long, global = true, default_value = "target/doc")]
    doc_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 打印条目树（调试用）。
    Tree,
    /// 解析并打印单个条目。
    Show {
        /// 条目 id，如 `doc_probe::Demo`。
        id: String,
    },
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
        Command::Show { id } => show(&cli.doc_dir, id),
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

/// 解析并打印单个条目。
fn show(doc_dir: &Path, id: &str) -> anyhow::Result<()> {
    let items = mcp_docs_core::discover_all(doc_dir)?;
    let discovered = items
        .iter()
        .find(|item| item.id.0 == id)
        .ok_or_else(|| anyhow::anyhow!("未找到条目 `{id}`"))?;

    let html_path = doc_dir.join(&discovered.html_path);
    let html = std::fs::read_to_string(&html_path)
        .with_context(|| format!("读取 {} 失败", html_path.display()))?;
    let item = mcp_docs_core::parse_item_html(&html, &discovered.html_path, &Default::default())?;

    println!("{}  [{}]", item.id, item.kind.file_prefix());

    if let Some(signature) = &item.signature {
        println!("\n--- 签名 ---\n{signature}");
    }
    if let Some(docs) = &item.docs_md {
        println!("\n--- 文档 ---\n{docs}");
    }
    if !item.sections.is_empty() {
        let titles: Vec<&str> = item.sections.iter().map(|s| s.title.as_str()).collect();
        println!("\n--- 分节 ---\n{}", titles.join(" / "));
    }
    if !item.members.is_empty() {
        println!("\n--- 成员（{}）---", item.members.len());
        for member in &item.members {
            println!("{:<12} {}", member.kind.file_prefix(), member.id);
            if let Some(docs) = &member.docs_md {
                println!("             {}", docs.lines().next().unwrap_or(""));
            }
        }
    }
    Ok(())
}
