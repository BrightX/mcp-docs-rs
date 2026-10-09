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

### #1.26 rustdoc 1.98 的文档分节 h2 不带 `section-header`

**错**：沿用 #1.6 的结论，认为分节标题都是 `<h2 class="section-header">`，用「`main-content` 直接子级 + `section-header`」收集分节。
**对**：文档作者书写的分节（`## Panics` / `## Examples` 等）是 `<h2 id="panics">`，**无 class**，且位于主文档 `details.top-doc .docblock` **内部**（不是 `main-content` 的直接子级）；只有 `implementations` / `trait-implementations` 这类结构分节才带 `section-header`、才是 `main` 直接子级。只按 class/层级收集会漏掉全部文档分节，`get_item_section` 对 examples/panics 恒报「没有分节」（且 `Section.body_md` 若未从正文切出也拿不到内容）。
**规避**：区分两类分节——结构分节看 `main` 直接子级 + `section-header`；文档分节看 `docblock` 内的 `h2[id]`（不看 class）。分节正文可从条目 markdown 按 `## 标题` 切出。
**相关**：`parse.rs::collect_sections`；issues E-1.1

### #1.27 trait.impl 文件按 trait 定义模块存放，与文档页路径可能不一致

**错**：认为实现清单 `trait.impl/{trait 页路径}.js` 与 trait 页同目录，直接由 `html_path` 拼路径。
**对**：rustdoc 把 `trait.<Name>.js` 放在 trait 的**定义模块**下。re-export 时文档页在根路径而实现清单在定义模块，甚至跨 crate：`bitflags::Flags` 页在 `bitflags/trait.Flags.html`、实现在 `trait.impl/bitflags/traits/trait.Flags.js`；`serde::Serialize` 的实现被拆到 `serde_core`。按 `html_path` 硬拼读不到文件，且若用 `unwrap_or_default()` 会静默返回 0 实现者。
**规避**：精确路径未命中时，在 `trait.impl` 下按文件名回退搜索（先同 crate 目录、再全局）；并把「文件不存在」与「文件为空」区分开。
**相关**：`nav.rs::trait_impl_rel_path` / `find_trait_impl_paths`；issues E-2.1

### #1.28 条目渲染结果里的代码块含签名，不等于「示例」

**错**：从条目渲染后的整篇 markdown 抽 ```rust 代码块当作「文档示例」。
**对**：`render_item` 的输出结构是 `# id` → 类型 → **签名块** → 文档正文 → 成员（每个成员也各带一个签名块）。签名同样是 ```rust 围栏，会被一并抽出，于是第一个「示例」就是签名本身，成员签名同理。签名不是示例。
**规避**：示例应从**文档正文**（`docs_md` / 成员的 `docs_md`）抽取，不要从整篇渲染结果抽取。
**相关**：`server.rs::get_examples_blocking`；issues E-1.2

### #1.29 trait 的「本 crate 实现」在页面里，`trait.impl` js 只列跨 crate

**错**：以为 `trait.impl/**/trait.<Name>.js` 列出了 trait 的全部实现者。
**对**：rustdoc 把**本 crate 与外来类型**的实现内联渲染在 trait 页面（`#implementors-list` / `#foreign-impls`），`trait.impl/*.js` 只放**跨 crate** 实现，且同 crate 那组是**空数组**（如 `["tokio",[]]`）。只读 js 会漏掉本 crate 全部实现（`DuplexStream`/`File`/`TcpStream`…），得出"实现者很少"的错误结论。完整结果须合并两处。
**相关**：`parse.rs::parse_page_impls`、`nav.rs::parse_trait_impls`；issues E-2.3

### #1.30 文档分节标题的 markdown / HTML 文本不一致（反引号）

**错**：用 HTML 取到的标题文本（内联代码去格式后）去精确匹配 markdown 的 `## 标题`。
**对**：markdown 标题保留内联代码的反引号（`## When to use \`X\``），而 HTML 标题文本没有；精确比对会切不到正文，`body_md` 静默为空（同页其它无内联代码的分节却正常）。比对前应规整（去反引号）。
**相关**：`parse.rs::extract_md_section`；issues E-1.3

### #1.31 重导出有三种形态，只有前两种需要额外发现

**错**：以为 `pub use` 重导出要么都有独立页面、要么都是 `#reexports` 文字。
**对**：分三种——① **crate 内部**重导出（`pub use deep::X as Y;`）**不生成页面**，只在 crate 首页 `#reexports` 列出（`href` 形如 `deep/struct.X.html`）；② **跨 crate 模块**重导出（`pub use ::gpui_component as component;`）同样不生成页面、只在 `#reexports` 列出，但 `href` 形如 `../gpui_component/index.html`（目标 crate 首页）；③ **跨 crate 条目**重导出（`pub use anyhow::Error;`）rustdoc 会生成本地页面、进 sidebar / `all.html`，常规发现已覆盖。①② 都必须单独解析 `#reexports` 才能被检索；③ 无需处理。跨 crate 模块别名指向的 crate 首页不是索引条目、不落盘，`get_item` 需委托目标 crate 概览渲染。
**相关**：`crates/mcp-docs-core/src/reexport.rs`；`crates/mcp-docs-server/src/project.rs::cross_crate_module_target`；design.md §1

### #1.32 共享同一 HTML 的条目，id 不能只从 html_path 推导

**错**：`process_entry` 用 `parse_item_html(html, html_path)` 推导的 `id` 作为 `ItemSummary.id`。
**对**：重导出别名条目与目标共享同一 `html_path`，解析出的 id 是中目标的 id，导致别名 id 丢失、且与目标条目撞 id（同页被多个条目引用时条目重复）。应以**发现阶段的 `entry.id`/`name`/`path`/`kind` 为准**覆盖。别名条目还不应继承目标成员（成员 id 带目标前缀会重复），也不应重复写盘（与目标同文件、内容相同，并发写会在 Windows 触发 os error 32）。
**相关**：`crates/mcp-docs-core/src/index.rs::process_entry`（`is_alias`）

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

### #3.2 共享库身份键不能含 mtime

**错**：直接复用现有产物指纹（`Fingerprint{file_count,max_mtime,size_sum}`）做跨项目复用键。
**对**：跨项目复用的前提是「两个项目里同一 crate 的文档可判为同一份」，而各项目各自 `target/doc` 副本的 **mtime 不同**，含 mtime 会导致永远无法命中。改用只 stat、不含 mtime 的 `CrateStat{file_count,size_sum}`，并叠加 crate 名 / 版本 / rustdoc 版本 / 粒度构成身份键。`file_count+size_sum` 是弱指纹（无内容哈希），用 crate 版本兜底；如需更强可并入 `sidebar-items.js` 哈希。
**相关**：`shared.rs::{key_dir_name,CrateStat,crate_scan}`、`design.md` §6

### #3.3 复用条目的 `src_mtime` 必须按本项目回填

**错**：把共享库里的 `ItemSummary` 原样复用回项目索引。
**对**：`src_mtime` 是增量判断的依据（`summary.src_mtime == mtime`）。共享库里的 mtime 来自**产出它的项目**，直接复用会让本项目的增量比对永远不成立、每次都重新解析全部条目。修复：复用同一趟 `crate_scan` 得到的 `html_path → mtime` 映射，按本项目 mtime 回填。
**相关**：`shared.rs::load_reused`、`index.rs::build`

### #3.4 多进程并发写共享库会撕裂临时文件

**错**：`atomic_write` 用固定名 `xxx.tmp` 作中转。
**对**：多个项目（甚至多进程）可能并发写同一共享库条目，固定临时名会互相覆盖/撕裂。改为进程唯一临时名（`pid` + 单调序号）。另：共享库条目的 `meta.json` **最后写**，作为「已完整落盘」的提交标记，缺它即视为半成品并忽略。
**相关**：`store.rs::unique_tmp_path`、`shared.rs::write_entry`

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

### #4.4 重导出别名不是模块，`module` 过滤会静默落空

**错**：以为 `list_items(module="component")` 能列出 `pub use gpui_component as component` 重导出的内容。
**对**：`module` 过滤按 `item.path[1..]` 的**真实文档模块段**匹配（`project.rs::matches_module`）；重导出别名不产生条目、不进入 `path`，故恒空且**不报错**；带 crate 前缀的写法（`crate::mod`）同样因首段不匹配而落空。空结果 ≠ "不存在"。规避：`module` 传 crate 内真实模块名（不带 crate 前缀），文件名/重导出路径用工作区 grep 确认。
**相关**：`project.rs::matches_module`；issues E-3.7

### #4.5 检索得分不含语境，同名不同 kind 会同分

**错**：以为名字精确命中就一定能排在前面。
**对**：`rank` 的 100 分对"名字精确相等"的所有条目一视同仁——`accesskit::Role::variant.Button` 与真正的 `button::Button` 同为 100，靠 id 升序时变体反而在前（实测约 102 条噪声）。得分之外还需 `has_docs` / 是否成员 / 路径深度等**语境次级键**才能把真结果顶上来。
**相关**：`search.rs::rank` / `ranked_hits`；issues E-3.9

### #4.6 crate 首页不是条目

**错**：以为 `get_item("tokio")` 能读到 crate 总览。
**对**：索引只收录 `sidebar-items.js` 的条目，crate 首页（`{crate}/index.html` 的 top-doc）不在此列，故 `get_item(crate 名)` 恒报「未找到条目」。要 crate 总览须用 `list_items(kind=module)` / `module_tree`，或走 E-3.8 的 `get_item` 回退合成概览。
**相关**：`project.rs::get_item_blocking`；issues E-3.8

### #4.7 schemars 的整数 `format` 可在 `list_tools` 里统一清洗

**错**：以为 `format: uint*`（`uint`/`uint32`/`uint64`）无解、只能逐字段手写 JSON Schema（E-4.1 因此一度判为不修复）。
**对**：`schemars` 为 `u32`/`u64`/`usize` **硬编码**生成这些 format（`json_schema_impls/primitives.rs`，无全局开关）。但 rmcp 的 `#[tool_handler]` **仅在方法缺失时**才生成 `list_tools` / `get_tool`（`has_method` 守卫），因此可自行实现这两个方法，在返回前递归删除整数 `format`（改动集中在 `server.rs`，未来新增字段自动覆盖）。注意 `format` 也出现在结构化输出的 `$defs` 内，必须**递归**清理。
**相关**：`crates/mcp-docs-server/src/server.rs::list_tools` / `strip_integer_formats`；issues E-4.1

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

### #5.5 `macro_rules!` 定义位置与规则必须覆盖调用形态

**错**：把 `selector!` 宏定义放在文件末尾（原 `selector` 函数的位置），并在规则里只写 `($css:expr)`。
**对**：两处都会编译失败。
- `macro_rules!` 是**文本作用域**，必须在首次使用之前定义，否则报 `cannot find macro`；把定义移到文件顶部（导入之后）即可。
- 原调用点形如 `main.select(&selector("a, b",))`（属性宏改写后留下尾随逗号），而 `($css:expr)` 不接受尾随 `,`，报 `no rules expected ','`；规则写成 `($css:expr $(,)?)` 兼容。
**相关**：`crates/mcp-docs-core/src/parse.rs`

### #5.6 rayon 并行迭代要能收集 `Result`

**错**：把 `par_iter().map(process_entry)` 的结果直接 `collect::<Vec<_>>()`，而 `process_entry` 返回 `Result<ProcessResult>`，导致类型不匹配。
**对**：rayon 为 `Result` 实现了 `FromParallelIterator`，直接 `collect::<Result<Vec<_>>>()?` 即可在并行中短路错误。注意 indexed 并行迭代的 `collect` **保持原顺序**，据此可保证条目顺序与旧串行实现逐字节一致。
**相关**：`crates/mcp-docs-core/src/index.rs::build`

### #5.7 测试里内嵌 HTML 别用 `r#"..."#`

**错**：写 `r#"<a href="#1">1</a>"#` 时，字符串里的 `"#`（`href="#1"`）会被当成原始字符串的结束符，编译器报 `expected one of ... found '1'`。
**对**：改用 `r##"..."##`（井号数量大于内容中出现的连续井号数）。凡是内嵌 HTML（含 `href="#..."`）或含 `"#` 序列的文本都要注意。
**相关**：`crates/mcp-docs-core/tests/extras.rs`

### #5.8 rmcp 结果结构体用 `..Default::default()` 而非先建后改

**错**：`let mut r = ListPromptsResult::default(); r.prompts = ...;` 触发 clippy `field_reassign_with_default`（`-D warnings` 下报错）。
**对**：rmcp 的 `ListPromptsResult` 等虽标注 `#[expect(clippy::exhaustive_structs)]`，仍可用函数式更新：
`ListPromptsResult { prompts, ..Default::default() }`。
**相关**：`crates/mcp-docs-server/src/server.rs`

### #5.9 rmcp 3.5 的 logging 通知已废弃

**错**：按 MCP 计划用 `enable_logging()` + `notify_logging_message` 发构建进度。
**对**：rmcp 3.5 中 `notify_logging_message` 已按 SEP-2577 标注 `#[deprecated]`（未来版本移除），启用会触发 deprecation 警告。改为只发 `notify_resource_list_changed`（`enable_resources_list_changed`），通知客户端刷新资源清单即可。
**相关**：`crates/mcp-docs-server/src/server.rs::notify_resources_changed`
