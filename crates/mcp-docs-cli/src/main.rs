//! mcp-docs 命令行工具。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};

/// 检索本地 rustdoc 文档的命令行工具。
#[derive(Debug, Parser)]
#[command(name = "mcp-docs", version, about = "检索本地 rustdoc 文档")]
struct Cli {
    /// rustdoc 产物目录（`cargo doc` 的输出目录）。
    #[arg(long, global = true, default_value = "target/doc")]
    doc_dir: PathBuf,

    /// 输出目录（markdown 与 index.json.gz 的落盘根）。
    #[arg(long, global = true, default_value = "target/doc-search")]
    out: PathBuf,

    /// 共享库根目录（跨项目复用 crate 文档索引）；缺省用平台缓存目录。
    #[arg(long, global = true, env = "MCP_DOCS_STORE")]
    store: Option<PathBuf>,

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
    /// 导出所有条目为 markdown 文件树，并生成 index.json.gz。
    Export {
        /// 只导出指定 crate。
        #[arg(long = "crate")]
        crate_name: Option<String>,
        /// 增量：复用未变化条目，只重建改动的部分。
        #[arg(long)]
        incremental: bool,
        /// 导出粒度：`member`（成员单独落盘，默认）/ `item`（成员只内联）。
        #[arg(long, value_enum, default_value_t = GranularityArg::Member)]
        granularity: GranularityArg,
    },
    /// 检索条目。
    Search {
        /// 查询词。
        query: String,
        /// 返回上限。
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// 跳过的命中数（分页）。
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// 匹配模式。
        #[arg(long, value_enum, default_value_t = ModeArg::Substring)]
        mode: ModeArg,
        /// 限定 crate。
        #[arg(long = "crate")]
        crate_name: Option<String>,
        /// 限定条目类型（如 struct / fn / trait）。
        #[arg(long)]
        kind: Option<String>,
    },
}

/// 命令行侧的匹配模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum ModeArg {
    /// 子串匹配。
    #[default]
    Substring,
    /// 前缀匹配。
    Prefix,
    /// 模糊子序列匹配。
    Fuzzy,
}

/// 命令行侧的导出粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum GranularityArg {
    /// 成员单独落盘（默认）。
    #[default]
    Member,
    /// 只写顶层条目，成员仅内联。
    Item,
}

impl From<GranularityArg> for mcp_docs_core::Granularity {
    fn from(value: GranularityArg) -> Self {
        match value {
            GranularityArg::Member => Self::Member,
            GranularityArg::Item => Self::Item,
        }
    }
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
    // 未显式指定共享库时回落到平台缓存目录；两者都没有则关闭共享。
    let store = cli.store.clone().or_else(mcp_docs_core::default_store_root);

    match &cli.command {
        Command::Tree => print_tree(&cli.doc_dir),
        Command::Show { id } => show(&cli.doc_dir, &cli.out, id),
        Command::Export {
            crate_name,
            incremental,
            granularity,
        } => export(
            &cli.doc_dir,
            &cli.out,
            *incremental,
            crate_name.as_deref(),
            (*granularity).into(),
            store.as_deref(),
        ),
        Command::Search {
            query,
            limit,
            offset,
            mode,
            crate_name,
            kind,
        } => run_search(
            &cli.doc_dir,
            &cli.out,
            query,
            *limit,
            *offset,
            *mode,
            crate_name.as_deref(),
            kind.as_deref(),
            store.as_deref(),
        ),
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
///
/// 走索引（`out_dir/index.json.gz`），与 MCP `get_item` 行为一致：id 可省略类型标记；
/// 命中跨 crate 模块重导出别名时委托目标 crate 的合成概览。
fn show(doc_dir: &Path, out_dir: &Path, id: &str) -> anyhow::Result<()> {
    let index_path = out_dir.join(mcp_docs_core::INDEX_FILE_NAME);
    let index = mcp_docs_core::load_index(&index_path).with_context(|| {
        format!(
            "读取索引失败：{}；请先运行 `mcp-docs export` 生成索引",
            index_path.display()
        )
    })?;
    print!("{}", render_show(&index, doc_dir, id)?);
    Ok(())
}

/// 渲染 `show` 的输出文本（可测的纯逻辑）。
///
/// 顺序与 MCP `get_item` 一致：① 命中跨 crate 模块别名 → 委托目标 crate 概览；
/// ② 未命中但 id 是已索引 crate 名 → 合成概览；③ 命中普通条目 → 结构化摘要。
fn render_show(index: &mcp_docs_core::Index, doc_dir: &Path, id: &str) -> anyhow::Result<String> {
    let ids = mcp_docs_core::IdIndex::build(&index.items);
    let summary = ids.find(id).map(|pos| &index.items[pos]);

    let Some(summary) = summary else {
        // 与 MCP `get_item` 一致：id 恰为已索引 crate 名时回退合成概览（crate 首页不入索引）。
        if let Some(overview) = mcp_docs_core::crate_overview(index, id) {
            return Ok(format!("{overview}\n"));
        }
        anyhow::bail!("未找到条目 `{id}`");
    };

    // 跨 crate 模块重导出别名：委托目标 crate 的合成概览（与 MCP `get_item` 一致）。
    if let Some(target) = mcp_docs_core::cross_crate_module_target(summary)
        && let Some(overview) = mcp_docs_core::crate_overview(index, target)
    {
        return Ok(format!("{overview}\n"));
    }

    let html_path = doc_dir.join(&summary.html_path);
    let html = std::fs::read_to_string(&html_path)
        .with_context(|| format!("读取 {} 失败", html_path.display()))?;
    let item =
        mcp_docs_core::parse_item_html(&html, Path::new(&summary.html_path), &Default::default())?;

    let mut out = format!("{}  [{}]\n", item.id, item.kind.file_prefix());
    if let Some(signature) = &item.signature {
        out.push_str(&format!("\n--- 签名 ---\n{signature}\n"));
    }
    if let Some(docs) = &item.docs_md {
        out.push_str(&format!("\n--- 文档 ---\n{docs}\n"));
    }
    if !item.sections.is_empty() {
        let titles: Vec<&str> = item.sections.iter().map(|s| s.title.as_str()).collect();
        out.push_str(&format!("\n--- 分节 ---\n{}\n", titles.join(" / ")));
    }
    if !item.members.is_empty() {
        out.push_str(&format!("\n--- 成员（{}）---\n", item.members.len()));
        for member in &item.members {
            out.push_str(&format!(
                "{:<12} {}\n",
                member.kind.file_prefix(),
                member.id
            ));
            if let Some(docs) = &member.docs_md {
                out.push_str(&format!(
                    "             {}\n",
                    docs.lines().next().unwrap_or("")
                ));
            }
        }
    }
    Ok(out)
}

/// 导出 markdown 文件树，并生成 `index.json.gz` 与 `meta.json`。
fn export(
    doc_dir: &Path,
    out_dir: &Path,
    incremental: bool,
    crate_filter: Option<&str>,
    granularity: mcp_docs_core::Granularity,
    store: Option<&Path>,
) -> anyhow::Result<()> {
    let opts = mcp_docs_core::BuildOptions {
        write_markdown: true,
        incremental,
        persist: true,
        crate_filter: crate_filter.map(str::to_string),
        granularity,
        store: store.map(Path::to_path_buf),
    };
    let report = mcp_docs_core::build(doc_dir, out_dir, &opts)?;

    println!(
        "已导出 {} 个 markdown 文件（重新解析 {}，复用 {}；共享库命中 {}，写入 {}），索引 {} 个条目 → {}",
        report.written,
        report.parsed,
        report.reused,
        report.shared_hits,
        report.shared_written,
        report.index.items.len(),
        out_dir.join(mcp_docs_core::INDEX_FILE_NAME).display()
    );
    if !report.skipped.is_empty() {
        println!(
            "跳过 {} 个无法解析的产物（如宏重定向页）：",
            report.skipped.len()
        );
        for entry in report.skipped.iter().take(5) {
            println!("  - {entry}");
        }
    }
    Ok(())
}

/// 检索索引（缺索引时先构建）。
#[allow(clippy::too_many_arguments)]
fn run_search(
    doc_dir: &Path,
    out_dir: &Path,
    query: &str,
    limit: usize,
    offset: usize,
    mode: ModeArg,
    crate_name: Option<&str>,
    kind: Option<&str>,
    store: Option<&Path>,
) -> anyhow::Result<()> {
    let index_path = out_dir.join(mcp_docs_core::INDEX_FILE_NAME);
    let index = match mcp_docs_core::load_index(&index_path) {
        Ok(index) => index,
        Err(_) => {
            // 缺索引时构建并落盘（共享库可加速依赖 crate）。
            let opts = mcp_docs_core::BuildOptions {
                persist: true,
                store: store.map(Path::to_path_buf),
                ..Default::default()
            };
            mcp_docs_core::build(doc_dir, out_dir, &opts)?.index
        }
    };

    let mut search = mcp_docs_core::SearchQuery::new(query);
    search.limit = limit;
    search.offset = offset;
    search.mode = match mode {
        ModeArg::Substring => mcp_docs_core::MatchMode::Substring,
        ModeArg::Prefix => mcp_docs_core::MatchMode::Prefix,
        ModeArg::Fuzzy => mcp_docs_core::MatchMode::Fuzzy,
    };
    search.crate_name = crate_name.map(str::to_string);
    if let Some(kind) = kind {
        let kind = mcp_docs_core::ItemKind::from_file_prefix(kind)
            .ok_or_else(|| anyhow::anyhow!("未知的条目类型 `{kind}`"))?;
        search.kinds.push(kind);
    }

    let hits = mcp_docs_core::search(&index, &search);
    println!("找到 {} 个条目：", hits.len());
    for hit in &hits {
        println!(
            "{:<5} {:<12} {:<34} {}",
            hit.score,
            hit.item.kind.file_prefix(),
            hit.item.id,
            hit.item.one_line
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_docs_core::{CrateSummary, Granularity, Index, ItemId, ItemKind, ItemSummary};

    /// 构造一个最小条目摘要。
    fn summary(
        id: &str,
        kind: ItemKind,
        name: &str,
        path: &[&str],
        html_path: &str,
    ) -> ItemSummary {
        ItemSummary {
            id: ItemId(id.to_string()),
            kind,
            name: name.to_string(),
            path: path.iter().map(|s| (*s).to_string()).collect(),
            one_line: String::new(),
            signature: None,
            has_docs: false,
            has_members: false,
            file: String::new(),
            html_path: html_path.to_string(),
            parent_id: None,
            src_mtime: None,
        }
    }

    /// fixture：`gpui_kit` 把 `gpui_component` 作为模块重导出，且 `gpui_component` 已索引。
    fn index_with_cross_crate_alias() -> Index {
        Index {
            schema_version: 0,
            rustdoc_version: None,
            generated_at: 0,
            target_doc: String::new(),
            granularity: Granularity::default(),
            crates: vec![
                CrateSummary {
                    name: "gpui_component".to_string(),
                    item_count: 1,
                },
                CrateSummary {
                    name: "gpui_kit".to_string(),
                    item_count: 1,
                },
            ],
            items: vec![
                summary(
                    "gpui_kit::mod.component",
                    ItemKind::Module,
                    "component",
                    &["gpui_kit"],
                    "gpui_component/index.html",
                ),
                summary(
                    "gpui_component::button::struct.Button",
                    ItemKind::Struct,
                    "Button",
                    &["gpui_component", "button"],
                    "gpui_component/button/struct.Button.html",
                ),
            ],
        }
    }

    /// 跨 crate 模块别名：省略与带类型标记两种写法都委托目标 crate 概览。
    #[test]
    fn render_show_delegates_cross_crate_module_alias() {
        let index = index_with_cross_crate_alias();
        let doc = Path::new("/nonexistent");
        for id in ["gpui_kit::component", "gpui_kit::mod.component"] {
            let out = render_show(&index, doc, id).unwrap();
            assert!(out.starts_with("# gpui_component"), "id={id}: {out}");
            assert!(out.contains("共 1 条目"), "id={id}: {out}");
        }
    }

    /// 已索引的 crate 名（crate 首页不入索引）回退合成概览。
    #[test]
    fn render_show_falls_back_to_crate_overview() {
        let index = index_with_cross_crate_alias();
        let out = render_show(&index, Path::new("/nonexistent"), "gpui_component").unwrap();
        assert!(out.starts_with("# gpui_component"), "{out}");
    }

    /// 完全不存在且非 crate 名的 id 报未找到。
    #[test]
    fn render_show_errors_on_unknown_id() {
        let index = index_with_cross_crate_alias();
        let err = render_show(&index, Path::new("/nonexistent"), "gpui_kit::Nope").unwrap_err();
        assert!(err.to_string().contains("未找到条目"), "{err}");
    }
}
