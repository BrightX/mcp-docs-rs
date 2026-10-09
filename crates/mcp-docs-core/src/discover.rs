//! 从 rustdoc 产物中发现条目。
//!
//! 以 `crates.js` 定位 crate，再递归各层 `sidebar-items.js` 构建条目清单。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::{DiscoveredItem, ItemId, ItemKind};
use crate::sidebar;

/// 读取 `crates.js`，返回 `doc_root` 下**实际存在文档**的 crate 列表。
pub fn list_crates(doc_root: &Path) -> Result<Vec<String>> {
    let path = doc_root.join("crates.js");
    let js = fs::read_to_string(&path).map_err(Error::Io)?;
    let json =
        crate::strip_js_assignment(&js, "window.ALL_CRATES").ok_or_else(|| Error::Parse {
            path: path.clone(),
            reason: "未找到 window.ALL_CRATES 赋值".to_string(),
        })?;

    let mut crates: Vec<String> = serde_json::from_str(json)?;
    // crates.js 可能包含未生成文档的 crate，用目录存在性过滤。
    crates.retain(|name| doc_root.join(name).is_dir());
    crates.sort();
    crates.dedup();
    Ok(crates)
}

/// 发现单个 crate 下的全部条目（含子模块条目，不含成员）。
pub fn discover_crate(doc_root: &Path, crate_name: &str) -> Result<Vec<DiscoveredItem>> {
    let mut out = Vec::new();
    walk_module(
        &doc_root.join(crate_name),
        doc_root,
        &[crate_name.to_string()],
        &mut out,
    )?;

    // 兜底：all.html 交叉校验，补全 sidebar 与目录扫描都遗漏的条目
    // （例如 sidebar 未列出的子模块，扫描不会下钻进去）。
    if let Ok(html) = fs::read_to_string(doc_root.join(crate_name).join("all.html")) {
        // 按 HTML 路径去重（re-export 别名下比按 id 去重更稳），只追加缺失项。
        let mut seen: HashSet<PathBuf> = out.iter().map(|item| item.html_path.clone()).collect();
        for item in crate::allpage::parse_all_str(&html, crate_name) {
            if doc_root.join(&item.html_path).is_file() && seen.insert(item.html_path.clone()) {
                out.push(item);
            }
        }
    }

    // crate 内部重导出别名（`pub use deep::RealStruct as Renamed;`）不生成独立页面，
    // 只在 crate 首页的 `#reexports` 区块列出，sidebar / all.html / 目录扫描都覆盖不到；
    // 这里补成别名条目（id 用别名，html_path 指向目标真实页）。
    if let Ok(html) = fs::read_to_string(doc_root.join(crate_name).join("index.html")) {
        // 按 id 去重：别名 id 唯一，且可能与本 crate 已有的真实条目同名（同名重导出）。
        let mut seen: HashSet<String> = out.iter().map(|item| item.id.0.clone()).collect();
        for item in crate::reexport::parse_reexports_str(&html, crate_name) {
            if doc_root.join(&item.html_path).is_file() && seen.insert(item.id.0.clone()) {
                out.push(item);
            }
        }
    }

    Ok(out)
}

/// 发现 `doc_root` 下所有 crate 的条目。
pub fn discover_all(doc_root: &Path) -> Result<Vec<DiscoveredItem>> {
    let mut out = Vec::new();
    for crate_name in list_crates(doc_root)? {
        out.extend(discover_crate(doc_root, &crate_name)?);
    }
    Ok(out)
}

/// 递归遍历一个模块目录：产出当前层条目，再下钻子模块。
///
/// `sidebar-items.js` 并不覆盖全部条目（例如属性宏 `attr.*.html` 就不在其中），
/// 因此还会扫描目录补全。
///
/// `path` 为当前模块的完整路径（含 crate）。
fn walk_module(
    dir: &Path,
    doc_root: &Path,
    path: &[String],
    out: &mut Vec<DiscoveredItem>,
) -> Result<()> {
    let mut entries: Vec<(ItemKind, String)> = Vec::new();

    // 主来源：sidebar-items.js。
    if let Some(groups) = sidebar::parse_sidebar_file(&dir.join("sidebar-items.js"))? {
        for (kind, names) in groups {
            for name in names {
                entries.push((kind, name));
            }
        }
    }

    // 补全：扫描目录，收入 sidebar 未列出的条目。
    let known: HashSet<(ItemKind, String)> = entries.iter().cloned().collect();
    for entry in scan_directory(dir) {
        if !known.contains(&entry) {
            entries.push(entry);
        }
    }

    let mut submodules = Vec::new();
    for (kind, name) in entries {
        // 模块的页面是子目录下的 index.html，其余条目是 `{前缀}.{名字}.html`。
        let html_path = if kind == ItemKind::Module {
            submodules.push(name.clone());
            dir.join(&name).join("index.html")
        } else {
            dir.join(format!("{}.{}.html", kind.file_prefix(), name))
        };
        let html_path = html_path
            .strip_prefix(doc_root)
            .unwrap_or(&html_path)
            .to_path_buf();

        let mut id_parts = path.to_vec();
        // 与 `DocItem` 保持一致：id 带条目类型标记，保证唯一。
        id_parts.push(format!("{}.{}", kind.kind_tag(), name));

        out.push(DiscoveredItem {
            id: ItemId(id_parts.join("::")),
            kind,
            name,
            path: path.to_vec(),
            html_path,
        });
    }

    // 先产出当前层全部条目，再下钻子模块，使输出顺序更自然。
    for name in submodules {
        let mut child_path = path.to_vec();
        child_path.push(name.clone());
        walk_module(&dir.join(&name), doc_root, &child_path, out)?;
    }
    Ok(())
}

/// 扫描目录下的条目 HTML，返回 `(类型, 名字)`。
///
/// 文件名形如 `{前缀}.{名字}.html`；无法识别的文件名（如 `index.html`、
/// `all.html`）会被忽略。
fn scan_directory(dir: &Path) -> Vec<(ItemKind, String)> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = file_name.strip_suffix(".html") else {
            continue;
        };
        let Some((prefix, name)) = stem.split_once('.') else {
            continue;
        };
        let Some(kind) = ItemKind::from_file_prefix(prefix) else {
            continue;
        };
        found.push((kind, name.to_string()));
    }
    found
}
