---
name: mcp-docs
description: 当用户询问某个 Rust crate 的 API、类型、函数签名、用法或示例，且该 crate 有本地 rustdoc 产物（cargo doc 生成的 target/doc）或已接入 mcp-docs MCP 服务时使用。本技能记录 mcp-docs 的「先检索再读取」工作流、全部工具、资源与 prompts，用于抓取最小且相关的文档片段，避免整读 rustdoc HTML。
---

# mcp-docs

## 概述

mcp-docs 把本地 `cargo doc` 产出的 rustdoc HTML **按条目切分**为 markdown 与扁平索引，并通过 MCP 服务（或 CLI）暴露检索能力。核心原则是**先检索、再读取**：检索只返回轻量摘要与指针，正文按需读取，从而用最小上下文拿到所需 API。

## 何时使用

在以下场景触发：

- 用户问「X 怎么用」「X 的签名是什么」「X 有哪些方法 / 变体」，而 X 属于某个 Rust crate。
- 需要在本地依赖（`target/doc` 内）中查找类型、函数、trait、宏的文档或示例。
- 需要探索某个陌生 crate 的接口，或按签名（如「返回 Result 的函数」）检索。
- 已连接 mcp-docs MCP 服务（工具名前缀 `search_items` / `get_item` 等），或本仓库的 `mcp-docs` CLI 可用。

不适用：crate 没有本地 rustdoc 产物且无法生成；需要联网抓取 docs.rs（本工具仅用本地产物）。

## 前置条件

- 目标项目已执行 `cargo doc`（默认产物在 `target/doc`，含全部依赖）。
- MCP 服务 `mcp-docs-server` 已注册，或可用 CLI `cargo run -p mcp-docs-cli --`。
- 索引默认落在 `target/doc-search/`。若索引缺失或过期，服务会在**后台**构建；工具/资源会等待就绪后再返回，首次冷启动可能稍慢。
- 服务可同时管理**多个项目**：不确定有哪些项目时先调用 `list_projects`；工具/资源的 `project` 参数省略时用缺省项目。

## 核心工作流：先检索，再读取

1. **定位条目**：用 `search_items`（按名字/路径/摘要）或 `list_items`（按 crate/模块/类型过滤）找到候选条目，只拿轻量摘要。
2. **读取正文**：对感兴趣的条目用 `get_item` 拿完整 markdown（签名 + 文档 + 成员）。
3. **按需补充**：需要示例用 `get_examples`；需要源码用 `get_item_source` / `get_source_text`；需要某分节用 `get_item_section`。

典型链路：

```
search_items(query="spawn", crate="tokio")      → 若干摘要
get_item(id="tokio::task::fn.spawn")            → 该条目完整 markdown
get_examples(id="tokio::task::fn.spawn")        → 代码示例
```

## 常见任务

- **查某个 API 的用法**：`search_items` → `get_item`（必要时 `get_examples`）。见 `references/workflows.md` §1。
- **探索陌生 crate**：`list_crates` → `get_item(crate)`（crate 概览）/ `module_tree(crate)` → 逐个 `get_item`。见 §2。
- **按签名找函数**：`find_by_signature(pattern)`。见 §3。
- **正文全文检索**：`search_docs(query)`（扫描已导出的 markdown 正文）。见 §4。
- **看 trait 有哪些实现者**：`get_trait_implementors(id)`。见 §5。
- **批量读取多个条目**：`batch_get_items(ids)`（一次上限 20）。见 §6。
- **用资源直接读**：`rustdoc://{crate}/{item}`、`rustdoc://crates`、`rustdoc://{crate}`（支持分页）。见 §7。
- **多项目**：先 `list_projects` 查看项目，再在工具/资源上带 `project` 参数。见 §11。

详细的参数与返回字段见 `references/tools.md`；逐任务示例见 `references/workflows.md`。

## 工具速查

| 工具 | 用途 |
|---|---|
| `list_projects` | 列出本服务当前服务的项目及就绪状态（多项目时先调用） |
| `list_crates` | 列出某个项目已索引的 crate 及条目数 |
| `list_items` | 列出条目摘要（按 crate / 模块 / 类型过滤，支持 `offset`） |
| `search_items` | 检索条目，返回摘要与得分（结构化输出；支持 `offset`） |
| `get_item` | 读取条目完整 markdown（可用 `max_bytes` 截断） |
| `get_item_source` | 查询条目的源码文件与行号 |
| `get_examples` | 抽取条目文档里的 rust 代码示例 |
| `get_item_section` | 返回某个分节的 markdown |
| `batch_get_items` | 批量读取多个条目（ids 上限 20） |
| `get_source_text` | 按源码位置读取源码文本 |
| `get_item_json` | 返回条目的结构化 JSON |
| `index_status` | 返回索引状态（就绪 / 构建中 / schema / 计数 / 是否过期） |
| `module_tree` | 返回 crate 的模块树（每级条目数） |
| `get_related_items` | 返回父条目、同模块兄弟与子成员 |
| `get_trait_implementors` | 列出实现了某个 trait 的类型 |
| `find_by_signature` | 按签名子串检索（如 `-> Result`） |
| `search_docs` | 在导出的 markdown 正文里全文检索 |
| `rebuild_index` | 重建索引（默认增量） |

### 参数速查（高频）

| 工具 | 参数（**加粗**为必填） | 取值 / 说明 |
|---|---|---|
| `search_items` | **query**, crate?, kind?, mode?, limit?, offset? | `mode`: `substring`(默认)/`prefix`/`fuzzy`；`crate` 只填 crate 名 |
| `list_items` | crate?, module?, kind?, limit?, offset? | `module` 填 crate 内**真实**模块名（不带 crate 前缀）；`kind` 见下 |
| `get_item` | **id**, max_bytes? | `id` 取自检索结果的 `id`，可省略类型标记（`tokio::task::spawn`） |
| `get_examples` | **id**, max_examples?, max_bytes? | `max_examples` 默认 5 |
| `get_item_section` | **id**, **section** | section 如 `examples` / `panics` / `implementations` |
| `module_tree` | **crate** | crate 的模块树（每级条目数） |
| `find_by_signature` | **pattern**, crate?, kind?, limit? | 按签名子串，如 `-> Result<` |
| `batch_get_items` | **ids**（≤20）, max_bytes_each? | 批量读正文 |
| `search_docs` | **query**, crate?, kind?, limit?, offset? | 扫已导出正文，较慢 |

`kind` 取值：`struct` / `enum` / `trait` / `fn`(function) / `method` / `field` / `variant` / `macro` / `module` / `const` / `type` / `union` 等，可用前缀、复数或自然名单数（`fn` / `functions` / `function`）；**非法值报错**，不会静默忽略。

### 常见坑

- **crate 首页不是条目**：`get_item(id="tokio")` 会回退返回合成概览（一级模块 + 一级条目 + 后续指引）；要模块树用 `module_tree`。
- **`module` 过滤可能静默为空**：只认 crate 内真实模块段，**重导出别名**（`pub use X as m`）不是模块；空结果会带 `note` 列出可用一级模块，别把空当"不存在"。
- **检索是名字/路径/子串匹配、无语义排序**：`search_items("Button")` 可能被 `Role::variant.Button` 之类同名成员刷屏；用 `crate` + `kind` 收窄，优先看 `has_docs:true` 的条目。
- **`max_bytes` 在行边界截断**：被截断时正文不完整，用更大的 `max_bytes` 或 `get_item_section` 读剩余分节。
- **资源 URI 用 `/{mod}/{Name}`**（如 `rustdoc://tokio/io/copy`）；带类型标记写作 `/{mod}/{kind}.{Name}`（如 `rustdoc://tokio/io/fn.copy`）。把模块分隔写成 `.`（`io.fn.copy`）会 404。
- **实现者查询**：`get_trait_implementors` 同时给出本 crate / 外来类型 / 跨 crate 实现（`source` 字段区分）；`get_item_section(id, section="implementors"|"foreign-impls")` 返回对应 impl 列表。
- **`search_docs` 翻页**：看返回的 `has_more`，下一页 `offset` 用 `next_offset`；步长不等于 `limit` 时两页会重叠。`find_by_signature` 带 `total`。
- **跨会话不代表行为一致**：结论落地前先看 `index_status` 的 `stale` / `schema_version`，空结果先读 `note`。

## 资源与 Prompts

- 资源：`rustdoc://crates`、`rustdoc://{crate}`（支持 `?offset=&limit=` 分页）、`rustdoc://{crate}/{item}`（item 段用 `/` 表示模块分隔 `::`，kind 与名字之间仍用 `.`，如 `rustdoc://tokio/task.Spawn`）；均可加 `?project=NAME` 指定项目。
- Prompts：`explain_api`（解释 API 用法）、`usage_example`（给出调用示例），入参 `id` + `project?`；返回引导模型先 `get_item` / `get_examples` 的消息。

## CLI 兜底（MCP 不可用时）

在仓库根目录（或已安装本工具的环境）：

```bash
cargo run -p mcp-docs-cli -- export [--incremental] [--crate NAME] [--granularity member|item]
cargo run -p mcp-docs-cli -- search "spawn" --crate tokio [--limit N] [--offset N] [--mode substring|prefix|fuzzy]
cargo run -p mcp-docs-cli -- tree
cargo run -p mcp-docs-cli -- show <id>
```

全局参数：`--doc-dir`（默认 `target/doc`）、`--out`（默认 `target/doc-search`）、`--store`（共享索引库，默认平台缓存目录）。

## 注意事项

- **条目 id 可省略类型标记**：索引内 id 形如 `tokio::task::fn.spawn`，但工具同时接受 `tokio::task::spawn`。检索返回值里的 `id` 可直接回填给 `get_item`。
- **善用 `max_bytes`**：超大条目（如 trait 页）用 `get_item(id, max_bytes=N)` 按字符边界截断，避免撑爆上下文。
- **`kind` 过滤要合法**：接受前缀（`fn`）、复数（`functions`）或自然名单数（`function` / `method`）；非法值会报错而非静默忽略。
- **索引过期会自动重建**：`index_status` 可查看 `ready` / `building` / `stale`；schema 或导出粒度变化会触发一次全量重建。
- **`search_docs` 较慢**：它扫描已导出的 markdown 正文，仅在需要正文命中时使用；若正文尚未导出（仅启动 server），会返回提示而非空结果。结构化字段检索优先用 `search_items` / `find_by_signature`。
- **通知**：后台构建完成或 `rebuild_index` 成功后，服务会发 `notifications/resources/list_changed`，客户端可据此刷新资源清单。
- **多项目用 `project`**：工具/资源省略 `project` 时用缺省项目；非缺省项目首次访问才后台构建，首次调用会等待其就绪。各项目共享同一份依赖索引库，同一版本依赖只解析一次。
- **共享索引库默认开启**：server 默认读写平台缓存目录（跨项目复用依赖索引）；不需要时用 `--no-store` 关闭。

## 附带资源

- `references/tools.md` — 全部工具、资源、prompts 的参数与返回字段详解。
- `references/workflows.md` — 面向具体任务的调用示例与决策建议。
