//! 从 rustdoc 产物中发现条目。
//!
//! 以 `crates.js` 定位 crate，再递归各层 `sidebar-items.js` 构建条目清单。

use std::fs;
use std::path::Path;

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

/// 递归遍历一个模块目录：读 `sidebar-items.js` 产出当前层条目，再下钻子模块。
///
/// `path` 为当前模块的完整路径（含 crate）。
fn walk_module(
    dir: &Path,
    doc_root: &Path,
    path: &[String],
    out: &mut Vec<DiscoveredItem>,
) -> Result<()> {
    let Some(groups) = sidebar::parse_sidebar_file(&dir.join("sidebar-items.js"))? else {
        // 叶模块没有 sidebar-items.js，属正常情况。
        return Ok(());
    };

    let mut submodules = Vec::new();
    for (kind, names) in groups {
        for name in names {
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
            id_parts.push(name.clone());

            out.push(DiscoveredItem {
                id: ItemId(id_parts.join("::")),
                kind,
                name,
                path: path.to_vec(),
                html_path,
            });
        }
    }

    // 先产出当前层全部条目，再下钻子模块，使输出顺序更自然。
    for name in submodules {
        let mut child_path = path.to_vec();
        child_path.push(name.clone());
        walk_module(&dir.join(&name), doc_root, &child_path, out)?;
    }
    Ok(())
}
