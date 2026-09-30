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
    /// 导出所有条目为 markdown 文件树。
    Export {
        /// 输出目录。
        #[arg(long, default_value = "target/doc-search")]
        out: PathBuf,
        /// 只导出指定 crate。
        #[arg(long = "crate")]
        crate_name: Option<String>,
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
        Command::Export { out, crate_name } => export(&cli.doc_dir, out, crate_name.as_deref()),
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

/// 导出所有条目为 markdown 文件树。
fn export(doc_dir: &Path, out_dir: &Path, crate_filter: Option<&str>) -> anyhow::Result<()> {
    let items = mcp_docs_core::discover_all(doc_dir)?;
    let opts = mcp_docs_core::RenderOptions::default();
    let parse_opts = mcp_docs_core::ParseOptions::default();
    let (mut written, mut skipped) = (0usize, 0usize);

    for discovered in &items {
        if let Some(filter) = crate_filter {
            if discovered.id.0.split("::").next() != Some(filter) {
                continue;
            }
        }

        let html_path = doc_dir.join(&discovered.html_path);
        let html = match std::fs::read_to_string(&html_path) {
            Ok(html) => html,
            Err(err) => {
                eprintln!("跳过 {}：{err}", html_path.display());
                skipped += 1;
                continue;
            }
        };
        let item = mcp_docs_core::parse_item_html(&html, &discovered.html_path, &parse_opts)?;

        // 条目文件本身。
        let markdown = mcp_docs_core::render_item(&item, &opts);
        let out_path = mcp_docs_core::item_output_path(out_dir, &discovered.html_path);
        mcp_docs_core::atomic_write(&out_path, markdown.as_bytes())?;
        written += 1;

        // 每个成员单独成文件，便于精确检索。
        for member in &item.members {
            let markdown = mcp_docs_core::render_member_item(member, &opts);
            let out_path =
                mcp_docs_core::member_output_path(out_dir, &discovered.html_path, &member.name);
            mcp_docs_core::atomic_write(&out_path, markdown.as_bytes())?;
            written += 1;
        }
    }

    println!("已导出 {written} 个 markdown 文件到 {}", out_dir.display());
    if skipped > 0 {
        println!("跳过 {skipped} 个（HTML 读取失败）");
    }
    Ok(())
}
