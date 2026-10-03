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
- **探索陌生 crate**：`list_crates` → `module_tree` / `list_items(crate, kind)` → 逐个 `get_item`。见 §2。
- **按签名找函数**：`find_by_signature(pattern)`。见 §3。
- **正文全文检索**：`search_docs(query)`（扫描已导出的 markdown 正文）。见 §4。
- **看 trait 有哪些实现者**：`get_trait_implementors(id)`。见 §5。
- **批量读取多个条目**：`batch_get_items(ids)`（一次上限 20）。见 §6。
- **用资源直接读**：`rustdoc://{crate}/{item}`、`rustdoc://crates`、`rustdoc://{crate}`（支持分页）。见 §7。

详细的参数与返回字段见 `references/tools.md`；逐任务示例见 `references/workflows.md`。

## 工具速查

| 工具 | 用途 |
|---|---|
| `list_crates` | 列出已索引的 crate 及条目数 |
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

## 资源与 Prompts

- 资源：`rustdoc://crates`、`rustdoc://{crate}`（支持 `?offset=&limit=` 分页）、`rustdoc://{crate}/{item}`（`/` 对应 `::`）。
- Prompts：`explain_api`（解释 API 用法）、`usage_example`（给出调用示例），入参 `id`；返回引导模型先 `get_item` / `get_examples` 的消息。

## CLI 兜底（MCP 不可用时）

在仓库根目录（或已安装本工具的环境）：

```bash
cargo run -p mcp-docs-cli -- export [--incremental] [--crate NAME] [--granularity member|item]
cargo run -p mcp-docs-cli -- search "spawn" --crate tokio [--limit N] [--offset N] [--mode substring|prefix|fuzzy]
cargo run -p mcp-docs-cli -- tree
cargo run -p mcp-docs-cli -- show <id>
```

全局参数：`--doc-dir`（默认 `target/doc`）、`--out`（默认 `target/doc-search`）。

## 注意事项

- **条目 id 可省略类型标记**：索引内 id 形如 `tokio::task::fn.spawn`，但工具同时接受 `tokio::task::spawn`。检索返回值里的 `id` 可直接回填给 `get_item`。
- **善用 `max_bytes`**：超大条目（如 trait 页）用 `get_item(id, max_bytes=N)` 按字符边界截断，避免撑爆上下文。
- **`kind` 过滤要合法**：接受前缀（`fn`）、复数（`functions`）或自然名单数（`function` / `method`）；非法值会报错而非静默忽略。
- **索引过期会自动重建**：`index_status` 可查看 `ready` / `building` / `stale`；schema 或导出粒度变化会触发一次全量重建。
- **`search_docs` 较慢**：它扫描已导出的 markdown 正文，仅在需要正文命中时使用；结构化字段检索优先用 `search_items` / `find_by_signature`。
- **通知**：后台构建完成或 `rebuild_index` 成功后，服务会发 `notifications/resources/list_changed`，客户端可据此刷新资源清单。

## 附带资源

- `references/tools.md` — 全部工具、资源、prompts 的参数与返回字段详解。
- `references/workflows.md` — 面向具体任务的调用示例与决策建议。
