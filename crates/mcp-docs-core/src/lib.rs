//! mcp-docs 核心库：解析 rustdoc HTML、构建索引并检索。
//!
//! 本 crate 为纯同步库（不含 async），CLI 与 MCP server 均基于它构建。

mod allpage;
mod cache;
mod discover;
mod error;
mod index;
mod link;
mod lookup;
mod markdown;
mod model;
mod nav;
mod parse;
mod search;
mod sidebar;
mod store;

pub use allpage::parse_all_str;
pub use cache::{
    fingerprint_doc_root, is_stale, path_mtime, read_meta, write_meta, DocCache, Fingerprint, Meta,
};
pub use discover::{discover_all, discover_crate, list_crates};
pub use error::{Error, Result};
pub use index::{build, build_index, load_index, write_index, BuildOptions, BuildReport};
pub use link::{html_to_md_relpath, resolve_href, Resolved};
pub use lookup::IdIndex;
pub use markdown::{
    extract_code_blocks, render_item, render_member_item, rewrite_links, LinkStyle, RenderOptions,
};
pub use model::{
    CrateSummary, DiscoveredItem, DocItem, Granularity, Index, ItemId, ItemKind, ItemSummary,
    Section, SourceRef, INDEX_SCHEMA_VERSION,
};
pub use nav::{
    module_tree, parse_trait_impls, related_items, trait_impl_rel_path, ModuleNode, RelatedItems,
    TraitImpl,
};
pub use parse::{
    extract_source_lines, parse_item_html, parse_one_line, parse_rustdoc_meta, path_to_identity,
    ParseOptions,
};
pub use search::{search, search_page, MatchMode, SearchHit, SearchOutcome, SearchQuery};
pub use sidebar::{parse_sidebar_file, parse_sidebar_str};
pub use store::{
    atomic_write, atomic_write_fast, encode_fs_name, item_output_path, member_output_path,
};

/// FNV-1a 32 位哈希，用于超长文件名截断与内容指纹。
pub(crate) fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

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
