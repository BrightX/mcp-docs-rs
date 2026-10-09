# 术语与代码位置对照

记录沟通过程中出现的术语 / 别名及其对应的代码位置，便于快速定位。

**编号格式 `#x.y`**：`x` = 章节号，`y` = 条目序号。新增术语追加到所属章节末尾，编号只追加、不复用。

**维护责任**：每当用户使用新术语 / 别名，或代码文件新增 / 改名后，同步更新本文件。标注「（规划）」的位置在代码落地后改为实际路径。

## 1. 核心概念

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #1.1 | 条目、文档条目 | 一个可落盘的文档单元（struct / trait / fn / 方法…） | `crates/mcp-docs-core/src/model.rs` → `DocItem`；解析 `parse.rs::parse_item_html` |
| #1.2 | 成员、成员条目 | 挂在父条目下的子条目：方法 / 字段 / 变体 / 关联项 | `model.rs` → `DocItem::members`；收集 `parse.rs::collect_members` |
| #1.3 | id、ItemId | 条目的稳定唯一标识，形如 `doc_probe::Demo::new` | `crates/mcp-docs-core/src/model.rs` → `ItemId` |
| #1.4 | 一行摘要、one_line | 条目简介，取自 `<meta name=description>` | `parse.rs::parse_one_line`；写入 `index.rs` |
| #1.5 | 发现、discover | 从产物目录构建条目清单的过程 | `crates/mcp-docs-core/src/discover.rs` |
| #1.6 | 索引、index.json | 全量条目的扁平清单，Agent 首读 | `crates/mcp-docs-core/src/index.rs` |
| #1.7 | 指纹、fingerprint | 判断产物是否变化、是否需要重建 | `crates/mcp-docs-core/src/cache.rs` → `Fingerprint` |
| #1.8 | 先搜后读 | 设计原则：搜索只返回轻摘要 + 指针，正文按需读取 | `design.md` §8 |
| #1.9 | DiscoveredItem | 发现阶段产出的条目（尚未解析正文） | `crates/mcp-docs-core/src/model.rs` → `DiscoveredItem` |
| #1.10 | 分节、Section | 页面按 `h2.section-header` 切分的区块 | `model.rs` → `Section`；切分 `parse.rs::collect_sections` |
| #1.11 | ParseOptions | 解析开关（含 `include_auto_impls` 是否保留噪声 impl） | `parse.rs` → `ParseOptions` |
| #1.12 | 源码位置、SourceRef | 条目对应的源码文件与行号 | `model.rs` → `SourceRef`；解析 `parse.rs::parse_source_href` |
| #1.13 | 条目摘要、ItemSummary | 索引里的轻量条目（不含正文） | `crates/mcp-docs-core/src/model.rs` → `ItemSummary` |
| #1.14 | 检索、search | 在索引中按名字 / 路径 / 摘要检索并排序 | `crates/mcp-docs-core/src/search.rs` → `search` / `rank` |
| #1.15 | MatchMode | 匹配模式：子串 / 前缀 / 模糊子序列 | `search.rs` → `MatchMode` |
| #1.16 | 增量、incremental | 复用未变化条目，只重建改动的部分 | `crates/mcp-docs-core/src/index.rs` → `BuildOptions::incremental` |
| #1.17 | Meta、meta.json | 指纹 + 版本 + 生成时间，用于判断是否重建 | `crates/mcp-docs-core/src/cache.rs` → `Meta` / `is_stale` |
| #1.18 | DocCache | 按需解析结果的内存缓存（给 MCP server 用） | `crates/mcp-docs-core/src/cache.rs` → `DocCache` |
| #1.19 | MCP 工具、tool | 暴露给 Agent 的检索能力 | `crates/mcp-docs-server/src/server.rs`（`#[tool]`） |
| #1.20 | MCP 资源、resource | 以 `rustdoc://` URI 暴露的文档节点 | `server.rs`（`read_resource` / `list_resource_templates`） |
| #1.21 | 成员文件名 | `{父stem}.{成员类型}.{成员名}.md`（类型用于避免同名冲突） | `store.rs::member_output_path` |
| #1.22 | 共享库、store | 跨项目复用的 crate 索引库（条目摘要 + md） | `crates/mcp-docs-core/src/shared.rs`；默认根 `shared::default_store_root` |
| #1.23 | 身份键、key | 判断「同一份 crate 文档」的键：名 + 版本 + rustdoc 版本 + 粒度 + stat 指纹 | `shared.rs::key_dir_name`；指纹 `shared.rs::CrateStat` / `crate_scan` |
| #1.24 | 物化、materialize | 把共享库里的 md 硬链接（同卷）/ 复制（跨卷）到项目目录 | `shared.rs::{materialize_file, materialize_crate}` |
| #1.25 | CratePlan、共享库规划 | 单 crate 的身份键 / 落盘位置 / 命中结果 | `shared.rs::CratePlan`；`shared.rs::plan` / `write_entry` |
| #1.26 | 共享库命中 | 构建时某 crate 命中共享库、整体复用（不重新解析） | `shared.rs::CratePlan::is_hit`；`index.rs`（`shared_hits`） |

## 2. rustdoc 产物（输入数据）

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #2.1 | `sidebar-items.js` | 每个模块目录一份的纯 JSON 条目清单，建索引入口 | `crates/mcp-docs-core/src/sidebar.rs` |
| #2.2 | `crates.js` | crate 列表（`window.ALL_CRATES`） | `crates/mcp-docs-core/src/discover.rs::list_crates` |
| #2.3 | `all.html` | 全部条目的扁平清单页，发现阶段的交叉校验兜底 | `allpage.rs::parse_all_str` |
| #2.4 | 签名、item-decl | `<pre class="rust item-decl">` 里的条目声明 | `parse.rs` → `DocItem::signature` |
| #2.5 | docblock | 文档正文区块；主文档在 `details.toggle.top-doc` 内 | `parse.rs::docblock_to_md` |
| #2.6 | 噪声区块 | `#synthetic-implementations` / `#blanket-implementations`，默认剥离 | `parse.rs::ParseOptions` / `is_noise_section` |
| #2.7 | `search.index` | rustdoc 定制二进制压缩索引，**明确不使用** | 见 `lessons.md` #1.1 |
| #2.8 | 实测事实底座 | design.md 中经真实产物核对的结构事实 | `design.md` §1 |
| #2.9 | `rustdoc-vars` | 页面头部 meta，含 `data-current-crate` / `data-rustdoc-version` | `parse.rs::parse_rustdoc_meta` |
| #2.10 | 宏条目二元组 | rustdoc 1.98+ 在 sidebar 里把宏写成 `[名字, 标志]` | `sidebar.rs::sidebar_entry_name`（见 `lessons.md` #1.24） |
| #2.11 | 导出粒度、Granularity | 成员是否单独落盘（`member` 默认 / `item` 仅内联） | `model.rs::Granularity`；`index.rs::BuildOptions` |
| #2.12 | `#reexports`、重导出区块 | crate 首页列出**内部重导出别名**的区块 | `crates/mcp-docs-core/src/reexport.rs::parse_reexports_str` |
| #2.13 | 别名条目、重导出别名 | crate 内部重导出补出的条目（id 用别名、`html_path` 指向目标页） | `reexport.rs`；`index.rs::process_entry`（`is_alias`） |

## 3. 代码位置对照

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #3.1 | core 库、核心库 | 纯库，零 async，解析与渲染全部在此 | `crates/mcp-docs-core/src/` |
| #3.2 | CLI | 命令行导出 / 查询工具 | `crates/mcp-docs-cli/src/main.rs` |
| #3.3 | server、MCP server | rmcp + tokio 的 MCP 服务（stdio） | `crates/mcp-docs-server/src/`（`main.rs` + `server.rs`） |
| #3.4 | 输出目录、doc-search | 落盘根，默认 `target/doc-search/` | `crates/mcp-docs-core/src/store.rs`；CLI 全局 `--out` |
| #3.5 | fixture | 实测产物裁剪副本，用于测试 | `crates/mcp-docs-core/tests/fixtures/doc_probe/` |
| #3.6 | 错题集 | 踩坑与错误记录 | `docs/lessons.md` |
| #3.7 | 开发规范 | 代码质量 / 风格 / 注释 / 提交规范 | `docs/conventions.md` |
| #3.8 | 渲染、render | 把条目转成 markdown | `crates/mcp-docs-core/src/markdown.rs` |
| #3.9 | 链接重写、LinkStyle | 链接输出风格（相对路径 / 纯文本 / 原样） | `crates/mcp-docs-core/src/markdown.rs`；归类 `link.rs` |
| #3.10 | 导出、export | 批量落盘 markdown 文件树 | CLI `mcp-docs export`；`store.rs` |
| #3.11 | 索引模块 | 构建 / 读写 `index.json.gz` | `crates/mcp-docs-core/src/index.rs` |
| #3.12 | 检索模块 | 检索与排序 | `crates/mcp-docs-core/src/search.rs` |
| #3.13 | 缓存模块 | 指纹、meta、解析缓存 | `crates/mcp-docs-core/src/cache.rs` |
| #3.14 | server 模块 | MCP 工具与资源实现 | `crates/mcp-docs-server/src/server.rs` |
| #3.15 | kind 参数、类型过滤 | 条目类型名的用户输入解析（前缀 / 复数 / 自然名） | `model.rs::ItemKind::parse_input`；`server.rs::parse_kind` |
| #3.16 | 源码位置、source | 条目对应的源码文件与行号 | `parse.rs::parse_source_href`；MCP `get_item_source` |
| #3.17 | 条目查找、IdIndex | 按 id 找条目（精确优先，允许省略类型标记）的预建查找表 | `lookup.rs::IdIndex`；`server.rs::Loaded::find` |
| #3.18 | 一行摘要、one_line | 索引里的简要描述，仅在有真实文档时生成 | `index.rs`（见 `lessons.md` #1.25） |
| #3.19 | 分页、offset | 检索 / 列表跳过的命中数；`total` 为分页前总数 | `search.rs::SearchQuery::offset`、`search_page` |
| #3.20 | allpage 模块 | 解析 `all.html`，补全 sidebar 遗漏的条目 | `crates/mcp-docs-core/src/allpage.rs` |
| #3.21 | 检索片段、snippet | 结果页条目摘要命中时的 ±40 字符上下文 | `search.rs::SearchHit::snippet` |
| #3.22 | 并行构建 | rayon 并行解析 / 渲染 / 落盘，串行汇总计数 | `index.rs::build` |
| #3.23 | 解析缓存、DocCache | 按需解析结果缓存，带容量上限与近似 LRU | `cache.rs::DocCache` |
| #3.24 | 模块树、module_tree | 由 `path` + `Module` 构建的嵌套树（每级条目数） | `nav.rs::module_tree` |
| #3.25 | 相关条目、related_items | 父条目 / 同模块兄弟 / 子成员 | `nav.rs::related_items` |
| #3.26 | trait 实现者 | 解析 `trait.impl/.../trait.*.js` | `nav.rs::parse_trait_impls`、`trait_impl_rel_path` |
| #3.27 | 源码文本、get_source_text | 按行号从 `src/*.rs.html` 切片 | `parse.rs::extract_source_lines` |
| #3.28 | 示例抽取、get_examples | 抽取 markdown 里的 ```rust 围栏块 | `markdown.rs::extract_code_blocks` |
| #3.29 | 签名、signature | 索引里的声明签名，供 find_by_signature | `model.rs::ItemSummary::signature` |
| #3.30 | 结构化输出 | 工具返回 `outputSchema` + `structured_content` | `rmcp::Json<T>`；`server.rs::{SummaryOutput,ItemDetailOutput,IndexStatusOutput,ModuleTreeNode}` |
| #3.31 | prompts 模板 | 可复用提示模板 | `server.rs::{prompt_definitions,build_prompt}`；`list_prompts` / `get_prompt` |
| #3.32 | 资源变更通知 | 索引变化后通知客户端刷新资源清单 | `server.rs::{remember_peer,notify_resources_changed}` |
| #3.33 | 项目、Project | 单个项目的运行时状态（索引 / 缓存 / 就绪 / 懒启动） | `crates/mcp-docs-server/src/project.rs::Project` |
| #3.34 | 项目解析、resolve | 由 `project` 名（或缺省）找到项目并确保启动 | `server.rs::DocsServer::resolve` |
| #3.35 | 项目清单、list_projects | 列出全部项目及就绪状态（不阻塞） | `server.rs::list_projects` |
| #3.36 | 启动配置 | 多项目启动配置解析（`MCP_DOCS_PROJECTS` / `MCP_DOCS_PROJECTS_FILE` / `MCP_DOCS_STORE` 等环境变量） | `crates/mcp-docs-server/src/config.rs::resolve_config` |
| #3.37 | 共享构建锁 | 串行化各项目全量构建，避免并发抢占资源 | `server.rs::from_projects`（`build_lock`）；`project.rs::refresh_index` |
| #3.38 | 落盘编码 | 索引类文件（`index.json.gz` / `items.json.gz`）落盘为 JSON + gzip；派生字段不落盘、加载后重建 | `crates/mcp-docs-core/src/codec.rs`；`model.rs::ItemSummary::rebuild_derived` |
| #3.39 | reexport 模块 | 解析 crate 首页 `#reexports`，补出内部重导出别名条目 | `crates/mcp-docs-core/src/reexport.rs`；接入 `discover.rs::discover_crate` |
