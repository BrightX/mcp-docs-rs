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

### re-export 的三种形态（实测）

重导出分三类，产物形态**完全不同**：

- **crate 内部重导出**（`pub use deep::RealStruct as Renamed;`）：**不生成独立页面**，也不进 `sidebar-items.js` / `all.html` / 目录扫描；只在 crate 首页列出：
  ```html
  <h2 id="reexports" class="section-header">Re-exports</h2>
  <dl class="item-table reexports">
    <dt id="reexport.Renamed"><code>pub use deep::<a class="struct"
       href="deep/struct.RealStruct.html" title="struct rp::deep::RealStruct">RealStruct</a> as Renamed;</code></dt>
  </dl>
  ```
  `dt#reexport.{别名}` 给别名，`a[title]`（`{kind前缀} {目标全路径}`）给目标类型，`a[href]` 给目标真实页。由 `reexport.rs` 补成别名条目（id 用别名、`html_path` 指向目标页），供 `search_items` 命中。
- **跨 crate 模块重导出**（`pub use ::gpui_component as component;`）：目标为**另一 crate 的根模块**，同样**不生成独立页面**、只在 `#reexports` 列出，但 `a[href]` 是 `../{other}/index.html`（目标 crate 首页）：
  ```html
  <dt id="reexport.component"><code>pub use ::<a class="mod"
     href="../gpui_component/index.html" title="mod gpui_component">gpui_component</a> as component;</code></dt>
  ```
  同样由 `reexport.rs` 补成别名条目（id 用别名、`html_path` 指向**目标 crate 首页**）。因 crate 首页不是索引条目、不落盘，`get_item` 命中该别名时**委托目标 crate 的合成概览**渲染（`project.rs::cross_crate_module_target` + `crate_overview`）。
- **跨 crate 条目重导出**（`pub use anyhow::Error;`）：rustdoc 生成**本地页面**（`struct.Error.html` 等），进 sidebar / `all.html`，由常规发现路径覆盖，**不进入 `#reexports`**。

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
│   │   │   ├── discover.rs            # crates.js + 递归 sidebar → 条目清单（all.html / reexports 兜底）
│   │   │   ├── allpage.rs             # 解析 all.html，补全 sidebar 遗漏条目
│   │   │   ├── reexport.rs            # 解析 crate 首页 #reexports，补全内部重导出别名
│   │   │   ├── parse.rs               # 单 HTML → DocItem（DOM 解析核心）
│   │   │   ├── markdown.rs            # DocItem → markdown（含降噪）
│   │   │   ├── link.rs                # 相对链接解析/重写
│   │   │   ├── index.rs               # index.json.gz 构建/读写
│   │   │   ├── store.rs               # 磁盘布局、原子写、文件名编码
│   │   │   ├── cache.rs               # 指纹 + 内存缓存 + 增量
│   │   │   ├── shared.rs              # 跨项目共享索引库（身份键 / 复用 / 物化）
│   │   │   └── search.rs              # 索引内检索与排序
│   │   └── tests\
│   │       ├── fixtures\doc_probe\    # 从 temp 裁剪拷贝的真实产物（非 ignored 路径）
│   │       └── *.rs
│   ├── mcp-docs-cli\                  # clap，无 tokio
│   │   └── src\main.rs
│   └── mcp-docs-server\               # rmcp + tokio
│       └── src\{main.rs, server.rs, project.rs, config.rs}
└── temp\                              # 实测用的 doc_probe 项目（.gitignore 已忽略）
```

**为什么是 3-crate workspace**：`rmcp + tokio` 依赖图很重，而 CLI 与核心库完全不需要 async。拆开后 `cargo test -p mcp-docs-core` 秒级完成，core 也可被其他项目嵌入；`rmcp` 类型只出现在 server crate，API 漂移的影响面被隔离。

**server 模块划分（M11 起）**：`server.rs` 只做「项目注册表 + `#[tool]` 转发 + 资源/prompts」；`project.rs` 承载单个项目的运行时状态（索引 / 缓存 / 就绪信号 / 全部 `*_blocking` 实现）；`config.rs` 解析多项目启动参数。

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

pub struct CrateSummary { name: String, item_count: usize }

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
// 排序：得分降序，同分时按语境次级键（有文档 → 非成员 → 路径浅 → 名字短 → id，见 issues E-3.9）。
```

## 5. 解析算法

### 5.1 全局发现

**索引范围**：默认覆盖 `crates.js` 列出的**全部 crate，含所有依赖** —— 即跟随 `cargo doc` 的默认行为（用户若加 `--no-deps`，则只有当前 crate）。这符合「Agent 需要查依赖 API」的诉求；发现阶段只读 `sidebar-items.js`（轻量），正文按需解析，因此全量范围开销可控。查询与导出阶段可按 crate 过滤。

1. `list_crates`：读 `crates.js` 解析数组；用目录存在性过滤；排除 `src/`、`static.files/`、`trait.impl/`。
2. `discover_crate`：从 `doc_root/<crate>` 起递归 `walk_module`；每层读 `sidebar-items.js`，`mod` 项下钻子目录，其余按 `{file_prefix}.{name}.html` 产出 `DiscoveredItem{ id = crate::mod::…::name }`。文件缺失记 warning，不 panic。
3. 兜底：sidebar 缺失时扫描目录下 `^(struct|enum|trait|fn|type|constant|static|macro|union|primitive)\.(.+)\.html$`。
4. 交叉校验兜底（`allpage.rs`）：解析 `all.html` 的 `ul.all-items a`，把 sidebar 与目录扫描都遗漏的条目按 **`html_path` 去重**后**追加**（保持发现顺序）。`foo!.html` 规整为真实文件 `foo.html`；外链、绝对路径、逃出 crate 的 `..` 跳过。
5. 重导出兜底（`reexport.rs`）：解析 crate 首页的 `#reexports` 区块，把**crate 内部重导出别名**按 **id 去重**后追加。别名条目 id 用别名、`html_path` 指向目标的真实页面；同名重导出与已发现的真实条目撞 id 时被跳过。

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
├── index.json.gz                        # 全局扁平索引（JSON + gzip；Agent 首读）
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

**落盘编码**

- 索引类文件（`index.json.gz`、共享库 `items.json.gz`）落盘为 **JSON + gzip**（flate2，纯 Rust backend），体积降到约 1/10；文件名带 `.gz` 后缀，加载时自动解压。落盘辅助在 `codec.rs`（`write_json_gz` / `read_json_gz`）。
- `ItemSummary` 的 `path` / `name` / `parent_id` / `file` **不落盘**：它们都能由 `id` / `kind` / `html_path` / `granularity` 推导，加载后由 `ItemSummary::rebuild_derived` 重建（见下方契约示例）。字段裁剪与压缩叠加，现网 1.9 万条目从约 9.9 MB 降到约 0.7 MB。
- `meta.json` 保持明文：体积小，且过期判定要快速读、无需解压。

**命名规则（Windows 安全是硬约束）**

- 顶层 = `{kind_prefix}.{Name}.md`；成员 = 父文件名去 `.md` + `.` + `member_name` + `.md`。
- **文件名禁用 `:`** → `Demo::new` 落盘为 `struct.Demo.new.md`；`::` 只留在 `index.json.gz` 的 `id` 里。
- `encode_fs_name()`：清洗 `< > : " / \ | ? *` → `_`；Windows 保留名（`CON/PRN/AUX/NUL/COM1…`）加 `_` 前缀；去尾部 `.` / 空格；泛型名（`Foo<Bar>`）超 200 字节时 `name-hash8` 截断；冲突时加成员 kind 前缀兜底。
- 写盘一律 `tmp → rename` 原子替换，避免 Agent 读到半截文件。

**导出粒度（`Granularity`）**

- `member`（默认）：每个成员额外写独立文件；成员摘要的 `file` 指向该文件。
- `item`：只写顶层条目文件，成员仅内联在父文件里；成员摘要的 `file` **指向父条目文件**（当前 `ItemSummary` 无 `anchor` 字段，不做深链）。
- 索引记录 `granularity`；增量构建在 schema 或粒度变化时全量重建（否则会复用 `file` 不一致的旧摘要，且不补写成员文件）。

**index.json.gz 契约示例**

落盘为 **JSON + gzip**（文件名 `index.json.gz`，内容以 gzip 魔数 `1f 8b` 开头）。下列为**解压后**的逻辑结构，已省略不落盘的派生字段：

```json
{
  "schema_version": 8,
  "rustdoc_version": "1.98.1 (48a229cea 2026-09-01)",
  "generated_at": 1790853560,
  "target_doc": "D:\\RustProjects\\mcp-docs-rs\\target\\doc",
  "granularity": "member",
  "crates": [{ "name": "doc_probe", "item_count": 11 }],
  "items": [
    { "id": "doc_probe::struct.Demo", "kind": "struct",
      "one_line": "A demo struct.", "has_docs": true, "has_members": true,
      "html_path": "doc_probe/struct.Demo.html", "src_mtime": 1790853560614 },
    { "id": "doc_probe::struct.Demo::method.new", "kind": "method",
      "one_line": "build a demo", "has_docs": true, "has_members": false,
      "html_path": "doc_probe/struct.Demo.html", "src_mtime": 1790853560614 }
  ]
}
```

> `path` / `name` / `parent_id` / `file` 不落盘，加载后由 `id`（+ `kind` / `html_path` / `granularity`）重建；`signature` 保留（供 `find_by_signature`）。产物指纹与 schema 版本另存于 `meta.json`（判定重建时无需反序列化大 index）。

**跨项目共享索引库（M11）**

多个项目依赖同一 crate（如同一版本 tokio）时，其条目摘要与 markdown 只解析渲染一次，存入共享库被各项目复用。

```
<store_root>/                          # 默认平台缓存目录（MCP_DOCS_STORE 覆盖）
└── shared/
    └── <key_dir>/                     # key = {crate}-{version}-{hash8}
        ├── meta.json                  # 身份字段 + schema + 时间（最后写；缺它视为半成品）
        ├── items.json.gz              # 该 crate 的 Vec<ItemSummary>（JSON + gzip）
        └── md/<crate>/...             # 渲染的 md，与项目 out_root/<crate>/ 逐字符同构
```

- **身份键**：`{crate 名}\0{版本}\0{rustdoc 版本}\0{粒度}\0{file_count}\0{size_sum}` 的 FNV-1a，目录名 `{encode(name)}-{encode(version)}-{hash8}`。
  - **不含 mtime**：各项目 `target/doc` 副本 mtime 不同，含 mtime 会导致永不命中。
  - **含粒度**：成员 `file` 在 `member` / `item` 粒度指向不同文件，粒度不同不可复用。
  - 版本取自 crate 首页 `<span class="version">`，rustdoc 版本取自 `data-rustdoc-version`（`parse_crate_version` / `parse_rustdoc_meta`）。
- **md 物化**：命中后把 `<key>/md/<crate>/...` 物化到项目 `out_root/<crate>/...`——同卷**硬链接**（Windows 无需提权），跨卷回退**复制**；不用软链接（Windows 需权限）。
- **并发**：`atomic_write` 的临时文件名带 `pid` + 单调序号（进程唯一）；`meta.json` 最后写作提交标记，避免读到半成品。
- **复用条目的 `src_mtime`**：按**本项目** html 的 mtime 回填（`crate_scan` 顺带收集），否则本项目后续 `--incremental` 会永远判定为变化。

## 7. 缓存策略

- **一级（启动即建，毫秒级）**：已落盘 `index.json.gz` → gunzip + `serde_json` 加载，并重建不落盘的派生字段。首次构建时只读 `crates.js` + 各 `sidebar-items.js`，再对每个 html **只读文件头抓 `<meta name=description>` 与 `data-rustdoc-version`**（不建 DOM）得到一行摘要。
- **二级（按需）**：`get_item` 时才建 DOM、抽 section/members、渲染 md；结果进内存缓存（`Arc<DocItem>` + LRU 上限，如 512）。
- **指纹**：`Fingerprint{ file_count, max_mtime, size_sum, crates_js_hash }`，一次 `walkdir` 只 stat 不读内容，复杂度 O(文件数)。
- **stale 判定**：fingerprint 不等 **或** 页面 `data-rustdoc-version` 变化 **或** `schema_version` 变化 → 重建。默认全量重建索引（很快）；`--incremental` 时按 `src_mtime` 逐文件比对，仅更新变化项。
- CPU 密集的解析/渲染在 async handler 中用 `tokio::task::spawn_blocking` 包裹。

**启动与就绪（不阻塞 `initialize`）**：server 构造时只同步加载已有 `index.json.gz`（毫秒~亚秒级）即开始服务；索引的过期判定与重建交给后台任务，避免冷启动（首次全量构建可能需 1~2 分钟）拖住 `initialize`。工具与资源在访问索引前等待「就绪」信号（`ensure_ready`）：有旧索引则先服务、后台再刷新；完全没有索引时用空占位并等待构建完成。

**多项目就绪（M11）**：每个项目有自己的 `ready` 与 `DocCache`。缺省项目在启动时 eager 构建；其余项目**懒启动**——首次被工具/资源访问时才启动后台构建。所有项目的构建共用一把 `build_lock` 串行执行，避免并发跑 rayon 抢占资源、叠加内存峰值。

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
> 冷启动时 `initialize` 立即返回，工具/资源会等待后台索引就绪后再返回（见 §7）。

**多项目（M11）**：全部工具新增可选入参 `project`（省略时用缺省项目）；`list_crates` / `index_status` 的空参改为可选 `project`，并在输出回显 `project`。新增 `list_projects` 工具（**不阻塞**，返回各项目名 / 目录 / 就绪 / 构建中 / 计数 / default 标记），是 Agent 发现项目名的入口。`rebuild_index` 输出增加 `shared_hits`。

### 资源

`rustdoc://crates`、`rustdoc://{crate}`（crate `index.md`）、`rustdoc://{crate}/{*item}`（条目），用 `ResourceTemplate` 实现 `list_resource_templates` + `read_resource`。

多项目下这些 URI 均支持 `?project=NAME` 查询参数（缺省回落默认项目），例如 `rustdoc://crates?project=cli`、`rustdoc://tokio?project=svc&offset=0&limit=50`。`list_resources` 只发项目级资源（`rustdoc://crates` + 多项目时每个项目的 `rustdoc://crates?project=NAME`），**不逐 crate 列举、不等待就绪**，避免列表膨胀与阻塞。

### 典型 Agent 流程

`search_items("spawn", crate="tokio")` → 命中若干摘要 → `get_item("tokio::spawn")` 或读 `rustdoc://tokio/spawn` → **只拿到这一小块**。

### rmcp 落点

- features：`["server", "transport-io", "schemars"]`（`macros` 默认开）。
- `#[tool_router]` + `#[tool]`；stdio 用 `server.serve((stdin, stdout)).await` + `service.waiting().await`。
- rmcp 类型只出现在 server crate，core 零感知。

## 9. CLI

```
mcp-docs export [--out DIR] [--doc-dir target/doc] [--incremental] [--granularity member|item]  # 导出 md + index.json.gz
mcp-docs tree   [--doc-dir target/doc]                               # 打印条目树（调试）
mcp-docs show   <id> [--out DIR] [--doc-dir target/doc]              # 打印单条目摘要（走索引）
mcp-docs search <query> [--crate X] [--limit N] [--offset N]         # 检索（可分页）
```

`show` 与 MCP `get_item` **行为一致**（输出为结构化摘要，非整篇 markdown）：id 走索引查找、可省略类型标记（`show tokio::spawn`）；命中**跨 crate 模块重导出别名**（如 `gpui_kit::component`）时委托目标 crate 的合成概览；id 恰为已索引 crate 名时同样回退该概览。因此 `show` 依赖 `index.json.gz`，需先 `export`。

全局参数新增 `--store DIR`（env `MCP_DOCS_STORE`），指向跨项目共享索引库；缺省用平台缓存目录。`export` 的输出会打印「共享库命中 / 写入」计数。

MCP server 的启动参数**全部通过环境变量传入**（不解析命令行）：`MCP_DOCS_DIR` / `MCP_DOCS_OUT`（单项目便捷写法）、`MCP_DOCS_PROJECTS`（分号分隔的 `NAME=DOC_DIR` 列表）、`MCP_DOCS_PROJECTS_FILE`（JSON 清单）、`MCP_DOCS_STORE`、`MCP_DOCS_NO_STORE`（非空即关闭共享库，不读也不写全局缓存）、`MCP_DOCS_DEFAULT_PROJECT`。

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
3. **CLI 集成（`assert_cmd` + `predicates` + `tempfile`）**：`export --out <tmp>` 后断言文件树与 `index.json.gz` 可解析、条目数正确。
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
| 共享库跨卷硬链接不可用 | md 物化失败 | `fs::hard_link` 失败回退 `fs::copy`；不用软链接（Windows 需权限） |
| 共享库无限增长 | 磁盘膨胀 | 每个 `(crate,version,rustdoc,粒度,指纹)` 一份；`meta.generated_at` 备 GC（本期不做自动清理） |
| 多进程并发写共享库条目 | 文件撕裂 | md 用 `atomic_write_fast`；临时名进程唯一；`meta.json` 最后写作提交标记 |
| stat 指纹弱（无内容哈希） | 极端下误复用 | 身份键含 crate 版本；如需更强可并入 `sidebar-items.js` 哈希 |
| N 项目并行全量构建 | rayon 抢占、内存峰值 | 全部构建共用一把 `build_lock` 串行化 |

## 12. 依赖版本

```toml
[workspace.dependencies]
scraper    = "0.27"
htmd       = "0.5"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
walkdir    = "2"
rayon      = "1"
thiserror  = "2"
anyhow     = "1"
clap       = { version = "4", features = ["derive", "env"] }
rmcp       = { version = "3.5", features = ["server", "transport-io", "schemars"] }
tokio      = { version = "1", features = ["rt-multi-thread", "macros", "io-std", "sync"] }
# dev
tempfile   = "3"
criterion  = "0.5"
```

（`tokio` 的 `sync` feature 供 `watch` / `Mutex` 使用。）
