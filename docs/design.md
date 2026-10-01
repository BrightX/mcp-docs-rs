# 设计文档

## 1. 实测事实底座

以下结构已在 rustc **1.95.0** 生成的真实产物上逐一核对（`cargo doc --no-deps`）。**解析器只依赖这些长期稳定的结构不变量**，不依赖任何随版本漂移的细节。

### 目录层

```
target/doc/
├── crates.js                  # window.ALL_CRATES = ["doc_probe"];
├── search.index/              # 新版分片压缩索引（root.js + rr_()/rd_() + base64 分片）
│                              # → 定制二进制编码，解析成本极高且不稳定，【明确不使用】
├── doc_probe/
│   ├── index.html             # crate 首页
│   ├── all.html               # 全部条目清单（跨模块），作兜底与交叉校验
│   ├── sidebar-items.js       # 纯 JSON，每个模块目录各一份
│   ├── struct.Demo.html       # 条目文件名自带类型前缀
│   ├── enum.Kind.html
│   ├── trait.DoIt.html
│   ├── fn.free_fn.html
│   └── inner/                 # 子模块是子目录，结构同上
│       ├── index.html
│       ├── sidebar-items.js
│       └── struct.Nested.html
└── src/                       # 源码高亮 HTML
```

- `crates.js`：`window.ALL_CRATES = ["doc_probe"];` → 去掉 `window.ALL_CRATES = ` 前缀与结尾 `;` 后可直接 `serde_json` 解析，无需 JS 引擎。
- `sidebar-items.js`：`window.SIDEBAR_ITEMS = {"enum":["Kind"],"fn":["free_fn"],"mod":["inner"],"struct":["Demo"],"trait":["DoIt"]};` —— key 是 kind 复数名，**这是建索引的主锚点**。
- 条目文件前缀：`struct.` / `enum.` / `trait.` / `fn.` / `type.` / `constant.` / `macro.` / `static.` / `union.` / `primitive.`。
- `all.html`：`<ul class="all-items"><li><a href="struct.Demo.html">Demo</a></li>`，跨模块显示为 `inner::Nested`。

### 页面层（位于 `section#main-content.content` 内）

- `<meta name="rustdoc-vars" data-current-crate="doc_probe" data-rustdoc-version="1.95.0 (59807616e 2026-04-14)" data-root-path="../">` —— crate 名、rustdoc 版本、链接根，**零 DOM 解析即可拿到**。
- `<meta name="description" content="A demo struct.">` —— 一行摘要，用于索引。
- 主文档包在 `<details class="toggle top-doc"><div class="docblock">` 中。**不能假设「第一个 docblock 就是主文档」**，必须按 `main-content` 的直接子节点顺序切分。
- 签名：`<pre class="rust item-decl"><code>…</code></pre>`。
- 分节标题：`<h2 class="section-header" id="fields|variants|implementations|trait-implementations|required-methods|provided-methods|synthetic-implementations|blanket-implementations">`。
- 字段：`<span id="structfield.field" class="structfield section-header">`（是 **span**，不是 section）+ 后继 `.docblock`。
- 枚举变体：`section#variant.A.variant` + `<h3 class="code-header">` + 后继 `.docblock`。
- impl 头：`section#impl-Demo.impl` + `<h3 class="code-header">impl Demo</h3>`，其下有 `.impl-items` 容器。
- 方法 / 关联项：`section#method.new.method`（或在 `.impl-items` 内）+ `<h4 class="code-header">pub fn new(...) -> Self</h4>` + 后继兄弟 `.docblock`。
  - trait 必需方法锚点为 `#tymethod.run`；trait 默认方法 / 关联常数 / 关联类型分别为 `#method.*` / `#associatedconstant.*` / `#associatedtype.*`。
- **噪声区块**：`#synthetic-implementations`（"Auto Trait Implementations"）、`#blanket-implementations`（"Blanket Implementations"）在 21 KB 的 struct 页里占绝大部分体积，**默认剥离**。
- **UI 噪声**（渲染前移除）：`#copy-path` 按钮、`rustdoc-toolbar`、`rustdoc-topbar`、`nav.sidebar`、`a.anchor`、`a.src.rightside`、`details > summary`（"Expand description"）。

## 2. 工程结构

```
d:\RustProjects\mcp-docs-rs\
├── Cargo.toml                         # [workspace] + [workspace.dependencies] 统一版本
├── docs\                              # 项目文档（需求 / 设计 / 进度）
├── crates\
│   ├── mcp-docs-core\                 # 纯库，零 async
│   │   ├── src\
│   │   │   ├── lib.rs                 # 门面导出 + 顶层错误
│   │   │   ├── model.rs               # DocItem / ItemKind / Section / Index / ItemSummary …
│   │   │   ├── error.rs               # thiserror
│   │   │   ├── config.rs              # DocSearchConfig（doc_root / out_dir / 各种开关）
│   │   │   ├── sidebar.rs             # sidebar-items.js → BTreeMap<ItemKind, Vec<String>>
│   │   │   ├── discover.rs            # crates.js + 递归 sidebar → 条目清单（all.html 兜底）
│   │   │   ├── parse.rs               # 单 HTML → DocItem（DOM 解析核心）
│   │   │   ├── markdown.rs            # DocItem → markdown（含降噪）
│   │   │   ├── link.rs                # 相对链接解析/重写
│   │   │   ├── index.rs               # index.json 构建/读写
│   │   │   ├── store.rs               # 磁盘布局、原子写、文件名编码
│   │   │   ├── cache.rs               # 指纹 + 内存缓存 + 增量
│   │   │   └── search.rs              # 索引内检索与排序
│   │   └── tests\
│   │       ├── fixtures\doc_probe\    # 从 temp 裁剪拷贝的真实产物（非 ignored 路径）
│   │       └── *.rs
│   ├── mcp-docs-cli\                  # clap，无 tokio
│   │   └── src\main.rs
│   └── mcp-docs-server\               # rmcp + tokio
│       └── src\{main.rs, server.rs, resources.rs}
└── temp\                              # 实测用的 doc_probe 项目（.gitignore 已忽略）
```

**为什么是 3-crate workspace**：`rmcp + tokio` 依赖图很重，而 CLI 与核心库完全不需要 async。拆开后 `cargo test -p mcp-docs-core` 秒级完成，core 也可被其他项目嵌入；`rmcp` 类型只出现在 server crate，API 漂移的影响面被隔离。

`[workspace.dependencies]` 集中声明：`scraper`、`htmd`、`serde`、`serde_json`、`walkdir`、`thiserror`、`anyhow`、`tracing`、`clap`、`rmcp`、`tokio`；dev：`insta`、`assert_cmd`、`tempfile`。

## 3. 核心数据模型（`model.rs`）

```rust
pub enum ItemKind { Module, Struct, Enum, Union, Trait, TraitAlias, Function,
    TypeAlias, Constant, Static, Macro, Primitive, Keyword, Derive, ProcMacro,
    Method, AssocConst, AssocType, Variant, Field, TyMethod, Impl, Unknown }
// serde rename_all = "snake_case"
// from_file_prefix("struct") / from_sidebar_key(..) / is_member() / is_noise_section()

pub struct ItemId(pub String);   // 唯一标识，序列化为可读路径：doc_probe::Demo::new
                                 // 注意：:: 只存在于 id 中，绝不用于文件名

pub struct SourceRef { file: String, line_start: Option<u32>, line_end: Option<u32> }

pub struct Section { id: String, title: String, body_md: String, members: Vec<DocItem> }

pub struct DocItem {
    id: ItemId, kind: ItemKind, name: String,
    path: Vec<String>,               // 模块路径（含 crate，不含自身）
    crate_name: String,
    signature: Option<String>,       // pre.rust.item-decl 文本
    docs_md: Option<String>,         // top-doc 内 docblock
    source: Option<SourceRef>,
    sections: Vec<Section>,
    members: Vec<DocItem>,           // 汇总
    html_path: PathBuf, rustdoc_version: Option<String>, src_mtime: Option<u64>,
}

pub struct ItemSummary {             // 索引条目（不含正文，Agent 先读这个）
    id: ItemId, kind: ItemKind, name: String, path: Vec<String>,
    one_line: String,                // <meta name=description>
    has_docs: bool, has_members: bool,
    file: String, anchor: Option<String>, source: Option<String>,
}

pub struct CrateSummary { name: String, version: Option<String>, item_count: usize }

pub struct Fingerprint { file_count: u64, max_mtime: u64, size_sum: u64, crates_js_hash: u64 }

pub struct Index { schema_version: u32, rustdoc_version: String, generated_at: String,
    target_doc: String, fingerprint: Fingerprint, crates: Vec<CrateSummary>, items: Vec<ItemSummary> }

pub struct SearchHit { item: ItemSummary, score: i32, snippet: Option<String> }
```

`items` 刻意**扁平化**：Agent 检索需要的是「一次线性扫描 / 前缀过滤」，树结构反而要递归；层级信息用 `path: Vec<String>` 表达，需要时在内存重建。

## 4. 模块职责与关键签名

```rust
// sidebar.rs
pub fn parse_sidebar_file(path: &Path) -> Result<BTreeMap<ItemKind, Vec<String>>>;
pub fn parse_sidebar_str(js: &str)  -> Result<BTreeMap<ItemKind, Vec<String>>>;

// discover.rs
pub fn list_crates(doc_root: &Path) -> Result<Vec<String>>;
pub fn discover_crate(doc_root: &Path, crate_name: &str) -> Result<Vec<DiscoveredItem>>;
pub fn discover_all(doc_root: &Path) -> Result<Vec<DiscoveredItem>>;

// parse.rs
pub struct ParseOptions { include_auto_impls: bool, include_trait_impls: bool, max_doc_bytes: Option<usize> }
pub fn parse_item_html(html: &str, rel_path: &Path, doc_root: &Path, opts: &ParseOptions) -> Result<DocItem>;
pub fn parse_one_line(html: &str) -> Option<String>;              // 极轻量，只抓 meta description
pub fn parse_rustdoc_meta(html: &str) -> Option<(String, String)>; // (version, crate)
pub fn path_to_identity(rel_path: &Path) -> Result<(ItemKind, Vec<String>, String)>;

// markdown.rs
pub enum LinkStyle { Relative, PlainPath, KeepOriginal }
pub struct RenderOptions { link_style: LinkStyle, include_source: bool, include_toc: bool, include_members_inline: bool }
pub fn render_item(item: &DocItem, opts: &RenderOptions, ctx: &LinkContext) -> String;
pub fn render_crate_index(summary: &CrateSummary, items: &[ItemSummary]) -> String;

// link.rs
pub enum Resolved { Item { id: ItemId, anchor: Option<String> }, Source(SourceRef), External(String), InPage(String) }
pub fn resolve_href(href: &str, current_rel: &Path) -> Resolved;
pub fn item_md_relpath(id: &ItemId, kind: ItemKind) -> PathBuf;

// index.rs / store.rs / cache.rs / search.rs
pub fn build_index(cfg: &Config, members: bool) -> Result<Index>;
pub fn load_index(path: &Path) -> Result<Index>;
pub fn write_index(idx: &Index, path: &Path) -> Result<()>;       // 原子写
pub fn encode_fs_name(name: &str) -> String;
pub fn item_output_path(root: &Path, s: &ItemSummary) -> PathBuf;
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()>;
pub fn fingerprint_doc_root(doc_root: &Path) -> Result<Fingerprint>;
pub struct DocCache { /* HashMap<ItemId, Arc<DocItem>> + FileStamp 校验 */ }
impl DocCache {
    pub fn get_or_parse(&self, cfg: &Config, id: &ItemId) -> Result<Arc<DocItem>>;
    pub fn is_stale(&self, cfg: &Config) -> Result<bool>;
    pub fn invalidate(&self);
}
pub enum MatchMode { Substring, Prefix, Fuzzy }
pub fn search(idx: &Index, q: &str, mode: MatchMode, kinds: &[ItemKind],
              crate_filter: Option<&str>, limit: usize) -> Vec<SearchHit>;
pub fn rank(item: &ItemSummary, q: &str) -> i32;   // 精确名 > 前缀 > path 命中 > 描述命中
```

## 5. 解析算法

### 5.1 全局发现

**索引范围**：默认覆盖 `crates.js` 列出的**全部 crate，含所有依赖** —— 即跟随 `cargo doc` 的默认行为（用户若加 `--no-deps`，则只有当前 crate）。这符合「Agent 需要查依赖 API」的诉求；发现阶段只读 `sidebar-items.js`（轻量），正文按需解析，因此全量范围开销可控。查询与导出阶段可按 crate 过滤。

1. `list_crates`：读 `crates.js` 解析数组；用目录存在性过滤；排除 `src/`、`static.files/`、`trait.impl/`。
2. `discover_crate`：从 `doc_root/<crate>` 起递归 `walk_module`；每层读 `sidebar-items.js`，`mod` 项下钻子目录，其余按 `{file_prefix}.{name}.html` 产出 `DiscoveredItem{ id = crate::mod::…::name }`。文件缺失记 warning，不 panic。
3. 兜底：sidebar 缺失时扫描目录下 `^(struct|enum|trait|fn|type|constant|static|macro|union|primitive)\.(.+)\.html$`。
4. 交叉校验兜底（`allpage.rs`）：解析 `all.html` 的 `ul.all-items a`，把 sidebar 与目录扫描都遗漏的条目按 **`html_path` 去重**后**追加**（保持发现顺序）。`foo!.html` 规整为真实文件 `foo.html`；外链、绝对路径、逃出 crate 的 `..` 跳过。

> 用 sidebar 而非只靠 all.html 的原因：sidebar 与目录树同构，能直接确定「这个 html 属于哪个模块」；all.html 的显示路径是 `inner::Nested` 字符串，仍需拆分。两者互补。

### 5.2 单条目解析

1. `path_to_identity(rel_path)` → (kind, module_path, name)；`parse_rustdoc_meta` → (version, crate)。
2. `scraper` 建 DOM，取 `section#main-content`。
3. 剥离 UI 噪声（见 §1 噪声清单）。
4. **按直接子节点顺序做状态机**切分：`details.top-doc` / 首个 `.docblock` → `docs_md`；遇 `h2.section-header` 开启新区块，直到下一个 h2。多个 docblock 用空行拼接。
5. 按 `section.id` 分派抽取成员：
   - `fields` → `span[id^="structfield."]`
   - `variants` → `section[id^="variant."]`
   - `implementations` / `trait-implementations` → `section.impl` 下 `.impl-items` 内的 `section[id^="method."]` / `#associatedconstant.*` / `#associatedtype.*`
   - `required-methods` → `section[id^="tymethod."]`
   - `synthetic-implementations` / `blanket-implementations` → 默认丢弃（`include_auto_impls=true` 时保留）
6. 每个成员后继兄弟 `.docblock` 即其文档；成员 id 为 `<父id>::<成员名>`。
7. 未知 section：保留标题 + 渲染正文（前向兼容），不解析成员。

### 5.3 成员条目化

- 每个 `section[id^="method."]` 成为独立 `DocItem{ id: "crate::Type::method", kind: Method }`，写进 `index.items`，**同时**：
  - 内联进父条目 markdown 的 Methods 小节（带 anchor `#method.new`），保证读父文件时上下文完整；
  - 额外落一个独立 md 文件，供精确检索后只读这一小块。
- 变体、字段同理（变体默认独立落盘，字段默认内联 —— 最终以 `--granularity` 可调）。

### 5.4 链接重写

以当前 `rel_path` 为基准解析 `href`：

| 输入 | 解析结果 |
|---|---|
| `#method.new` | 页内锚 `InPage("method.new")` |
| `../struct.Foo.html` | 归一化后反解为 `Item{ id: "doc_probe::Foo" }` |
| `struct.Bar.html#method.baz` | `Item{ id: "doc_probe::Bar", anchor: "method.baz" }` |
| `../src/doc_probe/lib.rs.html#L10-L13` | `Source(SourceRef)` |
| `http(s)://…` | `External`，**原样保留**（如指向 std 的 `doc.rust-lang.org/…/primitive.u32.html`，对 Agent 有价值） |

输出风格三种：`Relative`（默认，指向目标 md + 锚点，如 `[Demo::new](../struct.Demo.md#method.new)`）、`PlainPath`（一律 `` `doc_probe::Demo::new` ``，最省 token）、`KeepOriginal`（调试）。无法映射到已知条目的链接退化为纯文本路径。

### 5.5 Markdown 渲染

- HTML→MD 用 `htmd`；**代码块自己接管**：对 `pre.rust` / `pre.item-decl` 直接取 `text()`，用 ```` ```rust ```` 围栏包裹，避免语言推断错误与 HTML 实体残留。
- `where` 子句保留（对理解签名有用）；`details > summary` 只保留内容，丢弃 "Expand description"。

### 5.6 解析库选型

- **HTML 解析：`scraper`**（html5ever + selectors）。需要按 DOM 父子/兄弟关系切 section（"紧随 `.code-header` 的兄弟 `.docblock`"），这是 scraper + `ego_tree` 的核心能力。
- 不选 `lol_html`：流式回调 API 处理「找成员后回溯兄弟 docblock」这类树形逻辑非常别扭。
- **性能兜底**：若在大 crate 上 profile 发现 HTML 解析是瓶颈，可整体换用 `tl`（零拷贝，比 html5ever 快数倍，同样支持 `.children()` / 兄弟遍历）。**这正是把 `parse` 独立成模块的主要动机** —— 替换不影响其他模块。
- **HTML→MD：`htmd`**。对 `<pre><code>`、嵌套列表、表格、链接的处理明显好于 `html2md`，且仍在维护。

## 6. 存储布局与命名

```
target/doc-search/                       # 默认；CLI --out / server 参数可覆盖
├── index.json                           # 全局扁平索引（Agent 首读）
├── meta.json                            # 仅指纹/版本/时间（判定重建无需反序列化大 index）
└── doc_probe/
    ├── index.md                         # crate 首页
    ├── struct.Demo.md                   # 含内联的 Methods 小节
    ├── struct.Demo.new.md               # 成员独立文件（父名 + "." + 成员名）
    ├── enum.Kind.md
    ├── enum.Kind.A.md
    ├── fn.free_fn.md
    └── inner/{ index.md, struct.Nested.md }
```

**命名规则（Windows 安全是硬约束）**

- 顶层 = `{kind_prefix}.{Name}.md`；成员 = 父文件名去 `.md` + `.` + `member_name` + `.md`。
- **文件名禁用 `:`** → `Demo::new` 落盘为 `struct.Demo.new.md`；`::` 只留在 `index.json` 的 `id` 里。
- `encode_fs_name()`：清洗 `< > : " / \ | ? *` → `_`；Windows 保留名（`CON/PRN/AUX/NUL/COM1…`）加 `_` 前缀；去尾部 `.` / 空格；泛型名（`Foo<Bar>`）超 200 字节时 `name-hash8` 截断；冲突时加成员 kind 前缀兜底。
- 写盘一律 `tmp → rename` 原子替换，避免 Agent 读到半截文件。

**导出粒度（`Granularity`）**

- `member`（默认）：每个成员额外写独立文件；成员摘要的 `file` 指向该文件。
- `item`：只写顶层条目文件，成员仅内联在父文件里；成员摘要的 `file` **指向父条目文件**（当前 `ItemSummary` 无 `anchor` 字段，不做深链）。
- 索引记录 `granularity`；增量构建在 schema 或粒度变化时全量重建（否则会复用 `file` 不一致的旧摘要，且不补写成员文件）。

**index.json 契约示例**

```json
{
  "schema_version": 5,
  "rustdoc_version": "1.98.1 (48a229cea 2026-09-01)",
  "generated_at": 1790853560,
  "target_doc": "D:\\RustProjects\\mcp-docs-rs\\target\\doc",
  "granularity": "member",
  "crates": [{ "name": "doc_probe", "version": null, "item_count": 11 }],
  "items": [
    { "id": "doc_probe::struct.Demo", "kind": "struct", "name": "Demo",
      "path": ["doc_probe"], "one_line": "A demo struct.", "has_docs": true,
      "has_members": true, "file": "doc_probe/struct.Demo.md",
      "html_path": "doc_probe/struct.Demo.html", "parent_id": null, "src_mtime": 1790853560614 },
    { "id": "doc_probe::struct.Demo::method.new", "kind": "method", "name": "new",
      "path": ["doc_probe", "Demo"], "one_line": "build a demo",
      "has_docs": true, "has_members": false,
      "file": "doc_probe/struct.Demo.method.new.md",
      "html_path": "doc_probe/struct.Demo.html", "parent_id": "doc_probe::struct.Demo",
      "src_mtime": 1790853560614 }
  ]
}
```

> 产物指纹与 schema 版本另存于 `meta.json`（判定重建时无需反序列化大 index）。

## 7. 缓存策略

- **一级（启动即建，毫秒级）**：已落盘 `index.json` → 直接 `serde_json` 加载。首次构建时只读 `crates.js` + 各 `sidebar-items.js`，再对每个 html **只读文件头抓 `<meta name=description>` 与 `data-rustdoc-version`**（不建 DOM）得到一行摘要。
- **二级（按需）**：`get_item` 时才建 DOM、抽 section/members、渲染 md；结果进内存缓存（`Arc<DocItem>` + LRU 上限，如 512）。
- **指纹**：`Fingerprint{ file_count, max_mtime, size_sum, crates_js_hash }`，一次 `walkdir` 只 stat 不读内容，复杂度 O(文件数)。
- **stale 判定**：fingerprint 不等 **或** 页面 `data-rustdoc-version` 变化 **或** `schema_version` 变化 → 重建。默认全量重建索引（很快）；`--incremental` 时按 `src_mtime` 逐文件比对，仅更新变化项。
- CPU 密集的解析/渲染在 async handler 中用 `tokio::task::spawn_blocking` 包裹。

## 8. MCP 接口（rmcp 3.5.0）

设计原则：**先搜后读** —— 搜索工具只返回轻量摘要 + 指针，读取工具才返回正文，避免 Agent 上下文爆炸。

### 工具

| 工具 | 入参 | 返回 |
|---|---|---|
| `list_crates` | — | `{ crates: [{name, version, item_count}] }` |
| `list_items` | `{crate?, module?, kind?, limit?=100, offset?=0}` | `{ total, offset, returned, items: [ItemSummary] }` |
| `search_items` | `{query, crate?, kind?, mode?="substring"\|"prefix"\|"fuzzy", limit?=20, offset?=0}` | `{ total, offset, returned, hits: [{…ItemSummary, score}] }` |
| `get_item` | `{id, max_bytes?}` | 该条目完整 markdown（签名 + 主文档 + 各 section + 成员） |
| `get_item_source` | `{id}` | `{id, source: {file, path, line_start, line_end}}` |
| `rebuild_index` | `{force?}` | `{rebuilt, reused, item_count}` |

> `total` 为分页前的命中数；`id` 允许省略类型标记（如 `tokio::task::spawn`）。

### 资源

`rustdoc://crates`、`rustdoc://{crate}`（crate `index.md`）、`rustdoc://{crate}/{*item}`（条目），用 `ResourceTemplate` 实现 `list_resource_templates` + `read_resource`。

### 典型 Agent 流程

`search_items("spawn", crate="tokio")` → 命中若干摘要 → `get_item("tokio::spawn")` 或读 `rustdoc://tokio/spawn` → **只拿到这一小块**。

### rmcp 落点

- features：`["server", "transport-io", "schemars"]`（`macros` 默认开）。
- `#[tool_router]` + `#[tool]`；stdio 用 `server.serve((stdin, stdout)).await` + `service.waiting().await`。
- rmcp 类型只出现在 server crate，core 零感知。

## 9. CLI

```
mcp-docs export [--out DIR] [--doc-dir target/doc] [--incremental] [--granularity member|item]  # 导出 md + index.json
mcp-docs tree   [--doc-dir target/doc]                               # 打印条目树（调试）
mcp-docs show   <id> [--doc-dir target/doc]                          # 打印单条目 markdown
mcp-docs search <query> [--crate X] [--limit N] [--offset N]         # 检索（可分页）
```

## 10. 测试策略

fixture：把 `temp/doc_probe/target/doc` 裁剪（剔除 `static.files/`、`search.index/`、字体，仅留 html/js，约 90 KB → 30 KB）拷到 `crates/mcp-docs-core/tests/fixtures/doc_probe/`。注意 `temp/` 被 `.gitignore` 的 `/temp/` 规则忽略，**fixture 必须放在 `tests/fixtures/` 并确保未被忽略**。

1. **单元（core）**
   - `sidebar::parse_sidebar_str`：实测 JS → 断言 `struct{["Demo"]}`、`mod{["inner"]}`。
   - `discover`：对 fixture 断言 6 个顶层 + `Nested`，与 `all.html` 交叉校验一致。
   - `parse_item_html`：`struct.Demo.html` → 签名 `pub struct Demo { pub field: u32, }`、`docs_md` 含 "A demo struct."、成员含 `Demo::new`（docs = "build a demo"）、字段 `field` 文档 = "the field"；`enum.Kind.html` → 两变体 A/B；`trait.DoIt.html` → `run` 识别为 `TyMethod`。
   - `link::resolve_href`：4 类输入断言。
   - `store::encode_fs_name`：`Demo<Bar>` 合法、`CON`→`_CON`、`Demo::new`→`Demo.new`。
   - `cache::fingerprint`：两次相等；改 mtime 后不等。
   - `search`：`"new"` 命中 `Demo::new`；`"demo"` 命中 `Demo` 且排序合理。
2. **golden/snapshot（`insta`）**：`struct.Demo.md` 快照，断言无 "Copy item path"、无 blanket/synthetic impls、含 ```rust 围栏。
3. **CLI 集成（`assert_cmd` + `predicates` + `tempfile`）**：`export --out <tmp>` 后断言文件树与 `index.json` 可解析、条目数正确。
4. **MCP 集成**：rmcp client 经内存双工 transport 调 `search_items` / `get_item` / `read_resource`。
5. **可选 e2e（`#[ignore]`）**：临时建最小 crate → `cargo doc --no-deps` → 全流程，作为「新版 rustdoc 结构变化」的哨兵。
6. **健壮性**：`parse_item_html` 喂截断/畸形 HTML，断言不 panic。

## 11. 风险与缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| rustdoc 版本差异致 DOM/文件名变 | 解析失败或静默丢内容 | 只依赖稳定的结构不变量；解析失败**降级**为「抓正文文本」而非 panic；`data-rustdoc-version` 入指纹，版本变即重建；每版本维护 golden fixture |
| 大 crate 性能（tokio/serde） | 索引/导出慢、内存高 | 默认剥离 synthetic/blanket impls；索引期不建 DOM；正文按需 + LRU；`--max-doc-bytes` 截断；解析器可换 `tl` |
| `search.index` 二进制 | 想用会踩坑 | 明确不使用，只用 `sidebar-items.js` + `all.html` |
| Windows 路径/文件名 | 写盘失败、跨平台不一致 | id 用 `::`、文件名用 `.`；`encode_fs_name` 清洗非法字符与保留名；全程 `Path` API 不手拼 `\`；超长名 hash 截断 |
| 非 UTF-8 / BOM / 实体 | 乱码 | 按字节读 + `from_utf8_lossy`；DOM 库解码实体；输出前兜底转义 |
| 重名 / 跨 kind 冲突 | 文件覆盖 | kind 前缀 + 成员 kind 后缀兜底；写盘前检测冲突并记 warning |
| `rmcp` API 漂移 | 编译失败 | 锁精确版本 `rmcp = "=3.5.0"`；rmcp 类型仅存在于 server crate |
| fixture 被 gitignore 吃掉 | 测试不可复现 | 拷到 `crates/mcp-docs-core/tests/fixtures/doc_probe/`（不在 `/temp/` 下） |

## 12. 依赖版本

```toml
[workspace.dependencies]
scraper    = "0.24"
htmd       = "0.1"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
walkdir    = "2"
thiserror  = "2"
anyhow     = "1"
tracing    = "0.1"
clap       = { version = "4", features = ["derive"] }
rmcp       = { version = "=3.5.0", features = ["server", "transport-io", "schemars"] }
tokio      = { version = "1", features = ["rt-multi-thread", "macros", "io-std"] }
# dev
insta      = "1"
assert_cmd = "2"
tempfile   = "3"
```

（`scraper` / `htmd` 的确切可用版本在实施首步用 `cargo add` 校准。）
