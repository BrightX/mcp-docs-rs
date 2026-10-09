//! 单个项目的运行时状态与文档访问逻辑。
//!
//! 一个 [`Project`] 绑定一对「rustdoc 产物目录 + 导出目录」，持有自己的索引、
//! 解析缓存与就绪信号，项目之间互不干扰。MCP 服务的每个工具/资源最终都落到
//! 某个 `Project` 上执行。

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard};

use rmcp::schemars::{self, JsonSchema};
use rmcp::{ErrorData as McpError, Peer, RoleServer};
use tokio::sync::{Mutex, watch};

use mcp_docs_core::{
    BuildOptions, DocCache, DocItem, Granularity, INDEX_SCHEMA_VERSION, IdIndex, Index, ItemKind,
    ItemSummary, ParseOptions, RenderOptions, SearchQuery, build, crate_overview,
    cross_crate_module_target, extract_code_blocks, extract_source_lines, find_trait_impl_paths,
    is_stale, kind_serde_name, load_index, module_tree, parse_page_impls, parse_trait_impls,
    related_items, render_item, render_member_item, rewrite_links, search_page, top_level_modules,
};

/// `list_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct ListItemsParams {
    /// 限定 crate 名。
    #[serde(rename = "crate")]
    pub(crate) crate_name: Option<String>,
    /// 限定模块路径（如 `inner`）。
    pub(crate) module: Option<String>,
    /// 限定条目类型，接受 `fn`（前缀）或 `function`（自然名）。
    pub(crate) kind: Option<String>,
    /// 返回上限，默认 100。
    pub(crate) limit: Option<usize>,
    /// 跳过的条目数，默认 0。
    pub(crate) offset: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `search_items` 的匹配模式。
#[derive(Debug, Clone, Copy, serde::Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ModeArg {
    /// 子串匹配（默认）。
    Substring,
    /// 前缀匹配。
    Prefix,
    /// 模糊子序列匹配。
    Fuzzy,
}

/// `search_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct SearchItemsParams {
    /// 查询词。
    pub(crate) query: String,
    /// 限定 crate 名。
    #[serde(rename = "crate")]
    pub(crate) crate_name: Option<String>,
    /// 限定条目类型。
    pub(crate) kind: Option<String>,
    /// 匹配模式：`substring`（默认）/ `prefix` / `fuzzy`。
    pub(crate) mode: Option<ModeArg>,
    /// 返回上限，默认 20。
    pub(crate) limit: Option<usize>,
    /// 跳过的命中数，默认 0。
    pub(crate) offset: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `get_item` / `get_item_source` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct ItemParams {
    /// 条目 id，取自 `search_items` / `list_items` 的返回值（如 `tokio::task::fn.spawn`）；
    /// 也可省略类型标记（如 `tokio::task::spawn`）。
    pub(crate) id: String,
    /// 返回 markdown 的字节上限（超出时按字符边界截断）。
    pub(crate) max_bytes: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `rebuild_index` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct RebuildParams {
    /// 强制全量重建（默认增量）。
    pub(crate) force: Option<bool>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `get_examples` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct GetExamplesParams {
    /// 条目 id。
    pub(crate) id: String,
    /// 最多返回的示例数，默认 5。
    pub(crate) max_examples: Option<usize>,
    /// 每个示例的字节上限（超出按字符边界截断）。
    pub(crate) max_bytes: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `get_item_section` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct GetItemSectionParams {
    /// 条目 id。
    pub(crate) id: String,
    /// 分节名（如 `examples` / `panics` / `implementations`），大小写不敏感。
    pub(crate) section: String,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `batch_get_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct BatchGetItemsParams {
    /// 条目 id 列表（上限 20）。
    pub(crate) ids: Vec<String>,
    /// 每个条目的 markdown 字节上限。
    pub(crate) max_bytes_each: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `get_source_text` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct GetSourceTextParams {
    /// 条目 id。
    pub(crate) id: String,
    /// 源码文本的字节上限。
    pub(crate) max_bytes: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `module_tree` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct ModuleTreeParams {
    /// crate 名。
    #[serde(rename = "crate")]
    pub(crate) crate_name: String,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `get_related_items` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct RelatedItemsParams {
    /// 条目 id。
    pub(crate) id: String,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `find_by_signature` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct FindBySignatureParams {
    /// 签名子串（如 `-> Result<`），大小写不敏感。
    pub(crate) pattern: String,
    /// 限定 crate。
    #[serde(rename = "crate")]
    pub(crate) crate_name: Option<String>,
    /// 限定条目类型。
    pub(crate) kind: Option<String>,
    /// 返回上限，默认 20。
    pub(crate) limit: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// `search_docs` 的参数。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct SearchDocsParams {
    /// 查询词。
    pub(crate) query: String,
    /// 限定 crate。
    #[serde(rename = "crate")]
    pub(crate) crate_name: Option<String>,
    /// 限定条目类型。
    pub(crate) kind: Option<String>,
    /// 返回上限，默认 20。
    pub(crate) limit: Option<usize>,
    /// 跳过的命中数（分页）。
    pub(crate) offset: Option<usize>,
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// 仅带 `project` 的参数（用于 `list_crates` / `index_status` 这类原本无参的工具）。
#[derive(Debug, Default, serde::Deserialize, JsonSchema)]
pub(crate) struct ProjectParams {
    /// 目标项目名；省略时用缺省项目。
    #[serde(default)]
    pub(crate) project: Option<String>,
}

/// 结构化输出用的条目摘要。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(crate) struct SummaryOutput {
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
pub(crate) struct SearchItemsResponse {
    pub(crate) total: usize,
    pub(crate) offset: usize,
    pub(crate) returned: usize,
    pub(crate) hits: Vec<SummaryOutput>,
}

/// `get_item_json` 的结构化返回。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(crate) struct ItemDetailOutput {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) path: Vec<String>,
    pub(crate) signature: Option<String>,
    pub(crate) docs_md: Option<String>,
    pub(crate) source: Option<SourceOutput>,
    pub(crate) sections: Vec<SectionOutput>,
    pub(crate) members: Vec<MemberOutput>,
}

/// 源码位置的结构化表示。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(crate) struct SourceOutput {
    pub(crate) file: String,
    pub(crate) line_start: Option<u32>,
    pub(crate) line_end: Option<u32>,
}

/// 分节的结构化表示。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(crate) struct SectionOutput {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) body_md: String,
}

/// 成员的结构化表示。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(crate) struct MemberOutput {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) has_docs: bool,
}

/// `index_status` 的结构化返回。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(crate) struct IndexStatusOutput {
    pub(crate) project: String,
    pub(crate) ready: bool,
    pub(crate) building: bool,
    pub(crate) schema_version: u32,
    pub(crate) rustdoc_version: Option<String>,
    pub(crate) crate_count: usize,
    pub(crate) item_count: usize,
    pub(crate) granularity: String,
    pub(crate) doc_dir: String,
    pub(crate) out_dir: String,
    pub(crate) generated_at: u64,
    pub(crate) stale: bool,
}

/// `module_tree` 的结构化返回节点。
#[derive(Debug, serde::Serialize, JsonSchema)]
pub(crate) struct ModuleTreeNode {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) item_count: usize,
    pub(crate) children: Vec<ModuleTreeNode>,
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

/// 一个被服务的项目。
pub(crate) struct Project {
    /// 项目名（用于工具/资源定位）。
    pub(crate) name: String,
    doc_dir: Arc<PathBuf>,
    out_dir: Arc<PathBuf>,
    /// 共享库根目录（跨项目复用）；`None` 表示不启用。
    store: Option<PathBuf>,
    index: Arc<RwLock<Loaded>>,
    cache: Arc<DocCache>,
    /// 就绪信号：后台加载/重建完成时置 `true`。
    ready: watch::Receiver<bool>,
    /// 就绪信号的发送端（由后台任务在完成时置位）。
    ready_tx: watch::Sender<bool>,
    /// 是否正在后台构建索引。
    building: Arc<AtomicBool>,
    /// 与 server 共享的客户端句柄，构建完成后发资源变更通知。
    peer: Arc<OnceLock<Peer<RoleServer>>>,
    /// 与其它项目共享的构建锁，串行化全量构建。
    build_lock: Arc<Mutex<()>>,
    /// 后台构建是否已启动（lazy：首次访问才启动）。
    started: AtomicBool,
    /// 构造时是否已有可用索引（决定后台是否需要强制构建）。
    initial_ready: bool,
}

impl Project {
    /// 构造项目（**不启动**后台构建；由 [`Project::start`] 触发）。
    ///
    /// 同步加载已有索引（毫秒~亚秒级）即可立即服务；索引的过期判定与重建放到
    /// 后台任务，避免冷启动拖住 `initialize`。
    pub(crate) fn new(
        name: String,
        doc_dir: PathBuf,
        out_dir: PathBuf,
        store: Option<PathBuf>,
        peer: Arc<OnceLock<Peer<RoleServer>>>,
        build_lock: Arc<Mutex<()>>,
    ) -> Arc<Self> {
        let doc_dir = Arc::new(doc_dir);
        let out_dir = Arc::new(out_dir);

        let loaded = load_index(&out_dir.join(mcp_docs_core::INDEX_FILE_NAME)).ok();
        let has_index = loaded.is_some();
        let initial = loaded.unwrap_or_else(|| empty_index(&doc_dir));
        let (ready_tx, ready) = watch::channel(has_index);

        Arc::new(Self {
            name,
            doc_dir,
            out_dir,
            store,
            index: Arc::new(RwLock::new(Loaded::new(initial))),
            cache: Arc::new(DocCache::new()),
            ready,
            ready_tx,
            building: Arc::new(AtomicBool::new(false)),
            peer,
            build_lock,
            started: AtomicBool::new(false),
            initial_ready: has_index,
        })
    }

    /// 启动后台构建（幂等）：无论成功与否都要置位就绪，避免调用方永久等待。
    pub(crate) fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let worker = Arc::clone(self);
        let force = !self.initial_ready;
        tokio::spawn(async move {
            if let Err(err) = worker.refresh_index(force).await {
                eprintln!("[{}] 后台构建索引失败：{err:#}", worker.name);
            }
            worker.notify_resources_changed().await;
            let _ = worker.ready_tx.send(true);
        });
    }

    /// 等待索引就绪；已就绪时立即返回。
    pub(crate) async fn ensure_ready(&self) {
        let mut ready = self.ready.clone();
        loop {
            if *ready.borrow() {
                return;
            }
            if ready.changed().await.is_err() {
                return;
            }
        }
    }

    /// 当前是否就绪（不阻塞）。
    pub(crate) fn is_ready(&self) -> bool {
        *self.ready.borrow()
    }

    /// 是否正在后台构建。
    pub(crate) fn is_building(&self) -> bool {
        self.building.load(Ordering::Relaxed)
    }

    /// 项目名 / 目录等信息，供 `list_projects` 与 `index_status` 使用。
    pub(crate) fn doc_dir(&self) -> &Path {
        &self.doc_dir
    }

    /// 当前索引的 (crate 数, 条目数)；未就绪时为 0。
    pub(crate) fn counts(&self) -> (usize, usize) {
        let index = self.index();
        (index.crates.len(), index.items.len())
    }

    /// 导出目录。
    pub(crate) fn out_dir(&self) -> &Path {
        &self.out_dir
    }

    /// 后台刷新索引：缺失（`force`）或过期时全量重建，随后替换内存索引。
    async fn refresh_index(&self, force: bool) -> anyhow::Result<()> {
        let meta_path = self.out_dir.join("meta.json");
        if !force && !is_stale(&self.doc_dir, &meta_path)? {
            return Ok(());
        }

        // 串行化：多个项目依次全量构建，避免并发跑 rayon 抢占资源、叠加内存峰值。
        let _guard = self.build_lock.lock().await;
        // 只有确实要构建时才置位，避免 no-op 刷新期间 building 短暂为真（E-3.1）。
        self.building.store(true, Ordering::Relaxed);
        let outcome = self.build_into_memory().await;
        self.building.store(false, Ordering::Relaxed);
        outcome
    }

    /// 全量构建并替换内存索引（不含 building 状态管理）。
    async fn build_into_memory(&self) -> anyhow::Result<()> {
        let doc_dir = self.doc_dir.clone();
        let out_dir = self.out_dir.clone();
        let store = self.store.clone();
        // 解析/渲染是 CPU 密集的，放到阻塞线程池，别占着 async 执行器。
        let index = tokio::task::spawn_blocking(move || {
            build(
                &doc_dir,
                &out_dir,
                &BuildOptions {
                    persist: true,
                    store,
                    ..Default::default()
                },
            )
            .map(|report| report.index)
        })
        .await??;

        self.cache.invalidate();
        *self.write_index() = Loaded::new(index);
        Ok(())
    }

    /// 重建索引（工具触发）：默认增量，`force` 全量；完成后通知客户端。
    pub(crate) async fn rebuild(&self, force: bool) -> Result<String, String> {
        let options = BuildOptions {
            persist: true,
            incremental: !force,
            store: self.store.clone(),
            ..Default::default()
        };
        let doc_dir = self.doc_dir.clone();
        let out_dir = self.out_dir.clone();
        // 与后台构建共用同一把锁，避免并发全量构建。
        let _guard = self.build_lock.lock().await;
        self.building.store(true, Ordering::Relaxed);
        let report = tokio::task::spawn_blocking(move || build(&doc_dir, &out_dir, &options))
            .await
            .map_err(|err| format!("重建任务失败：{err}"))?
            .map_err(|err| format!("重建索引失败：{err}"))?;
        self.building.store(false, Ordering::Relaxed);

        self.cache.invalidate();
        let item_count = report.index.items.len();
        *self.write_index() = Loaded::new(report.index);

        self.notify_resources_changed().await;

        Ok(to_json(&serde_json::json!({
            "project": self.name,
            "rebuilt": true,
            "reused": report.reused,
            "shared_hits": report.shared_hits,
            "item_count": item_count,
        })))
    }

    /// 只读访问索引。
    fn index(&self) -> RwLockReadGuard<'_, Loaded> {
        self.index
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 可写访问索引。
    fn write_index(&self) -> std::sync::RwLockWriteGuard<'_, Loaded> {
        self.index
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 尽力通知客户端资源清单已变化（未捕获到 peer 时静默跳过）。
    pub(crate) async fn notify_resources_changed(&self) {
        if let Some(peer) = self.peer.get() {
            let _ = peer.notify_resource_list_changed().await;
        }
    }

    /// 解析条目（命中内存缓存时直接复用）。
    fn load_item(&self, summary: &ItemSummary) -> Result<Arc<DocItem>, String> {
        let rel = PathBuf::from(&summary.html_path);
        self.cache
            .get_or_parse(&self.doc_dir, &rel, &ParseOptions::default())
            .map_err(|err| format!("解析条目 `{}` 失败：{err}", summary.id))
    }

    /// 列出已索引 crate 的 JSON 文本。
    pub(crate) fn list_crates_text(&self) -> String {
        let index = self.index();
        let crates: Vec<serde_json::Value> = index
            .crates
            .iter()
            .map(|krate| {
                serde_json::json!({
                    "name": krate.name,
                    "item_count": krate.item_count,
                })
            })
            .collect();
        to_json(&serde_json::json!({
            "project": self.name,
            "crates": crates,
        }))
    }

    /// 列出某个 crate 的条目摘要（支持 `offset` / `limit` 分页）。
    pub(crate) fn crate_listing(
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
            "project": self.name,
            "crate": crate_name,
            "item_count": all.len(),
            "offset": offset,
            "limit": limit,
            "returned": items.len(),
            "truncated": offset + items.len() < all.len(),
            "items": items,
        })))
    }

    /// 解析 `rustdoc://{crate}/{item}` 形式的条目并返回其 markdown。
    pub(crate) fn render_uri_item(&self, crate_name: &str, item: &str) -> Result<String, McpError> {
        // 条目 id 形如 `crate::mod::…::kind.name`：URI 里模块分隔 `::` 写作 `/`，
        // 但 kind 与 name 之间的 `.` 必须保留。为容错，依次尝试几种归一化写法：
        //   1) 全部 `/` → `::`（标准写法，如 `inner/struct.Nested`）
        //   2) 最后一个 `/` 视作 kind 与 name 的分隔（如把 `struct.Demo` 误写成 `struct/Demo`）
        //   3) 全部 `/` → `.`
        let mut candidates = vec![format!("{crate_name}::{}", item.replace('/', "::"))];
        if let Some(pos) = item.rfind('/') {
            let head = item[..pos].replace('/', "::");
            let tail = &item[pos + 1..];
            let sep = if head.is_empty() { "" } else { "::" };
            candidates.push(format!("{crate_name}{sep}{head}.{tail}"));
        }
        candidates.push(format!("{crate_name}::{}", item.replace('/', ".")));
        candidates.dedup();

        let resolved = {
            let index = self.index();
            candidates
                .iter()
                .find_map(|wanted| index.find(wanted).map(|summary| summary.id.0.clone()))
        };
        let id = resolved.ok_or_else(|| {
            McpError::invalid_params(
                format!(
                    "未找到条目 `{crate_name}::{}`；可用写法：`rustdoc://{crate_name}/{{mod}}/{{Name}}`\
                     （如 `rustdoc://tokio/io/copy`）、带类型标记的 `rustdoc://{crate_name}/{{mod}}/{{kind}}.{{Name}}`\
                     （如 `rustdoc://tokio/io/fn.copy`），或先 `search_items` 拿到 id 再用 `get_item`。",
                    item.replace('/', "::"),
                ),
                None,
            )
        })?;
        self.get_item_blocking(ItemParams {
            id,
            max_bytes: None,
            project: None,
        })
        .map_err(|message| McpError::invalid_params(message, None))
    }
}

/// 工具与资源的同步实现（在 `spawn_blocking` 中执行，避免阻塞 async 执行器）。
impl Project {
    /// `list_items` 的同步实现。
    pub(crate) fn list_items_blocking(&self, params: ListItemsParams) -> Result<String, String> {
        let index = self.index();
        let kind = params.kind.as_deref().map(parse_kind).transpose()?;

        // 归一化 `module`：剥离前导 `::`，首段等于 crate 名时剥离 crate 前缀
        //（`gpui_kit::component` → `component`）。见 issues E-3.7。
        let module = params
            .module
            .as_deref()
            .map(|module| normalize_module(module, params.crate_name.as_deref()));
        let module = module.as_deref();

        let filtered: Vec<&ItemSummary> = index
            .items
            .iter()
            .filter(|item| matches_crate(item, params.crate_name.as_deref()))
            .filter(|item| matches_module(item, module))
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

        // 指定了非空 `module` 却零命中：多半是重导出别名或写错模块名（过滤时静默落空）。
        // 给一条可执行提示，避免把空结果误读成"不存在"（见 issues E-3.7）。
        let note = match (filtered.is_empty(), module) {
            (true, Some(module)) if !module.is_empty() => {
                module_miss_note(&index.items, params.crate_name.as_deref(), module)
            }
            _ => String::new(),
        };

        Ok(to_json(&serde_json::json!({
            "total": filtered.len(),
            "offset": offset,
            "returned": items.len(),
            "items": items,
            "note": note,
        })))
    }

    /// `search_items` 的同步实现。
    pub(crate) fn search_items_blocking(
        &self,
        params: SearchItemsParams,
    ) -> Result<SearchItemsResponse, String> {
        let index = self.index();
        let mut query = SearchQuery::new(params.query);
        query.crate_name = params.crate_name;
        query.limit = params.limit.unwrap_or(20);
        query.offset = params.offset.unwrap_or(0);
        query.mode = match params.mode {
            Some(ModeArg::Prefix) => mcp_docs_core::MatchMode::Prefix,
            Some(ModeArg::Fuzzy) => mcp_docs_core::MatchMode::Fuzzy,
            _ => mcp_docs_core::MatchMode::Substring,
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
    pub(crate) fn get_item_blocking(&self, params: ItemParams) -> Result<String, String> {
        let index = self.index();
        let Some(summary) = index.find(&params.id) else {
            // crate 首页不是索引条目（见 issues E-3.8）：id 恰为已索引 crate 名时，
            // 回退给一份合成概览，而不是直接报「未找到条目」。
            return match crate_overview(&index, &params.id) {
                Some(overview) => Ok(truncate(overview, params.max_bytes)),
                None => Err(format!("未找到条目 `{}`", params.id)),
            };
        };
        // 跨 crate 模块重导出别名（`pub use ::gpui_component as component;`）：条目
        // 指向**另一 crate 的首页**，本身不生成页面。委托该 crate 的合成概览渲染，
        // 否则会输出一份「无摘要、无文档」的 crate 首页 markdown。
        if let Some(target) = cross_crate_module_target(summary)
            && let Some(overview) = crate_overview(&index, target)
        {
            return Ok(truncate(overview, params.max_bytes));
        }
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
    pub(crate) fn get_item_source_blocking(&self, params: ItemParams) -> Result<String, String> {
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
    pub(crate) fn get_examples_blocking(
        &self,
        params: GetExamplesParams,
    ) -> Result<String, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;

        // 只从文档正文抽取示例，排除条目/成员的签名代码块（E-1.2）。
        let mut text = String::new();
        if summary.kind.is_member() {
            if let Some(member) = item.members.iter().find(|member| member.id == summary.id)
                && let Some(docs) = &member.docs_md
            {
                text.push_str(docs);
                text.push('\n');
            }
        } else {
            if let Some(docs) = &item.docs_md {
                text.push_str(docs);
                text.push_str("\n\n");
            }
            for member in &item.members {
                if let Some(docs) = &member.docs_md {
                    text.push_str(docs);
                    text.push_str("\n\n");
                }
            }
        }

        let max_examples = params.max_examples.unwrap_or(5);
        let examples: Vec<serde_json::Value> = extract_code_blocks(&text)
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
    pub(crate) fn get_item_section_blocking(
        &self,
        params: GetItemSectionParams,
    ) -> Result<String, String> {
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

        // 结构分节 `implementors` / `foreign-impls` 的正文是 impl 列表，不在 markdown 里，
        // `body_md` 天然为空（见 issues E-2.4）：改从 trait 页面抽出实现者签名填充。
        let mut body_md = rewrite_links(&section.body_md, options.link_style);
        if body_md.trim().is_empty()
            && matches!(section.id.as_str(), "implementors" | "foreign-impls")
        {
            body_md = trait_section_body(&self.doc_dir, &item.html_path, &section.id);
        }

        Ok(to_json(&serde_json::json!({
            "id": params.id,
            "section": section.id,
            "title": section.title,
            "body_md": body_md,
            "returned": members.len(),
            "members": members,
        })))
    }

    /// `batch_get_items` 的同步实现。
    pub(crate) fn batch_get_items_blocking(
        &self,
        params: BatchGetItemsParams,
    ) -> Result<String, String> {
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
    pub(crate) fn get_source_text_blocking(
        &self,
        params: GetSourceTextParams,
    ) -> Result<String, String> {
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
    pub(crate) fn get_item_json_blocking(
        &self,
        params: ItemParams,
    ) -> Result<ItemDetailOutput, String> {
        let index = self.index();
        let summary = index
            .find(&params.id)
            .ok_or_else(|| format!("未找到条目 `{}`", params.id))?;
        let item = self.load_item(summary)?;
        let options = RenderOptions::default();

        let sections: Vec<SectionOutput> = item
            .sections
            .iter()
            .map(|section| SectionOutput {
                id: section.id.clone(),
                title: section.title.clone(),
                body_md: rewrite_links(&section.body_md, options.link_style),
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
            docs_md: item
                .docs_md
                .as_deref()
                .map(|md| rewrite_links(md, options.link_style)),
            source,
            sections,
            members,
        })
    }

    /// `index_status` 的同步实现。
    pub(crate) fn index_status_blocking(&self) -> Result<IndexStatusOutput, String> {
        let building = self.is_building();
        let meta_path = self.out_dir.join("meta.json");
        let stale = is_stale(&self.doc_dir, &meta_path).unwrap_or(true);
        let index = self.index();
        Ok(IndexStatusOutput {
            project: self.name.clone(),
            ready: self.is_ready(),
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
    pub(crate) fn module_tree_blocking(
        &self,
        params: ModuleTreeParams,
    ) -> Result<ModuleTreeNode, String> {
        let index = self.index();
        let tree = module_tree(&index.items, &params.crate_name)
            .ok_or_else(|| format!("未找到 crate `{}`", params.crate_name))?;
        Ok(ModuleTreeNode::from(tree))
    }

    /// `get_related_items` 的同步实现。
    pub(crate) fn get_related_items_blocking(
        &self,
        params: RelatedItemsParams,
    ) -> Result<String, String> {
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
    ///
    /// 实现者分两处：**本 crate 与外来类型**的实现写在 trait 页面 HTML 里
    /// （`#implementors-list` / `#foreign-impls`），**跨 crate** 的在
    /// `trait.impl/**/trait.<Name>.js`。只读后者会漏掉本 crate 的实现（见 issues E-2.3）。
    pub(crate) fn get_trait_implementors_blocking(
        &self,
        params: ItemParams,
    ) -> Result<String, String> {
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
        let trait_crate = summary
            .id
            .0
            .split("::")
            .next()
            .unwrap_or_default()
            .to_string();

        let mut implementors: Vec<serde_json::Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut push = |crate_name: &str, text: String, source: &str| {
            if seen.insert(text.clone()) {
                implementors.push(serde_json::json!({
                    "crate": crate_name,
                    "impl": text,
                    "source": source,
                }));
            }
        };

        // 1) trait 页面：本 crate 的 `impl` 与对外来类型的 `impl`。
        if let Ok(html) = fs::read_to_string(self.doc_dir.join(&summary.html_path)) {
            let (in_crate, foreign) = parse_page_impls(&html);
            for text in in_crate {
                push(&trait_crate, text, "implementors");
            }
            for text in foreign {
                push(&trait_crate, text, "foreign-impls");
            }
        }

        // 2) trait.impl js：跨 crate 的实现（按 crate 分组）。
        let mut missing_manifest = true;
        for path in find_trait_impl_paths(&self.doc_dir, Path::new(&summary.html_path)) {
            missing_manifest = false;
            let js = fs::read_to_string(&path)
                .map_err(|err| format!("读取实现清单 `{}` 失败：{err}", path.display()))?;
            for imp in parse_trait_impls(&js) {
                push(&imp.crate_name, imp.text, "cross-crate");
            }
        }

        let note = if implementors.is_empty() && missing_manifest {
            "未找到实现清单文件（trait.impl/**/trait.<Name>.js），也未从 trait 页面解析到实现者"
        } else {
            ""
        };

        Ok(to_json(&serde_json::json!({
            "id": summary.id.0,
            "count": implementors.len(),
            "implementors": implementors,
            "note": note,
        })))
    }

    /// `find_by_signature` 的同步实现。
    pub(crate) fn find_by_signature_blocking(
        &self,
        params: FindBySignatureParams,
    ) -> Result<String, String> {
        let index = self.index();
        let needle = params.pattern.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(to_json(&serde_json::json!({
                "pattern": params.pattern,
                "total": 0,
                "returned": 0,
                "hits": [],
            })));
        }
        let kind = params.kind.as_deref().map(parse_kind).transpose()?;
        let limit = params.limit.unwrap_or(20);

        // 全量扫描以给出准确的 `total`，但只物化前 `limit` 条（见 issues E-3.13）。
        let mut total = 0usize;
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
                total += 1;
                if hits.len() < limit {
                    hits.push(summary_json(item));
                }
            }
        }
        Ok(to_json(&serde_json::json!({
            "pattern": params.pattern,
            "total": total,
            "returned": hits.len(),
            "hits": hits,
        })))
    }

    /// `search_docs` 的同步实现（方案 a：扫描已导出的 markdown 正文）。
    pub(crate) fn search_docs_blocking(&self, params: SearchDocsParams) -> Result<String, String> {
        let index = self.index();
        let needle = params.query.trim().to_lowercase();
        let offset = params.offset.unwrap_or(0);
        if needle.is_empty() {
            return Ok(to_json(&serde_json::json!({
                "query": params.query,
                "offset": offset,
                "returned": 0,
                "has_more": false,
                "hits": [],
            })));
        }
        let kind = params.kind.as_deref().map(parse_kind).transpose()?;
        let limit = params.limit.unwrap_or(20);

        let mut seen = std::collections::HashSet::new();
        let mut matched = 0usize;
        let mut hits = Vec::new();
        // 命中数达到 limit 就停：`has_more` 表示"可能还有更多"，供调用方翻页
        // （下一页 offset 用 `next_offset`）。见 issues E-3.13。
        let mut truncated = false;
        // 统计"应扫描的正文文件数"与"实际读到"的数量，用于区分"无匹配"与"正文缺失"。
        let mut candidates = 0usize;
        let mut read_ok = 0usize;
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
            candidates += 1;
            let Ok(text) = fs::read_to_string(self.out_dir.join(&item.file)) else {
                continue;
            };
            read_ok += 1;
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
                truncated = true;
                break;
            }
        }

        // 一个正文文件都读不到：多半是还没导出 markdown（server 内存构建不落盘 md）。
        // 此时返回空结果会让调用方误判为"无匹配"，故显式提示。
        if read_ok == 0 && candidates > 0 {
            return Err(format!(
                "未找到已导出的 markdown 正文（输出目录 {}）；请先运行 `mcp-docs export` 生成正文，\
                 或改用 search_items / get_item 检索索引内容。",
                self.out_dir.display()
            ));
        }

        let returned = hits.len();
        Ok(to_json(&serde_json::json!({
            "query": params.query,
            "offset": offset,
            "returned": returned,
            "has_more": truncated,
            "next_offset": offset + returned,
            "hits": hits,
        })))
    }
}

/// 尚无可用索引时的空占位。
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

/// 从 trait 页面抽取实现者签名，渲染成 `implementors` / `foreign-impls` 分节正文。
///
/// 这两类结构分节的正文是 impl 列表、不在条目 markdown 里，`body_md` 天然为空
/// （见 issues E-2.4）。
fn trait_section_body(doc_dir: &Path, html_path: &Path, section_id: &str) -> String {
    let Ok(html) = fs::read_to_string(doc_dir.join(html_path)) else {
        return String::new();
    };
    let (implementors, foreign) = parse_page_impls(&html);
    let signatures = if section_id == "implementors" {
        implementors
    } else {
        foreign
    };
    signatures
        .into_iter()
        .map(|signature| format!("```rust\n{signature}\n```"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 条目摘要的 JSON 表示。
pub(crate) fn summary_json(item: &ItemSummary) -> serde_json::Value {
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
pub(crate) fn kind_name(kind: ItemKind) -> String {
    kind_serde_name(kind)
}

/// 解析 `kind` 参数。
///
/// 无法识别的值直接报错，避免像早先那样静默忽略过滤器、误返回全部类型
/// （见 `docs/lessons.md` #1.23）。
pub(crate) fn parse_kind(value: &str) -> Result<ItemKind, String> {
    ItemKind::parse_input(value).ok_or_else(|| {
        format!(
            "无法识别的 kind：`{value}`（可用：struct / fn / method / field / trait / macro 等）"
        )
    })
}

/// 是否属于指定 crate（`None` 表示不限）。
pub(crate) fn matches_crate(item: &ItemSummary, crate_name: Option<&str>) -> bool {
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
pub(crate) fn matches_module(item: &ItemSummary, module: Option<&str>) -> bool {
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

/// 归一化 `module` 参数：剥离前导 `::`；首段等于 crate 名时剥离 crate 前缀。
///
/// 让 `module="gpui_kit::component"` 与 `module="component"` 等价（见 issues E-3.7）。
pub(crate) fn normalize_module(module: &str, crate_name: Option<&str>) -> String {
    let trimmed = module.trim().trim_start_matches("::");
    if let Some(crate_name) = crate_name
        && let Some(rest) = trimmed.strip_prefix(crate_name)
        && let Some(rest) = rest.strip_prefix("::")
    {
        return rest.to_string();
    }
    trimmed.to_string()
}

/// 指定 `module` 却零命中时的提示：列出该 crate 的一级模块，指引改用 `search_items`。
///
/// 重导出别名（`pub use X as m`）不是模块、不会进入条目的 `path`，过滤时静默落空，
/// 空结果与"确实无此模块"无法区分（见 issues E-3.7）。
fn module_miss_note(items: &[ItemSummary], crate_name: Option<&str>, module: &str) -> String {
    let Some(crate_name) = crate_name.filter(|name| !name.is_empty()) else {
        return format!(
            "没有条目命中 module=`{module}`；module 需为 crate 内的真实模块名（不带 crate 前缀），\
             重导出别名不是模块。可用 search_items 复核。"
        );
    };
    let modules = top_level_modules(items, crate_name);
    if modules.is_empty() {
        format!("crate `{crate_name}` 下没有条目命中 module=`{module}`。")
    } else {
        format!(
            "crate `{crate_name}` 下没有条目命中 module=`{module}`；可用一级模块：{}。\
             注意 module 需为 crate 内的真实模块名（不带 crate 前缀），重导出别名不是模块；\
             也可用 search_items 复核。",
            modules.join(", ")
        )
    }
}

/// ASCII 大小写不敏感的子串匹配（`needle` 需已小写）。
pub(crate) fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let hay = haystack.as_bytes();
    let target = needle.as_bytes();
    target.is_empty()
        || (target.len() <= hay.len()
            && hay
                .windows(target.len())
                .any(|window| window.eq_ignore_ascii_case(target)))
}

/// 取 `needle` 命中处前后各 40 字符的上下文片段（按字符边界）。
pub(crate) fn snippet_around(text: &str, needle: &str) -> Option<String> {
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
pub(crate) fn to_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|err| format!("序列化失败：{err}"))
}

/// 按字节上限截断文本，并附带提示。
///
/// 先退到 UTF-8 字符边界；若附近有行边界（`\n`）则对齐整行，避免把正文 / 示例
/// 从中间切断（见 issues E-3.10）。
pub(crate) fn truncate(text: String, max_bytes: Option<usize>) -> String {
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
    // 仅在回退幅度不大时对齐整行，避免为凑行边界而丢太多内容。
    if let Some(newline) = text[..end].rfind('\n')
        && newline > 0
        && end - newline <= 200
    {
        end = newline;
    }
    format!(
        "{}\n\n…（内容已按 max_bytes={limit} 截断；可用更大的 max_bytes 或 `get_item_section` 读取其余分节）",
        &text[..end]
    )
}
