# mcp-docs 工具参考

全部工具的参数与返回。`?` 表示可选参数。条目「摘要」的通用字段为：
`id` / `kind` / `name` / `path` / `one_line` / `has_docs` / `has_members` / `file`。

> 说明：`search_items` / `get_item_json` / `index_status` / `module_tree` 使用 MCP 结构化输出
> （`structured_content` + `outputSchema`），同时保留等价的文本 JSON。
>
> **多项目**：全部工具都接受可选入参 `project`（省略时用缺省项目）。项目名可用 `list_projects` 查询。

## 项目

### list_projects
- 参数：无。
- 返回：`{ projects: [{ name, doc_dir, out_dir, is_default, ready, building, crate_count, item_count }] }`。
- 不阻塞（未就绪项目计数为 0）；是 Agent 发现可用项目名的入口。

## 列表与检索

### list_crates
- 参数：`project?`。
- 返回：`{ project, crates: [{ name, item_count }] }`。

### list_items
- 参数：`crate?`（限定 crate）、`module?`（限定模块路径，按整段匹配）、`kind?`（条目类型）、
  `limit?`（默认 100）、`offset?`（默认 0）。
- 返回：`{ total, offset, returned, items: [摘要] }`。`total` 为过滤后的总数（与分页无关）。

### search_items
- 参数：`query`（必填）、`crate?`、`kind?`、`mode?`（`substring` 默认 / `prefix` / `fuzzy`）、
  `limit?`（默认 20）、`offset?`（默认 0）。
- 返回（结构化）：`{ total, offset, returned, hits: [{ id, kind, name, path, one_line, has_docs,
  has_members, file, score, snippet? }] }`。
- 打分：名字精确 100 / 前缀 80 / 子串 60 / 路径 40 / 摘要 20；多词查询为 AND（各词得分之和）。
  摘要命中会附带 `snippet`（±40 字符窗口）。

### find_by_signature
- 参数：`pattern`（签名子串，必填，大小写不敏感）、`crate?`、`kind?`、`limit?`（默认 20）。
- 返回：`{ pattern, returned, hits: [摘要] }`。

### search_docs
- 参数：`query`（必填）、`crate?`、`kind?`、`limit?`（默认 20）、`offset?`（默认 0）。
- 返回：`{ query, offset, returned, hits: [{ file, id, snippet }] }`。
- 在**已导出的 markdown 正文**里检索；较慢，正文命中场景才用。

## 读取

### get_item
- 参数：`id`（必填）、`max_bytes?`（按字符边界截断）。
- 返回：条目的完整 markdown 文本（签名 + 文档 + 成员）。

### get_examples
- 参数：`id`（必填）、`max_examples?`（默认 5）、`max_bytes?`（每个示例的字节上限）。
- 返回：`{ id, returned, examples: [{ language, code }] }`（仅 `rust` 代码块）。

### get_item_section
- 参数：`id`（必填）、`section`（分节名，如 `examples` / `panics` / `implementations`，必填）。
- 返回：`{ id, section, title, body_md, returned, members: [{ id, name, markdown }] }`；
  未命中时报错并列出可用分节名。

### batch_get_items
- 参数：`ids`（必填，最多 20 个）、`max_bytes_each?`。
- 返回：`{ requested, returned, truncated, items: [{ id, markdown } | { id, error }] }`。

### get_item_json
- 参数：`id`（必填）。
- 返回（结构化）：`{ id, kind, name, path, signature, docs_md,
  source: { file, line_start, line_end } | null,
  sections: [{ id, title, body_md }],
  members: [{ id, kind, name, has_docs }] }`。

## 源码

### get_item_source
- 参数：`id`（必填）、`max_bytes?`（未使用，保留）。
- 返回：`{ id, source: { file, path, line_start, line_end } | null }`。
  `file` 相对 `--doc-dir`；`path` 为可直接打开的绝对路径。

### get_source_text
- 参数：`id`（必填）、`max_bytes?`。
- 返回：`{ id, file, path, line_start, line_end, text | null, note }`。
  `text` 无法提取时为 `null`（仅返回路径与行号）。

## 导航与关系

### module_tree
- 参数：`crate`（必填）。
- 返回（结构化）：`{ name, path, item_count, children: [同构节点] }`。

### get_related_items
- 参数：`id`（必填）。
- 返回：`{ id, parent: 摘要 | null, siblings: [摘要], children: [摘要] }`。

### get_trait_implementors
- 参数：`id`（必须是 trait / trait alias）。
- 返回：`{ id, count, implementors: [{ crate, impl }], note }`。

## 状态与维护

### index_status
- 参数：`project?`。
- 返回（结构化）：`{ project, ready, building, schema_version, rustdoc_version, crate_count,
  item_count, granularity, doc_dir, out_dir, generated_at, stale }`。

### rebuild_index
- 参数：`force?`（`true` 强制全量，默认增量）、`project?`。
- 返回：`{ project, rebuilt, reused, shared_hits, item_count }`。完成后会发资源变更通知。

## 资源

| URI | 内容 |
|---|---|
| `rustdoc://crates` | 缺省项目的 crate 清单（同 `list_crates`） |
| `rustdoc://{crate}` | 该 crate 的条目清单，支持 `?offset=N&limit=M`（limit 上限 500） |
| `rustdoc://{crate}/{item}` | 某条目的 markdown；`/` 对应 `::`，id 可省略类型标记 |

以上 URI 均可追加 `?project=NAME` 指定项目（缺省回落默认项目），
分页参数与 `project` 可共存，如 `rustdoc://tokio?project=svc&offset=0&limit=50`。

`rustdoc://{crate}` 返回：`{ project, crate, item_count, offset, limit, returned, truncated, items: [摘要] }`。

## Prompts

| 名称 | 入参 | 说明 |
|---|---|---|
| `explain_api` | `id`, `project?` | 引导先读条目文档再解释用途与用法 |
| `usage_example` | `id`, `project?` | 引导先取文档与示例再给最小可运行示例 |

两者均返回一条 `user` 角色的消息。
