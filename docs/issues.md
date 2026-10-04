# 缺陷跟踪（Issues）

记录**功能缺陷**——代码行为与预期或文档声明不符的问题，跟踪到修复关闭。

与 [lessons.md](lessons.md) 的分工：

- `lessons.md`：开发过程中**因认知偏差踩的坑**（对 rustdoc 结构、工具链、协议的错误假设），修复后作为知识保留。
- `issues.md`：**功能缺陷**，按状态跟踪。同一问题若既含认知教训又需跟踪修复，教训记 `lessons.md`、修复进度记本文件，互相链接，不重复正文。

**编号格式 `E-x.y`**：`x` = 章节号（与主题绑定），`y` = 该章节内条目序号。编号只追加、不复用；`x` 拆分后不变（参照 [conventions.md](conventions.md) §6.5）。

**严重度**：`P1` 功能失效（工具不可用/结果错误）；`P2` 语义或一致性（结果误导、输出不一致）；`P3` 协议或可移植性（告警、兼容性）。

**状态**：⬜ 待修复 / 🔧 修复中 / ✅ 已修复 / ⛔ 不修复。

## 目录

1. [解析与渲染](#1-解析与渲染)
2. [导航与关系](#2-导航与关系)
3. [MCP 接口](#3-mcp-接口)
4. [协议与可移植性](#4-协议与可移植性)
5. [存储与共享库](#5-存储与共享库)

## 索引

| 编号 | 严重度 | 标题 | 状态 |
|---|---|---|---|
| [E-1.1](#e-11) | P1 | `get_item_section` 拿不到文档分节（examples/panics 等） | ✅ 已修复 |
| [E-1.2](#e-12) | P2 | `get_examples` 把签名块当示例返回 | ✅ 已修复 |
| [E-2.1](#e-21) | P1 | `get_trait_implementors` 对 re-export 的 trait 失效 | ✅ 已修复 |
| [E-2.2](#e-22) | P2 | `get_related_items` 对成员条目返回全空 | ✅ 已修复 |
| [E-3.1](#e-31) | P2 | `index_status.building` 恒为 `true` | ✅ 已修复 |
| [E-3.2](#e-32) | P2 | `get_item_json.docs_md` 未重写链接 | ✅ 已修复 |
| [E-3.3](#e-33) | P2 | `list_crates` 的 `version` 恒为 `null` | ✅ 已修复 |
| [E-4.1](#e-41) | P3 | JSON Schema 使用非标准 `format`（uint/uint32/uint64） | ⛔ 不修复 |
| [E-3.4](#e-34) | P1 | `search_docs` 在未导出正文时静默返回空 | ✅ 已修复 |
| [E-3.5](#e-35) | P2 | 资源条目 URI 的 item 段写法易错 | ✅ 已修复 |
| [E-3.6](#e-36) | P2 | 工具与资源/prompt 的错误模型不一致 | ⬜ 待修复 |
| [E-4.2](#e-42) | P3 | `Option<T>` 参数生成 `type:[T,"null"]` 联合类型 | ⬜ 待修复 |
| [E-4.3](#e-43) | P3 | 枚举候选值未暴露为 schema `enum` | ⬜ 待修复 |
| [E-5.1](#e-51) | P3 | 默认写入全局共享库且无上限 / GC | ⬜ 待修复 |

---

## 1. 解析与渲染

### E-1.1 <a id="e-11"></a>`get_item_section` 拿不到文档分节（examples / panics 等）

**现象**：`get_item_json.sections` 恒为空数组；`get_item_section{section:"examples"}` 报「没有分节」。实测 `anstream::macro.eprint`（导出 md 含 `## Panics` / `## Examples`）、`futures::future::fn.select_all` 均如此；`tokio::runtime::struct.Runtime` 仅返回 `["implementations","trait-implementations"]`。

**复现**：
```
tools/call get_item_section {"id":"anstream::macro.eprint","section":"examples"}
→ isError:true  条目 `anstream::macro.eprint` 没有分节 `examples`
```

**根因**：`collect_sections` 只收集 `main-content` **直接子级**中的 `h2.section-header`（`crates/mcp-docs-core/src/parse.rs`，`is_section_header`）。而 rustdoc 1.98 的用户文档分节是 `<h2 id="panics">`（**无 class**）且位于 `details.top-doc .docblock` **内部**，两条都不满足；带 `section-header` 的只有 `implementations` / `trait-implementations` 这类结构分节。另 `Section.body_md` 恒为 `String::new()`，文档分节即便被收集也拿不到正文。测试 `tests/parse.rs` 只断言了带 class 的 `fields` / `implementations`，未覆盖无 class 的文档分节，故长期未暴露。

**影响**：`get_item_section` 对最常用的 `examples` / `panics` 场景完全不可用。

**修复**：分节识别扩展到 docblock 内的 `h2[id]`（无 class 也算），并按标题从 `docs_md` 切出该节正文填充 `body_md`。

**验证**：Inspector CLI 复验 `anstream::macro.eprint`：`get_item_section{section:"examples"}` 返回 rust 代码块、`{section:"panics"}` 返回正文；`get_item_json.sections` 为 `["panics","examples"]`（均带 `body_md`）。

### E-1.2 <a id="e-12"></a>`get_examples` 把签名块当示例返回

**现象**：`get_examples{id:"tokio::task::fn.spawn"}` 的第一个 example 是签名 `pub fn spawn<F>(...)`。

**根因**：`markdown.rs::extract_code_blocks` 抽出条目 markdown 中所有 ```rust 围栏，未排除签名块。

**影响**：示例列表混入非示例内容。

**修复**：`get_examples` 改为只从条目文档正文与成员文档抽取示例，排除签名代码块。

**验证**：`get_examples{id:"tokio::task::fn.spawn"}` 首个示例为文档中的真实示例，不以 `pub fn` 开头。

## 2. 导航与关系

### E-2.1 <a id="e-21"></a>`get_trait_implementors` 对 re-export 的 trait 失效

**现象**：返回 `count:0`，但实现清单文件确实存在且非空。

**复现**：
```
tools/call get_trait_implementors {"id":"bitflags::trait.Flags"}          → count 0（文件 trait.impl/bitflags/traits/trait.Flags.js 有内容）
tools/call get_trait_implementors {"id":"serde::ser::trait.Serialize"}    → count 0（实现在 serde_core）
```

**根因**：`trait_impl_rel_path` 直接用条目 `html_path` 推导 impl 路径（`crates/mcp-docs-core/src/nav.rs`），假设实现清单与 trait 页同目录。但 rustdoc 把实现清单放在 **trait 定义模块**下：`bitflags::Flags` 正文页在 `bitflags/trait.Flags.html`，实现清单在 `trait.impl/bitflags/traits/trait.Flags.js`；`serde` 的实现页进一步被拆到 `serde_core`。此外 `server.rs` 用 `unwrap_or_default()` 静默吞掉读文件失败，把「路径推导错误」与「确实无实现」混同。

**影响**：对 rustdoc 中大量 re-export 的 trait，实现者查询恒为空且无任何提示。

**修复**：路径推导改为「精确路径优先，否则在 `trait.impl` 下按文件名回退搜索」，并区分「未找到文件」与「文件为空」。

**验证**：Inspector CLI 复验 `bitflags::trait.Flags` → `count:2`；`serde::ser::trait.Serialize`（实现被拆到 `serde_core`）→ `count:215`。

### E-2.2 <a id="e-22"></a>`get_related_items` 对成员条目返回全空

**现象**：`tokio::runtime::struct.Runtime::method.spawn` 返回 `siblings:[] children:[]`，同 struct 其余 8 个方法既不在 `siblings` 也不在 `children`。

**根因**：`related_items` 中 `siblings` 只收「同模块顶层条目」（`nav.rs`），`children` 只收 `parent_id == 自身`；成员条目的兄弟成员无对应字段。

**影响**：关系导航对成员条目形同虚设。

**修复**：`related_items` 中成员条目的 `siblings` 改为「同父条目下的其它成员」，顶层条目保持「同模块顶层条目」。

**验证**：`get_related_items{id:"tokio::runtime::struct.Runtime::method.spawn"}` 返回 8 个兄弟方法。

## 3. MCP 接口

### E-3.1 <a id="e-31"></a>`index_status.building` 恒为 `true`

**现象**：`ready:true`、`stale:false`、`item_count:18922` 时 `building` 仍为 `true`（多次复查一致）。

**根因**：`server.rs` 的后台刷新任务**无条件**先置 `building=true`，即便刷新是 no-op（指纹未变）；`index_status` 在启动后立即查询时读到该瞬时值。

**影响**：误导 Agent 认为索引仍在构建。

**修复**：把 `building` 置位移入 `refresh_index` 的「确实需要构建」分支；no-op 刷新不再置位。

**验证**：`index_status` 在已有索引且不 stale 时返回 `building:false`。

### E-3.2 <a id="e-32"></a>`get_item_json.docs_md` 未重写链接

**现象**：`docs_md` 中链接为 `[`JoinHandle`](struct.JoinHandle.html "…")`，而 `get_item` / 导出 md 为 `.md`。

**根因**：`get_item_json_blocking` 直接 `item.docs_md.clone()`，未过 `rewrite_links`。

**影响**：字段名 `docs_md` 却给 HTML 链接，与其它输出不一致，客户端需自行处理。

**修复**：`get_item_json` 的 `docs_md`、以及 `get_item_json` / `get_item_section` 的分节 `body_md` 均过 `rewrite_links`。

**验证**：`get_item_json{id:"tokio::task::fn.spawn"}` 的 `docs_md` 链接为 `.md`，无 `.html`。

### E-3.3 <a id="e-33"></a>`list_crates` 的 `version` 恒为 `null`

**根因**：rustdoc 产物不含 crate 版本，`CrateSummary.version` 无数据来源（`model.rs`）。

**影响**：字段无信息量；Agent 无法据此判断 crate 版本。

**修复**：移除 `CrateSummary.version`（rustdoc 产物无版本来源，保留恒 `null` 只会误导），`INDEX_SCHEMA_VERSION` 6 → 7。

**验证**：`list_crates` 输出的 crate 项仅含 `name` / `item_count`。

### E-3.4 <a id="e-34"></a>`search_docs` 在未导出正文时静默返回空

**现象**：不先运行 `mcp-docs export` 时，`search_docs` 永远返回 `{"hits":[],"returned":0}`，无任何提示。

**复现**：
```
# 仅启动 server（后台为内存构建，不写 markdown）
tools/call search_docs {"query":"Demo"}  → hits:[]
# 运行 mcp-docs export 生成正文档后
tools/call search_docs {"query":"Demo"}  → 命中
```

**根因**：`search_docs` 扫描 `out_dir` 下已导出的 markdown，而 server 后台构建 `write_markdown=false` 不落盘 md；`search_docs_blocking` 对读文件失败直接 `continue`（`crates/mcp-docs-server/src/project.rs`）。

**影响**：Agent 会把「正文缺失」误判为「无匹配内容」，该工具在纯 server 场景形同失效。

**修复**：扫描范围内一个可读 md 都没有（`read_ok == 0 && candidates > 0`）时返回明确提示，指引先 `export` 或改用 `search_items` / `get_item`；正常无匹配仍返回空 `hits`。

**验证**：构造仅含索引、无 md 的输出目录 → `search_docs` 返回提示；`export` 后同查询返回命中。

### E-3.5 <a id="e-35"></a>资源条目 URI 的 item 段写法易错

**现象**：`rustdoc://doc_probe/struct/Demo` 报「未找到条目 `doc_probe::struct::Demo`」，需写成 `rustdoc://doc_probe/struct.Demo` 才命中。

**根因**：条目 id 形如 `crate::…::kind.name`，URI 把 item 段里的 `/` 一律映射为 `::`，但 kind 与 name 之间的 `.` 必须保留；文档示例 `tokio/task/spawn`（恰好是省略 kind 的 id）掩盖了该细节（`crates/mcp-docs-server/src/project.rs::render_uri_item`）。

**影响**：客户端按直觉把 `::` 全换成 `/` 时命中失败。

**修复**：`render_uri_item` 对 item 段依次尝试多种归一化（`/`→`::`、最后一个 `/`→`.`、`/`→`.`），命中即用。

**验证**：`rustdoc://doc_probe/struct/Demo`（容错）、`rustdoc://doc_probe/struct.Demo`（标准）、`rustdoc://doc_probe/inner/struct.Nested`（模块斜杠）均能命中。

### E-3.6 <a id="e-36"></a>工具与资源/prompt 的错误模型不一致

**现象**：工具业务错误返回 `isError:true` + 可读文本；资源 / prompt 错误走 JSON-RPC `error`（如读不到条目时返回 `{"error":...}`）。

**根因**：工具用 `Result<String,String>`（由 rmcp 转为 `isError`），资源 / prompt 用 `McpError`（`crates/mcp-docs-server/src/server.rs`）。

**影响**：同一类「未找到」错误在两类入口表现不同，Agent 处理方式不一致。

**建议**：评估统一策略（资源「未找到」改为可读内容，或统一错误文案）。

## 4. 协议与可移植性

### E-4.1 <a id="e-41"></a>JSON Schema 使用非标准 `format`（uint / uint32 / uint64）

**现象**：Inspector `tools/list` 报告 `Schema portability: 0 errors, 33 warnings across 13 tools`，全部为 `unknown format "uint"/"uint32"/"uint64" ignored`。

**根因**：rmcp / schemars 为 `usize` / `u64` / `u32` 生成 `format: "uint*"`（`tools/list` 输出）。

**影响**：严格客户端可能拒绝或忽略这些约束。

**结论**：⛔ 不修复。`format: uint*` 是上游 `schemars` 对整数的既定输出（OpenAPI 风格），非本项目逻辑缺陷；消除它需为每个数值字段手写 JSON Schema，收益不抵复杂度。Inspector 报告为 `0 errors, 33 warnings`，主流客户端忽略未知 `format`。若未来接入严格校验客户端再处理。

### E-4.2 <a id="e-42"></a>`Option<T>` 参数生成 `type:[T,"null"]` 联合类型

**现象**：Inspector `tools/list --strict` 报告 50 处 `type` 为数组（`["string","null"]` / `["integer","null"]`），覆盖各工具的 `project` 等可选参数。

**根因**：rmcp / schemars（1.2.2）对 `Option<T>` 生成含 `null` 的联合 `type`（`crates/mcp-docs-server/src/project.rs` 各 `*Params`）。

**影响**：把工具 schema 映射到单一 `type` 方言的客户端（如 Gemini function declarations / OpenAPI 子集）可能拒绝该工具或丢约束。与 E-4.1（非标准 `format`）同源，但可独立处理。

**建议**：让可选参数生成 `anyOf`，或"非 required 的单一类型"（缺省即 None）。

### E-4.3 <a id="e-43"></a>枚举候选值未暴露为 schema `enum`

**现象**：`search_items.mode`（`substring|prefix|fuzzy`）在 schema 中是裸 `string`，客户端无法得知合法取值。

**根因**：`SearchItemsParams.mode` 为 `Option<String>`（`crates/mcp-docs-server/src/project.rs`）。

**建议**：为 `mode` 生成 schema `enum`；`kind` 取值多且支持前缀（`fn`）/自然名（`function`），暂不收敛为 enum，靠描述提示。

## 5. 存储与共享库

### E-5.1 <a id="e-51"></a>默认写入全局共享库且无上限 / GC

**现象**：单项目运行 server 也会把共享库写到平台缓存目录（`default_store_root`）；真实项目首次可达数百 MB，且无上限 / 淘汰，用户无感知。

**根因**：`crates/mcp-docs-server/src/config.rs`（及 CLI）使 `store` 缺省回落到平台缓存目录。

**建议**：文档提示；或增加 `--no-store` / 尺寸上限 / 统计输出。

---

## 记录约定

- 新缺陷追加到所属章节并在「索引」表登记，编号 `E-x.y` 递增；新主题新增章节号。
- 关闭缺陷时更新状态并注明修复提交；若含认知教训，链接到 `lessons.md`。
- 与 `lessons.md` 重叠时只在一处写正文，另一处用相对链接引用。
