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

## 2. rustdoc 产物（输入数据）

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #2.1 | `sidebar-items.js` | 每个模块目录一份的纯 JSON 条目清单，建索引入口 | `crates/mcp-docs-core/src/sidebar.rs` |
| #2.2 | `crates.js` | crate 列表（`window.ALL_CRATES`） | `crates/mcp-docs-core/src/discover.rs::list_crates` |
| #2.3 | `all.html` | 全部条目的扁平清单页，发现兜底与交叉校验 | `discover.rs`（规划，M6 按需增强） |
| #2.4 | 签名、item-decl | `<pre class="rust item-decl">` 里的条目声明 | `parse.rs` → `DocItem::signature` |
| #2.5 | docblock | 文档正文区块；主文档在 `details.toggle.top-doc` 内 | `parse.rs::docblock_to_md` |
| #2.6 | 噪声区块 | `#synthetic-implementations` / `#blanket-implementations`，默认剥离 | `parse.rs::ParseOptions` / `is_noise_section` |
| #2.7 | `search.index` | rustdoc 定制二进制压缩索引，**明确不使用** | 见 `lessons.md` #1.1 |
| #2.8 | 实测事实底座 | design.md 中经真实产物核对的结构事实 | `design.md` §1 |
| #2.9 | `rustdoc-vars` | 页面头部 meta，含 `data-current-crate` / `data-rustdoc-version` | `parse.rs::parse_rustdoc_meta` |

## 3. 代码位置对照

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #3.1 | core 库、核心库 | 纯库，零 async，解析与渲染全部在此 | `crates/mcp-docs-core/src/` |
| #3.2 | CLI | 命令行导出 / 查询工具 | `crates/mcp-docs-cli/src/main.rs` |
| #3.3 | server、MCP server | rmcp + tokio 的 MCP 服务 | `crates/mcp-docs-server/src/`（规划，M5） |
| #3.4 | 输出目录、doc-search | 落盘根，默认 `target/doc-search/` | `crates/mcp-docs-core/src/store.rs`；CLI 全局 `--out` |
| #3.5 | fixture | 实测产物裁剪副本，用于测试 | `crates/mcp-docs-core/tests/fixtures/doc_probe/` |
| #3.6 | 错题集 | 踩坑与错误记录 | `docs/lessons.md` |
| #3.7 | 开发规范 | 代码质量 / 风格 / 注释 / 提交规范 | `docs/conventions.md` |
| #3.8 | 渲染、render | 把条目转成 markdown | `crates/mcp-docs-core/src/markdown.rs` |
| #3.9 | 链接重写、LinkStyle | 链接输出风格（相对路径 / 纯文本 / 原样） | `crates/mcp-docs-core/src/markdown.rs`；归类 `link.rs` |
| #3.10 | 导出、export | 批量落盘 markdown 文件树 | CLI `mcp-docs export`；`store.rs` |
| #3.11 | 索引模块 | 构建 / 读写 `index.json` | `crates/mcp-docs-core/src/index.rs` |
| #3.12 | 检索模块 | 检索与排序 | `crates/mcp-docs-core/src/search.rs` |
| #3.13 | 缓存模块 | 指纹、meta、解析缓存 | `crates/mcp-docs-core/src/cache.rs` |
