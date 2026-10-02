//! 构建索引，并可选地渲染 markdown、落盘 index.json 与 meta.json。

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use rayon::prelude::*;

use crate::cache::{fingerprint_doc_root, path_mtime, write_meta, Meta};
use crate::discover;
use crate::error::Result;
use crate::markdown::{render_item, render_member_item, rewrite_links, LinkStyle, RenderOptions};
use crate::model::{
    CrateSummary, DiscoveredItem, Granularity, Index, ItemSummary, INDEX_SCHEMA_VERSION,
};
use crate::parse::{self, ParseOptions};
use crate::store::{atomic_write, atomic_write_fast, item_output_path, member_output_path};

/// 构建选项。
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// 渲染并写入 markdown 文件。
    pub write_markdown: bool,
    /// 复用未变化条目（增量）。需要 `out_root/index.json` 存在。
    pub incremental: bool,
    /// 把 `index.json` 与 `meta.json` 落盘。
    pub persist: bool,
    /// 只处理指定 crate。
    pub crate_filter: Option<String>,
    /// 导出粒度：控制成员是否单独落盘。
    pub granularity: Granularity,
}

/// 构建结果。
#[derive(Debug, Clone)]
pub struct BuildReport {
    /// 构建出的索引。
    pub index: Index,
    /// 重新解析的条目数。
    pub parsed: usize,
    /// 复用旧索引的条目数。
    pub reused: usize,
    /// 写入的 markdown 文件数。
    pub written: usize,
    /// 解析失败而跳过的条目（`id` 与原因）。
    ///
    /// rustdoc 会为宏重导出生成 `macro.foo!.html` 之类的**重定向页**，
    /// 这类页面没有正文；单个条目失败不应中断整体构建。
    pub skipped: Vec<String>,
}

/// 构建索引。
///
/// `out_root` 用于推导条目的 markdown 路径；`opts.persist` 为真时还会写出
/// `index.json` 与 `meta.json`。
pub fn build(doc_root: &Path, out_root: &Path, opts: &BuildOptions) -> Result<BuildReport> {
    let parse_opts = ParseOptions::default();
    let render_opts = RenderOptions::default();

    // 增量：读取上次索引，用于复用未变化条目。
    // 仅在 schema 与导出粒度都一致时才复用，否则按当前选项全量重建
    // （切换粒度会改变成员的 `file`，且不会补写/删除成员文件）。
    let previous = if opts.incremental {
        load_index(&out_root.join("index.json"))
            .ok()
            .filter(|index| {
                index.schema_version == INDEX_SCHEMA_VERSION
                    && index.granularity == opts.granularity
            })
    } else {
        None
    };
    // rustdoc 版本来自首个解析到的页面；并行构建下用 `OnceLock` 采集。
    let rustdoc_version_lock = OnceLock::new();
    if let Some(version) = previous
        .as_ref()
        .and_then(|index| index.rustdoc_version.clone())
    {
        let _ = rustdoc_version_lock.set(version);
    }
    let previous_items: Vec<ItemSummary> = previous.map(|index| index.items).unwrap_or_default();
    // 预建索引，避免增量时对每个条目线性扫描整份旧索引（O(n×m)）。
    let previous_by_id: HashMap<&str, &ItemSummary> = previous_items
        .iter()
        .filter(|summary| !summary.kind.is_member())
        .map(|summary| (summary.id.0.as_str(), summary))
        .collect();
    let mut previous_members: HashMap<&str, Vec<&ItemSummary>> = HashMap::new();
    for summary in previous_items.iter().filter(|s| s.kind.is_member()) {
        if let Some(parent_id) = summary.parent_id.as_deref() {
            previous_members.entry(parent_id).or_default().push(summary);
        }
    }

    // 1) 串行发现：确定待处理的 crate 与条目，保持稳定顺序（并行后需按序汇总）。
    let mut crate_names: Vec<String> = Vec::new();
    let mut jobs: Vec<(usize, DiscoveredItem)> = Vec::new();
    for crate_name in discover::list_crates(doc_root)? {
        if let Some(filter) = &opts.crate_filter {
            if &crate_name != filter {
                continue;
            }
        }
        let crate_index = crate_names.len();
        for entry in discover::discover_crate(doc_root, &crate_name)? {
            jobs.push((crate_index, entry));
        }
        crate_names.push(crate_name);
    }

    // 2) 并行解析 / 渲染 / 落盘：条目之间彼此独立，是构建的主要瓶颈。
    let results: Vec<ProcessResult> = jobs
        .par_iter()
        .map(|(crate_index, entry)| {
            process_entry(
                doc_root,
                out_root,
                entry,
                *crate_index,
                &parse_opts,
                &render_opts,
                opts,
                &previous_by_id,
                &previous_members,
                &rustdoc_version_lock,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    // 3) 串行汇总：保持条目顺序与计数语义不变。
    let mut items = Vec::with_capacity(results.iter().map(|r| r.summaries.len()).sum());
    let (mut parsed, mut reused, mut written) = (0usize, 0usize, 0usize);
    let mut skipped = Vec::new();
    let mut crate_counts = vec![0usize; crate_names.len()];
    for result in results {
        crate_counts[result.crate_index] += result.summaries.len();
        items.extend(result.summaries);
        parsed += result.parsed;
        reused += result.reused;
        written += result.written;
        if let Some(entry) = result.skipped {
            skipped.push(entry);
        }
    }

    let crates: Vec<CrateSummary> = crate_names
        .into_iter()
        .enumerate()
        .map(|(index, name)| CrateSummary {
            name,
            version: None,
            item_count: crate_counts[index],
        })
        .collect();

    let rustdoc_version = rustdoc_version_lock.into_inner();

    let index = Index {
        schema_version: INDEX_SCHEMA_VERSION,
        rustdoc_version,
        generated_at: unix_now(),
        target_doc: doc_root.to_string_lossy().into_owned(),
        granularity: opts.granularity,
        crates,
        items,
    };

    if opts.persist {
        write_index(&index, &out_root.join("index.json"))?;
        let meta = Meta {
            schema_version: INDEX_SCHEMA_VERSION,
            rustdoc_version: index.rustdoc_version.clone(),
            generated_at: index.generated_at,
            fingerprint: fingerprint_doc_root(doc_root),
        };
        write_meta(&meta, &out_root.join("meta.json"))?;
    }

    Ok(BuildReport {
        index,
        parsed,
        reused,
        written,
        skipped,
    })
}

/// 单个条目的处理结果（用于并行构建后串行汇总）。
struct ProcessResult {
    /// 父条目摘要 + 成员摘要（父在前）。
    summaries: Vec<ItemSummary>,
    parsed: usize,
    reused: usize,
    written: usize,
    /// 解析失败而跳过的条目（`id：原因`）。
    skipped: Option<String>,
    /// 所属 crate 在 crate 列表里的下标。
    crate_index: usize,
}

impl ProcessResult {
    /// 空结果（产物缺失等静默跳过场景）。
    fn empty(crate_index: usize) -> Self {
        Self {
            summaries: Vec::new(),
            parsed: 0,
            reused: 0,
            written: 0,
            skipped: None,
            crate_index,
        }
    }
}

/// 处理单个条目：增量复用或解析、渲染、落盘，并产出摘要。
///
/// 该函数在并行迭代器内执行，因此只读共享外部状态、不做跨条目协调。
#[allow(clippy::too_many_arguments)]
fn process_entry(
    doc_root: &Path,
    out_root: &Path,
    entry: &DiscoveredItem,
    crate_index: usize,
    parse_opts: &ParseOptions,
    render_opts: &RenderOptions,
    opts: &BuildOptions,
    previous_by_id: &HashMap<&str, &ItemSummary>,
    previous_members: &HashMap<&str, Vec<&ItemSummary>>,
    rustdoc_version: &OnceLock<String>,
) -> Result<ProcessResult> {
    let html_path = doc_root.join(&entry.html_path);
    let mtime = path_mtime(&html_path);

    // 增量：父条目未变则整体复用（含其成员）。
    let unchanged = previous_by_id
        .get(entry.id.0.as_str())
        .filter(|summary| summary.src_mtime.is_some() && summary.src_mtime == mtime);
    if let Some(previous) = unchanged {
        let mut summaries = vec![(*previous).clone()];
        // 成员的 `path`（含 crate）正好等于父条目 id。
        if let Some(members) = previous_members.get(entry.id.0.as_str()) {
            summaries.extend(members.iter().map(|member| (*member).clone()));
        }
        let reused = summaries.len();
        return Ok(ProcessResult {
            summaries,
            parsed: 0,
            reused,
            written: 0,
            skipped: None,
            crate_index,
        });
    }

    // 产物缺失时跳过，不影响整体索引。
    let Ok(html) = fs::read_to_string(&html_path) else {
        return Ok(ProcessResult::empty(crate_index));
    };
    if rustdoc_version.get().is_none() {
        if let Some((version, _)) = parse::parse_rustdoc_meta(&html) {
            let _ = rustdoc_version.set(version);
        }
    }

    let item = match parse::parse_item_html(&html, &entry.html_path, parse_opts) {
        Ok(item) => item,
        Err(err) => {
            // 重定向页等异常产物：跳过该条目，不影响整体构建。
            return Ok(ProcessResult {
                skipped: Some(format!("{}：{err}", entry.id)),
                ..ProcessResult::empty(crate_index)
            });
        }
    };
    let parsed = 1;

    // 输出路径只推导一次，父条目与成员摘要共用。
    let item_path = item_output_path(out_root, &entry.html_path);
    let mut written = 0;
    if opts.write_markdown {
        let markdown = render_item(&item, render_opts);
        atomic_write_fast(&item_path, markdown.as_bytes())?;
        written += 1;
    }

    // 无真实文档时，rustdoc 生成的 `<meta name="description">` 是占位文本
    // （如 "API documentation for the Rust `X` struct ..."），不应作为摘要返回。
    let has_docs = item.docs_md.is_some();
    let one_line = if has_docs {
        // 摘要可能含 rustdoc 生成的 markdown 链接，统一重写为 `.md`。
        rewrite_links(
            &parse::parse_one_line(&html).unwrap_or_default(),
            LinkStyle::Relative,
        )
    } else {
        String::new()
    };

    let mut summaries = Vec::with_capacity(item.members.len() + 1);
    summaries.push(ItemSummary {
        id: item.id.clone(),
        kind: item.kind,
        name: item.name.clone(),
        path: item.path.clone(),
        one_line,
        signature: item.signature.clone(),
        has_docs,
        has_members: !item.members.is_empty(),
        file: rel_string(out_root, &item_path),
        html_path: rel_string(doc_root, &entry.html_path),
        parent_id: None,
        src_mtime: mtime,
    });

    for member in &item.members {
        let member_path = member_output_path(out_root, &entry.html_path, member.kind, &member.name);
        // 成员文件仅在「成员粒度」下写出；条目粒度时只内联在父文件里。
        if opts.write_markdown && opts.granularity == Granularity::Member {
            let markdown = render_member_item(member, render_opts);
            atomic_write_fast(&member_path, markdown.as_bytes())?;
            written += 1;
        }
        // 条目粒度下成员没有独立文件，`file` 指向父条目文件。
        let file = match opts.granularity {
            Granularity::Member => rel_string(out_root, &member_path),
            Granularity::Item => rel_string(out_root, &item_path),
        };
        summaries.push(ItemSummary {
            id: member.id.clone(),
            kind: member.kind,
            name: member.name.clone(),
            path: member.path.clone(),
            one_line: rewrite_links(&first_line(member.docs_md.as_deref()), LinkStyle::Relative),
            signature: member.signature.clone(),
            has_docs: member.docs_md.is_some(),
            has_members: false,
            file,
            html_path: rel_string(doc_root, &entry.html_path),
            parent_id: Some(item.id.0.clone()),
            src_mtime: mtime,
        });
    }

    Ok(ProcessResult {
        summaries,
        parsed,
        reused: 0,
        written,
        skipped: None,
        crate_index,
    })
}

/// 扫描 `doc_root` 构建索引（不写出任何文件）。
pub fn build_index(doc_root: &Path, out_root: &Path) -> Result<Index> {
    Ok(build(doc_root, out_root, &BuildOptions::default())?.index)
}

/// 把索引写入文件（原子写）。
pub fn write_index(index: &Index, path: &Path) -> Result<()> {
    // 紧凑 JSON：索引动辄数万条目，缩进空白会显著放大文件与解析成本。
    let json = serde_json::to_vec(index)?;
    atomic_write(path, &json)
}

/// 从文件读取索引。
pub fn load_index(path: &Path) -> Result<Index> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// 取相对 `out_root` 的路径字符串，统一用 `/` 分隔。
fn rel_string(out_root: &Path, path: &Path) -> String {
    path.strip_prefix(out_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 文档的首个非空行，作为成员的一行摘要。
fn first_line(docs: Option<&str>) -> String {
    docs.and_then(|docs| docs.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or_default()
        .to_string()
}

/// 当前 Unix 时间戳（秒）。
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
