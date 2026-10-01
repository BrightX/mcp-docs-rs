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
| M6 | 打磨与真实规模验证 | ✅ 已完成 |
| M7 | 遗留特性补齐（分页 / 导出粒度 / all.html 兜底） | ✅ 已完成 |

状态标记：⬜ 未开始 / 🟡 进行中 / ✅ 已完成 / ⛔ 阻塞。

## M0 — 骨架 + 文档发现

**交付物**
- workspace 骨架：`mcp-docs-core`（纯库）+ `mcp-docs-cli`，根 `Cargo.toml` 统一依赖版本。`mcp-docs-server` 推迟到 M5 建立（避免无用的占位 main）。
- `sidebar.rs`：`parse_sidebar_str` / `parse_sidebar_file`。
- `discover.rs`：`list_crates` + `discover_crate` + `discover_all`（递归 sidebar）。`all.html` 兜底与交叉校验推迟到 M7 的按需增强。
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
- 条目文件 + 成员独立文件按 `target/doc-search/` 结构落盘（fixture 导出 11 个文件；默认 `member` 粒度，`item` 粒度下只写顶层条目）。

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
- 项目根 `README.md`（特性、快速开始、CLI / MCP 用法、文档索引）。
- MCP `get_item` 支持 `max_bytes`（按字符边界截断，避免超大条目撑爆上下文）。
- 清理签名里的 rustdoc 装饰字符（`ⓘ`）。
- 真实规模验证：`serde` + `serde_json` + `tokio(full)` 依赖树（35 个 crate / 1034 顶层条目 / 5526 条目）。
- 修复大规模暴露的两个缺陷：增量复用误收后代成员（lessons #1.11）、成员文件名同名冲突（lessons #1.12）。
- 在根项目自身（123 crate / 19231 条目）上验证并修复三处缺口：属性宏未被索引（#1.14）、宏重定向页导致构建中断（#1.15）、trait 签名混入折叠控件文本（#1.16）。
- 增量导出性能：全量 2m52s → 增量 **2.2s**（预建 HashMap 索引，见 lessons #1.18）；条目 id 加类型标记以保证唯一（#1.17、#1.19）。
- 再次用 MCP Inspector CLI 调试全部 6 个工具与 3 类资源，修复四处缺陷：链接重写对带 title 的链接失效（#1.22）、源码路径未归一化到 `doc_root`（#1.23）、`kind` 过滤器被静默忽略（#4.1）、资源 crate 清单无上限（大 crate 会返回超大 JSON）。
- 工具链升到 rustdoc **1.98.1** 后再做一次 Inspector 全能力复验，又修复三处问题：宏条目二元组格式导致导出**整体中断**（#1.24）、无文档条目的占位 `one_line`（#1.25）、`get_item` / `get_item_source` 不接受省略类型标记的 id（#4.2）。

**验收标准**（均已达成）
- 全量导出 5526 个 md 耗时 46 秒；二次增量 1.0 秒（复用 5523）。
- 条目数 == 落盘文件数 == 5526（无覆盖）。
- `search_items("spawn", crate="tokio")` 命中且无重复；`get_item(..., max_bytes)` 正确截断。
- MCP 全能力（`tools/list`、6 个 `tools/call`、`resources/list`、`resources/templates/list`、`resources/read`）经 Inspector CLI 复验通过。
- 真实规模下 MCP stdio 联调通过。
- rustdoc 1.98.1 下全量导出 18922 条目（114 crate）；Inspector 复验 6 工具 + 3 资源全部通过（含省略类型标记的 id、非法 `kind` 报错、`max_bytes` 截断）。

**未做（记录为按需增强）**
- `--include-auto-impls`（保留 rustdoc 的 synthetic / blanket impl 噪声区块）。
- 发布形态元数据（crates.io 的 `repository` / `keywords` 等）。
- 宏生成的内部方法噪声（如 rmcp `#[tool]` 展开的 `list_crates_tool_attr`）：HTML 结构与普通方法同构，无通用可辨识信号，暂不特殊过滤。
- 服务冷启动重建索引耗时（debug 下约 2 分钟）可能超过 MCP 客户端的初始化超时：可考虑由 CLI 预建索引，或先应答 `initialize` 再后台构建（见 lessons #4.3）。

## M7 — 遗留特性补齐

**交付物**
- 检索 / 列表分页 `offset`：`SearchQuery` 新增 `offset`，新增 `search_page`（返回分页前 `total` 与当前页 `hits`）；MCP `list_items` / `search_items` 与 CLI `search` 均支持。
- 导出粒度 `--granularity`（两档）：`member`（默认，成员单独落盘）/ `item`（只写顶层条目、成员仅内联）；索引记录 `granularity`，增量构建在 schema 或粒度变化时全量重建。
- 发现阶段 `all.html` 交叉校验兜底：新增 `allpage.rs`，补全 `sidebar-items.js` 与目录扫描都遗漏的条目（如未列出的子模块）。
- 索引结构版本递增至 5。

**验收标准**
- `search --offset 2 --limit 2` 与整体结果切片一致且无重叠；`offset` 越界 / `limit=0` 返回空；`total` 与分页无关。
- `export --granularity item` 不写成员文件、`written`==顶层条目数、成员摘要 `file` 指向父文件；切换粒度后增量构建 `reused==0`。
- `all.html` 能补全 sidebar 遗漏的子模块条目，且不产生重复（fixture 仍为 6 项、顺序不变）。

## 工作约定

- 每阶段完成即更新本文件的「状态总览」。
- 阶段内的设计变更同步回 `design.md`；需求变更同步回 `requirements.md`。
- 每阶段结束应保证 `cargo test` 全绿、`cargo clippy` 无警告。
