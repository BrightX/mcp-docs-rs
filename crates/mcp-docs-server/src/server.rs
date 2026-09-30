//! MCP 服务：把索引、检索与 markdown 渲染暴露为工具与资源。
//!
//! 设计原则是「先搜后读」：检索类工具只返回轻量摘要，正文由 `get_item`
//! 按需读取（内部走 `DocCache`，见 `design.md` §7、§8）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, ServerCapabilities, ServerConfig,
};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler};

use mcp_docs_core::MatchMode;
use mcp_docs_core::{
    build, is_stale, load_index, render_item, render_member_item, search, BuildOptions, DocCache,
    DocItem, Index, ItemKind, ItemSummary, ParseOptions, RenderOptions, SearchQuery,
};

/// `list_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ListItemsParams {
    /// 限定 crate 名。
    #[serde(rename = "crate")]
    crate_name: Option<String>,
    /// 限定模块路径（如 `inner`）。
    module: Option<String>,
    /// 限定条目类型（如 `struct` / `fn` / `method`）。
    kind: Option<String>,
    /// 返回上限，默认 100。
    limit: Option<usize>,
}

/// `search_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct SearchItemsParams {
    /// 查询词。
    query: String,
    /// 限定 crate 名。
    #[serde(rename = "crate")]
    crate_name: Option<String>,
    /// 限定条目类型。
    kind: Option<String>,
    /// 匹配模式：`substring`（默认）/ `prefix` / `fuzzy`。
    mode: Option<String>,
    /// 返回上限，默认 20。
    limit: Option<usize>,
}

/// `get_item` / `get_item_source` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ItemParams {
    /// 条目 id，如 `doc_probe::Demo::new`。
    id: String,
    /// 返回 markdown 的字节上限（超出时按字符边界截断）。
    max_bytes: Option<usize>,
}

/// `rebuild_index` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct RebuildParams {
    /// 强制全量重建（默认增量）。
    force: Option<bool>,
}

/// mcp-docs 的 MCP 服务。
#[derive(Clone)]
pub struct DocsServer {
    doc_dir: Arc<PathBuf>,
    out_dir: Arc<PathBuf>,
    index: Arc<RwLock<Index>>,
    cache: Arc<DocCache>,
}

impl DocsServer {
    /// 构建服务：加载已有索引，缺失或过期时重建。
    pub fn new(doc_dir: PathBuf, out_dir: PathBuf) -> anyhow::Result<Self> {
        let index = load_or_build(&doc_dir, &out_dir)?;
        Ok(Self {
            doc_dir: Arc::new(doc_dir),
            out_dir: Arc::new(out_dir),
            index: Arc::new(RwLock::new(index)),
            cache: Arc::new(DocCache::new()),
        })
    }

    /// 只读访问索引。
    fn index(&self) -> RwLockReadGuard<'_, Index> {
        self.index
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 解析条目（命中内存缓存时直接复用）。
    fn load_item(&self, summary: &ItemSummary) -> Result<Arc<DocItem>, String> {
        let rel = PathBuf::from(&summary.html_path);
        self.cache
            .get_or_parse(&self.doc_dir, &rel, &ParseOptions::default())
            .map_err(|err| format!("解析条目 `{}` 失败：{err}", summary.id))
    }
}

#[tool_router]
impl DocsServer {
    /// 列出已索引的 crate。
    #[tool(description = "列出已索引的 crate 及其条目数")]
    fn list_crates(&self) -> String {
        let index = self.index();
        let crates: Vec<serde_json::Value> = index
            .crates
            .iter()
            .map(|krate| {
                serde_json::json!({
                    "name": krate.name,
                    "version": krate.version,
                    "item_count": krate.item_count,
                })
            })
            .collect();
        to_json(&serde_json::json!({ "crates": crates }))
    }

    /// 列出条目摘要（不含正文）。
    #[tool(description = "列出条目摘要（可按 crate / 模块 / 类型过滤），不含正文")]
    fn list_items(&self, Parameters(params): Parameters<ListItemsParams>) -> String {
        let index = self.index();
        let kind = params.kind.as_deref().and_then(ItemKind::from_file_prefix);

        let filtered: Vec<&ItemSummary> = index
            .items
            .iter()
            .filter(|item| matches_crate(item, params.crate_name.as_deref()))
            .filter(|item| matches_module(item, params.module.as_deref()))
            .filter(|item| kind.is_none_or(|wanted| item.kind == wanted))
            .collect();

        let limit = params.limit.unwrap_or(100);
        let items: Vec<serde_json::Value> = filtered
            .iter()
            .take(limit)
            .map(|item| summary_json(item))
            .collect();

        to_json(&serde_json::json!({
            "total": filtered.len(),
            "returned": items.len(),
            "items": items,
        }))
    }

    /// 检索条目。
    #[tool(description = "按名字 / 路径 / 摘要检索条目，返回轻量摘要与得分")]
    fn search_items(&self, Parameters(params): Parameters<SearchItemsParams>) -> String {
        let index = self.index();
        let mut query = SearchQuery::new(params.query);
        query.crate_name = params.crate_name;
        query.limit = params.limit.unwrap_or(20);
        query.mode = match params.mode.as_deref() {
            Some("prefix") => MatchMode::Prefix,
            Some("fuzzy") => MatchMode::Fuzzy,
            _ => MatchMode::Substring,
        };
        if let Some(kind) = params.kind.as_deref().and_then(ItemKind::from_file_prefix) {
            query.kinds.push(kind);
        }

        let hits = search(&index, &query);
        let items: Vec<serde_json::Value> = hits
            .iter()
            .map(|hit| {
                let mut value = summary_json(&hit.item);
                value["score"] = serde_json::json!(hit.score);
                value
            })
            .collect();

        to_json(&serde_json::json!({ "returned": items.len(), "hits": items }))
    }

    /// 读取条目的完整 markdown。
    #[tool(description = "读取某个条目的完整 markdown 文档（签名 + 文档 + 成员）")]
    fn get_item(&self, Parameters(params): Parameters<ItemParams>) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .items
            .iter()
            .find(|item| item.id.0 == params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;
        let options = RenderOptions::default();

        // 成员只返回自身小节，其余返回整篇。
        let markdown = if summary.kind.is_member() {
            match item.members.iter().find(|member| member.id == summary.id) {
                Some(member) => render_member_item(member, &options),
                None => render_item(&item, &options),
            }
        } else {
            render_item(&item, &options)
        };
        Ok(truncate(markdown, params.max_bytes))
    }

    /// 查询条目的源码位置。
    #[tool(description = "查询条目对应的源码文件与行号")]
    fn get_item_source(
        &self,
        Parameters(params): Parameters<ItemParams>,
    ) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .items
            .iter()
            .find(|item| item.id.0 == params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;

        // 成员自身的源码位置未单独采集，这里返回其所属条目的位置。
        let source = item.source.as_ref().map(|source| {
            serde_json::json!({
                "file": source.file,
                "line_start": source.line_start,
                "line_end": source.line_end,
            })
        });
        Ok(to_json(&serde_json::json!({
            "id": params.id,
            "source": source,
        })))
    }

    /// 重建索引。
    #[tool(description = "重新扫描产物目录并重建索引（默认增量）")]
    fn rebuild_index(
        &self,
        Parameters(params): Parameters<RebuildParams>,
    ) -> Result<String, String> {
        let options = BuildOptions {
            persist: true,
            incremental: !params.force.unwrap_or(false),
            ..Default::default()
        };
        let report = build(&self.doc_dir, &self.out_dir, &options)
            .map_err(|err| format!("重建索引失败：{err}"))?;

        self.cache.invalidate();
        let item_count = report.index.items.len();
        *self
            .index
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = report.index;

        Ok(to_json(&serde_json::json!({
            "rebuilt": true,
            "reused": report.reused,
            "item_count": item_count,
        })))
    }
}

#[tool_handler]
impl ServerHandler for DocsServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(
            "检索本地 rustdoc 文档。先用 search_items / list_items 找到条目，再用 get_item 读取 markdown。",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let index = self.index();
        let mut resources = vec![
            Resource::new("rustdoc://crates", "crates").with_description("所有已索引的 crate")
        ];
        for krate in &index.crates {
            resources.push(
                Resource::new(format!("rustdoc://{}", krate.name), krate.name.clone())
                    .with_description(format!("crate {} 的条目列表", krate.name)),
            );
        }
        Ok(ListResourcesResult {
            resources,
            ..Default::default()
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![ResourceTemplate::new("rustdoc://{crate}/{+item}", "item")
                .with_description("某个条目的 markdown 文档")],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let text = self.read_uri(&request.uri)?;
        let contents =
            ResourceContents::text(text, request.uri.clone()).with_mime_type("text/markdown");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

impl DocsServer {
    /// 读取一个 `rustdoc://` URI 对应的文本内容。
    fn read_uri(&self, uri: &str) -> Result<String, McpError> {
        let path = uri
            .strip_prefix("rustdoc://")
            .ok_or_else(|| McpError::invalid_params(format!("非 rustdoc URI：{uri}"), None))?;

        if path == "crates" {
            return Ok(self.list_crates());
        }

        let (crate_name, item) = match path.split_once('/') {
            Some((crate_name, item)) => (crate_name, Some(item)),
            None => (path, None),
        };

        match item {
            // rustdoc://{crate}/{item}：item 的 `/` 对应 `::`。
            Some(item) => {
                let id = format!("{crate_name}::{}", item.replace('/', "::"));
                self.get_item(Parameters(ItemParams {
                    id,
                    max_bytes: None,
                }))
                .map_err(|message| McpError::invalid_params(message, None))
            }
            // rustdoc://{crate}：该 crate 的条目清单。
            None => self.crate_listing(crate_name),
        }
    }

    /// 列出某个 crate 的全部条目摘要。
    fn crate_listing(&self, crate_name: &str) -> Result<String, McpError> {
        let index = self.index();
        let items: Vec<serde_json::Value> = index
            .items
            .iter()
            .filter(|item| matches_crate(item, Some(crate_name)))
            .map(summary_json)
            .collect();

        if items.is_empty() {
            return Err(McpError::invalid_params(
                format!("未找到 crate `{crate_name}`"),
                None,
            ));
        }
        Ok(to_json(&serde_json::json!({
            "crate": crate_name,
            "item_count": items.len(),
            "items": items,
        })))
    }
}

/// 加载已有索引；缺失或过期时重建。
fn load_or_build(doc_dir: &Path, out_dir: &Path) -> anyhow::Result<Index> {
    let index_path = out_dir.join("index.json");
    let meta_path = out_dir.join("meta.json");

    if !is_stale(doc_dir, &meta_path)? {
        if let Ok(index) = load_index(&index_path) {
            return Ok(index);
        }
    }

    let report = build(
        doc_dir,
        out_dir,
        &BuildOptions {
            persist: true,
            ..Default::default()
        },
    )?;
    Ok(report.index)
}

/// 条目摘要的 JSON 表示。
fn summary_json(item: &ItemSummary) -> serde_json::Value {
    serde_json::json!({
        "id": item.id.0,
        "kind": item.kind,
        "name": item.name,
        "path": item.path,
        "one_line": item.one_line,
        "has_docs": item.has_docs,
        "has_members": item.has_members,
        "file": item.file,
    })
}

/// 是否属于指定 crate（`None` 表示不限）。
fn matches_crate(item: &ItemSummary, crate_name: Option<&str>) -> bool {
    match crate_name {
        Some(name) => item.id.0.split("::").next() == Some(name),
        None => true,
    }
}

/// 是否位于指定模块下（`None` 表示不限）。
fn matches_module(item: &ItemSummary, module: Option<&str>) -> bool {
    match module {
        Some(module) => item.path.len() > 1 && item.path[1..].join("::").starts_with(module),
        None => true,
    }
}

/// 序列化为带缩进的 JSON 文本。
fn to_json(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|err| format!("序列化失败：{err}"))
}

/// 按字节上限截断文本（保证 UTF-8 字符边界），并附带提示。
fn truncate(text: String, max_bytes: Option<usize>) -> String {
    let Some(limit) = max_bytes else {
        return text;
    };
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n…（内容已按 max_bytes={limit} 截断）", &text[..end])
}
