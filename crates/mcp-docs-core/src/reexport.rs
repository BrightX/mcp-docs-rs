//! 解析 crate 首页的 `#reexports` 区块（crate 内部重导出别名）。
//!
//! rustdoc 对**本 crate 内**的重导出（`pub use deep::RealStruct as Renamed;`）
//! 不生成独立页面，只在 crate 首页列出一个 `Re-exports` 区块；这类别名因此
//! 无法由 `sidebar-items.js` / `all.html` / 目录扫描发现（见 `docs/lessons.md`）。
//! 本模块把它们补成别名条目：`id` 用别名，`html_path` 指向目标的真实页面，
//! 从而让 `search_items` 能命中别名、`get_item` 能返回目标文档。
//!
//! **跨 crate 重导出**（`pub use anyhow::Error;`）rustdoc 会生成本地页面，
//! 已由常规发现路径覆盖，且不进入 `#reexports`，故此处天然不涉及。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use scraper::{Html, Selector};

use crate::model::{DiscoveredItem, ItemId, ItemKind};

/// 每个选择器字面量各自一份缓存（`OnceLock` 与字面量一一对应），避免复用同一缓存。
macro_rules! selector {
    ($css:expr $(,)?) => {{
        static CACHE: OnceLock<Selector> = OnceLock::new();
        CACHE.get_or_init(|| Selector::parse($css).expect("内置选择器应始终合法"))
    }};
}

/// 解析 crate 首页 HTML 的 `#reexports` 区块，返回别名条目。
///
/// `crate_name` 为该页所属 crate（即 `index.html` 所在目录名）。
/// 无法解析或目标不在本 crate 的别名会被忽略。
pub fn parse_reexports_str(html: &str, crate_name: &str) -> Vec<DiscoveredItem> {
    let document = Html::parse_document(html);
    let mut out = Vec::new();

    for dt in document.select(selector!("dl.reexports dt[id^=\"reexport.\"]")) {
        let Some(alias) = dt
            .value()
            .attr("id")
            .and_then(|id| id.strip_prefix("reexport."))
        else {
            continue;
        };
        // 目标页链接与标题都在 `dt` 内的 `<a>` 上；取第一个即可。
        let Some(anchor) = dt.select(selector!("a")).next() else {
            continue;
        };
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(title) = anchor.value().attr("title") else {
            continue;
        };
        let Some(rel) = href_to_rel(href, crate_name) else {
            continue;
        };
        let Some(kind) = target_kind(title) else {
            continue;
        };
        // 只收本 crate 的目标：跨 crate 的目标不进入 `#reexports`，若出现则跳过。
        if rel.components().next().map(|c| c.as_os_str()) != Some(crate_name.as_ref()) {
            continue;
        }

        out.push(DiscoveredItem {
            id: ItemId(format!("{crate_name}::{}.{alias}", kind.kind_tag())),
            kind,
            name: alias.to_string(),
            path: vec![crate_name.to_string()],
            html_path: rel,
        });
    }

    // 同一别名极少重复，但保守去重（按 id）。
    let mut seen = HashSet::new();
    out.retain(|item| seen.insert(item.id.0.clone()));
    out
}

/// 从链接 `title` 推断目标条目类型。
///
/// rustdoc 的 `title` 形如 `struct rp::deep::RealStruct`：首词是文件前缀
/// （`struct` / `trait` / `fn` / `mod` / `type` …），其后是目标的完整路径。
pub(crate) fn target_kind(title: &str) -> Option<ItemKind> {
    let prefix = title.split_whitespace().next()?;
    ItemKind::from_file_prefix(prefix)
}

/// 把 `#reexports` 里的 `href` 规整为相对 `doc_root` 的路径；不可用返回 `None`。
///
/// 只接受指向本 crate 内 `.html` 页面的相对链接：丢弃页内锚点、外链、绝对路径、
/// 逃出 crate 的 `..`。
fn href_to_rel(href: &str, crate_name: &str) -> Option<PathBuf> {
    let href = href.split('#').next().unwrap_or("");
    if href.is_empty() || href.starts_with('/') || href.starts_with("..") || href.contains("://") {
        return None;
    }
    let stem = href.strip_suffix(".html")?;
    // 模块目录链接 `deep/index.html` 指向模块页；`strip_suffix` 后为 `deep/index`，
    // 这里统一规整为 `deep/index.html`（保留 `index`，与 discover 的模块路径一致）。
    let rel = Path::new(crate_name).join(format!("{stem}.html"));
    Some(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// crate 首页的 `#reexports` 片段能解析出别名条目，且指向目标的真实页面。
    #[test]
    fn parse_reexports_extracts_aliases() {
        let html = r##"<html><body><section id="main-content">
<h2 id="reexports" class="section-header">Re-exports</h2>
<dl class="item-table reexports">
<dt id="reexport.RealStruct"><code>pub use deep::<a class="struct" href="deep/struct.RealStruct.html" title="struct rp::deep::RealStruct">RealStruct</a>;</code></dt>
<dt id="reexport.RenamedStruct"><code>pub use deep::<a class="struct" href="deep/struct.RealStruct.html" title="struct rp::deep::RealStruct">RealStruct</a> as RenamedStruct;</code></dt>
<dt id="reexport.renamed_fn"><code>pub use deep::<a class="fn" href="deep/fn.real_fn.html" title="fn rp::deep::real_fn">real_fn</a> as renamed_fn;</code></dt>
<dt id="reexport.renamed_mod"><code>pub use <a class="mod" href="deep/index.html" title="mod rp::deep">deep</a> as renamed_mod;</code></dt>
</dl>
<h2 id="modules" class="section-header">Modules</h2>
<dl class="item-table"><dt><a class="mod" href="deep/index.html" title="mod rp::deep">deep</a></dt></dl>
</section></body></html>"##;
        let items = parse_reexports_str(html, "rp");
        let ids: Vec<&str> = items.iter().map(|item| item.id.0.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "rp::struct.RealStruct",
                "rp::struct.RenamedStruct",
                "rp::fn.renamed_fn",
                "rp::mod.renamed_mod",
            ]
        );
        // 别名指向目标的真实页面（别名条目与目标共享 html_path）。
        assert_eq!(
            items[1].html_path,
            Path::new("rp").join("deep/struct.RealStruct.html")
        );
        assert_eq!(items[0].name, "RealStruct");
        assert_eq!(items[0].kind, ItemKind::Struct);
        assert_eq!(items[2].kind, ItemKind::Function);
        assert_eq!(items[3].kind, ItemKind::Module);
    }

    /// `title` 首词即目标类型前缀。
    #[test]
    fn target_kind_from_title() {
        assert_eq!(
            target_kind("struct rp::deep::RealStruct"),
            Some(ItemKind::Struct)
        );
        assert_eq!(target_kind("trait rp::T"), Some(ItemKind::Trait));
        assert_eq!(target_kind("type rp::A"), Some(ItemKind::TypeAlias));
        assert_eq!(target_kind("mod rp::m"), Some(ItemKind::Module));
        assert_eq!(target_kind(""), None);
    }
}
