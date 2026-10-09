# 需求文档

## 背景

AI Agent 在编写 Rust 代码时，经常需要了解某个 crate 的最新用法。当前可行的手段是把 `cargo doc` 生成的 HTML 文档交给 Agent，但存在明显缺口：

- rustdoc HTML 混杂大量 UI 元素（导航、按钮、内联 CSS/JS）和噪声区块（`Auto Trait Implementations`、`Blanket Implementations`），不适合 Agent 直接阅读。
- 已有工具 `rustdoc-text`、`rustdoc-md` 都是**整包转一个大 markdown 或输出到终端**，没有检索能力。大 crate（tokio / serde / reqwest）转出来可达几十 MB，Agent 无法整读，必然超出上下文。

结论：需要一个**可检索**的、面向 Agent 的本地 crate 文档访问工具。

## 目标

解析本地 `cargo doc` 生成的 rustdoc HTML，**按条目切分**为 markdown 文件并建立全局索引，通过 **MCP Server**（Agent 先搜后读）与 **CLI**（批量导出）两种形态交付，使 Agent 能用最小上下文拿到所需 API 的用法。

## 用户场景

**场景 1：Agent 查某个 API 的用法**
Agent 需要知道 `tokio::spawn` 的签名与用法 →
`search_items("spawn", crate="tokio")` 得到若干轻量摘要 →
`get_item("tokio::spawn")` 拿到该条目的完整 markdown（签名 + 文档 + 示例）。

**场景 2：Agent 探索一个陌生 crate**
`list_crates` 找到目标 crate → `list_items(crate="X", kinds=["struct"])` 浏览接口 →
对感兴趣的条目逐个 `get_item`。

**场景 3：开发者批量导出**
开发者在脚本或编辑器里跑 `mcp-docs export`，把 `target/doc` 转成 `target/doc-search/` 下的 markdown 树 + `index.json.gz`，供 Grep / Read 等工具直接检索。

## 功能需求

| 编号 | 需求 | 说明 |
|---|---|---|
| F1 | 发现 crates 与条目 | 从 `crates.js` 与各层 `sidebar-items.js` 递归构建条目清单，`all.html` 与 crate 首页 `#reexports` 作兜底与交叉校验（后者补出 crate 内部重导出与跨 crate 模块重导出别名） |
| F2 | 解析单个条目 | 提取标题、签名（`pre.rust.item-decl`）、主文档、各分节、方法/字段/变体/关联项等成员 |
| F3 | 成员条目化 | 方法、变体、字段等成员成为可检索的独立条目（id 形如 `crate::Type::method`） |
| F4 | 渲染 markdown | 剥离 rustdoc UI 与噪声区块；代码示例用 ```rust 围栏；重写相对链接 |
| F5 | 建立索引 | 生成扁平 `index.json.gz`，含每个条目的 id/kind/path/一行摘要/文件位置 |
| F6 | 检索 | 支持按名字、路径、描述的子串 / 前缀 / 模糊匹配，可按 crate、kind 过滤与排序 |
| F7 | 按需读取 | 只解析并返回被请求的条目，避免全量加载 |
| F8 | CLI 导出 | `export` / `tree` / `show` / `search` 子命令 |
| F9 | MCP Server | 暴露工具与资源，stdio 传输 |
| F10 | 增量与缓存 | 指纹判定是否需要重建；`--incremental` 时仅更新变化的条目 |
| F11 | 单进程多项目 | 一个 `mcp-docs-server` 进程服务多个项目（多个 rustdoc 产物目录）；工具 / 资源用可选 `project` 参数定位项目，缺省用默认项目 |
| F12 | 跨项目共享索引库 | 多个项目依赖同一 crate（如同一版本 tokio）时，其条目摘要与 markdown 只解析渲染一次，存入共享库被各项目复用 |

## 非功能需求

- **上下文友好**：搜索只返回轻摘要 + 指针，正文按需读取 —— 这是核心诉求，优先于其他一切。
- **性能**：大 crate（数千 html）的索引构建应在秒级；正文按需解析带缓存。
- **跨平台**：Windows / macOS / Linux 一致；文件名编码必须 Windows 安全。
- **健壮性**：解析失败降级而非 panic；容忍畸形 / 截断 HTML。
- **可测试**：以真实 rustdoc 产物作为 fixture，解析行为有 golden 快照。

## 约束

| 约束 | 说明 |
|---|---|
| 数据源 | 解析 `cargo doc` 生成的 HTML。**不用 rustdoc JSON**，因其至今 unstable，需 nightly 或 `RUSTC_BOOTSTRAP=1` |
| 文档来源 | 仅本地 `target/doc`，**不联网**（不接 docs.rs） |
| 索引范围 | 默认覆盖 `crates.js` 列出的**全部 crate（含所有依赖）**，跟随 `cargo doc` 的默认行为；查询与导出阶段可按 crate 过滤 |
| 工程结构 | 3-crate workspace：`mcp-docs-core`（纯库）/ `mcp-docs-cli` / `mcp-docs-server` |
| 检索粒度 | 按条目分文件 + 全局索引；成员默认独立成文件**且**内联进父文件；`--granularity=item` 时成员仅内联、不落盘 |
| 输出位置 | 可配置，默认项目内 `target/doc-search/`；CLI `--out` 与 server 启动参数可覆盖 |
| 共享库位置 | 跨项目复用的索引库默认位于平台缓存目录（可用 `MCP_DOCS_STORE` 覆盖）；键为「crate 名 + 版本 + rustdoc 版本 + 粒度 + stat 指纹」 |
| MCP SDK | `rmcp`（官方 Rust SDK，当前 3.5.0），锁精确版本 |
| 文档管理 | 需求 / 设计 / 进度文档统一放在 `docs/` 下 |

## 非目标（本期不做）

- 不解析 rustdoc JSON，不依赖 nightly / bootstrap。
- 不联网抓取 docs.rs。
- 不提供全文检索引擎（如 tantivy）—— 先靠扁平索引 + 子串/前缀/模糊匹配满足需求。
- 不做跨项目检索（一次查询覆盖所有项目的合并结果）—— 每个工具默认作用于单个项目。
- 不做 RAG / 向量检索 —— 目标是对 API 条目做符号定位，词法匹配已足够。
- 不做源码符号跳转/LSP 级能力（`get_item_source` 仅给源码位置）。
- 不生成 HTML 站点或文档网站。
