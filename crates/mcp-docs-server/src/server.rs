//! MCP 服务：把索引、检索与 markdown 渲染暴露为工具与资源。
//!
//! 设计原则是「先搜后读」：检索类工具只返回轻量摘要，正文由 `get_item`
//! 按需读取（内部走 `DocCache`，见 `design.md` §7、§8）。

use std::ops::Deref;
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
use tokio::sync::watch;

use mcp_docs_core::MatchMode;
use mcp_docs_core::{
    build, is_stale, load_index, render_item, render_member_item, search_page, BuildOptions,
    DocCache, DocItem, Granularity, IdIndex, Index, ItemKind, ItemSummary, ParseOptions,
    RenderOptions, SearchQuery, INDEX_SCHEMA_VERSION,
};

/// `list_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ListItemsParams {
    /// 限定 crate 名。
    #[serde(rename = "crate")]
    crate_name: Option<String>,
    /// 限定模块路径（如 `inner`）。
    module: Option<String>,
    /// 限定条目类型，接受 `fn`（前缀）或 `function`（自然名）。
    kind: Option<String>,
    /// 返回上限，默认 100。
    limit: Option<usize>,
    /// 跳过的条目数，默认 0。
    offset: Option<usize>,
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
    /// 跳过的命中数，默认 0。
    offset: Option<usize>,
}

/// `get_item` / `get_item_source` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ItemParams {
    /// 条目 id，取自 `search_items` / `list_items` 的返回值（如 `tokio::task::fn.spawn`）；
    /// 也可省略类型标记（如 `tokio::task::spawn`）。
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

/// 已加载的索引，附带预建的 id 查找表。
///
/// 每个索引只构建一次查找表，避免每次按 id 查找都线性扫描全部条目。
struct Loaded {
    index: Index,
    ids: IdIndex,
}

impl Loaded {
    /// 用索引构建查找表。
    fn new(index: Index) -> Self {
        let ids = IdIndex::build(&index.items);
        Self { index, ids }
    }

    /// 按 id 查找条目摘要（精确优先，退化到省略类型标记的写法）。
    fn find(&self, id: &str) -> Option<&ItemSummary> {
        self.ids
            .find(id)
            .map(|index_of| &self.index.items[index_of])
    }
}

impl Deref for Loaded {
    type Target = Index;

    fn deref(&self) -> &Index {
        &self.index
    }
}

/// mcp-docs 的 MCP 服务。
#[derive(Clone)]
pub struct DocsServer {
    doc_dir: Arc<PathBuf>,
    out_dir: Arc<PathBuf>,
    index: Arc<RwLock<Loaded>>,
    cache: Arc<DocCache>,
    /// 索引就绪信号：后台加载/重建完成时置 `true`。
    ready: watch::Receiver<bool>,
}

impl DocsServer {
    /// 构建服务：**不在启动阶段阻塞**。
    ///
    /// 同步加载已有索引（毫秒~亚秒级）即可立即服务；索引的过期判定与重建
    /// 放到后台任务，避免冷启动（首次全量构建可能需 1~2 分钟）拖住
    /// `initialize`。没有可用索引时用空占位，工具/资源会等待构建完成
    /// （见 `ensure_ready`）。
    pub fn new(doc_dir: PathBuf, out_dir: PathBuf) -> anyhow::Result<Self> {
        let doc_dir = Arc::new(doc_dir);
        let out_dir = Arc::new(out_dir);

        let loaded = load_index(&out_dir.join("index.json")).ok();
        let has_index = loaded.is_some();
        let initial = loaded.unwrap_or_else(|| empty_index(&doc_dir));
        let (ready_tx, ready) = watch::channel(has_index);

        let server = Self {
            doc_dir,
            out_dir,
            index: Arc::new(RwLock::new(Loaded::new(initial))),
            cache: Arc::new(DocCache::new()),
            ready,
        };

        // 后台刷新索引；无论成功与否都要置位就绪，避免调用方永久等待。
        let worker = server.clone();
        tokio::spawn(async move {
            if let Err(err) = worker.refresh_index(!has_index).await {
                eprintln!("后台构建索引失败：{err:#}");
            }
            let _ = ready_tx.send(true);
        });

        Ok(server)
    }

    /// 等待索引就绪；已就绪时立即返回。
    async fn ensure_ready(&self) {
        let mut ready = self.ready.clone();
        loop {
            let is_ready = *ready.borrow();
            if is_ready {
                return;
            }
            if ready.changed().await.is_err() {
                return;
            }
        }
    }

    /// 后台刷新索引：缺失（`force`）或过期时全量重建，随后替换内存索引。
    async fn refresh_index(&self, force: bool) -> anyhow::Result<()> {
        let meta_path = self.out_dir.join("meta.json");
        if !force && !is_stale(&self.doc_dir, &meta_path)? {
            return Ok(());
        }

        let doc_dir = self.doc_dir.clone();
        let out_dir = self.out_dir.clone();
        // 解析/渲染是 CPU 密集的，放到阻塞线程池，别占着 async 执行器。
        let index = tokio::task::spawn_blocking(move || {
            build(
                &doc_dir,
                &out_dir,
                &BuildOptions {
                    persist: true,
                    ..Default::default()
                },
            )
            .map(|report| report.index)
        })
        .await??;

        self.cache.invalidate();
        *self
            .index
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Loaded::new(index);
        Ok(())
    }

    /// 只读访问索引。
    fn index(&self) -> RwLockReadGuard<'_, Loaded> {
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
    async fn list_crates(&self) -> String {
        self.ensure_ready().await;
        self.list_crates_text()
    }

    /// 列出条目摘要（不含正文）。
    #[tool(description = "列出条目摘要（可按 crate / 模块 / 类型过滤），不含正文")]
    async fn list_items(
        &self,
        Parameters(params): Parameters<ListItemsParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.list_items_blocking(params))
            .await
            .map_err(|err| format!("列条目任务失败：{err}"))?
    }

    /// 检索条目。
    #[tool(description = "按名字 / 路径 / 摘要检索条目，返回轻量摘要与得分")]
    async fn search_items(
        &self,
        Parameters(params): Parameters<SearchItemsParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.search_items_blocking(params))
            .await
            .map_err(|err| format!("检索任务失败：{err}"))?
    }

    /// 读取条目的完整 markdown。
    #[tool(description = "读取某个条目的完整 markdown 文档（签名 + 文档 + 成员）")]
    async fn get_item(&self, Parameters(params): Parameters<ItemParams>) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.get_item_blocking(params))
            .await
            .map_err(|err| format!("读取条目任务失败：{err}"))?
    }

    /// 查询条目的源码位置。
    #[tool(description = "查询条目对应的源码文件与行号")]
    async fn get_item_source(
        &self,
        Parameters(params): Parameters<ItemParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.get_item_source_blocking(params))
            .await
            .map_err(|err| format!("查询源码任务失败：{err}"))?
    }

    /// 重建索引。
    #[tool(description = "重新扫描产物目录并重建索引（默认增量）")]
    async fn rebuild_index(
        &self,
        Parameters(params): Parameters<RebuildParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;

        let options = BuildOptions {
            persist: true,
            incremental: !params.force.unwrap_or(false),
            ..Default::default()
        };
        let doc_dir = self.doc_dir.clone();
        let out_dir = self.out_dir.clone();
        let report = tokio::task::spawn_blocking(move || build(&doc_dir, &out_dir, &options))
            .await
            .map_err(|err| format!("重建任务失败：{err}"))?
            .map_err(|err| format!("重建索引失败：{err}"))?;

        self.cache.invalidate();
        let item_count = report.index.items.len();
        *self
            .index
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Loaded::new(report.index);

        Ok(to_json(&serde_json::json!({
            "rebuilt": true,
            "reused": report.reused,
            "item_count": item_count,
        })))
    }
}

/// 工具的同步实现（在 `spawn_blocking` 中执行，避免阻塞 async 执行器）。
impl DocsServer {
    /// 列出已索引 crate 的 JSON 文本。
    fn list_crates_text(&self) -> String {
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

    /// `list_items` 的同步实现。
    fn list_items_blocking(&self, params: ListItemsParams) -> Result<String, String> {
        let index = self.index();
        let kind = params.kind.as_deref().map(parse_kind).transpose()?;

        let filtered: Vec<&ItemSummary> = index
            .items
            .iter()
            .filter(|item| matches_crate(item, params.crate_name.as_deref()))
            .filter(|item| matches_module(item, params.module.as_deref()))
            .filter(|item| kind.is_none_or(|wanted| item.kind == wanted))
            .collect();

        let limit = params.limit.unwrap_or(100);
        let offset = params.offset.unwrap_or(0);
        let items: Vec<serde_json::Value> = filtered
            .iter()
            .skip(offset)
            .take(limit)
            .map(|item| summary_json(item))
            .collect();

        Ok(to_json(&serde_json::json!({
            "total": filtered.len(),
            "offset": offset,
            "returned": items.len(),
            "items": items,
        })))
    }

    /// `search_items` 的同步实现。
    fn search_items_blocking(&self, params: SearchItemsParams) -> Result<String, String> {
        let index = self.index();
        let mut query = SearchQuery::new(params.query);
        query.crate_name = params.crate_name;
        query.limit = params.limit.unwrap_or(20);
        query.offset = params.offset.unwrap_or(0);
        query.mode = match params.mode.as_deref() {
            Some("prefix") => MatchMode::Prefix,
            Some("fuzzy") => MatchMode::Fuzzy,
            _ => MatchMode::Substring,
        };
        if let Some(kind) = params.kind.as_deref().map(parse_kind).transpose()? {
            query.kinds.push(kind);
        }

        let outcome = search_page(&index, &query);
        let items: Vec<serde_json::Value> = outcome
            .hits
            .iter()
            .map(|hit| {
                let mut value = summary_json(&hit.item);
                value["score"] = serde_json::json!(hit.score);
                if let Some(snippet) = &hit.snippet {
                    value["snippet"] = serde_json::json!(snippet);
                }
                value
            })
            .collect();

        Ok(to_json(&serde_json::json!({
            "total": outcome.total,
            "offset": query.offset,
            "returned": items.len(),
            "hits": items,
        })))
    }

    /// `get_item` 的同步实现。
    fn get_item_blocking(&self, params: ItemParams) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
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

    /// `get_item_source` 的同步实现。
    fn get_item_source_blocking(&self, params: ItemParams) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;

        // 成员自身的源码位置未单独采集，这里返回其所属条目的位置。
        // `file` 为相对 `--doc-dir` 的规范路径，`path` 为可直接打开的绝对路径。
        let source = item.source.as_ref().map(|source| {
            serde_json::json!({
                "file": source.file,
                "path": self.doc_dir.join(&source.file).to_string_lossy(),
                "line_start": source.line_start,
                "line_end": source.line_end,
            })
        });
        Ok(to_json(&serde_json::json!({
            "id": params.id,
            "source": source,
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
        self.ensure_ready().await;
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
            resource_templates: vec![
                ResourceTemplate::new("rustdoc://{crate}/{+item}", "item")
                    .with_description("某个条目的 markdown 文档"),
                ResourceTemplate::new("rustdoc://{crate}", "crate")
                    .with_description("某个 crate 的条目列表（支持 ?offset=&limit= 分页）"),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let text = self.read_uri(&request.uri).await?;
        let contents =
            ResourceContents::text(text, request.uri.clone()).with_mime_type("text/markdown");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

impl DocsServer {
    /// 读取一个 `rustdoc://` URI 对应的文本内容。
    async fn read_uri(&self, uri: &str) -> Result<String, McpError> {
        self.ensure_ready().await;
        let path = uri
            .strip_prefix("rustdoc://")
            .ok_or_else(|| McpError::invalid_params(format!("非 rustdoc URI：{uri}"), None))?;

        // 支持资源分页：`rustdoc://{crate}?offset=N&limit=M`。
        let (path, query) = match path.split_once('?') {
            Some((path, query)) => (path, query),
            None => (path, ""),
        };

        if path == "crates" {
            return Ok(self.list_crates().await);
        }

        let (crate_name, item) = match path.split_once('/') {
            Some((crate_name, item)) => (crate_name, Some(item)),
            None => (path, None),
        };

        match item {
            // rustdoc://{crate}/{item}：item 的 `/` 对应 `::`。
            Some(item) => {
                let wanted = format!("{crate_name}::{}", item.replace('/', "::"));
                // 条目 id 带类型标记（如 `rmcp::attr.tool_router`），
                // 这里同时接受省略标记的写法（`rmcp::tool_router`）。
                let id = self
                    .index()
                    .find(&wanted)
                    .map(|summary| summary.id.0.clone())
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("未找到条目 `{wanted}`"), None)
                    })?;

                self.get_item(Parameters(ItemParams {
                    id,
                    max_bytes: None,
                }))
                .await
                .map_err(|message| McpError::invalid_params(message, None))
            }
            // rustdoc://{crate}：该 crate 的条目清单（可带 offset / limit）。
            None => self.crate_listing(
                crate_name,
                query_param(query, "offset").unwrap_or(0),
                query_param(query, "limit"),
            ),
        }
    }

    /// 列出某个 crate 的条目摘要（支持 `offset` / `limit` 分页）。
    fn crate_listing(
        &self,
        crate_name: &str,
        offset: usize,
        limit: Option<usize>,
    ) -> Result<String, McpError> {
        // 资源没有分页参数时也设安全上限，避免大 crate（rmcp 2000+ 条）
        // 返回超大 JSON 撑爆上下文；细粒度浏览应改用 `list_items`。
        const MAX_LIMIT: usize = 500;
        let limit = limit.unwrap_or(MAX_LIMIT).min(MAX_LIMIT);

        let index = self.index();
        let all: Vec<&ItemSummary> = index
            .items
            .iter()
            .filter(|item| matches_crate(item, Some(crate_name)))
            .collect();

        if all.is_empty() {
            return Err(McpError::invalid_params(
                format!("未找到 crate `{crate_name}`"),
                None,
            ));
        }

        let items: Vec<serde_json::Value> = all
            .iter()
            .skip(offset)
            .take(limit)
            .map(|item| summary_json(item))
            .collect();
        Ok(to_json(&serde_json::json!({
            "crate": crate_name,
            "item_count": all.len(),
            "offset": offset,
            "limit": limit,
            "returned": items.len(),
            "truncated": offset + items.len() < all.len(),
            "items": items,
        })))
    }
}

/// 尚无可用索引时的空占位。
///
/// 只有当 `index.json` 缺失或反序列化失败时才会用到；此时 `ready` 为
/// `false`，`ensure_ready` 会挡住对它的访问，直到后台构建完成。
fn empty_index(doc_dir: &Path) -> Index {
    Index {
        schema_version: INDEX_SCHEMA_VERSION,
        rustdoc_version: None,
        generated_at: 0,
        target_doc: doc_dir.to_string_lossy().into_owned(),
        granularity: Granularity::default(),
        crates: Vec::new(),
        items: Vec::new(),
    }
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

/// 解析 `kind` 参数。
///
/// 无法识别的值直接报错，避免像早先那样静默忽略过滤器、误返回全部类型
/// （见 `docs/lessons.md` #1.23）。
fn parse_kind(value: &str) -> Result<ItemKind, String> {
    ItemKind::parse_input(value).ok_or_else(|| {
        format!(
            "无法识别的 kind：`{value}`（可用：struct / fn / method / field / trait / macro 等）"
        )
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
///
/// 按「路径段」精确匹配，避免 `in` 误配到 `inner`（见 `docs/lessons.md` #4.1）。
/// 保留旧语义：只有位于 crate 之下（`path.len() > 1`）的条目才可能命中，
/// 空串模块名视为「任意子模块」。
fn matches_module(item: &ItemSummary, module: Option<&str>) -> bool {
    let Some(module) = module else {
        return true;
    };
    if item.path.len() <= 1 {
        return false;
    }
    if module.is_empty() {
        return true;
    }
    let mut wanted = module.split("::");
    let mut actual = item.path[1..].iter();
    wanted.all(|segment| actual.next().map(String::as_str) == Some(segment))
}

/// 从查询串里取一个 `usize` 参数，如 `offset=10&limit=5`。
fn query_param(query: &str, key: &str) -> Option<usize> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .and_then(|(_, value)| value.parse().ok())
}

/// 序列化为紧凑 JSON 文本。
///
/// 工具结果是给模型读的结构化数据，缩进空白只增加 token 成本、不增信息量。
fn to_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|err| format!("序列化失败：{err}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 冷启动不阻塞：`new` 立即返回（用空占位），后台构建完成后索引可用。
    #[tokio::test]
    async fn cold_start_builds_in_background() {
        let doc_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mcp-docs-core/tests/fixtures/doc_probe");
        let out = tempfile::tempdir().unwrap();

        // 全新 out 目录：此时没有索引，构造不应阻塞。
        let server = DocsServer::new(doc_dir, out.path().to_path_buf()).unwrap();

        // 等待后台就绪后，索引应已填充。
        server.ensure_ready().await;
        let index = server.index();
        assert_eq!(index.crates.len(), 1, "后台应构建出 doc_probe 的索引");
        assert!(!index.items.is_empty());
    }

    /// 已有索引时，就绪信号初始即为 true，工具无需等待。
    #[tokio::test]
    async fn existing_index_is_ready_immediately() {
        let doc_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mcp-docs-core/tests/fixtures/doc_probe");
        let out = tempfile::tempdir().unwrap();
        build(
            &doc_dir,
            out.path(),
            &BuildOptions {
                persist: true,
                ..Default::default()
            },
        )
        .unwrap();

        let server = DocsServer::new(doc_dir, out.path().to_path_buf()).unwrap();
        assert!(*server.ready.borrow(), "已有索引时应立即就绪");
        assert_eq!(server.index().crates.len(), 1);
    }

    /// 构造一个最小条目摘要，用于纯函数测试。
    fn summary(path: &[&str]) -> ItemSummary {
        ItemSummary {
            id: mcp_docs_core::ItemId("x".to_string()),
            kind: ItemKind::Struct,
            name: "x".to_string(),
            path: path.iter().map(|segment| (*segment).to_string()).collect(),
            one_line: String::new(),
            has_docs: false,
            has_members: false,
            file: String::new(),
            html_path: String::new(),
            parent_id: None,
            src_mtime: None,
        }
    }

    /// 模块过滤按整段匹配，`in` 不应误配 `inner`。
    #[test]
    fn module_filter_matches_whole_segments() {
        assert!(!matches_module(&summary(&["k", "inner"]), Some("in")));
        assert!(matches_module(&summary(&["k", "inner"]), Some("inner")));
        assert!(matches_module(
            &summary(&["k", "inner", "sub"]),
            Some("inner::sub")
        ));
        assert!(!matches_module(
            &summary(&["k", "inner"]),
            Some("inner::sub")
        ));
        // 不在 crate 之下时不匹配。
        assert!(!matches_module(&summary(&["k"]), Some("k")));
        // None / 空串保持旧语义。
        assert!(matches_module(&summary(&["k"]), None));
        assert!(matches_module(&summary(&["k", "inner"]), Some("")));
    }

    /// 资源分页参数解析。
    #[test]
    fn query_params_parse_usize() {
        assert_eq!(query_param("offset=10&limit=5", "offset"), Some(10));
        assert_eq!(query_param("offset=10&limit=5", "limit"), Some(5));
        assert_eq!(query_param("offset=x", "offset"), None);
        assert_eq!(query_param("", "offset"), None);
    }
}
