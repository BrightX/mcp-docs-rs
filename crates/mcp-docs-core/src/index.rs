//! 构建与读写全量索引。

use std::fs;
use std::path::Path;

use crate::discover;
use crate::error::Result;
use crate::model::{CrateSummary, Index, ItemSummary, INDEX_SCHEMA_VERSION};
use crate::parse::{self, ParseOptions};
use crate::store::{atomic_write, item_output_path, member_output_path};

/// 扫描 `doc_root` 构建索引。
///
/// `out_root` 仅用于推导条目的 markdown 路径，不要求目录已存在。
pub fn build_index(doc_root: &Path, out_root: &Path) -> Result<Index> {
    let parse_opts = ParseOptions::default();
    let mut crates = Vec::new();
    let mut items = Vec::new();
    let mut rustdoc_version = None;

    for crate_name in discover::list_crates(doc_root)? {
        let mut item_count = 0;

        for entry in discover::discover_crate(doc_root, &crate_name)? {
            let html_path = doc_root.join(&entry.html_path);
            // 产物缺失时跳过，不影响整体索引。
            let Ok(html) = fs::read_to_string(&html_path) else {
                continue;
            };

            if rustdoc_version.is_none() {
                rustdoc_version = parse::parse_rustdoc_meta(&html).map(|(version, _)| version);
            }

            let item = parse::parse_item_html(&html, &entry.html_path, &parse_opts)?;

            items.push(ItemSummary {
                id: item.id.clone(),
                kind: item.kind,
                name: item.name.clone(),
                path: item.path.clone(),
                one_line: parse::parse_one_line(&html).unwrap_or_default(),
                has_docs: item.docs_md.is_some(),
                has_members: !item.members.is_empty(),
                file: rel_string(out_root, &item_output_path(out_root, &entry.html_path)),
            });
            item_count += 1;

            for member in &item.members {
                items.push(ItemSummary {
                    id: member.id.clone(),
                    kind: member.kind,
                    name: member.name.clone(),
                    path: member.path.clone(),
                    one_line: first_line(member.docs_md.as_deref()),
                    has_docs: member.docs_md.is_some(),
                    has_members: false,
                    file: rel_string(
                        out_root,
                        &member_output_path(out_root, &entry.html_path, &member.name),
                    ),
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

    Ok(Index {
        schema_version: INDEX_SCHEMA_VERSION,
        rustdoc_version,
        generated_at: unix_now(),
        target_doc: doc_root.to_string_lossy().into_owned(),
        crates,
        items,
    })
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
