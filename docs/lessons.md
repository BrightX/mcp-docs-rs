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

（暂无条目）

## 4. MCP 接口

（暂无条目）

## 5. 工程、依赖与工具链

（暂无条目）
