# 进度计划与里程碑

## 状态总览

| 阶段 | 目标 | 状态 |
|---|---|---|
| M0 | 骨架 + 文档发现 | ✅ 已完成 |
| M1 | 单条目解析 | ✅ 已完成 |
| M2 | Markdown 渲染 + 链接重写 + 降噪 | ✅ 已完成 |
| M3 | 索引 + 检索 | ⬜ 未开始 |
| M4 | 缓存 / 增量 / 指纹 | ⬜ 未开始 |
| M5 | MCP Server | ⬜ 未开始 |
| M6 | 打磨 | ⬜ 未开始 |

状态标记：⬜ 未开始 / 🟡 进行中 / ✅ 已完成 / ⛔ 阻塞。

## M0 — 骨架 + 文档发现

**交付物**
- workspace 骨架：`mcp-docs-core`（纯库）+ `mcp-docs-cli`，根 `Cargo.toml` 统一依赖版本。`mcp-docs-server` 推迟到 M5 建立（避免无用的占位 main）。
- `sidebar.rs`：`parse_sidebar_str` / `parse_sidebar_file`。
- `discover.rs`：`list_crates` + `discover_crate` + `discover_all`（递归 sidebar）。`all.html` 兜底与交叉校验经评估价值有限（sidebar 已能完整覆盖），改为 M6 的按需增强。
- CLI：`mcp-docs tree`。
- 测试 fixture：从 `temp/doc_probe/target/doc` 裁剪拷贝到 `tests/fixtures/doc_probe/`（约 99 KB）。
- 集成测试 5 个，全部通过。

**验收标准**
- `mcp-docs tree` 对 fixture 打印出 6 个条目：`inner` / `Demo` / `Kind` / `DoIt` / `free_fn` / `inner::Nested`。
- `parse_sidebar_str` 单元测试通过（含单数 / 复数 key 兼容、未知 key 忽略、缺失赋值报错）。

## M1 — 单条目解析

**交付物**
- `model.rs` 扩展：`DocItem` / `Section` / `SourceRef`。
- `parse.rs`：`parse_item_html`（签名、主文档、分节、方法 / 字段 / 变体 / 关联项），以及 `parse_rustdoc_meta` / `parse_one_line` / `path_to_identity`。
- 依赖：引入 `scraper`（HTML 解析）与 `htmd`（HTML→markdown）。
- CLI：`mcp-docs show <id>`。
- 集成测试 6 个（`tests/parse.rs`）。

**验收标准**（均已达成）
- `Demo` 签名 = `pub struct Demo { pub field: u32, }`。
- `Demo::new` 文档含 "build a demo"；字段 `field` 文档含 "the field"。
- `Kind` 识别出 A / B 两个变体。
- `trait.DoIt.html` 的 `run` 识别为 `TyMethod`（trait 必需方法）。
- 噪声区块（synthetic / blanket impl）默认剥离。

## M2 — Markdown 渲染 + 链接重写 + 降噪

**交付物**
- `markdown.rs`：`render_item` / `render_member_item`，成员按类型分组内联。
- `link.rs`：`resolve_href`（页内锚 / 条目 / 源码 / 外链）+ `html_to_md_relpath`。
- `store.rs`：`encode_fs_name` / `atomic_write` / `item_output_path` / `member_output_path`。
- 降噪：`strip_anchor_links` 去掉 `[§](#锚点)`；代码块用占位符法补 `rust` 语言标注。
- CLI：`mcp-docs export [--out DIR] [--crate NAME]`。
- 测试 5 个（`tests/render.rs`）。

**验收标准**（均已达成）
- 导出的 markdown 不含 "Copy item path"、不含 blanket / synthetic impls、不含 `[§]` 噪声。
- 代码示例带 ```rust 围栏。
- 链接重写为 `.md`（默认 `Relative`；`PlainPath` / `KeepOriginal` 亦实现）。
- `encode_fs_name` 对 `Demo<Bar>` / `CON` / `Demo::new` / 空串的断言通过。
- 条目文件 + 成员独立文件按 `target/doc-search/` 结构落盘（fixture 导出 11 个文件）。

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
- 可选增强：`all.html` 交叉校验兜底（当 `sidebar-items.js` 缺失时补全条目）。
- 可选项：`--granularity`、`--max-doc-bytes`、`--include-auto-impls`。
- 错误信息完善、README、发布配置（license、Cargo 元数据）。

**验收标准**
- 在真实大 crate（如 `serde` 或 `tokio`）上跑通全流程，导出与检索性能可接受。
- 文档与代码一致。

## 工作约定

- 每阶段完成即更新本文件的「状态总览」。
- 阶段内的设计变更同步回 `design.md`；需求变更同步回 `requirements.md`。
- 每阶段结束应保证 `cargo test` 全绿、`cargo clippy` 无警告。
