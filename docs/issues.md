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
| [E-3.6](#e-36) | P2 | 工具与资源/prompt 的错误模型不一致 | ⛔ 不修复 |
| [E-3.7](#e-37) | P2 | `list_items` 的 `module` 过滤在重导出 / 带 crate 前缀时静默返回空 | ✅ 已修复 |
| [E-3.8](#e-38) | P2 | crate 根不可查（`get_item` 读不到 crate 总览） | ✅ 已修复 |
| [E-3.9](#e-39) | P2 | 检索排序无语境权重，同名成员 / 变体淹没真结果 | ✅ 已修复 |
| [E-3.10](#e-310) | P2 | `get_item` 的 `max_bytes` 盲截，切断点落在正文 / 示例中间 | ✅ 已修复 |
| [E-3.11](#e-311) | P3 | 工具集合键名不一致（`items` vs `hits`） | ⛔ 不修复 |
| [E-3.12](#e-312) | P3 | skill / server instructions 缺参数速查，探测成本高 | ✅ 已修复 |
| [E-4.2](#e-42) | P3 | `Option<T>` 参数生成 `type:[T,"null"]` 联合类型 | ⛔ 不修复 |
| [E-4.3](#e-43) | P3 | 枚举候选值未暴露为 schema `enum` | ✅ 已修复 |
| [E-5.1](#e-51) | P3 | 默认写入全局共享库且无上限 / GC | ✅ 已修复 |

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

**结论**：⛔ 不修复。工具的业务失败用 `isError`、资源 / prompt 的无效请求用 JSON-RPC `error`，是 MCP 协议下的两种既定语义（前者表示"调用已执行但失败"，后者表示"请求无效 / 资源不存在"），并非缺陷。保持现状更贴合协议，只需保证文案可读。

### E-3.7 <a id="e-37"></a>`list_items` 的 `module` 过滤在重导出 / 带 crate 前缀时静默返回空

**现象**：`list_items{crate:"gpui_kit", module:"component"}` 返回 `{"items":[],"total":0}`（`isError:false`）；`module:"gpui_kit::component"` 同样空。而 `module:"io"`（tokio）/ `module:"button"`（gpui_component）正常。实测对话（检索 gpui-kit）中 Agent 连续两次拿到空结果，最终改用工作区 grep 绕过。

**根因**：`matches_module`（`crates/mcp-docs-server/src/project.rs`）按 `item.path[1..]` 的**真实文档模块段**精确匹配；`component` 是 `pub use gpui_component as component` 的**重导出别名**，不产生条目、不进入 `path`，故恒不命中；带 crate 前缀的写法（`gpui_kit::component`）也因首段不匹配而落空。空结果与"确实无此模块"无法区分。

**影响**：Agent 把静默空结果当成否定结论，误判"依赖里没有该模块"。

**修复**：过滤前归一化 `module`（剥离前导 `::`；当首段等于该 crate 名时剥离 crate 前缀）；结果为空且显式指定了 `module` 时，返回 `note` 列出该 crate 可用的一级模块，并提示改用 `search_items` 或正确模块名。

**验证**：`list_items{crate:"gpui_kit", module:"component"}` 返回带 `note` 的空结果（列出 gpui_kit 一级模块）；`list_items{crate:"tokio", module:"tokio::io"}` 正常命中（前缀被剥离）。

### E-3.8 <a id="e-38"></a>crate 根不可查（`get_item` / 资源读不到 crate 总览）

**现象**：`get_item{id:"tokio"}` / `get_item{id:"gpui_kit"}` / `get_item{id:"gpui_component"}` 均 `isError:true 未找到条目`。

**根因**：索引只收录 sidebar-items 的条目，crate 首页（`{crate}/index.html` 的 top-doc）不是条目，`IdIndex::find` 必然落空；资源 `rustdoc://{crate}` 只给条目清单，没有 crate 概览。

**影响**：想拿 crate 总览只能绕道 `list_items(kind=module)` / `module_tree`，且要先 `mcp_get_tool_description` 才知道后者存在。

**修复**：`get_item_blocking` 未命中、且 id 恰好等于某个已索引 crate 名时，回退返回**合成概览**：条目数 + 一级模块清单 + 一级条目清单 + 后续调用指引；同时区分「crate 未索引」与「条目未找到」。

**验证**：`get_item{id:"tokio"}` 返回 tokio 的概览（含 `io` / `task` 等一级模块）；`get_item{id:"tokio::io::trait.AsyncReadExt"}` 行为不变。

### E-3.9 <a id="e-39"></a>检索排序无语境权重，同名成员 / 变体淹没真结果

**现象**：`search_items{query:"Button", crate:"gpui_kit"}` 中真正的组件与无文档的 `accesskit::Role::variant.Button` 同为 `score:100`，约 102 条噪声；`search_items{query:"component", kind:"mod"}` 唯一命中是 one_line 含 "URI component" 的 `http::mod.uri`。

**根因**：`rank`（`crates/mcp-docs-core/src/search.rs`）只按名字 / 路径 / 摘要打分，`ranked_hits` 的排序键为「score → 名字长度 → id」，不区分顶层 vs 成员、有文档 vs 无文档、路径深浅。

**影响**：真结果被名称相同的成员 / 变体挤出前若干条。

**修复**：同分时追加次级键 `has_docs 降序 → 非成员优先 → 路径深度升序 → 名字长度升序 → id 升序`；保持 100/80/60/40/20 分数语义不变（同步更新 `roadmap.md` M9 的排序红线说明）。

**验证**：`search_items{crate:"gpui_kit", query:"Button"}` 中 `button` 模块相关条目排在 `accesskit::Role::variant.Button` 之前；`mcp-docs-core` 检索测试仍全绿。

### E-3.10 <a id="e-310"></a>`get_item` 的 `max_bytes` 盲截，切断点落在正文 / 示例中间

**现象**：`get_item{id:"tokio::io::trait.AsyncReadExt", max_bytes:12000}` 截在 `read_i16` 示例代码中间，后半段内容丢失。

**根因**：`truncate`（`crates/mcp-docs-server/src/project.rs`）按字节前缀硬切，只保证 UTF-8 字符边界，不对齐行 / 分节。

**影响**：截断处割裂正文与示例，调用方看到的"完整"文档实则缺内容。

**修复**：截断点回退到最近的行边界（`\n`，找不到再退回字符边界），并改进提示文案（提示用更大 `max_bytes`、或 `get_item_section` 读剩余分节）。

**验证**：`get_item(..., max_bytes=N)` 的截断处落在整行边界；截断提示含后续读取指引。

### E-3.11 <a id="e-311"></a>工具集合键名不一致（`items` vs `hits`）

**现象**：`list_items` / `batch_get_items` 返回 `items`；`search_items` / `find_by_signature` / `search_docs` 返回 `hits`。

**根因**：两族工具分别手写 JSON / 结构体，未统一键名。

**结论**：⛔ 不修复。list 族 → `items`（列举清单）、search 族 → `hits`（检索命中）各有语义，且在 `skills/mcp-docs/references/tools.md` 已如实记录；统一属破坏性变更，收益不抵复杂度（与 E-3.6 / E-4.1 同理）。

### E-3.12 <a id="e-312"></a>skill / server instructions 缺参数速查，探测成本高

**现象**：实测对话中 Agent 每轮先调 `mcp_get_tool_description` 摸 schema（共 3 次），并在 `kind:"mod"` / `"module"` 之间试探；回答一个「tokio IO 用法」用了 17 次工具调用、8 个 assistant 轮次。

**根因**：`skills/mcp-docs/SKILL.md` 的工具表只写用途不写参数；参数细节在 `references/tools.md` 但不会随技能加载自动注入；`get_info` 的 instructions 仅一句泛化链路。

**修复**：`SKILL.md` 增加紧凑参数速查（关键参数名 + 取值 + 空结果坑）；`DocsServer::get_info` 的 instructions 给出一次到位的推荐链路与常见坑。

**验证**：读 SKILL.md 即可写出正确调用，无需 `mcp_get_tool_description`。

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

**结论**：⛔ 不修复。`type:[T,"null"]` 来自 `schemars` 1.x 对 `Option<T>` 的核心实现（`json_schema_impls/core.rs`），且 rmcp 在内部固定用 `SchemaSettings::draft2020_12()` 生成 schema（`rmcp/src/handler/server/common.rs`），项目侧无法改全局配置；消除它需为每个可选字段手写 JSON Schema，与 E-4.1 同理，收益不抵复杂度。主流客户端接受合法联合 `type`。

### E-4.3 <a id="e-43"></a>枚举候选值未暴露为 schema `enum`

**现象**：`search_items.mode`（`substring|prefix|fuzzy`）在 schema 中是裸 `string`，客户端无法得知合法取值。

**根因**：`SearchItemsParams.mode` 为 `Option<String>`（`crates/mcp-docs-server/src/project.rs`）。

**修复**：`SearchItemsParams.mode` 由 `Option<String>` 改为枚举 `Option<ModeArg>`（`#[serde(rename_all = "lowercase")]` 派生 `substring|prefix|fuzzy`），schema 暴露 `enum`；未知取值不再静默回退为 `substring`，而是报错（与 `parse_kind` 的"不静默忽略"一致）。`kind` 取值多且支持前缀（`fn`）/自然名（`function`），不收敛为 enum。

**验证**：`tools/list` 的 `search_items.mode` schema 变为 `anyOf[$ref ModeArg, null]`，`$defs.ModeArg` 含 `enum: [substring, prefix, fuzzy]`。

## 5. 存储与共享库

### E-5.1 <a id="e-51"></a>默认写入全局共享库且无上限 / GC

**现象**：单项目运行 server 也会把共享库写到平台缓存目录（`default_store_root`）；真实项目首次可达数百 MB，且无上限 / 淘汰，用户无感知。

**根因**：`crates/mcp-docs-server/src/config.rs`（及 CLI）使 `store` 缺省回落到平台缓存目录。

**修复**：server 新增 `--no-store` 开关，关闭后不读也不写全局缓存（`crates/mcp-docs-server/src/config.rs`）。默认仍为平台缓存目录（跨项目复用的既定设计）；尺寸上限 / GC 属运维范畴，暂未实现。

**验证**：`--no-store` 启动后 `list_crates` 正常返回，不再触及共享库。

---

## 记录约定

- 新缺陷追加到所属章节并在「索引」表登记，编号 `E-x.y` 递增；新主题新增章节号。
- 关闭缺陷时更新状态并注明修复提交；若含认知教训，链接到 `lessons.md`。
- 与 `lessons.md` 重叠时只在一处写正文，另一处用相对链接引用。
