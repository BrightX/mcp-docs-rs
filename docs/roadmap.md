# 进度计划与里程碑

## 状态总览

| 阶段 | 目标 | 状态 |
|---|---|---|
| M0 | 骨架 + 文档发现 | ✅ 已完成 |
| M1 | 单条目解析 | ✅ 已完成 |
| M2 | Markdown 渲染 + 链接重写 + 降噪 | ✅ 已完成 |
| M3 | 索引 + 检索 | ✅ 已完成 |
| M4 | 缓存 / 增量 / 指纹 | ✅ 已完成 |
| M5 | MCP Server | ✅ 已完成 |
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
- `model.rs` 扩展：`ItemSummary` / `CrateSummary` / `Index`。
- `index.rs`：`build_index` / `write_index` / `load_index`，生成扁平 `index.json`。
- `search.rs`：`search` + `rank`（子串 / 前缀 / 模糊；按 crate、kind 过滤；得分排序）。
- `export` 顺带产出 `index.json`；`--out` 提升为全局参数。
- CLI：`mcp-docs search <query> [--limit] [--mode] [--crate] [--kind]`。
- 测试 7 个（`tests/search.rs`）。

**验收标准**（均已达成）
- 搜 `new` 命中 `doc_probe::Demo::new`（得分 100）。
- 搜 `demo struct` 命中 `Demo`（描述匹配，得分 20）。
- `index.json` 可被 `serde_json` 反序列化，条目数（11）与发现阶段一致。
- 成员条目进入索引，`file` 指向独立 markdown 文件。

## M4 — 缓存 / 增量 / 指纹

**交付物**
- `cache.rs`：`Fingerprint` / `fingerprint_doc_root` / `Meta` / `is_stale` / `path_mtime` / `DocCache`。
- 索引与导出统一进 `index.rs::build`（`BuildOptions` / `BuildReport`）。
- 写出 `meta.json`（指纹 + rustdoc 版本 + schema 版本）；`index.json` 条目带 `src_mtime`。
- `export --incremental`：复用未变化条目（含其成员），跳过重新解析与重复写盘。
- 依赖 `walkdir`；测试依赖 `tempfile`。
- 测试 5 个（`tests/cache.rs`）。

**验收标准**（均已达成）
- 二次增量运行「重新解析 0，复用 11，写 0 文件」。
- 改动单个 html 后仅该页面被重新解析（`parsed == 1`）。
- 指纹稳定；`meta.json` 缺失或指纹变化时判定为过期。
- `DocCache` 二次取用返回同一 `Arc`。

## M5 — MCP Server

**交付物**
- 新建 `crates/mcp-docs-server`（`main.rs` 启动 + `server.rs` 实现），依赖 `rmcp` 3.5 + `tokio`。
- 6 个工具：`list_crates` / `list_items` / `search_items` / `get_item` / `get_item_source` / `rebuild_index`（`#[tool_router]` + `#[tool_handler]`）。
- 资源：`rustdoc://crates`、`rustdoc://{crate}`，模板 `rustdoc://{crate}/{+item}`。
- 启动时加载或重建索引；`get_item` 经 `DocCache` 按需解析（复用 M4 缓存）。
- `ItemSummary` 增加 `html_path`，供按需解析定位源 HTML。
- stdio 传输（`transport-io`）。

**验收标准**（均已达成）
- `initialize` 协商成功，能力含 `tools` + `resources`。
- `tools/list` 返回 6 个工具及其 JSON Schema。
- `tools/call search_items {query:"new"}` 命中 `doc_probe::Demo::new`（得分 100）。
- `resources/read rustdoc://doc_probe/Demo` 返回完整 markdown。

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
