//! 构建索引，并可选地渲染 markdown、落盘 index.json 与 meta.json。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::cache::{fingerprint_doc_root, path_mtime, write_meta, Meta};
use crate::discover;
use crate::error::Result;
use crate::markdown::{render_item, render_member_item, rewrite_links, LinkStyle, RenderOptions};
use crate::model::{CrateSummary, Granularity, Index, ItemSummary, INDEX_SCHEMA_VERSION};
use crate::parse::{self, ParseOptions};
use crate::store::{atomic_write, item_output_path, member_output_path};

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
    let mut rustdoc_version = previous
        .as_ref()
        .and_then(|index| index.rustdoc_version.clone());
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

    let mut crates = Vec::new();
    let mut items = Vec::new();
    let (mut parsed, mut reused, mut written) = (0usize, 0usize, 0usize);
    let mut skipped = Vec::new();

    for crate_name in discover::list_crates(doc_root)? {
        if let Some(filter) = &opts.crate_filter {
            if &crate_name != filter {
                continue;
            }
        }
        let mut item_count = 0;

        for entry in discover::discover_crate(doc_root, &crate_name)? {
            let html_path = doc_root.join(&entry.html_path);
            let mtime = path_mtime(&html_path);

            // 增量：父条目未变则整体复用（含其成员）。
            let unchanged = previous_by_id
                .get(entry.id.0.as_str())
                .filter(|summary| summary.src_mtime.is_some() && summary.src_mtime == mtime);
            if let Some(previous) = unchanged {
                items.push((*previous).clone());
                item_count += 1;
                reused += 1;

                // 成员的 `path`（含 crate）正好等于父条目 id。
                if let Some(members) = previous_members.get(entry.id.0.as_str()) {
                    for member in members {
                        items.push((*member).clone());
                        item_count += 1;
                        reused += 1;
                    }
                }
                continue;
            }

            // 产物缺失时跳过，不影响整体索引。
            let Ok(html) = fs::read_to_string(&html_path) else {
                continue;
            };
            if rustdoc_version.is_none() {
                rustdoc_version = parse::parse_rustdoc_meta(&html).map(|(version, _)| version);
            }

            let item = match parse::parse_item_html(&html, &entry.html_path, &parse_opts) {
                Ok(item) => item,
                Err(err) => {
                    // 重定向页等异常产物：跳过该条目，不影响整体构建。
                    skipped.push(format!("{}：{err}", entry.id));
                    continue;
                }
            };
            parsed += 1;

            if opts.write_markdown {
                let markdown = render_item(&item, &render_opts);
                let path = item_output_path(out_root, &entry.html_path);
                atomic_write(&path, markdown.as_bytes())?;
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

            items.push(ItemSummary {
                id: item.id.clone(),
                kind: item.kind,
                name: item.name.clone(),
                path: item.path.clone(),
                one_line,
                has_docs,
                has_members: !item.members.is_empty(),
                file: rel_string(out_root, &item_output_path(out_root, &entry.html_path)),
                html_path: rel_string(doc_root, &entry.html_path),
                parent_id: None,
                src_mtime: mtime,
            });
            item_count += 1;

            for member in &item.members {
                // 成员文件仅在「成员粒度」下写出；条目粒度时只内联在父文件里。
                if opts.write_markdown && opts.granularity == Granularity::Member {
                    let markdown = render_member_item(member, &render_opts);
                    let path =
                        member_output_path(out_root, &entry.html_path, member.kind, &member.name);
                    atomic_write(&path, markdown.as_bytes())?;
                    written += 1;
                }
                // 条目粒度下成员没有独立文件，`file` 指向父条目文件。
                let file = match opts.granularity {
                    Granularity::Member => rel_string(
                        out_root,
                        &member_output_path(out_root, &entry.html_path, member.kind, &member.name),
                    ),
                    Granularity::Item => {
                        rel_string(out_root, &item_output_path(out_root, &entry.html_path))
                    }
                };
                items.push(ItemSummary {
                    id: member.id.clone(),
                    kind: member.kind,
                    name: member.name.clone(),
                    path: member.path.clone(),
                    one_line: rewrite_links(
                        &first_line(member.docs_md.as_deref()),
                        LinkStyle::Relative,
                    ),
                    has_docs: member.docs_md.is_some(),
                    has_members: false,
                    file,
                    html_path: rel_string(doc_root, &entry.html_path),
                    parent_id: Some(item.id.0.clone()),
                    src_mtime: mtime,
                });
                item_count += 1;
            }
        }

        crates.push(CrateSummary {
            name: crate_name,
            version: None,
            item_count,
        });
    }

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

/// 扫描 `doc_root` 构建索引（不写出任何文件）。
pub fn build_index(doc_root: &Path, out_root: &Path) -> Result<Index> {
    Ok(build(doc_root, out_root, &BuildOptions::default())?.index)
}

/// 把索引写入文件（原子写）。
pub fn write_index(index: &Index, path: &Path) -> Result<()> {
    let json = serde_json::to_vec_pretty(index)?;
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
