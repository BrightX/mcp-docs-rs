# mcp-docs-rs

让 AI Agent 快速检索**本地 crate 的最新文档**。

`cargo doc` 生成的 HTML 不适合 Agent 直接阅读：混杂导航与噪声区块（`Auto Trait Implementations`、`Blanket Implementations`），且大 crate 的文档体积巨大，无法整读。本项目把 rustdoc HTML **按条目切分**为 markdown 与扁平索引，并通过 MCP 服务提供「先搜后读」的检索能力。

> [English](README.md) | 简体中文

## 技术特点

- **纯 Rust 工具链**：Rust 2024，3-crate workspace（纯库 core / CLI / MCP server），`[workspace.dependencies]` 统一版本。
- **直接解析 rustdoc HTML**：基于 `scraper`（html5ever）解析 `cargo doc` 产物，**不用 rustdoc JSON**（至今 unstable），无需 nightly 或 `RUSTC_BOOTSTRAP`；仅依赖长期稳定的结构不变量，跨 rustdoc 版本健壮。
- **只认本地、不联网**：数据源仅本地 `target/doc`，不接 docs.rs。
- **按条目切分**：每个 struct / trait / fn / 方法 / 变体 / 字段都是独立条目，成员默认既独立落盘、又内联进父条目。
- **两级产物**：`index.json`（轻量摘要，Agent 首读）+ 按条目落盘的 markdown 正文。
- **降噪**：剥离 UI 噪声（`copy-path`、侧栏、锚点）与 `blanket` / `synthetic` impl 区块；代码块统一补 ```rust 围栏；`htmd` 负责 HTML→Markdown。
- **可检索**：名字 / 路径 / 摘要的子串、前缀、模糊匹配，多词 AND，按 crate 与类型过滤并按相关度排序。
- **增量与缓存**：基于产物指纹与文件 mtime 只重建变化项；正文按需解析并带 LRU 缓存。
- **多项目 + 共享索引库**：单进程服务多个项目；多个项目依赖同一 crate（同一版本）时只解析渲染一次，跨项目复用（硬链接物化）。
- **MCP 集成**：官方 SDK `rmcp` 3.5 + `tokio`，stdio 传输；18 个工具 + `rustdoc://` 资源 + prompts，支持结构化输出与资源变更通知。
- **工程健壮性**：`rayon` 并行构建、`spawn_blocking` 隔离 CPU 密集任务、Windows 安全文件名（`encode_fs_name`）、原子写、`initialize` 冷启动不阻塞。

## 功能

### 一、文档发现与解析

- **全量发现**：从 `crates.js` 与各层 `sidebar-items.js` 递归构建条目清单，覆盖 `cargo doc` 默认的全部 crate（含依赖）；`all.html` 作兜底与交叉校验，补全 sidebar 与目录扫描都遗漏的条目。
- **条目清单指纹与格式**：支持 `struct` / `enum` / `union` / `trait` / `trait alias` / `fn` / `type` / `constant` / `static` / `macro` / `primitive` / `derive` / `proc-macro` 以及成员（方法 / 关联项 / 变体 / 字段 / trait 必需方法）。
- **签名与文档抽取**：提取 `pre.rust.item-decl` 签名、主文档、各分节（`examples` / `panics` / `implementations` / `trait-implementations` 等）以及成员。
- **成员条目化**：方法与变体等成员成为可检索的独立条目（id 形如 `crate::Type::method`），既可精确检索后单独读取，又内联在父条目中保证上下文完整。
- **链接重写**：把 rustdoc 相对链接解析为条目 md 链接 / 源码引用 / 外链（如指向 std 的链接原样保留）；条目内锚点保留。

### 二、Markdown 渲染与降噪

- 剥离 rustdoc UI 元素与噪声区块，移除 `§` 锚点噪声，`where` 子句保留。
- 代码块由渲染器接管，直接取 `pre.rust` 文本并以 ```rust 围栏包裹，避免语言推断错误与 HTML 实体残留。
- 三种链接风格：`Relative`（默认）、`PlainPath`（最省 token）、`KeepOriginal`（调试）。

### 三、索引与检索

- **扁平索引 `index.json`**：每个条目的 id / kind / 路径 / 一行摘要 / 文件位置 / 源码位置，Agent 首读。
- **匹配模式**：子串、前缀、模糊（子序列）；支持**多词 AND**（各词得分之和）。
- **过滤与排序**：按 crate、kind 过滤；排序按精确名 > 前缀 > 路径命中 > 描述命中打分，摘要命中附带 snippet。
- **分页**：`limit` / `offset`，并返回分页前的 `total`。

### 四、增量、缓存与性能

- **指纹判定**：`file_count` / `max_mtime` / `size_sum` / `crates.js` 哈希 + rustdoc 版本 + schema 版本，任一变化即重建。
- **增量导出**：`--incremental` 按 `src_mtime` 逐文件比对，仅重建变化项。
- **两级缓存**：启动毫秒级加载已有 `index.json`；`get_item` 时才解析正文，`Arc<DocItem>` + LRU（默认 512）。
- **并行构建**：`rayon` 并行解析与落盘；CPU 密集任务在 `spawn_blocking` 中执行，不阻塞异步执行器。

### 五、多项目与跨项目共享索引库

- **单进程多项目**：一个 `mcp-docs-server` 进程服务多个项目；工具 / 资源用可选 `project` 参数定位，缺省用默认项目。除缺省项目外，其余项目首次被访问时才懒启动构建，所有项目构建共用一把锁串行执行。
- **跨项目共享库**：多个项目依赖同一 crate（同一版本 + rustdoc 版本 + 粒度）时，其索引摘要与 markdown 只解析渲染一次；命中后以**硬链接**（同卷）/ **复制**（跨卷）物化到项目目录。默认位于平台缓存目录，可用 `MCP_DOCS_STORE` 覆盖，`MCP_DOCS_NO_STORE` 关闭。

### 六、命令行工具 `mcp-docs`

| 命令 | 说明 |
|---|---|
| `mcp-docs tree` | 打印条目树（调试用） |
| `mcp-docs show <id>` | 解析并打印单个条目 |
| `mcp-docs export [--incremental] [--crate NAME] [--granularity member\|item]` | 导出 markdown + `index.json` + `meta.json` |
| `mcp-docs search <query> [--limit] [--offset] [--mode] [--crate] [--kind]` | 检索条目 |

全局参数：`--doc-dir`（默认 `target/doc`）、`--out`（默认 `target/doc-search`）、`--store`（共享索引库，默认平台缓存目录）。

### 七、MCP 服务

参数**全部通过环境变量传入**（不再解析命令行参数）。

单项目：

```bash
MCP_DOCS_DIR=target/doc MCP_DOCS_OUT=target/doc-search cargo run -p mcp-docs-server
```

多项目（共享一份依赖索引库）：

```bash
MCP_DOCS_PROJECTS="core=/repo/a/target/doc;svc=/repo/b/target/doc" \
  MCP_DOCS_STORE=~/.cache/mcp-docs \
  cargo run -p mcp-docs-server
```

环境变量：

| 变量 | 含义 |
|---|---|
| `MCP_DOCS_DIR` | 单项目 rustdoc 产物目录（映射为名为 `default` 的项目） |
| `MCP_DOCS_OUT` | 单项目输出目录 |
| `MCP_DOCS_PROJECTS` | 分号分隔的 `NAME=DOC_DIR` 列表 |
| `MCP_DOCS_PROJECTS_FILE` | 项目清单 JSON 文件（`{ default?, store?, projects: [{name, doc_dir, out_dir?}] }`） |
| `MCP_DOCS_STORE` | 共享索引库根目录（缺省用平台缓存目录） |
| `MCP_DOCS_NO_STORE` | 非空取值（`1`/`true`/`yes`/`on`）时关闭共享库 |
| `MCP_DOCS_DEFAULT_PROJECT` | 缺省项目名 |

在 MCP 客户端中注册该服务（在其 `env` 字段中配置上述变量）即可使用。除缺省项目外，其余项目**首次被访问时**才后台构建。设计原则是**先搜后读**：检索工具只返回轻量摘要 + 指针，读取工具才返回正文。

**工具（18 个）**

| 工具 | 用途 |
|---|---|
| `list_projects` | 列出本服务当前服务的项目及就绪状态 |
| `list_crates` | 列出某个项目已索引的 crate |
| `list_items` | 列出条目摘要（按 crate / 模块 / 类型过滤，支持 `offset` 分页） |
| `search_items` | 检索条目，返回轻量摘要与得分（支持 `offset` 分页） |
| `get_item` | 读取条目的完整 markdown |
| `get_item_source` | 查询条目的源码位置 |
| `get_examples` | 抽取条目文档里的 rust 代码示例 |
| `get_item_section` | 返回条目某个分节（如 `examples` / `panics`）的 markdown |
| `batch_get_items` | 批量读取多个条目（ids 上限 20） |
| `get_source_text` | 按源码位置读取源码文本（失败时回退为路径与行号） |
| `get_item_json` | 返回条目的结构化 JSON |
| `index_status` | 返回某个项目的索引状态（就绪 / 构建中 / schema / 版本 / 计数 / 是否过期） |
| `module_tree` | 返回 crate 的模块树（含每级条目数） |
| `get_related_items` | 返回条目的父条目、兄弟与子成员 |
| `get_trait_implementors` | 列出实现了某个 trait 的类型 |
| `find_by_signature` | 按签名子串（如 `-> Result<`）检索 |
| `search_docs` | 在已导出的 markdown 正文里全文检索（需先 `mcp-docs export`） |
| `rebuild_index` | 重建索引（默认增量） |

多项目时，上述工具都接受可选入参 `project`（省略则用缺省项目）。

**资源**：`rustdoc://crates`、`rustdoc://{crate}`（支持 `?offset=&limit=` 分页）、`rustdoc://{crate}/{item}`；均可加 `?project=NAME` 指定项目。item 段用 `/` 表示模块分隔 `::`（如 `rustdoc://tokio/task/spawn`），kind 与名字之间仍用 `.`（如 `rustdoc://tokio/task.Spawn`）。

**Prompts**：`explain_api` / `usage_example`（入参 `id`，引导模型先读文档再作答）。

**协议能力**：`search_items` / `get_item_json` / `index_status` / `module_tree` 返回带 `outputSchema` 的结构化内容（同时保留文本）；后台构建完成或 `rebuild_index` 后发送 `notifications/resources/list_changed`。

**冷启动不阻塞**：`initialize` 立即返回；首次的索引构建在后台进行，工具 / 资源会等待就绪后再返回（已有索引则直接服务、后台按需刷新）。

典型流程：`search_items("spawn", crate="tokio")` → `get_item("tokio::spawn")` → 只拿到这一小块。

## 快速开始

```bash
# 1. 在你的项目里生成文档（含依赖）
cargo doc

# 2. 导出 markdown + 索引（默认输出到 target/doc-search）
cargo run -p mcp-docs-cli -- export

# 3. 检索
cargo run -p mcp-docs-cli -- search "spawn" --crate tokio
```

## 项目结构

| crate | 说明 |
|---|---|
| `crates/mcp-docs-core` | 核心库：发现、解析、渲染、索引、检索、缓存（无 async） |
| `crates/mcp-docs-cli` | 命令行工具 `mcp-docs` |
| `crates/mcp-docs-server` | MCP 服务 `mcp-docs-server`（stdio） |

## 开发

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

规范详见 [docs/conventions.md](docs/conventions.md)。

## 文档

需求、设计、里程碑与开发规范都在 [`docs/`](docs/)：

- [需求](docs/requirements.md)
- [设计](docs/design.md)
- [里程碑与进度](docs/roadmap.md)
- [开发规范](docs/conventions.md)
- [错题集](docs/lessons.md)
- [缺陷跟踪](docs/issues.md)
- [术语与代码位置对照](docs/glossary.md)

## License

MIT OR Apache-2.0
