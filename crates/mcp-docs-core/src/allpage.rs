//! 解析 rustdoc 的 `all.html`（crate 全量条目清单）。
//!
//! 它列出该 crate 的全部条目（含跨模块条目），用作发现阶段的兜底：
//! 当 `sidebar-items.js` 遗漏某条目（例如未列出的子模块）时，由此补全。

use std::path::{Path, PathBuf};

use scraper::{Html, Selector};

use crate::model::{DiscoveredItem, ItemId};
use crate::parse::path_to_identity;

/// 解析 `all.html`，返回其中的条目。
///
/// `crate_name` 为该页所属 crate（即 `all.html` 所在目录名）。
/// 无法识别或逃出 crate 的链接会被忽略。
pub fn parse_all_str(html: &str, crate_name: &str) -> Vec<DiscoveredItem> {
    let document = Html::parse_document(html);
    // 选择器是常量字符串，解析失败属编程错误。
    let selector = Selector::parse("ul.all-items a").expect("选择器常量应可解析");
    let mut out = Vec::new();

    for anchor in document.select(&selector) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(rel) = href_to_rel(href, crate_name) else {
            continue;
        };
        let Ok((kind, path, name)) = path_to_identity(&rel) else {
            continue;
        };
        // crate 根自身的 index.html 不作为一个条目。
        if path.is_empty() {
            continue;
        }
        // 与 `discover.rs` 保持一致：id 带条目类型标记。
        let id = format!("{}::{}.{}", path.join("::"), kind.kind_tag(), name);
        out.push(DiscoveredItem {
            id: ItemId(id),
            kind,
            name,
            path,
            html_path: rel,
        });
    }

    out
}

/// 把 `all.html` 里的 `href` 规整为相对 `doc_root` 的路径；不可用返回 `None`。
///
/// 只接受指向本 crate 内 `.html` 页面的链接：丢弃页内锚点、外链、绝对路径、
/// 逃出 crate 的 `..`；宏重定向页 `foo!.html` 规整为 `foo.html`。
fn href_to_rel(href: &str, crate_name: &str) -> Option<PathBuf> {
    let href = href.split('#').next().unwrap_or("");
    if href.is_empty() || href.starts_with('/') || href.starts_with("..") || href.contains("://") {
        return None;
    }
    let stem = href.strip_suffix(".html")?;
    let stem = stem.strip_suffix('!').unwrap_or(stem);
    if stem.is_empty() {
        return None;
    }
    Some(Path::new(crate_name).join(format!("{stem}.html")))
}
