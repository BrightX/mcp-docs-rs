# mcp-docs 工作流

面向具体任务的调用示例。先看决策，再按需查阅对应小节。

## 决策：拿到需求后先想清楚「检索还是列出」

- 知道名字 / 关键词 → `search_items`。
- 只知道 crate 和大致类型 → `list_crates` → `list_items(crate, kind)`。
- 只记得签名特征 → `find_by_signature`。
- 要正文命中（描述里出现的词）→ `search_docs`。
- 命中后一律用 `get_item` 读正文，不要整读某个 crate 的 HTML。

## §1 查某个 API 的用法

```
search_items(query="spawn", crate="tokio")
  → hits: [... { id: "tokio::task::fn.spawn", score: 100, ... }]

get_item(id="tokio::task::fn.spawn", max_bytes=8000)
  → 签名 + 文档 + 成员 markdown

get_examples(id="tokio::task::fn.spawn")
  → examples: [{ language: "rust", code: "..." }]
```

要点：
- 直接回填检索结果里的 `id`（带类型标记，最准确）；也接受省略标记的写法。
- 条目很大时给 `max_bytes`，避免撑爆上下文。

## §2 探索陌生 crate

```
list_crates()
  → crates: [{ name: "tokio", item_count: 1234 }, ...]

module_tree(crate="tokio")
  → 嵌套模块树，含每级条目数

list_items(crate="tokio", module="task", kind="struct", limit=50)
  → 摘要列表；对感兴趣的条目再 get_item
```

要点：`module_tree` 先建立整体地图，再用 `list_items` + `kind` 聚焦某类接口。

## §3 按签名找函数

```
find_by_signature(pattern="-> Result", crate="serde_json", kind="fn", limit=10)
  → hits: [摘要]
```

要点：`pattern` 是签名子串（大小写不敏感）；适合「找返回 Result / 接收 &Path 的函数」这类需求。

## §4 正文全文检索

```
search_docs(query="panic", crate="regex", limit=10)
  → hits: [{ file, id, snippet }]
```

要点：扫描已导出的 markdown 正文，比 `search_items` 慢；命中后按 `id` 走 `get_item` 读全文。

## §5 查看 trait 的实现者

```
get_trait_implementors(id="std::fmt::trait.Display")
  → { count, implementors: [{ crate, impl: "impl Display for Foo" }] }
```

要点：入参必须是 trait；结果来自 rustdoc 的 `trait.impl/*.js`，若产物缺失该文件会返回空并给出 `note`。

## §6 批量读取多个条目

```
batch_get_items(ids=["tokio::task::fn.spawn", "tokio::task::fn.sleep"], max_bytes_each=4000)
  → items: [{ id, markdown } | { id, error }]
```

要点：一次最多 20 个；适合已知一组 id 时一次性取回。

## §7 用资源直接读

```
resources/read rustdoc://crates
resources/read rustdoc://tokio?offset=0&limit=100
resources/read rustdoc://tokio/task/spawn      # 注意用 / 代替 ::
resources/read rustdoc://tokio?project=svc     # 指定项目
```

要点：资源适合「一次拿一块」的场景；`rustdoc://{crate}` 的 `limit` 上限 500，大 crate 请分页或用 `list_items`。多项目时用 `?project=NAME` 选择项目。

## §8 用 prompts 起头

```
prompts/get explain_api { id: "tokio::task::fn.spawn" }
prompts/get usage_example { id: "tokio::task::fn.spawn" }
```

要点：返回的是引导消息；配合 `get_item` / `get_examples` 使用。

## §9 索引状态与重建

```
index_status()
  → { project: "default", ready: true, building: false, schema_version: 7, item_count: 12000, stale: false, ... }

rebuild_index()          # 默认增量
rebuild_index(force=true) # 全量
```

要点：若 `ready=false` 且 `building=true`，说明后台正在构建，工具/资源会等待；`stale=true` 表示产物已变、下次访问会触发刷新。

## §11 多项目

服务可同时管理多个项目；不知道有哪些项目时先查：

```
list_projects()
  → projects: [{ name: "core", is_default: true, ready: true, ... },
               { name: "svc",  is_default: false, ready: false, building: true, ... }]
```

之后在任意工具/资源上带 `project` 指定：

```
search_items(query="spawn", crate="tokio", project="svc")
get_item(id="tokio::task::fn.spawn", project="svc")
resources/read rustdoc://tokio?project=svc
```

要点：
- 省略 `project` 时用缺省项目（`list_projects` 里 `is_default=true` 的那个）。
- 非缺省项目**首次访问时才后台构建**，首次调用该项目的工具会等待其就绪（可用 `index_status(project=...)` 查看 `ready` / `building`）。
- 各项目共用一份共享依赖索引库：同一版本依赖（如 tokio）只解析渲染一次，跨项目复用，因此切换项目的额外成本很低。

## §12 CLI 兜底（未接 MCP）

```bash
cargo run -p mcp-docs-cli -- export --incremental
cargo run -p mcp-docs-cli -- search "spawn" --crate tokio --limit 10
cargo run -p mcp-docs-cli -- show "tokio::task::fn.spawn"
cargo run -p mcp-docs-cli -- tree
```

要点：CLI 与 MCP 共享同一份 `index.json`；`export` 负责落盘 markdown 与索引。全局参数 `--doc-dir`（默认 `target/doc`）、`--out`（默认 `target/doc-search`）、`--store`（共享索引库，默认平台缓存目录）。
