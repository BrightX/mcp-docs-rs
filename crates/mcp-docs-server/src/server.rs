//! MCP 服务：把索引、检索与 markdown 渲染暴露为工具与资源。
//!
//! 设计原则是「先搜后读」：检索类工具只返回轻量摘要，正文由 `get_item`
//! 按需读取（内部走 `DocCache`，见 `design.md` §7、§8）。

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    GetPromptRequestParams, GetPromptResponse, GetPromptResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams, Prompt,
    PromptArgument, PromptMessage, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Role, ServerCapabilities,
    ServerConfig,
};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::RequestContext;
use rmcp::{
    tool, tool_handler, tool_router, ErrorData as McpError, Json, RoleServer, ServerHandler,
};
use tokio::sync::watch;

use mcp_docs_core::MatchMode;
use mcp_docs_core::{
    build, extract_code_blocks, extract_source_lines, is_stale, load_index, module_tree,
    parse_trait_impls, related_items, render_item, render_member_item, search_page,
    trait_impl_rel_path, BuildOptions, DocCache, DocItem, Granularity, IdIndex, Index, ItemKind,
    ItemSummary, ParseOptions, RenderOptions, SearchQuery, INDEX_SCHEMA_VERSION,
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

/// `get_examples` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct GetExamplesParams {
    /// 条目 id。
    id: String,
    /// 最多返回的示例数，默认 5。
    max_examples: Option<usize>,
    /// 每个示例的字节上限（超出按字符边界截断）。
    max_bytes: Option<usize>,
}

/// `get_item_section` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct GetItemSectionParams {
    /// 条目 id。
    id: String,
    /// 分节名（如 `examples` / `panics` / `implementations`），大小写不敏感。
    section: String,
}

/// `batch_get_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct BatchGetItemsParams {
    /// 条目 id 列表（上限 20）。
    ids: Vec<String>,
    /// 每个条目的 markdown 字节上限。
    max_bytes_each: Option<usize>,
}

/// `get_source_text` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct GetSourceTextParams {
    /// 条目 id。
    id: String,
    /// 源码文本的字节上限。
    max_bytes: Option<usize>,
}

/// `module_tree` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct ModuleTreeParams {
    /// crate 名。
    #[serde(rename = "crate")]
    crate_name: String,
}

/// `get_related_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct RelatedItemsParams {
    /// 条目 id。
    id: String,
}

/// `find_by_signature` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct FindBySignatureParams {
    /// 签名子串（如 `-> Result<`），大小写不敏感。
    pattern: String,
    /// 限定 crate。
    #[serde(rename = "crate")]
    crate_name: Option<String>,
    /// 限定条目类型。
    kind: Option<String>,
    /// 返回上限，默认 20。
    limit: Option<usize>,
}

/// `search_docs` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
struct SearchDocsParams {
    /// 查询词。
    query: String,
    /// 限定 crate。
    #[serde(rename = "crate")]
    crate_name: Option<String>,
    /// 限定条目类型。
    kind: Option<String>,
    /// 返回上限，默认 20。
    limit: Option<usize>,
    /// 跳过的命中数（分页）。
    offset: Option<usize>,
}

/// 结构化输出用的条目摘要。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct SummaryOutput {
    id: String,
    kind: String,
    name: String,
    path: Vec<String>,
    one_line: String,
    has_docs: bool,
    has_members: bool,
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,
}

impl SummaryOutput {
    /// 由条目摘要构造（不含检索得分）。
    fn from_summary(item: &ItemSummary) -> Self {
        Self {
            id: item.id.0.clone(),
            kind: kind_name(item.kind),
            name: item.name.clone(),
            path: item.path.clone(),
            one_line: item.one_line.clone(),
            has_docs: item.has_docs,
            has_members: item.has_members,
            file: item.file.clone(),
            score: None,
            snippet: None,
        }
    }
}

/// `search_items` 的结构化返回。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct SearchItemsResponse {
    total: usize,
    offset: usize,
    returned: usize,
    hits: Vec<SummaryOutput>,
}

/// `get_item_json` 的结构化返回。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct ItemDetailOutput {
    id: String,
    kind: String,
    name: String,
    path: Vec<String>,
    signature: Option<String>,
    docs_md: Option<String>,
    source: Option<SourceOutput>,
    sections: Vec<SectionOutput>,
    members: Vec<MemberOutput>,
}

/// 源码位置的结构化表示。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct SourceOutput {
    file: String,
    line_start: Option<u32>,
    line_end: Option<u32>,
}

/// 分节的结构化表示。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct SectionOutput {
    id: String,
    title: String,
    body_md: String,
}

/// 成员的结构化表示。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct MemberOutput {
    id: String,
    kind: String,
    name: String,
    has_docs: bool,
}

/// `index_status` 的结构化返回。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct IndexStatusOutput {
    ready: bool,
    building: bool,
    schema_version: u32,
    rustdoc_version: Option<String>,
    crate_count: usize,
    item_count: usize,
    granularity: String,
    doc_dir: String,
    out_dir: String,
    generated_at: u64,
    stale: bool,
}

/// `module_tree` 的结构化返回节点。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct ModuleTreeNode {
    name: String,
    path: String,
    item_count: usize,
    children: Vec<ModuleTreeNode>,
}

impl From<mcp_docs_core::ModuleNode> for ModuleTreeNode {
    fn from(node: mcp_docs_core::ModuleNode) -> Self {
        Self {
            name: node.name,
            path: node.path,
            item_count: node.item_count,
            children: node.children.into_iter().map(Self::from).collect(),
        }
    }
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
    /// 是否正在后台构建索引（供 `index_status` 暴露进度）。
    building: Arc<AtomicBool>,
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
            building: Arc::new(AtomicBool::new(false)),
        };

        // 后台刷新索引；无论成功与否都要置位就绪，避免调用方永久等待。
        let worker = server.clone();
        let building = server.building.clone();
        tokio::spawn(async move {
            building.store(true, Ordering::Relaxed);
            if let Err(err) = worker.refresh_index(!has_index).await {
                eprintln!("后台构建索引失败：{err:#}");
            }
            building.store(false, Ordering::Relaxed);
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
    ) -> Result<Json<SearchItemsResponse>, String> {
        self.ensure_ready().await;
        let server = self.clone();
        let value = tokio::task::spawn_blocking(move || server.search_items_blocking(params))
            .await
            .map_err(|err| format!("检索任务失败：{err}"))??;
        Ok(Json(value))
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

    /// 抽取条目文档里的 rust 代码示例。
    #[tool(description = "抽取某个条目文档里的 rust 代码示例")]
    async fn get_examples(
        &self,
        Parameters(params): Parameters<GetExamplesParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.get_examples_blocking(params))
            .await
            .map_err(|err| format!("抽取示例任务失败：{err}"))?
    }

    /// 读取条目某个分节的 markdown。
    #[tool(description = "返回条目某个分节（如 examples / panics / implementations）的 markdown")]
    async fn get_item_section(
        &self,
        Parameters(params): Parameters<GetItemSectionParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.get_item_section_blocking(params))
            .await
            .map_err(|err| format!("读取分节任务失败：{err}"))?
    }

    /// 批量读取条目 markdown。
    #[tool(description = "批量读取多个条目的 markdown（ids 上限 20）")]
    async fn batch_get_items(
        &self,
        Parameters(params): Parameters<BatchGetItemsParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.batch_get_items_blocking(params))
            .await
            .map_err(|err| format!("批量读取任务失败：{err}"))?
    }

    /// 按源码位置读取源码文本。
    #[tool(description = "按源码位置读取条目对应的源码文本；取不到时回退为路径与行号")]
    async fn get_source_text(
        &self,
        Parameters(params): Parameters<GetSourceTextParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.get_source_text_blocking(params))
            .await
            .map_err(|err| format!("读取源码任务失败：{err}"))?
    }

    /// 返回结构化条目。
    #[tool(
        description = "返回条目的结构化 JSON（id / kind / signature / docs / sections / members）"
    )]
    async fn get_item_json(
        &self,
        Parameters(params): Parameters<ItemParams>,
    ) -> Result<Json<ItemDetailOutput>, String> {
        self.ensure_ready().await;
        let server = self.clone();
        let value = tokio::task::spawn_blocking(move || server.get_item_json_blocking(params))
            .await
            .map_err(|err| format!("读取结构化条目任务失败：{err}"))??;
        Ok(Json(value))
    }

    /// 返回索引状态。
    #[tool(description = "返回索引状态：就绪 / 构建中 / schema / 版本 / 计数 / 是否过期")]
    async fn index_status(&self) -> Result<Json<IndexStatusOutput>, String> {
        let server = self.clone();
        let value = tokio::task::spawn_blocking(move || server.index_status_blocking())
            .await
            .map_err(|err| format!("读取索引状态任务失败：{err}"))??;
        Ok(Json(value))
    }

    /// 返回 crate 的模块树。
    #[tool(description = "返回某个 crate 的模块树（含每级条目数）")]
    async fn module_tree(
        &self,
        Parameters(params): Parameters<ModuleTreeParams>,
    ) -> Result<Json<ModuleTreeNode>, String> {
        self.ensure_ready().await;
        let server = self.clone();
        let value = tokio::task::spawn_blocking(move || server.module_tree_blocking(params))
            .await
            .map_err(|err| format!("构建模块树任务失败：{err}"))??;
        Ok(Json(value))
    }

    /// 返回条目的相关条目。
    #[tool(description = "返回条目的父条目、同模块兄弟与子成员")]
    async fn get_related_items(
        &self,
        Parameters(params): Parameters<RelatedItemsParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.get_related_items_blocking(params))
            .await
            .map_err(|err| format!("查询相关条目任务失败：{err}"))?
    }

    /// 列出 trait 的实现者。
    #[tool(description = "列出实现了某个 trait 的类型")]
    async fn get_trait_implementors(
        &self,
        Parameters(params): Parameters<ItemParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.get_trait_implementors_blocking(params))
            .await
            .map_err(|err| format!("查询 trait 实现者任务失败：{err}"))?
    }

    /// 按签名子串检索。
    #[tool(description = "按签名子串（如 `-> Result<`）检索条目")]
    async fn find_by_signature(
        &self,
        Parameters(params): Parameters<FindBySignatureParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.find_by_signature_blocking(params))
            .await
            .map_err(|err| format!("按签名检索任务失败：{err}"))?
    }

    /// 正文全文检索。
    #[tool(description = "在已导出的 markdown 正文里做全文检索")]
    async fn search_docs(
        &self,
        Parameters(params): Parameters<SearchDocsParams>,
    ) -> Result<String, String> {
        self.ensure_ready().await;
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.search_docs_blocking(params))
            .await
            .map_err(|err| format!("正文检索任务失败：{err}"))?
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
        self.building.store(true, Ordering::Relaxed);
        let report = tokio::task::spawn_blocking(move || build(&doc_dir, &out_dir, &options))
            .await
            .map_err(|err| format!("重建任务失败：{err}"))?
            .map_err(|err| format!("重建索引失败：{err}"))?;
        self.building.store(false, Ordering::Relaxed);

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
    fn search_items_blocking(
        &self,
        params: SearchItemsParams,
    ) -> Result<SearchItemsResponse, String> {
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
        let hits: Vec<SummaryOutput> = outcome
            .hits
            .iter()
            .map(|hit| {
                let mut output = SummaryOutput::from_summary(&hit.item);
                output.score = Some(hit.score);
                output.snippet = hit.snippet.clone();
                output
            })
            .collect();

        Ok(SearchItemsResponse {
            total: outcome.total,
            offset: query.offset,
            returned: hits.len(),
            hits,
        })
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

    /// 定位条目并渲染其完整 markdown（成员只渲染自身小节）。
    fn render_full_markdown(&self, id: &str) -> Result<String, String> {
        let index = self.index();
        let summary = index.find(id).ok_or_else(|| format!("未找到条目 `{id}`"))?;
        let item = self.load_item(summary)?;
        let options = RenderOptions::default();
        Ok(if summary.kind.is_member() {
            match item.members.iter().find(|member| member.id == summary.id) {
                Some(member) => render_member_item(member, &options),
                None => render_item(&item, &options),
            }
        } else {
            render_item(&item, &options)
        })
    }

    /// `get_examples` 的同步实现。
    fn get_examples_blocking(&self, params: GetExamplesParams) -> Result<String, String> {
        let markdown = self.render_full_markdown(&params.id)?;
        let max_examples = params.max_examples.unwrap_or(5);
        let examples: Vec<serde_json::Value> = extract_code_blocks(&markdown)
            .into_iter()
            .filter(|(language, _)| language.starts_with("rust"))
            .take(max_examples)
            .map(|(language, code)| {
                let code = truncate(code, params.max_bytes);
                serde_json::json!({ "language": language, "code": code })
            })
            .collect();
        Ok(to_json(&serde_json::json!({
            "id": params.id,
            "returned": examples.len(),
            "examples": examples,
        })))
    }

    /// `get_item_section` 的同步实现。
    fn get_item_section_blocking(&self, params: GetItemSectionParams) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;
        let wanted = params.section.trim().to_ascii_lowercase();
        let options = RenderOptions::default();

        let Some(section) = item.sections.iter().find(|section| {
            section.id.eq_ignore_ascii_case(&wanted) || section.title.to_ascii_lowercase() == wanted
        }) else {
            let available: Vec<&str> = item.sections.iter().map(|s| s.id.as_str()).collect();
            return Err(format!(
                "条目 `{}` 没有分节 `{}`；可用分节：{}",
                params.id,
                params.section,
                available.join(", ")
            ));
        };

        let members: Vec<serde_json::Value> = section
            .members
            .iter()
            .map(|member| {
                serde_json::json!({
                    "id": member.id.0,
                    "name": member.name,
                    "markdown": render_member_item(member, &options),
                })
            })
            .collect();
        Ok(to_json(&serde_json::json!({
            "id": params.id,
            "section": section.id,
            "title": section.title,
            "body_md": section.body_md,
            "returned": members.len(),
            "members": members,
        })))
    }

    /// `batch_get_items` 的同步实现。
    fn batch_get_items_blocking(&self, params: BatchGetItemsParams) -> Result<String, String> {
        const MAX_IDS: usize = 20;
        let mut items = Vec::new();
        for id in params.ids.iter().take(MAX_IDS) {
            match self.render_full_markdown(id) {
                Ok(markdown) => items.push(serde_json::json!({
                    "id": id,
                    "markdown": truncate(markdown, params.max_bytes_each),
                })),
                Err(err) => items.push(serde_json::json!({ "id": id, "error": err })),
            }
        }
        Ok(to_json(&serde_json::json!({
            "requested": params.ids.len(),
            "returned": items.len(),
            "truncated": params.ids.len() > MAX_IDS,
            "items": items,
        })))
    }

    /// `get_source_text` 的同步实现。
    fn get_source_text_blocking(&self, params: GetSourceTextParams) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;

        let Some(source) = item.source.as_ref() else {
            return Ok(to_json(&serde_json::json!({
                "id": params.id,
                "source": null,
                "text": null,
                "note": "该条目没有源码位置",
            })));
        };

        let path = self.doc_dir.join(&source.file);
        let text = fs::read_to_string(&path).ok().and_then(|html| {
            let start = source.line_start.unwrap_or(1);
            let end = source.line_end.unwrap_or(u32::MAX);
            extract_source_lines(&html, start, end)
        });
        let text = text.map(|text| truncate(text, params.max_bytes));

        Ok(to_json(&serde_json::json!({
            "id": params.id,
            "file": source.file,
            "path": path.to_string_lossy(),
            "line_start": source.line_start,
            "line_end": source.line_end,
            "text": text,
            "note": if text.is_none() { "无法从源码页提取文本，仅返回路径与行号" } else { "" },
        })))
    }

    /// `get_item_json` 的同步实现。
    fn get_item_json_blocking(&self, params: ItemParams) -> Result<ItemDetailOutput, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;

        let sections: Vec<SectionOutput> = item
            .sections
            .iter()
            .map(|section| SectionOutput {
                id: section.id.clone(),
                title: section.title.clone(),
                body_md: section.body_md.clone(),
            })
            .collect();
        let members: Vec<MemberOutput> = item
            .members
            .iter()
            .map(|member| MemberOutput {
                id: member.id.0.clone(),
                kind: kind_name(member.kind),
                name: member.name.clone(),
                has_docs: member.docs_md.is_some(),
            })
            .collect();
        let source = item.source.as_ref().map(|source| SourceOutput {
            file: source.file.clone(),
            line_start: source.line_start,
            line_end: source.line_end,
        });

        Ok(ItemDetailOutput {
            id: item.id.0.clone(),
            kind: kind_name(item.kind),
            name: item.name.clone(),
            path: item.path.clone(),
            signature: item.signature.clone(),
            docs_md: item.docs_md.clone(),
            source,
            sections,
            members,
        })
    }

    /// `index_status` 的同步实现。
    fn index_status_blocking(&self) -> Result<IndexStatusOutput, String> {
        let building = self.building.load(Ordering::Relaxed);
        let meta_path = self.out_dir.join("meta.json");
        let stale = is_stale(&self.doc_dir, &meta_path).unwrap_or(true);
        let index = self.index();
        Ok(IndexStatusOutput {
            ready: *self.ready.borrow(),
            building,
            schema_version: index.schema_version,
            rustdoc_version: index.rustdoc_version.clone(),
            crate_count: index.crates.len(),
            item_count: index.items.len(),
            granularity: serde_json::to_value(index.granularity)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default(),
            doc_dir: self.doc_dir.to_string_lossy().into_owned(),
            out_dir: self.out_dir.to_string_lossy().into_owned(),
            generated_at: index.generated_at,
            stale,
        })
    }

    /// `module_tree` 的同步实现。
    fn module_tree_blocking(&self, params: ModuleTreeParams) -> Result<ModuleTreeNode, String> {
        let index = self.index();
        let tree = module_tree(&index.items, &params.crate_name)
            .ok_or_else(|| format!("未找到 crate `{}`", params.crate_name))?;
        Ok(ModuleTreeNode::from(tree))
    }

    /// `get_related_items` 的同步实现。
    fn get_related_items_blocking(&self, params: RelatedItemsParams) -> Result<String, String> {
        let index = self.index();
        let id = index
            .find(&params.id)
            .map(|summary| summary.id.0.clone())
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let related =
            related_items(&index.items, &id).ok_or_else(|| format!("未找到条目 `{id}`"))?;

        Ok(to_json(&serde_json::json!({
            "id": id,
            "parent": related.parent.as_ref().map(summary_json),
            "siblings": related.siblings.iter().map(summary_json).collect::<Vec<_>>(),
            "children": related.children.iter().map(summary_json).collect::<Vec<_>>(),
        })))
    }

    /// `get_trait_implementors` 的同步实现。
    fn get_trait_implementors_blocking(&self, params: ItemParams) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        if !matches!(summary.kind, ItemKind::Trait | ItemKind::TraitAlias) {
            return Err(format!(
                "`{}` 不是 trait（kind={:?}）",
                summary.id.0, summary.kind
            ));
        }

        let rel = trait_impl_rel_path(Path::new(&summary.html_path));
        let path = self.doc_dir.join(&rel);
        let impls = fs::read_to_string(&path)
            .map(|js| parse_trait_impls(&js))
            .unwrap_or_default();
        let implementors: Vec<serde_json::Value> = impls
            .iter()
            .map(|imp| serde_json::json!({ "crate": imp.crate_name, "impl": imp.text }))
            .collect();

        Ok(to_json(&serde_json::json!({
            "id": summary.id.0,
            "count": implementors.len(),
            "implementors": implementors,
            "note": if implementors.is_empty() {
                "未找到实现者（可能确实无实现，或产物缺少 trait.impl 文件）"
            } else {
                ""
            },
        })))
    }

    /// `find_by_signature` 的同步实现。
    fn find_by_signature_blocking(&self, params: FindBySignatureParams) -> Result<String, String> {
        let index = self.index();
        let needle = params.pattern.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(to_json(&serde_json::json!({ "total": 0, "hits": [] })));
        }
        let kind = params.kind.as_deref().map(parse_kind).transpose()?;
        let limit = params.limit.unwrap_or(20);

        let mut hits = Vec::new();
        for item in index
            .items
            .iter()
            .filter(|item| matches_crate(item, params.crate_name.as_deref()))
            .filter(|item| kind.is_none_or(|wanted| item.kind == wanted))
        {
            let Some(signature) = &item.signature else {
                continue;
            };
            if contains_ignore_ascii_case(signature, &needle) {
                hits.push(summary_json(item));
                if hits.len() >= limit {
                    break;
                }
            }
        }
        Ok(to_json(&serde_json::json!({
            "pattern": params.pattern,
            "returned": hits.len(),
            "hits": hits,
        })))
    }

    /// `search_docs` 的同步实现（方案 a：扫描已导出的 markdown 正文）。
    fn search_docs_blocking(&self, params: SearchDocsParams) -> Result<String, String> {
        let index = self.index();
        let needle = params.query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(to_json(&serde_json::json!({ "returned": 0, "hits": [] })));
        }
        let kind = params.kind.as_deref().map(parse_kind).transpose()?;
        let limit = params.limit.unwrap_or(20);
        let offset = params.offset.unwrap_or(0);

        let mut seen = std::collections::HashSet::new();
        let mut matched = 0usize;
        let mut hits = Vec::new();
        for item in index
            .items
            .iter()
            .filter(|item| !item.file.is_empty())
            .filter(|item| matches_crate(item, params.crate_name.as_deref()))
            .filter(|item| kind.is_none_or(|wanted| item.kind == wanted))
        {
            // 条目粒度下多个条目共享同一文件，按文件去重避免重复扫描。
            if !seen.insert(item.file.as_str()) {
                continue;
            }
            let Ok(text) = fs::read_to_string(self.out_dir.join(&item.file)) else {
                continue;
            };
            let Some(snippet) = snippet_around(&text, &needle) else {
                continue;
            };
            matched += 1;
            if matched <= offset {
                continue;
            }
            hits.push(serde_json::json!({
                "file": item.file,
                "id": item.id.0,
                "snippet": snippet,
            }));
            if hits.len() >= limit {
                break;
            }
        }

        Ok(to_json(&serde_json::json!({
            "query": params.query,
            "offset": offset,
            "returned": hits.len(),
            "hits": hits,
        })))
    }
}

#[tool_handler]
impl ServerHandler for DocsServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .enable_resources_list_changed()
                .build(),
        )
        .with_instructions(
            "检索本地 rustdoc 文档。先用 search_items / list_items 找到条目，再用 get_item 读取 markdown。",
        )
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult {
            prompts: prompt_definitions(),
            ..Default::default()
        })
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let id = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("id"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let messages = build_prompt(&request.name, id)
            .map_err(|message| McpError::invalid_params(message, None))?;
        Ok(GetPromptResult::new(messages).into())
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

/// 条目类型的 serde 名（`snake_case`），供结构化输出使用。
fn kind_name(kind: ItemKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// 服务提供的 prompt 模板定义。
fn prompt_definitions() -> Vec<Prompt> {
    let id_argument = || {
        PromptArgument::new("id")
            .with_description("条目 id（如 tokio::spawn）")
            .with_required(true)
    };
    vec![
        Prompt::new(
            "explain_api",
            Some("解释某个 API 的用途与用法"),
            Some(vec![id_argument()]),
        ),
        Prompt::new(
            "usage_example",
            Some("给出某个 API 的调用示例"),
            Some(vec![id_argument()]),
        ),
    ]
}

/// 按模板名与条目 id 构造 prompt 消息。
fn build_prompt(name: &str, id: &str) -> Result<Vec<PromptMessage>, String> {
    if id.trim().is_empty() {
        return Err("缺少必需参数 `id`".to_string());
    }
    let text = match name {
        "explain_api" => format!(
            "请解释条目 `{id}` 的用途与用法：先调用 `get_item`（id = \"{id}\"）读取它的完整文档（签名 + 文档 + 成员），再结合签名与示例给出简明讲解。"
        ),
        "usage_example" => format!(
            "请给出条目 `{id}` 的调用示例：先调用 `get_item` 与 `get_examples`（id = \"{id}\"）获取文档与示例代码，再给出可直接运行的最小示例。"
        ),
        other => return Err(format!("未知的 prompt：`{other}`")),
    };
    Ok(vec![PromptMessage::new_text(Role::User, text)])
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

/// ASCII 大小写不敏感的子串匹配（`needle` 需已小写）。
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let hay = haystack.as_bytes();
    let target = needle.as_bytes();
    target.is_empty()
        || (target.len() <= hay.len()
            && hay
                .windows(target.len())
                .any(|window| window.eq_ignore_ascii_case(target)))
}

/// 取 `needle` 命中处前后各 40 字符的上下文片段（按字符边界）。
fn snippet_around(text: &str, needle: &str) -> Option<String> {
    const WINDOW: usize = 40;
    let lower = text.to_lowercase();
    let mut pos = lower.find(needle)?.min(text.len());
    while pos > 0 && !text.is_char_boundary(pos) {
        pos -= 1;
    }
    let chars: Vec<char> = text.chars().collect();
    let center = text[..pos].chars().count().min(chars.len());
    let start = center.saturating_sub(WINDOW);
    let end = (center + WINDOW).min(chars.len());

    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(chars[start..end].iter().copied());
    if end < chars.len() {
        out.push('…');
    }
    Some(out)
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
            signature: None,
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

    /// prompt 定义与消息构造。
    #[test]
    fn prompt_definitions_and_build() {
        let definitions = prompt_definitions();
        assert_eq!(definitions.len(), 2);
        assert!(definitions
            .iter()
            .any(|prompt| prompt.name == "explain_api"));

        let messages = build_prompt("explain_api", "doc_probe::struct.Demo").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);

        assert!(build_prompt("explain_api", "").is_err());
        assert!(build_prompt("unknown", "x").is_err());
    }

    /// 构建带索引与 markdown 的服务，供工具测试。
    fn server_with_index() -> (DocsServer, tempfile::TempDir) {
        let doc_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mcp-docs-core/tests/fixtures/doc_probe");
        let out = tempfile::tempdir().unwrap();
        build(
            &doc_dir,
            out.path(),
            &BuildOptions {
                persist: true,
                write_markdown: true,
                ..Default::default()
            },
        )
        .unwrap();
        let server = DocsServer::new(doc_dir, out.path().to_path_buf()).unwrap();
        (server, out)
    }

    /// `index_status` 暴露 schema / 计数 / 粒度。
    #[tokio::test]
    async fn index_status_reports_counts() {
        let (server, _out) = server_with_index();
        server.ensure_ready().await;
        let status = server.index_status_blocking().unwrap();
        assert_eq!(status.schema_version, INDEX_SCHEMA_VERSION);
        assert!(status.item_count > 0);
        assert_eq!(status.granularity, "member");
        assert!(status.ready);
    }

    /// `batch_get_items` 对 ids 数量设上限。
    #[tokio::test]
    async fn batch_get_items_caps_ids() {
        let (server, _out) = server_with_index();
        server.ensure_ready().await;
        let ids = vec!["doc_probe::struct.Demo".to_string(); 25];
        let text = server
            .batch_get_items_blocking(BatchGetItemsParams {
                ids,
                max_bytes_each: None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["returned"], 20);
        assert_eq!(value["truncated"], true);
    }

    /// `get_examples` 至少返回签名围栏块。
    #[tokio::test]
    async fn get_examples_returns_code() {
        let (server, _out) = server_with_index();
        server.ensure_ready().await;
        let text = server
            .get_examples_blocking(GetExamplesParams {
                id: "doc_probe::struct.Demo".to_string(),
                max_examples: None,
                max_bytes: None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value["returned"].as_u64().unwrap() >= 1);
    }

    /// `find_by_signature` 能按签名子串命中。
    #[tokio::test]
    async fn find_by_signature_matches() {
        let (server, _out) = server_with_index();
        server.ensure_ready().await;
        let text = server
            .find_by_signature_blocking(FindBySignatureParams {
                pattern: "struct Demo".to_string(),
                crate_name: None,
                kind: None,
                limit: None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value["returned"].as_u64().unwrap() >= 1);
    }
}
