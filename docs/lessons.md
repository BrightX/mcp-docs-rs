# 错题集

记录本项目踩过的坑与犯过的错，避免重复犯错。

**编号格式 `#x.y`**：`x` = 章节号，`y` = 该章节内条目序号（如 `#1.3`）。新增条目追加到所属章节末尾，**编号只追加、不复用**。

**内容说明**：① 开工前通过实测已确认的坑（预填，开发时直接规避）；② 开发过程中实际犯过的错。两者统一编号。

## 目录

1. [rustdoc HTML 解析](#1-rustdoc-html-解析)
2. [文件系统与路径](#2-文件系统与路径)
3. [索引、缓存与增量](#3-索引缓存与增量)
4. [MCP 接口](#4-mcp-接口)
5. [工程、依赖与工具链](#5-工程依赖与工具链)

---

## 1. rustdoc HTML 解析

### #1.1 不要依赖 `search.index`

**错**：以为 `target/doc/search-index.js`（或新版 `search.index/`）是条目索引，可直接解析。
**对**：它是 rustdoc 的定制二进制压缩格式（`root.js` + `rr_()`/`rd_()` + base64 分片），解析成本极高且随版本变动。索引应基于 `crates.js` + 各模块 `sidebar-items.js`（都是可 JSON 化的单行 JS），`all.html` 作兜底与交叉校验。
**相关**：`design.md` §1

### #1.2 方法/关联项的真实 DOM 结构

**错**：以为方法是 `<h3 id="method.new">` 直接挂在 `<h2>` 下。
**对**：真实结构是嵌套的 —— `h2#implementations` → `div#implementations-list` → `section#impl-Demo.impl` → `.impl-items` → `section#method.new.method`，**签名在 `<h4 class="code-header">`**，文档是其后继兄弟 `.docblock`。trait 必需方法锚点是 `#tymethod.run`，关联常数/关联类型是 `#associatedconstant.*` / `#associatedtype.*`。
**相关**：`design.md` §1、§5.2

### #1.3 主文档不是「第一个 `.docblock`」

**错**：假设 `section#main-content` 下第一个 `.docblock` 就是条目主文档。
**对**：主文档包在 `<details class="toggle top-doc"><div class="docblock">` 中。必须**按直接子节点顺序做状态机**切分：`top-doc` / 首个 docblock → `docs_md`；遇 `h2.section-header` 开启新区块。
**相关**：`design.md` §5.2

### #1.4 `blanket` / `synthetic` impls 是体积噪声

**错**：把 `#synthetic-implementations`（"Auto Trait Implementations"）和 `#blanket-implementations` 原样转换。
**对**：这两块在一个 21 KB 的 struct 页里占绝大部分体积，且对理解 API 无用。**默认剥离**（`include_auto_impls=true` 时才保留）。
**相关**：`design.md` §1、§5.2

### #1.5 字段是 `<span>` 不是 `<section>`

**错**：按 `<section id="structfield.field">` 找字段。
**对**：字段是 `<span id="structfield.field" class="structfield section-header">`，其后跟 `.docblock`。
**相关**：`design.md` §1

### #1.6 分节标题必须限定 `h2.section-header`

**错**：只用「`class` 含 `section-header`」判断分节标题。
**对**：字段的 `span` 也带 `section-header` class（见 #1.5），必须同时限定标签名为 `h2`，否则字段会被误判成新分节。
**相关**：`parse.rs::is_section_header`

### #1.7 成员文档有两种位置

**错**：统一用「紧随其后的兄弟 `.docblock`」找成员文档。
**对**：方法 / 关联项的文档在**最近祖先 `details` 内**（`details.method-toggle > .docblock`）；字段 / 变体的文档才是**紧随其后**。实现分两条路径：先向上找 `details`，找不到再取下一个兄弟。
**相关**：`parse.rs::find_member_doc`

### #1.8 移除 `§` 锚点要走输出层，别做 HTML 片段匹配

**错**：用 scraper 选中 `a.anchor` 后 `html.replace(&anchor.html(), "")` 删除；或试图用 htmd 的 `add_handler(vec!["a"], …)` 拦截。
**对**：`anchor.html()` 的序列化结果与 `docblock.inner_html()` 中的对应片段**并不逐字一致**，字符串替换会静默失效；htmd 内置的 `a` 处理器是带 `append` 的有状态实现（`AnchorElementHandler`），函数式 handler 拦截也未生效。最终改为在 markdown 输出层删除固定形态的 `[§](#锚点)` 文本，简单可靠。
**相关**：`parse.rs::strip_anchor_links`

### #1.9 代码块语言标注用「占位符」法

**错**：指望 htmd 自动为 rustdoc 代码块补语言标注（实际输出的是无语言围栏）。
**对**：转换前用占位符抽出 `pre.rust` / `pre.rust-example-rendered`（按 class 判断语言），转换后再还原为 ```rust 围栏；代码内容取 `pre.text()`，避免高亮 `<span>` 干扰。
**相关**：`parse.rs::docblock_to_md`

### #1.10 增量复用成员不能只看 id 前缀

**错**：复用未变化条目的成员时，用 `summary.id.starts_with("父id::")` 收集。
**对**：子模块条目的 id 同样带该前缀 —— 模块 `doc_probe::inner` 的前缀会匹配到子条目 `doc_probe::inner::Nested`，而 `Nested` 本身也是独立条目，于是被重复计入（条目数从 11 变 12）。必须再加「是成员类型」的约束（`ItemKind::is_member()`）。
**相关**：`index.rs` 的增量复用分支

### #1.11 增量复用成员要用 `path` 精确匹配，不能用 id 前缀

**错**：#1.10 的修复加了 `is_member()` 约束，但仍按 `id` 前缀收成员。
**对**：真实规模（tokio 依赖树）下仍出错 —— 模块 `tokio::runtime` 的前缀会把**后代模块的成员** `tokio::runtime::Handle::spawn` 也收进来，条目数从 5526 涨到 8259（并导致检索结果重复）。正确做法是用成员的 `path`（含 crate、不含自身）精确等于父条目 id 来判断，因为成员条目的 `path` 记录的就是"它挂在哪"。
**相关**：`index.rs` 的增量复用分支

### #1.12 成员文件名必须带成员类型

**错**：成员文件命名为 `{父stem}.{成员名}.md`。
**对**：同一类型下字段与方法可能同名 —— `syn::LitBool` 同时有字段 `value` 和方法 `value()`，两者都生成 `struct.LitBool.value.md` 而互相覆盖（真实规模下 5526 个条目只落盘 5519 个文件）。命名改为 `{父stem}.{成员类型}.{成员名}.md`。
**相关**：`store.rs::member_output_path`

### #1.13 签名里要清理 `ⓘ` 装饰字符

**错**：直接把 `pre.rust.item-decl` 的文本当作签名。
**对**：rustdoc 会在签名中插入可点击的提示图标（`ⓘ`，用于 portability / unstable 提示），文本形如 `JoinHandle<F::Output> ⓘwhere F: …`。需要过滤该字符，否则签名会混入噪声。
**相关**：`parse.rs::clean_signature`

### #1.14 属性宏不在 `sidebar-items.js` 里

**错**：只从 `sidebar-items.js` 收集条目。
**对**：属性宏（`#[tool_router]`、`#[tool_handler]` 等）对应 `attr.*.html`，**不出现在 sidebar 中**，于是完全检索不到（真实项目里 `search tool_router` 返回 0 条）。修复方式是扫描目录、把 `{前缀}.{名字}.html` 形式且 sidebar 未列出的条目补进来。
**相关**：`discover.rs::scan_directory`

### #1.15 宏重定向页没有正文，解析失败必须容错

**错**：`build` 里对每个条目直接 `parse_item_html(...)?`，一处失败就中断整个导出。
**对**：rustdoc 会为宏重导出生成 `macro.eprint!.html` 这类**重定向页**（约 340 字节，`http-equiv="refresh"`，无 `section#main-content`）。真实项目里这类页面有 141 个，会让导出整体失败。应改为「解析失败则跳过该条目并记录」，不影响其余条目。
**相关**：`index.rs`（`BuildReport::skipped`）

### #1.16 trait 签名里混入折叠控件文本

**错**：签名直接取 `pre.rust.item-decl` 的 `text()`。
**对**：rustdoc 把 trait 的方法列表折叠在 `<details>` 里，其 `<summary>` 文本（如 `Show 29 methods`）会混进签名。需要遍历文本时跳过 `details` 子树；方法本身已作为成员单独列出，无需重复。
**相关**：`parse.rs::signature_text`

### #1.17 条目 id 必须带类型标记，否则同名不同类型会冲突

**错**：id 直接用 `{路径}::{名字}`。
**对**：同名但不同类型的条目非常常见 —— `serde::Deserialize` / `serde::Serialize` / `schemars::JsonSchema` 既是 trait 又是 derive 宏，`tracing::event` 等宏与同名条目同理。真实项目里产生 48 个 id 冲突，进而导致增量复用互相覆盖、条目数虚增。id 改为 `{路径}::{类型标记}.{名字}`（与 rustdoc 的 `trait.Deserialize.html` / `derive.Deserialize.html` 严格对应）；成员的父 id 也随之带上标记。
**相关**：`parse.rs`、`discover.rs`、`model.rs::kind_tag`

### #1.18 增量复用必须用索引查找，不能线性扫描

**错**：对每个条目 `previous_items.iter().find(...)` 查旧索引，再 `iter().filter(...)` 收成员。
**对**：真实规模下这是 O(n×m) —— 5077 个条目 × 19231 条旧数据 ≈ 1 亿次字符串比较，增量耗时 1m1s。改为预建 `HashMap`（父按 id、成员按 `parent_id` 分组）后降到 **2.2 秒**。
**相关**：`index.rs::build`

### #1.19 成员归属要用显式的 `parent_id` 字段

**错**：#1.11 用「成员的 `path` 拼接结果 == 父 id」判断归属。
**对**：父 id 加上类型标记后（#1.17），`path` 拼接结果（`doc_probe::Demo`）不再等于父 id（`doc_probe::struct.Demo`），成员复用静默失效、条目数对不上。应给 `ItemSummary` 加 `parent_id` 字段显式记录归属，不依赖字符串拼接的巧合。
**相关**：`model.rs::ItemSummary::parent_id`、`index.rs`

### #1.20 影响索引内容的改动必须递增 `schema_version`

**错**：改了 id 格式（加类型标记）、增删了索引字段，却没动 `INDEX_SCHEMA_VERSION`。
**对**：`is_stale` 只比较**产物指纹**（文件数 / mtime / 大小）与 schema 版本。解析逻辑变了但 HTML 没变时，指纹相同 → 判定"不过期" → **继续复用按旧逻辑生成的索引**。实际表现是「属性宏已支持，但 `search tool_router` 仍返回 0 条」。凡是影响索引内容的改动（字段增删、id 格式、解析行为）都要递增该常量。
**相关**：`model.rs::INDEX_SCHEMA_VERSION`

### #1.21 改了 core 必须重新编译 server 二进制

**错**：改动 core 后只跑了 `cargo test` / `cargo run -p mcp-docs-cli`，没重建 `mcp-docs-server`。
**对**：旧二进制与新二进制会**交替覆盖同一份索引** —— CLI（新）写出 schema 2 的索引，旧 server 启动时看到 `meta.schema_version(2) != 自己认为的 1`，判定过期 → 用**旧解析逻辑**全量重建（实测耗时 1m53s）并覆盖索引，属性宏又消失了。改 core 后应用 `cargo build --workspace` 确保所有二进制同步。
**相关**：`crates/mcp-docs-server/`、`crates/mcp-docs-cli/`

### #1.22 链接重写要解析链接目标，不能用 `.html)` 字符串替换

**错**：`LinkStyle::Relative` 用 `markdown.replace(".html)", ".md)")` 重写链接。
**对**：htmd 会把 rustdoc 的 `<a title="...">` 转成**带 title 的 markdown 链接** —— 形如 `` [`JoinHandle`](struct.JoinHandle.html "struct tokio::task::JoinHandle") ``，`.html` 后面跟的是空格 + 引号，`".html)"` 与 `".html#"` 都匹配不上，链接**静默保持 `.html`**，导出的文件树里点不开。正确做法是扫描 `](目标)`、只重写**目标里的路径部分**（保留 title 与锚点），顺带避免误改正文里出现的 `.html` 字样。同一问题也影响 `one_line` 摘要（它取自 rustdoc 的 `meta description`，同样是 markdown 形式），故摘要也要过一遍重写。
**相关**：`markdown.rs::rewrite_links`、`index.rs` 的 `one_line` 生成

### #1.23 源码路径要归一化到 `doc_root`

**错**：`get_item_source` 直接返回页面里的 `href` 原值，如 `../../src/tokio/task/spawn.rs.html`。
**对**：这是**相对当前 HTML 页面**的路径，换个页面就失效，调用方无法据此打开文件。应复用 `link::resolve_href` 归一化成**相对 `doc_root`** 的规范路径（`src/tokio/task/spawn.rs.html`），调用方（MCP server）再拼上 `--doc-dir` 得到绝对路径。
**相关**：`parse.rs::parse_source_href`、`server.rs::get_item_source`

### #1.24 rustdoc 1.98 的宏条目变成了二元组

**错**：按 `BTreeMap<String, Vec<String>>` 反序列化 `sidebar-items.js`（1.95 的实测形态）。
**对**：rustdoc 1.98 起，`macro` 组的条目从纯字符串变成 `[名字, 标志]` 二元组 —— `"macro":[["eprint",1],["println",1]]`，其余类型仍是字符串。旧解析直接 `JSON 解析失败：invalid type: sequence, expected a string`，**整个导出中断**（真实规模下 114 个 crate 全部失败）。改为按 `serde_json::Value` 解析、只取名字、丢弃标志位。注意同样的宏条目会同时存在 `macro.eprint.html`（正文）与 `macro.eprint!.html`（重定向页），由 sidebar 给出无 `!` 的正名后即可命中正文页。
**相关**：`sidebar.rs::sidebar_entry_name`

### #1.25 无文档条目的 `one_line` 是 rustdoc 占位文本

**错**：`one_line` 无条件取页面的 `<meta name="description">`。
**对**：没有文档注释时，rustdoc 会填入自动生成的占位描述（`API documentation for the Rust \`X\` struct in crate \`Y\`.`），于是出现 `has_docs:false` 却带一段"摘要"的条目 —— 真实规模 18922 条里有 1586 条。应只在 `docs_md` 存在时才生成 `one_line`（两者实测 100% 对应）。该改动影响索引内容，需递增 `INDEX_SCHEMA_VERSION`（#1.20）。
**相关**：`index.rs`

## 2. 文件系统与路径

### #2.1 文件名绝不能用 `::`

**错**：把 `ItemId`（如 `Demo::new`）直接当文件名。
**对**：Windows 文件名不允许 `:`。**id 用 `::`，落盘一律用 `.`**：`struct.Demo.new.md`。`::` 只存在于 `index.json` 的 `id` 字段。
**相关**：`design.md` §6

### #2.2 Windows 保留名与非法字符

**错**：直接用类型名做文件名。
**对**：清洗 `< > : " / \ | ? *` → `_`；Windows 保留名（`CON/PRN/AUX/NUL/COM1…`）加 `_` 前缀；泛型名（`Foo<Bar>`）含 `<>`，超 200 字节时 `name-hash8` 截断；去尾部 `.` / 空格。
**相关**：`design.md` §6 `encode_fs_name`

## 3. 索引、缓存与增量

### #3.1 切换导出粒度必须让增量失效

**错**：`build` 增量时只要 `index.json` 存在就复用它。
**对**：导出粒度（`member` / `item`）会改变成员摘要的 `file`（指向成员文件还是父文件），而增量复用是**整条复用旧的 `ItemSummary`**；切换粒度后复用旧索引会得到与当前产物不一致的 `file`，且 `item→member` 方向不会补写成员文件。修复：加载旧索引时校验 `schema_version` 与 `granularity`，任一不符即全量重建。顺带修掉了「CLI `--incremental` 从不校验 schema」的隐患（此前只有 server 的 `is_stale` 校验）。
**相关**：`index.rs::build`（增量守卫）、`model.rs::Granularity`

## 4. MCP 接口

### #4.1 过滤器参数无法识别时必须报错，不能静默忽略

**错**：`list_items` / `search_items` 的 `kind` 用 `ItemKind::from_file_prefix(value)` 解析，取不到就当作"不限定"。
**对**：`from_file_prefix` 只认**文件名前缀**（`fn` / `struct`），而工具描述里推荐的 `method`、以及更自然的 `function` 都不在其中；传这些值时过滤器**被静默忽略**，返回的是**全部类型**——Agent 会以为已过滤，得出错误结论。且 `from_file_prefix` 根本不含成员类型（`method` / `field` / `variant`）。修复：新增 `ItemKind::parse_input`（前缀 + 复数 + 自然名，自然名走 serde 反序列化，与 `index.json` 的 `kind` 同源），无法识别时**返回错误**。
**相关**：`model.rs::ItemKind::parse_input`、`server.rs::parse_kind`

### #4.2 省略类型标记的 id 必须能被 `get_item` / `get_item_source` 接受

**错**：这两个工具只做 `item.id.0 == id` 精确匹配，而 `read_uri`（资源读取）用 `strip_kind_tags` 接受省略类型标记的写法。
**对**：条目 id 带类型标记（`tokio::task::fn.spawn`），但工具描述里的示例、以及人的直觉都是省略形式（`tokio::task::spawn`）——两者不一致时 Agent 只会拿到 `未找到条目`。抽出 `find_summary`（精确优先、未中再退化匹配），让 `get_item` / `get_item_source` / 资源读取共用同一逻辑。
**相关**：`server.rs::find_summary`

### #4.3 MCP Inspector CLI 不转发服务自身的命令行参数

**错**：以为 `inspector --cli <server.exe> --doc-dir X --out-dir Y` 会把 `--doc-dir` / `--out-dir` 传给服务进程。
**对**：Inspector 把这些 `--xxx` 当作**自己的选项**消费掉（未识别也不报错），服务只拿到可执行文件本身、回落到默认参数。用包装进程实测：服务收到的 argv 只有 `["./target/debug/mcp-docs-server.exe"]`。此前多轮 Inspector 调试"成功"实属巧合——默认参数恰好就是要用的 `target/doc` + `target/doc-search`；`INDEX_SCHEMA_VERSION` 升到 5 后默认目录的旧索引过期，服务**启动时全量重建**（debug 约 2 分钟）→ 超过 Inspector 15s 超时，一度被误判为 Inspector/npx 故障（最小 node stdio MCP 服务 2s 通过即可排除）。
**规避**：调试前先用默认参数（或服务实际会用的目录）把索引建好；需要非默认目录时改用直接 stdio JSON-RPC 握手。
**相关**：`crates/mcp-docs-server/src/main.rs`（`--doc-dir` / `--out-dir`）

## 5. 工程、依赖与工具链

### #5.1 scraper 0.27 的 `ElementRef` API 与旧版不同

**错**：按旧版写法调用 `el.children()` 取子元素、把 `el.ancestors()` / `el.next_sibling()` 的返回值当 `ElementRef` 用、用 `el.value().has_class("x")` 判断 class。
**对**：scraper 0.27 中
- 直接子元素用 `child_elements()`（返回 `ElementRef`）；
- `ancestors()` / `next_sibling()` 经 `Deref` 走底层 `NodeRef`，返回 `NodeRef`，要用 `ElementRef::wrap(node)` 转回；
- `Element::has_class` 需要额外的 `CaseSensitivity` 参数，改为自实现更省事（读 `class` 属性按空白切分）。
**相关**：`parse.rs`、`crates/mcp-docs-core/Cargo.toml`（`scraper = "0.27"`）

### #5.2 Git-Bash 的 `/usr/bin/link.exe` 可能干扰 cargo 链接

**错**：在 Git-Bash 里直接跑 `cargo build`，假定它必然使用 MSVC 的 `link.exe`。
**对**：Git for Windows 自带 `/usr/bin/link.exe`（coreutils 的硬链接工具），且它在 `PATH` 里**排在 MSVC 的 `link.exe` 之前**（`which -a link` 可验证）。host 为 `x86_64-pc-windows-msvc` 时，若链接器按名字查找，会命中错误的 link，导致链接失败或行为异常。
**规避**：把 cargo 命令放进 cmd 执行 —— `cmd //c "cargo build"`（Git-Bash 中 `/c` 需写成 `//c`）；或确保 MSVC 环境（vcvars）就位、链接器走完整路径。
**相关**：本机 `which -a link` → `/usr/bin/link` 优先于 MSVC link

### #5.3 rmcp 的 `#[tool_router]` 必须配 `#[tool_handler]`

**错**：只写 `#[tool_router] impl DocsServer { ... }` 再手写 `impl ServerHandler { fn get_info }`，以为工具已经挂上。
**对**：`#[tool_router]` 只生成 `Self::tool_router()` 关联函数；必须再加 `#[tool_handler]` 才会填充 `call_tool` / `list_tools`，否则工具调用走 trait 的空默认实现（外部表现为字段 `tool_router` "never read" 警告）。
**相关**：`crates/mcp-docs-server/src/server.rs`

### #5.4 `JsonSchema` derive 需要 `schemars` 名字在作用域

**错**：只写 `use rmcp::schemars::JsonSchema;`，编译报 `cannot find module or crate schemars`。
**对**：derive 展开后会引用 `schemars::...` 绝对路径，需把模块名也引入：`use rmcp::schemars::{self, JsonSchema};`（或直接在 Cargo.toml 加 `schemars` 依赖）。rmcp 已 re-export `schemars`，无需重复添加依赖。
**相关**：`crates/mcp-docs-server/src/server.rs`
