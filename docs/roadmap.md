# 进度计划与里程碑

## 状态总览

| 阶段 | 目标 | 状态 |
|---|---|---|
| M0 | 骨架 + 文档发现 | ⬜ 未开始 |
| M1 | 单条目解析 | ⬜ 未开始 |
| M2 | Markdown 渲染 + 链接重写 + 降噪 | ⬜ 未开始 |
| M3 | 索引 + 检索 | ⬜ 未开始 |
| M4 | 缓存 / 增量 / 指纹 | ⬜ 未开始 |
| M5 | MCP Server | ⬜ 未开始 |
| M6 | 打磨 | ⬜ 未开始 |

状态标记：⬜ 未开始 / 🟡 进行中 / ✅ 已完成 / ⛔ 阻塞。

## M0 — 骨架 + 文档发现

**交付物**
- 3-crate workspace（`mcp-docs-core` / `mcp-docs-cli` / `mcp-docs-server`）骨架，根 `Cargo.toml` 统一依赖版本。
- `sidebar.rs`：`parse_sidebar_str` / `parse_sidebar_file`。
- `discover.rs`：`list_crates` + `discover_crate`（递归 sidebar，`all.html` 兜底）。
- CLI：`mcp-docs tree`。
- 测试 fixture：从 `temp/doc_probe/target/doc` 裁剪拷贝到 `tests/fixtures/doc_probe/`。

**验收标准**
- `mcp-docs tree` 对 fixture 打印出 6 个顶层条目 + `inner::Nested`，与 `all.html` 内容一致。
- `parse_sidebar_str` 单元测试通过。

## M1 — 单条目解析

**交付物**
- `parse.rs`：`parse_item_html`，抽出标题、签名、主文档、各 section、方法 / 字段 / 变体 / 关联项。
- `parse_rustdoc_meta` / `parse_one_line` / `path_to_identity`。
- CLI：`mcp-docs show <id>`。

**验收标准**
- `Demo` 签名 = `pub struct Demo { pub field: u32, }`。
- `Demo::new` 文档 = "build a demo"；字段 `field` 文档 = "the field"。
- `Kind` 识别出 A / B 两个变体。
- `trait.DoIt.html` 的 `run` 识别为 `TyMethod`。

## M2 — Markdown 渲染 + 链接重写 + 降噪

**交付物**
- `markdown.rs`：`render_item` / `render_crate_index`，含噪声剥离与代码块接管。
- `link.rs`：`resolve_href` + 三种 `LinkStyle`。
- `store.rs`：`encode_fs_name` / `atomic_write` / `item_output_path`。
- CLI：`mcp-docs export`。

**验收标准**
- `struct.Demo.md` 不含 "Copy item path"、不含 blanket / synthetic impls。
- 代码示例带 ```rust 围栏，无 HTML 实体残留。
- 内部链接可解析（`Relative` 指向目标 md + 锚点，或 `PlainPath` 纯路径）。
- `encode_fs_name` 对 `Demo<Bar>` / `CON` / `Demo::new` 的断言通过。

## M3 — 索引 + 检索

**交付物**
- `index.rs`：`build_index` / `write_index` / `load_index`，生成扁平 `index.json`。
- `search.rs`：`search` + `rank`（子串 / 前缀 / 模糊，按 crate、kind 过滤）。
- 成员条目化落盘（独立文件 + 内联父文件）。
- CLI：`mcp-docs search <query>`。

**验收标准**
- 搜 `new` 命中 `doc_probe::Demo::new`。
- 搜 `demo` 命中 `Demo`（描述匹配）。
- `index.json` 可被 `serde_json` 解析，条目数与发现阶段一致。

## M4 — 缓存 / 增量 / 指纹

**交付物**
- `cache.rs`：`fingerprint_doc_root` / `DocCache`（`get_or_parse` / `is_stale` / `invalidate`）。
- `meta.json` 落盘与 stale 判定（fingerprint / rustdoc 版本 / schema 版本）。
- `--incremental`：按 `src_mtime` 逐文件比对，仅更新变化项。

**验收标准**
- 二次运行不重新解析全部条目。
- `touch` 一个 html 后，仅该条目重建。

## M5 — MCP Server

**交付物**
- `mcp-docs-server`：rmcp `#[tool_router]` 实现 6 个工具。
- MCP 资源：`rustdoc://crates`、`rustdoc://{crate}`、`rustdoc://{crate}/{*item}`。
- stdio 传输（`transport-io`）。

**验收标准**
- `search_items` → `get_item` 流程打通。
- `read_resource("rustdoc://doc_probe/Demo")` 返回 markdown。
- 用 MCP Inspector 或 rmcp client 联调通过。

## M6 — 打磨

**交付物**
- 检索排序 / 分页 / 模糊匹配优化。
- 可选项：`--granularity`、`--max-doc-bytes`、`--include-auto-impls`。
- 错误信息完善、README、发布配置（license、Cargo 元数据）。

**验收标准**
- 在真实大 crate（如 `serde` 或 `tokio`）上跑通全流程，导出与检索性能可接受。
- 文档与代码一致。

## 工作约定

- 每阶段完成即更新本文件的「状态总览」。
- 阶段内的设计变更同步回 `design.md`；需求变更同步回 `requirements.md`。
- 每阶段结束应保证 `cargo test` 全绿、`cargo clippy` 无警告。
