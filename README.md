# mcp-docs-rs

让 AI Agent 快速检索**本地 crate 的最新文档**。

`cargo doc` 生成的 HTML 不适合 Agent 直接阅读：混杂导航与噪声区块（`Auto Trait Implementations`、`Blanket Implementations`），且大 crate 的文档体积巨大，无法整读。本项目把 rustdoc HTML **按条目切分**为 markdown 与扁平索引，并通过 MCP 服务提供「先搜后读」的检索能力。

## 特性

- **按条目切分**：每个 struct / trait / fn / 方法 / 变体 / 字段都是独立条目。
- **两级产物**：`index.json`（轻量摘要，Agent 首读）+ 按条目落盘的 markdown。
- **降噪**：剥离 rustdoc 的 UI 噪声、`§` 锚点、blanket / synthetic impl；代码块补 `rust` 语言标注。
- **可检索**：按名字 / 路径 / 摘要检索并给出相关度得分，支持按 crate 与类型过滤。
- **增量**：基于产物指纹与文件 mtime，只重建变化的部分。
- **MCP 服务**：17 个工具 + `rustdoc://` 资源，stdio 传输，按需解析并缓存。

## 快速开始

```bash
# 1. 在你的项目里生成文档（含依赖）
cargo doc

# 2. 导出 markdown + 索引（默认输出到 target/doc-search）
cargo run -p mcp-docs-cli -- export

# 3. 检索
cargo run -p mcp-docs-cli -- search "spawn" --crate tokio
```

### 命令行

| 命令 | 说明 |
|---|---|
| `mcp-docs tree` | 打印条目树（调试用） |
| `mcp-docs show <id>` | 解析并打印单个条目 |
| `mcp-docs export [--incremental] [--crate NAME] [--granularity member\|item]` | 导出 markdown + `index.json` + `meta.json` |
| `mcp-docs search <query> [--limit] [--offset] [--mode] [--crate] [--kind]` | 检索条目 |

全局参数：`--doc-dir`（默认 `target/doc`）、`--out`（默认 `target/doc-search`）。

### MCP 服务

```bash
cargo run -p mcp-docs-server -- --doc-dir target/doc --out-dir target/doc-search
```

在 MCP 客户端中注册该命令即可使用。工具：

| 工具 | 用途 |
|---|---|
| `list_crates` | 列出已索引的 crate |
| `list_items` | 列出条目摘要（按 crate / 模块 / 类型过滤，支持 `offset` 分页） |
| `search_items` | 检索条目，返回轻量摘要与得分（支持 `offset` 分页） |
| `get_item` | 读取条目的完整 markdown |
| `get_item_source` | 查询条目的源码位置 |
| `get_examples` | 抽取条目文档里的 rust 代码示例 |
| `get_item_section` | 返回条目某个分节（如 `examples` / `panics`）的 markdown |
| `batch_get_items` | 批量读取多个条目（ids 上限 20） |
| `get_source_text` | 按源码位置读取源码文本（失败时回退为路径与行号） |
| `get_item_json` | 返回条目的结构化 JSON |
| `index_status` | 返回索引状态（就绪 / 构建中 / schema / 版本 / 计数 / 是否过期） |
| `module_tree` | 返回 crate 的模块树（含每级条目数） |
| `get_related_items` | 返回条目的父条目、兄弟与子成员 |
| `get_trait_implementors` | 列出实现了某个 trait 的类型 |
| `find_by_signature` | 按签名子串（如 `-> Result<`）检索 |
| `search_docs` | 在已导出的 markdown 正文里全文检索 |
| `rebuild_index` | 重建索引（默认增量） |

资源：`rustdoc://crates`、`rustdoc://{crate}`（支持 `?offset=&limit=` 分页）、`rustdoc://{crate}/{item}`。

Prompts：`explain_api` / `usage_example`（入参 `id`，引导模型先读文档再作答）。结构化输出：`search_items` / `get_item_json` / `index_status` / `module_tree` 返回带 `outputSchema` 的结构化内容（同时保留文本）。通知：后台构建完成或 `rebuild_index` 后发送 `notifications/resources/list_changed`。

冷启动不阻塞：`initialize` 立即返回；首次的索引构建在后台进行，工具/资源会等待就绪后再返回（已有索引则直接服务、后台按需刷新）。

典型流程：`search_items("spawn", crate="tokio")` → `get_item("tokio::spawn")` → 只拿到这一小块。

## 项目结构

| crate | 说明 |
|---|---|
| `crates/mcp-docs-core` | 核心库：发现、解析、渲染、索引、检索、缓存（无 async） |
| `crates/mcp-docs-cli` | 命令行工具 `mcp-docs` |
| `crates/mcp-docs-server` | MCP 服务 `mcp-docs-server`（stdio） |

## 文档

需求、设计、里程碑与开发规范都在 [`docs/`](docs/)：

- [需求](docs/requirements.md)
- [设计](docs/design.md)
- [里程碑与进度](docs/roadmap.md)
- [开发规范](docs/conventions.md)
- [错题集](docs/lessons.md)
- [术语与代码位置对照](docs/glossary.md)

## 开发

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

规范详见 [docs/conventions.md](docs/conventions.md)。

## License

MIT OR Apache-2.0
