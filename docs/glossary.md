# 术语与代码位置对照

记录沟通过程中出现的术语 / 别名及其对应的代码位置，便于快速定位。

**编号格式 `#x.y`**：`x` = 章节号，`y` = 条目序号。新增术语追加到所属章节末尾，编号只追加、不复用。

**维护责任**：每当用户使用新术语 / 别名，或代码文件新增 / 改名后，同步更新本文件。标注「（规划）」的位置在代码落地后改为实际路径。

## 1. 核心概念

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #1.1 | 条目、文档条目 | 一个可落盘的文档单元（struct / trait / fn / 方法…） | `crates/mcp-docs-core/src/model.rs` → `DocItem`（规划） |
| #1.2 | 成员、成员条目 | 挂在父条目下的子条目：方法 / 字段 / 变体 / 关联项 | `model.rs` → `DocItem::members`（规划） |
| #1.3 | id、ItemId | 条目的稳定唯一标识，形如 `doc_probe::Demo::new` | `model.rs` → `ItemId`（规划） |
| #1.4 | 一行摘要 | 索引里条目的简介，取自 `<meta name=description>` | `model.rs` → `ItemSummary::one_line`（规划） |
| #1.5 | 发现、discover | 从产物目录构建条目清单的过程 | `crates/mcp-docs-core/src/discover.rs`（规划） |
| #1.6 | 索引、index.json | 全量条目的扁平清单，Agent 首读 | `crates/mcp-docs-core/src/index.rs`（规划） |
| #1.7 | 指纹、fingerprint | 判断产物是否变化、是否需要重建 | `crates/mcp-docs-core/src/cache.rs` → `Fingerprint`（规划） |
| #1.8 | 先搜后读 | 设计原则：搜索只返回轻摘要 + 指针，正文按需读取 | `design.md` §8 |

## 2. rustdoc 产物（输入数据）

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #2.1 | `sidebar-items.js` | 每个模块目录一份的纯 JSON 条目清单，建索引入口 | `sidebar.rs`（规划） |
| #2.2 | `crates.js` | crate 列表（`window.ALL_CRATES`） | `discover.rs::list_crates`（规划） |
| #2.3 | `all.html` | 全部条目的扁平清单页，发现兜底与交叉校验 | `discover.rs`（规划） |
| #2.4 | 签名、item-decl | `<pre class="rust item-decl">` 里的条目声明 | `parse.rs::signature`（规划） |
| #2.5 | docblock | 文档正文区块；主文档在 `details.toggle.top-doc` 内 | `parse.rs`（规划） |
| #2.6 | 噪声区块 | `#synthetic-implementations` / `#blanket-implementations`，默认剥离 | `parse.rs::ParseOptions`（规划） |
| #2.7 | `search.index` | rustdoc 定制二进制压缩索引，**明确不使用** | 见 `lessons.md` #1.1 |
| #2.8 | 实测事实底座 | design.md 中经真实产物核对的结构事实 | `design.md` §1 |

## 3. 代码位置对照

| 编号 | 术语 / 别名 | 含义 | 代码位置 |
|---|---|---|---|
| #3.1 | core 库、核心库 | 纯库，零 async，解析与渲染全部在此 | `crates/mcp-docs-core/src/`（规划） |
| #3.2 | CLI | 命令行导出 / 查询工具 | `crates/mcp-docs-cli/src/main.rs`（规划） |
| #3.3 | server、MCP server | rmcp + tokio 的 MCP 服务 | `crates/mcp-docs-server/src/`（规划） |
| #3.4 | 输出目录、doc-search | 落盘根，默认 `target/doc-search/` | `store.rs`（规划） |
| #3.5 | fixture | 实测产物裁剪副本，用于测试 | `crates/mcp-docs-core/tests/fixtures/doc_probe/`（规划） |
| #3.6 | 错题集 | 踩坑与错误记录 | `docs/lessons.md` |
| #3.7 | 开发规范 | 代码质量 / 风格 / 注释 / 提交规范 | `docs/conventions.md` |
