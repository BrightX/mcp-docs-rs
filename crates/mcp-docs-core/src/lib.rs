//! mcp-docs 核心库：解析 rustdoc HTML、构建索引并检索。
//!
//! 本 crate 为纯同步库（不含 async），CLI 与 MCP server 均基于它构建。

mod discover;
mod error;
mod index;
mod link;
mod markdown;
mod model;
mod parse;
mod search;
mod sidebar;
mod store;

pub use discover::{discover_all, discover_crate, list_crates};
pub use error::{Error, Result};
pub use index::{build_index, load_index, write_index};
pub use link::{html_to_md_relpath, resolve_href, Resolved};
pub use markdown::{render_item, render_member_item, LinkStyle, RenderOptions};
pub use model::{
    CrateSummary, DiscoveredItem, DocItem, Index, ItemId, ItemKind, ItemSummary, Section,
    SourceRef, INDEX_SCHEMA_VERSION,
};
pub use parse::{
    parse_item_html, parse_one_line, parse_rustdoc_meta, path_to_identity, ParseOptions,
};
pub use search::{search, MatchMode, SearchHit, SearchQuery};
pub use sidebar::{parse_sidebar_file, parse_sidebar_str};
pub use store::{atomic_write, encode_fs_name, item_output_path, member_output_path};

/// 去掉 `window.XXX = ` 前缀与结尾 `;`，返回中间的 JSON 文本。
///
/// rustdoc 生成的 `crates.js` / `sidebar-items.js` 形如
/// `window.SIDEBAR_ITEMS = {...};`，去掉 JS 赋值外壳后即为合法 JSON。
pub(crate) fn strip_js_assignment<'a>(js: &'a str, var: &str) -> Option<&'a str> {
    let start = js.find(var)? + var.len();
    let rest = js[start..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let end = rest.find(';').unwrap_or(rest.len());
    Some(rest[..end].trim())
}
