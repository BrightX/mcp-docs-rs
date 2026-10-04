//! MCP 服务：把索引、检索与 markdown 渲染暴露为工具与资源。
//!
//! 服务可同时管理多个项目（多个仓库的 rustdoc 文档），每个项目一个 [`Project`]。
//! 设计原则是「先搜后读」：检索类工具只返回轻量摘要，正文由 `get_item`
//! 按需读取（内部走 `DocCache`，见 `docs/design.md` §7、§8）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

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
    tool, tool_handler, tool_router, ErrorData as McpError, Json, Peer, RoleServer, ServerHandler,
};
use tokio::sync::Mutex;

use crate::config::ProjectConfig;
use crate::project::{
    BatchGetItemsParams, FindBySignatureParams, GetExamplesParams, GetItemSectionParams,
    GetSourceTextParams, IndexStatusOutput, ItemDetailOutput, ItemParams, ListItemsParams,
    ModuleTreeNode, ModuleTreeParams, Project, ProjectParams, RebuildParams, RelatedItemsParams,
    SearchDocsParams, SearchItemsParams, SearchItemsResponse,
};

/// `list_projects` 返回的单个项目概要。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct ProjectInfo {
    name: String,
    doc_dir: String,
    out_dir: String,
    is_default: bool,
    ready: bool,
    building: bool,
    crate_count: usize,
    item_count: usize,
}

/// `list_projects` 的结构化返回。
#[derive(Debug, serde::Serialize, JsonSchema)]
struct ListProjectsResponse {
    projects: Vec<ProjectInfo>,
}

/// mcp-docs 的 MCP 服务（可同时服务多个项目）。
#[derive(Clone)]
pub struct DocsServer {
    /// 按 `order` 索引的项目表。
    projects: Arc<HashMap<String, Arc<Project>>>,
    /// 项目名的声明顺序（用于 `list_projects` 与错误提示）。
    order: Arc<Vec<String>>,
    /// 缺省项目名：工具/资源未显式给 `project` 时使用。
    default_project: Arc<String>,
    /// 客户端句柄（全服务共享，注入到各项目用于资源变更通知）。
    peer: Arc<OnceLock<Peer<RoleServer>>>,
}

impl DocsServer {
    /// 由项目清单构造服务。
    ///
    /// 缺省项目在构造时立即启动后台构建；其余项目「懒启动」，首次被
    /// [`DocsServer::resolve`] 访问时才启动，避免为未使用的项目白付构建成本。
    pub fn from_projects(
        specs: Vec<ProjectConfig>,
        default_project: String,
        store: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(!specs.is_empty(), "至少需要一个项目");

        let peer: Arc<OnceLock<Peer<RoleServer>>> = Arc::new(OnceLock::new());
        // 全服务共享的构建锁：串行化各项目的全量构建，避免并发抢占资源。
        let build_lock = Arc::new(Mutex::new(()));

        let mut projects = HashMap::new();
        let mut order = Vec::new();
        for spec in specs {
            anyhow::ensure!(
                !projects.contains_key(&spec.name),
                "项目名重复：`{}`",
                spec.name
            );
            let project = Project::new(
                spec.name.clone(),
                spec.doc_dir,
                spec.out_dir,
                store.clone(),
                Arc::clone(&peer),
                Arc::clone(&build_lock),
            );
            order.push(spec.name.clone());
            projects.insert(spec.name, project);
        }

        anyhow::ensure!(
            projects.contains_key(&default_project),
            "缺省项目 `{default_project}` 不存在"
        );

        let server = Self {
            projects: Arc::new(projects),
            order: Arc::new(order),
            default_project: Arc::new(default_project),
            peer,
        };

        // 缺省项目 eager 启动；其余项目首次访问才启动（见 resolve）。
        if let Some(project) = server.projects.get(server.default_project.as_str()) {
            project.start();
        }
        Ok(server)
    }

    /// 解析 `project` 参数为具体项目，并确保其后台构建已启动。
    ///
    /// `None` / 空串回落到缺省项目；未知项目名报错并列出可用项目。
    fn resolve(&self, project: Option<&str>) -> Result<Arc<Project>, String> {
        let name = project
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(self.default_project.as_str());
        let resolved = self
            .projects
            .get(name)
            .cloned()
            .ok_or_else(|| format!("未知项目 `{name}`；可用项目：{}", self.order.join(", ")))?;
        resolved.start();
        Ok(resolved)
    }

    /// 记录客户端句柄（首次请求时），供后续发通知。
    fn remember_peer(&self, peer: Peer<RoleServer>) {
        let _ = self.peer.set(peer);
    }
}

#[tool_router]
impl DocsServer {
    /// 列出已服务的项目及就绪状态。
    #[tool(description = "列出本服务当前服务的所有项目及其就绪状态")]
    async fn list_projects(&self) -> Json<ListProjectsResponse> {
        let projects = self
            .order
            .iter()
            .map(|name| {
                let project = &self.projects[name];
                let (crate_count, item_count) = project.counts();
                ProjectInfo {
                    name: name.clone(),
                    doc_dir: project.doc_dir().to_string_lossy().into_owned(),
                    out_dir: project.out_dir().to_string_lossy().into_owned(),
                    is_default: name == self.default_project.as_str(),
                    ready: project.is_ready(),
                    building: project.is_building(),
                    crate_count,
                    item_count,
                }
            })
            .collect();
        Json(ListProjectsResponse { projects })
    }

    /// 列出某个项目已索引的 crate。
    #[tool(description = "列出某个项目已索引的 crate 及其条目数")]
    async fn list_crates(
        &self,
        Parameters(params): Parameters<ProjectParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        Ok(project.list_crates_text())
    }

    /// 列出条目摘要（不含正文）。
    #[tool(description = "列出条目摘要（可按 crate / 模块 / 类型过滤），不含正文")]
    async fn list_items(
        &self,
        Parameters(params): Parameters<ListItemsParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.list_items_blocking(params))
            .await
            .map_err(|err| format!("列条目任务失败：{err}"))?
    }

    /// 检索条目。
    #[tool(description = "按名字 / 路径 / 摘要检索条目，返回轻量摘要与得分")]
    async fn search_items(
        &self,
        Parameters(params): Parameters<SearchItemsParams>,
    ) -> Result<Json<SearchItemsResponse>, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        let value = tokio::task::spawn_blocking(move || project.search_items_blocking(params))
            .await
            .map_err(|err| format!("检索任务失败：{err}"))??;
        Ok(Json(value))
    }

    /// 读取条目的完整 markdown。
    #[tool(description = "读取某个条目的完整 markdown 文档（签名 + 文档 + 成员）")]
    async fn get_item(&self, Parameters(params): Parameters<ItemParams>) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.get_item_blocking(params))
            .await
            .map_err(|err| format!("读取条目任务失败：{err}"))?
    }

    /// 查询条目的源码位置。
    #[tool(description = "查询条目对应的源码文件与行号")]
    async fn get_item_source(
        &self,
        Parameters(params): Parameters<ItemParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.get_item_source_blocking(params))
            .await
            .map_err(|err| format!("查询源码任务失败：{err}"))?
    }

    /// 抽取条目文档里的 rust 代码示例。
    #[tool(description = "抽取某个条目文档里的 rust 代码示例")]
    async fn get_examples(
        &self,
        Parameters(params): Parameters<GetExamplesParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.get_examples_blocking(params))
            .await
            .map_err(|err| format!("抽取示例任务失败：{err}"))?
    }

    /// 读取条目某个分节的 markdown。
    #[tool(description = "返回条目某个分节（如 examples / panics / implementations）的 markdown")]
    async fn get_item_section(
        &self,
        Parameters(params): Parameters<GetItemSectionParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.get_item_section_blocking(params))
            .await
            .map_err(|err| format!("读取分节任务失败：{err}"))?
    }

    /// 批量读取条目 markdown。
    #[tool(description = "批量读取多个条目的 markdown（ids 上限 20）")]
    async fn batch_get_items(
        &self,
        Parameters(params): Parameters<BatchGetItemsParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.batch_get_items_blocking(params))
            .await
            .map_err(|err| format!("批量读取任务失败：{err}"))?
    }

    /// 按源码位置读取源码文本。
    #[tool(description = "按源码位置读取条目对应的源码文本；取不到时回退为路径与行号")]
    async fn get_source_text(
        &self,
        Parameters(params): Parameters<GetSourceTextParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.get_source_text_blocking(params))
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
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        let value = tokio::task::spawn_blocking(move || project.get_item_json_blocking(params))
            .await
            .map_err(|err| format!("读取结构化条目任务失败：{err}"))??;
        Ok(Json(value))
    }

    /// 返回索引状态。
    #[tool(description = "返回某个项目的索引状态：就绪 / 构建中 / schema / 版本 / 计数 / 是否过期")]
    async fn index_status(
        &self,
        Parameters(params): Parameters<ProjectParams>,
    ) -> Result<Json<IndexStatusOutput>, String> {
        let project = self.resolve(params.project.as_deref())?;
        let value = tokio::task::spawn_blocking(move || project.index_status_blocking())
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
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        let value = tokio::task::spawn_blocking(move || project.module_tree_blocking(params))
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
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.get_related_items_blocking(params))
            .await
            .map_err(|err| format!("查询相关条目任务失败：{err}"))?
    }

    /// 列出 trait 的实现者。
    #[tool(description = "列出实现了某个 trait 的类型")]
    async fn get_trait_implementors(
        &self,
        Parameters(params): Parameters<ItemParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.get_trait_implementors_blocking(params))
            .await
            .map_err(|err| format!("查询 trait 实现者任务失败：{err}"))?
    }

    /// 按签名子串检索。
    #[tool(description = "按签名子串（如 `-> Result<`）检索条目")]
    async fn find_by_signature(
        &self,
        Parameters(params): Parameters<FindBySignatureParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.find_by_signature_blocking(params))
            .await
            .map_err(|err| format!("按签名检索任务失败：{err}"))?
    }

    /// 正文全文检索。
    #[tool(description = "在已导出的 markdown 正文里做全文检索")]
    async fn search_docs(
        &self,
        Parameters(params): Parameters<SearchDocsParams>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        project.ensure_ready().await;
        tokio::task::spawn_blocking(move || project.search_docs_blocking(params))
            .await
            .map_err(|err| format!("正文检索任务失败：{err}"))?
    }

    /// 重建索引。
    #[tool(description = "重新扫描产物目录并重建索引（默认增量）")]
    async fn rebuild_index(
        &self,
        Parameters(params): Parameters<RebuildParams>,
        peer: Peer<RoleServer>,
    ) -> Result<String, String> {
        let project = self.resolve(params.project.as_deref())?;
        self.remember_peer(peer);
        project.ensure_ready().await;
        project.rebuild(params.force.unwrap_or(false)).await
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
            "检索本地 rustdoc 文档（可服务多个项目）。先用 list_projects 查看项目，\
             再用 search_items / list_items 找到条目，最后用 get_item 读取 markdown；\
             多项目时用 `project` 参数指定项目。",
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
        let arguments = request.arguments.as_ref();
        let id = arguments
            .and_then(|args| args.get("id"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let project = arguments
            .and_then(|args| args.get("project"))
            .and_then(|value| value.as_str());
        let messages = build_prompt(&request.name, id, project)
            .map_err(|message| McpError::invalid_params(message, None))?;
        Ok(GetPromptResult::new(messages).into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        self.remember_peer(context.peer.clone());
        // 不逐 crate 列举、也不等待就绪，避免多项目下资源列表膨胀或阻塞。
        let mut resources =
            vec![Resource::new("rustdoc://crates", "crates")
                .with_description("缺省项目的全部 crate")];
        if self.order.len() > 1 {
            for name in self.order.iter() {
                resources.push(
                    Resource::new(
                        format!("rustdoc://crates?project={name}"),
                        format!("{name} 的 crate 清单"),
                    )
                    .with_description(format!("项目 {name} 的全部 crate")),
                );
            }
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
                    .with_description("某个条目的 markdown 文档（支持 `?project=NAME`）"),
                ResourceTemplate::new("rustdoc://{crate}", "crate").with_description(
                    "某个 crate 的条目列表（支持 `?project=NAME&offset=&limit=` 分页）",
                ),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        self.remember_peer(context.peer.clone());
        let text = self.read_uri(&request.uri).await?;
        let contents =
            ResourceContents::text(text, request.uri.clone()).with_mime_type("text/markdown");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

impl DocsServer {
    /// 读取一个 `rustdoc://` URI 对应的文本内容。
    async fn read_uri(&self, uri: &str) -> Result<String, McpError> {
        let path = uri
            .strip_prefix("rustdoc://")
            .ok_or_else(|| McpError::invalid_params(format!("非 rustdoc URI：{uri}"), None))?;

        // 支持 `?project=NAME&offset=&limit=`。
        let (path, query) = match path.split_once('?') {
            Some((path, query)) => (path, query),
            None => (path, ""),
        };

        let project = self
            .resolve(query_str(query, "project").as_deref())
            .map_err(|message| McpError::invalid_params(message, None))?;
        project.ensure_ready().await;

        if path == "crates" {
            return Ok(project.list_crates_text());
        }

        let (crate_name, item) = match path.split_once('/') {
            Some((crate_name, item)) => (crate_name, Some(item)),
            None => (path, None),
        };

        match item {
            // rustdoc://{crate}/{item}：item 的 `/` 对应 `::`。
            Some(item) => project.render_uri_item(crate_name, item),
            // rustdoc://{crate}：该 crate 的条目清单（可带 offset / limit）。
            None => project.crate_listing(
                crate_name,
                query_param(query, "offset").unwrap_or(0),
                query_param(query, "limit"),
            ),
        }
    }
}

/// 服务提供的 prompt 模板定义。
fn prompt_definitions() -> Vec<Prompt> {
    let id_argument = || {
        PromptArgument::new("id")
            .with_description("条目 id（如 tokio::spawn）")
            .with_required(true)
    };
    let project_argument =
        || PromptArgument::new("project").with_description("项目名（可选，缺省用默认项目）");
    vec![
        Prompt::new(
            "explain_api",
            Some("解释某个 API 的用途与用法"),
            Some(vec![id_argument(), project_argument()]),
        ),
        Prompt::new(
            "usage_example",
            Some("给出某个 API 的调用示例"),
            Some(vec![id_argument(), project_argument()]),
        ),
    ]
}

/// 按模板名与条目 id 构造 prompt 消息。
fn build_prompt(name: &str, id: &str, project: Option<&str>) -> Result<Vec<PromptMessage>, String> {
    if id.trim().is_empty() {
        return Err("缺少必需参数 `id`".to_string());
    }
    let project_hint = match project.map(str::trim).filter(|value| !value.is_empty()) {
        Some(project) => format!("（project = \"{project}\"）"),
        None => String::new(),
    };
    let text = match name {
        "explain_api" => format!(
            "请解释条目 `{id}`{project_hint} 的用途与用法：先调用 `get_item`（id = \"{id}\"）读取它的完整文档（签名 + 文档 + 成员），再结合签名与示例给出简明讲解。"
        ),
        "usage_example" => format!(
            "请给出条目 `{id}`{project_hint} 的调用示例：先调用 `get_item` 与 `get_examples`（id = \"{id}\"）获取文档与示例代码，再给出可直接运行的最小示例。"
        ),
        other => return Err(format!("未知的 prompt：`{other}`")),
    };
    Ok(vec![PromptMessage::new_text(Role::User, text)])
}

/// 从查询串里取一个 `usize` 参数，如 `offset=10&limit=5`。
fn query_param(query: &str, key: &str) -> Option<usize> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .and_then(|(_, value)| value.parse().ok())
}

/// 从查询串里取一个字符串参数，如 `project=tokio`。
fn query_str(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use mcp_docs_core::{build, BuildOptions};

    use super::*;

    /// fixture 根目录，等价于一个 `target/doc`。
    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mcp-docs-core/tests/fixtures/doc_probe")
    }

    /// 构造单项目服务（测试用，等价于旧的单项目语义）。
    fn single_project(doc_dir: PathBuf, out_dir: PathBuf) -> DocsServer {
        DocsServer::from_projects(
            vec![ProjectConfig {
                name: "default".to_string(),
                doc_dir,
                out_dir,
            }],
            "default".to_string(),
            None,
        )
        .unwrap()
    }

    /// 递归复制目录（测试用）。
    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// 取服务的缺省项目。
    fn default_project(server: &DocsServer) -> Arc<Project> {
        server.resolve(None).unwrap()
    }

    /// 冷启动不阻塞：`new` 立即返回（用空占位），后台构建完成后索引可用。
    #[tokio::test]
    async fn cold_start_builds_in_background() {
        let out = tempfile::tempdir().unwrap();
        let server = single_project(fixture_root(), out.path().to_path_buf());

        let project = default_project(&server);
        project.ensure_ready().await;
        let status = project.index_status_blocking().unwrap();
        assert_eq!(status.crate_count, 1, "后台应构建出 doc_probe 的索引");
        assert!(status.item_count > 0);
    }

    /// 已有索引时，就绪信号初始即为 true，工具无需等待。
    #[tokio::test]
    async fn existing_index_is_ready_immediately() {
        let out = tempfile::tempdir().unwrap();
        build(
            &fixture_root(),
            out.path(),
            &BuildOptions {
                persist: true,
                ..Default::default()
            },
        )
        .unwrap();

        let server = single_project(fixture_root(), out.path().to_path_buf());
        assert!(default_project(&server).is_ready(), "已有索引时应立即就绪");
    }

    /// 构造一个最小条目摘要，用于纯函数测试。
    fn summary(path: &[&str]) -> mcp_docs_core::ItemSummary {
        mcp_docs_core::ItemSummary {
            id: mcp_docs_core::ItemId("x".to_string()),
            kind: mcp_docs_core::ItemKind::Struct,
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
        use crate::project::matches_module;
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
        assert!(!matches_module(&summary(&["k"]), Some("k")));
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
        assert_eq!(
            query_str("project=a&limit=5", "project").as_deref(),
            Some("a")
        );
        assert_eq!(query_str("offset=1", "project"), None);
    }

    /// prompt 定义与消息构造。
    #[test]
    fn prompt_definitions_and_build() {
        let definitions = prompt_definitions();
        assert_eq!(definitions.len(), 2);
        assert!(definitions
            .iter()
            .any(|prompt| prompt.name == "explain_api"));

        let messages = build_prompt("explain_api", "doc_probe::struct.Demo", None).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);

        assert!(build_prompt("explain_api", "", None).is_err());
        assert!(build_prompt("unknown", "x", None).is_err());
    }

    /// 构建带索引与 markdown 的服务，供工具测试。
    fn server_with_index() -> (DocsServer, tempfile::TempDir) {
        let out = tempfile::tempdir().unwrap();
        build(
            &fixture_root(),
            out.path(),
            &BuildOptions {
                persist: true,
                write_markdown: true,
                ..Default::default()
            },
        )
        .unwrap();
        let server = single_project(fixture_root(), out.path().to_path_buf());
        (server, out)
    }

    /// `index_status` 暴露 schema / 计数 / 粒度。
    #[tokio::test]
    async fn index_status_reports_counts() {
        let (server, _out) = server_with_index();
        let project = default_project(&server);
        project.ensure_ready().await;
        let status = project.index_status_blocking().unwrap();
        assert_eq!(status.schema_version, mcp_docs_core::INDEX_SCHEMA_VERSION);
        assert!(status.item_count > 0);
        assert_eq!(status.granularity, "member");
        assert!(status.ready);
        // 已有索引且不 stale 时后台刷新为 no-op，不应标记 building（E-3.1）。
        assert!(!status.building);
        assert_eq!(status.project, "default");
    }

    /// `batch_get_items` 对 ids 数量设上限。
    #[tokio::test]
    async fn batch_get_items_caps_ids() {
        let (server, _out) = server_with_index();
        let project = default_project(&server);
        project.ensure_ready().await;
        let ids = vec!["doc_probe::struct.Demo".to_string(); 25];
        let text = project
            .batch_get_items_blocking(BatchGetItemsParams {
                ids,
                max_bytes_each: None,
                project: None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["returned"], 20);
        assert_eq!(value["truncated"], true);
    }

    /// `get_examples` 至少返回文档里的代码块。
    #[tokio::test]
    async fn get_examples_returns_code() {
        let (server, _out) = server_with_index();
        let project = default_project(&server);
        project.ensure_ready().await;
        let text = project
            .get_examples_blocking(GetExamplesParams {
                id: "doc_probe::struct.Demo".to_string(),
                max_examples: None,
                max_bytes: None,
                project: None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value["returned"].as_u64().unwrap() >= 1);
        let first = value["examples"][0]["code"].as_str().unwrap();
        assert!(first.contains("doc_probe::Demo"), "示例内容：{first}");
        assert!(
            !first.starts_with("pub struct"),
            "签名不应作为示例：{first}"
        );
    }

    /// `find_by_signature` 能按签名子串命中。
    #[tokio::test]
    async fn find_by_signature_matches() {
        let (server, _out) = server_with_index();
        let project = default_project(&server);
        project.ensure_ready().await;
        let text = project
            .find_by_signature_blocking(FindBySignatureParams {
                pattern: "struct Demo".to_string(),
                crate_name: None,
                kind: None,
                limit: None,
                project: None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value["returned"].as_u64().unwrap() >= 1);
    }

    /// 多项目：解析、缺省选择、懒启动与 `list_projects`。
    #[tokio::test]
    async fn multi_project_resolve_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        let doc_a = tmp.path().join("a/doc");
        let doc_b = tmp.path().join("b/doc");
        copy_dir(&fixture_root(), &doc_a);
        copy_dir(&fixture_root(), &doc_b);

        let server = DocsServer::from_projects(
            vec![
                ProjectConfig {
                    name: "a".to_string(),
                    doc_dir: doc_a,
                    out_dir: tmp.path().join("a/out"),
                },
                ProjectConfig {
                    name: "b".to_string(),
                    doc_dir: doc_b,
                    out_dir: tmp.path().join("b/out"),
                },
            ],
            "a".to_string(),
            None,
        )
        .unwrap();

        // 缺省与显式解析。
        assert_eq!(server.resolve(None).unwrap().name, "a");
        assert_eq!(server.resolve(Some("b")).unwrap().name, "b");
        assert!(server.resolve(Some("nope")).is_err(), "未知项目应报错");

        // list_projects 返回全部项目，且标记缺省。
        let Json(list) = server.list_projects().await;
        assert_eq!(list.projects.len(), 2);
        assert!(
            list.projects
                .iter()
                .find(|project| project.name == "a")
                .unwrap()
                .is_default
        );

        // 懒启动：非缺省项目被 resolve 后也会构建并就绪。
        server.resolve(Some("b")).unwrap().ensure_ready().await;
        assert!(server.resolve(Some("b")).unwrap().is_ready());
    }

    /// 资源 URI 的 `?project=` 选择：缺省回落、显式指定、未知项目报错。
    #[tokio::test]
    async fn resource_query_selects_project() {
        let tmp = tempfile::tempdir().unwrap();
        let doc_a = tmp.path().join("a/doc");
        let doc_b = tmp.path().join("b/doc");
        copy_dir(&fixture_root(), &doc_a);
        copy_dir(&fixture_root(), &doc_b);

        let server = DocsServer::from_projects(
            vec![
                ProjectConfig {
                    name: "a".to_string(),
                    doc_dir: doc_a,
                    out_dir: tmp.path().join("a/out"),
                },
                ProjectConfig {
                    name: "b".to_string(),
                    doc_dir: doc_b,
                    out_dir: tmp.path().join("b/out"),
                },
            ],
            "a".to_string(),
            None,
        )
        .unwrap();

        // 缺省项目：不带 query 落到 a。
        let crates = server.read_uri("rustdoc://crates").await.unwrap();
        assert!(crates.contains("doc_probe"));
        assert!(crates.contains("\"project\":\"a\""));

        // 显式指定 project=b。
        let crates_b = server.read_uri("rustdoc://crates?project=b").await.unwrap();
        assert!(crates_b.contains("\"project\":\"b\""));

        // 条目资源带 project。
        let markdown = server
            .read_uri("rustdoc://doc_probe/Demo?project=b")
            .await
            .unwrap();
        assert!(markdown.contains("Demo"));

        // 未知项目报错。
        assert!(server
            .read_uri("rustdoc://crates?project=nope")
            .await
            .is_err());
    }

    /// 资源条目 id 容错：`struct/Demo`（把 kind 与 name 间的点误写成斜杠）也能命中。
    #[tokio::test]
    async fn resource_item_tolerates_kind_slash() {
        let (server, _out) = server_with_index();

        // 误写成 `struct/Demo` 也应命中（容错）。
        let tolerant = server
            .read_uri("rustdoc://doc_probe/struct/Demo")
            .await
            .unwrap();
        assert!(tolerant.contains("Demo"));

        // 标准写法（保留点号）仍可用。
        let standard = server
            .read_uri("rustdoc://doc_probe/struct.Demo")
            .await
            .unwrap();
        assert!(standard.contains("Demo"));

        // 模块分隔的斜杠写法仍可用。
        let nested = server
            .read_uri("rustdoc://doc_probe/inner/struct.Nested")
            .await
            .unwrap();
        assert!(nested.contains("Nested"));
    }

    /// `search_docs` 在未导出 markdown 时给出明确提示，而非静默空结果（P1）。
    #[tokio::test]
    async fn search_docs_hints_when_no_markdown() {
        let out = tempfile::tempdir().unwrap();
        // 内存构建：只写索引，不写 markdown。
        build(
            &fixture_root(),
            out.path(),
            &BuildOptions {
                persist: true,
                write_markdown: false,
                ..Default::default()
            },
        )
        .unwrap();
        let server = single_project(fixture_root(), out.path().to_path_buf());
        let project = default_project(&server);
        project.ensure_ready().await;

        let err = project
            .search_docs_blocking(SearchDocsParams {
                query: "Demo".to_string(),
                crate_name: None,
                kind: None,
                limit: None,
                offset: None,
                project: None,
            })
            .unwrap_err();
        assert!(err.contains("export"), "应提示先导出正文：{err}");
    }
}
