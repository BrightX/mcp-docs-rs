//! 解析单个 rustdoc HTML 页面。
//!
//! 只依赖 rustdoc 长期稳定的结构不变量（见 `docs/design.md` §1）：
//! `section#main-content`、`pre.rust.item-decl`、`details.top-doc .docblock`、
//! `h2.section-header`、`section[id^="method."]` 等。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use scraper::{ElementRef, Html, Node, Selector};

use crate::error::{Error, Result};
use crate::link::{resolve_href, Resolved};
use crate::model::{DocItem, ItemId, ItemKind, Section, SourceRef};

/// 解析并缓存内置选择器。
///
/// 选择器字面量在每个调用点只编译一次并缓存（`OnceLock`），避免每次解析
/// 页面都重新编译同一批 CSS。选择器是常量字符串，解析失败属编程错误。
macro_rules! selector {
    ($css:expr $(,)?) => {{
        static CACHE: OnceLock<Selector> = OnceLock::new();
        CACHE.get_or_init(|| Selector::parse($css).expect("内置选择器应始终合法"))
    }};
}

/// 解析选项。
#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// 是否保留自动 / blanket trait impl。
    ///
    /// 默认剥离：它们是页面体积的主要来源，且对理解 API 无用。
    pub include_auto_impls: bool,
}

/// 解析单个条目 HTML。
///
/// `rel_path` 为相对 `doc_root` 的路径，用于推导条目身份并记录来源。
pub fn parse_item_html(html: &str, rel_path: &Path, opts: &ParseOptions) -> Result<DocItem> {
    let (kind, path, name) = path_to_identity(rel_path)?;
    let (rustdoc_version, crate_name) = match parse_rustdoc_meta(html) {
        Some((version, krate)) => (Some(version), krate),
        None => (None, path.first().cloned().unwrap_or_default()),
    };

    let document = Html::parse_document(html);
    let main = document
        .select(selector!("section#main-content"))
        .next()
        .ok_or_else(|| Error::Parse {
            path: rel_path.to_path_buf(),
            reason: "未找到 section#main-content".to_string(),
        })?;

    let signature = main
        .select(selector!("pre.rust.item-decl"))
        .next()
        .map(|el| clean_signature(&signature_text(&el)))
        .filter(|text| !text.is_empty());

    let source = main
        .select(selector!(".main-heading a.src"))
        .next()
        .and_then(|el| el.value().attr("href"))
        .and_then(|href| parse_source_href(href, rel_path));

    let docs_md = main
        .select(selector!("details.top-doc .docblock"))
        .next()
        .map(|db| docblock_to_md(&db))
        .filter(|text| !text.is_empty());

    // id 带条目类型标记：同名但不同类型的条目很常见
    // （如 `serde::Deserialize` 既是 trait 又是 derive 宏），
    // 不带类型会导致 id 冲突、增量复用互相覆盖。
    let mut id_parts = path.clone();
    id_parts.push(format!("{}.{}", kind.kind_tag(), name));
    let id = ItemId(id_parts.join("::"));

    let mut sections = collect_sections(&main, opts);
    let members = collect_members(&main, &path, &name, &id);

    // 按类型把成员归组到对应分节；找不到归属的成员仍保留在扁平列表里。
    for member in &members {
        if let Some(section) = sections
            .iter_mut()
            .find(|s| s.id == section_id_for(member.kind))
        {
            section.members.push(member.clone());
        }
    }

    Ok(DocItem {
        id,
        kind,
        name,
        path,
        crate_name,
        signature,
        docs_md,
        source,
        sections,
        members,
        html_path: rel_path.to_path_buf(),
        rustdoc_version,
    })
}

/// 从相对 `doc_root` 的 HTML 路径推导 (条目类型, 模块路径, 名字)。
///
/// - `doc_probe/struct.Demo.html` → `(Struct, ["doc_probe"], "Demo")`
/// - `doc_probe/inner/struct.Nested.html` → `(Struct, ["doc_probe", "inner"], "Nested")`
/// - `doc_probe/inner/index.html` → `(Module, ["doc_probe"], "inner")`
pub fn path_to_identity(rel_path: &Path) -> Result<(ItemKind, Vec<String>, String)> {
    let dirs: Vec<String> = rel_path
        .parent()
        .map(|parent| {
            parent
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let file_name = rel_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| Error::Parse {
            path: rel_path.to_path_buf(),
            reason: "路径没有文件名".to_string(),
        })?;

    if file_name == "index.html" {
        // 模块页：目录名即模块名，其上才是该模块的所属路径。
        let Some(name) = dirs.last().cloned() else {
            return Err(Error::Parse {
                path: rel_path.to_path_buf(),
                reason: "index.html 缺少所属模块目录".to_string(),
            });
        };
        let module_path = dirs[..dirs.len() - 1].to_vec();
        return Ok((ItemKind::Module, module_path, name));
    }

    let stem = file_name.strip_suffix(".html").unwrap_or(&file_name);
    let (prefix, name) = stem.split_once('.').ok_or_else(|| Error::Parse {
        path: rel_path.to_path_buf(),
        reason: format!("文件名 `{file_name}` 不符合 `{{前缀}}.{{名字}}.html` 形式"),
    })?;
    let kind = ItemKind::from_file_prefix(prefix).ok_or_else(|| Error::Parse {
        path: rel_path.to_path_buf(),
        reason: format!("未知的条目前缀 `{prefix}`"),
    })?;
    Ok((kind, dirs, name.to_string()))
}

/// 从页面头部的 `<meta name="rustdoc-vars">` 抓取 (rustdoc 版本, crate 名)。
pub fn parse_rustdoc_meta(html: &str) -> Option<(String, String)> {
    let tag = find_tag(head_slice(html), r#"name="rustdoc-vars""#)?;
    let version = extract_attr(tag, "data-rustdoc-version")?;
    let crate_name = extract_attr(tag, "data-current-crate")?;
    Some((version, crate_name))
}

/// 从页面头部抓取 `<meta name="description">` 的内容，作为条目的一行摘要。
pub fn parse_one_line(html: &str) -> Option<String> {
    let tag = find_tag(head_slice(html), r#"name="description""#)?;
    extract_attr(tag, "content")
}

/// 只取页面开头的一段，避免为了抓 meta 而扫描整页。
fn head_slice(html: &str) -> &str {
    let mut end = html.len().min(4096);
    while end > 0 && !html.is_char_boundary(end) {
        end -= 1;
    }
    &html[..end]
}

/// 找到含 `marker` 的标签，返回从匹配处到 `>` 的片段。
fn find_tag<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
    let start = html.find(marker)?;
    let rest = &html[start..];
    let end = rest.find('>')?;
    Some(&rest[..end])
}

/// 从标签片段中提取属性值。
fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 解析源码链接，如 `../src/doc_probe/lib.rs.html#15-18`。
///
/// `current_rel` 为当前页面相对 `doc_root` 的路径。结果里的 `file` 会被
/// 归一化成**相对 `doc_root`** 的路径（`src/...`），调用方配合 `--doc-dir`
/// 即可直接定位；若沿用原始的 `../../src/...`，对使用者毫无意义。
fn parse_source_href(href: &str, current_rel: &Path) -> Option<SourceRef> {
    let (file, fragment) = match resolve_href(href, current_rel) {
        Resolved::Source { rel, anchor } | Resolved::Item { rel, anchor } => (
            rel.to_string_lossy().replace('\\', "/"),
            anchor.unwrap_or_default(),
        ),
        Resolved::External(url) => (url, String::new()),
        Resolved::InPage(_) => return None,
    };
    if file.is_empty() {
        return None;
    }

    let (line_start, line_end) = match fragment.split_once('-') {
        Some((start, end)) => (start.parse().ok(), end.parse().ok()),
        None if !fragment.is_empty() => (fragment.parse().ok(), fragment.parse().ok()),
        None => (None, None),
    };
    Some(SourceRef {
        file,
        line_start,
        line_end,
    })
}

/// 收集页面上的可见分节（`h2.section-header`），按出现顺序。
fn collect_sections(main: &ElementRef, opts: &ParseOptions) -> Vec<Section> {
    let mut sections = Vec::new();
    for heading in main.child_elements().filter(is_section_header) {
        let id = heading.value().attr("id").unwrap_or_default().to_string();
        if !opts.include_auto_impls && is_noise_section(&id) {
            continue;
        }
        sections.push(Section {
            id,
            title: heading_text(&heading),
            body_md: String::new(),
            members: Vec::new(),
        });
    }
    sections
}

/// 收集页面上全部成员条目（字段 / 变体 / 方法 / 关联项）。
fn collect_members(
    main: &ElementRef,
    parent_path: &[String],
    parent_name: &str,
    parent_id: &ItemId,
) -> Vec<DocItem> {
    let mut members = Vec::new();

    // 字段是 `span`（不是 section），文档紧随其后。
    for anchor in main.select(selector!("span.structfield")) {
        if let Some(member) = build_member(
            &anchor,
            ItemKind::Field,
            parent_path,
            parent_name,
            parent_id,
        ) {
            members.push(member);
        }
    }

    // 枚举变体。
    for anchor in main.select(selector!("section.variant")) {
        if let Some(member) = build_member(
            &anchor,
            ItemKind::Variant,
            parent_path,
            parent_name,
            parent_id,
        ) {
            members.push(member);
        }
    }

    // 方法 / 关联常数 / 关联类型。
    // blanket / synthetic impl 里的成员带 `trait-impl` class，默认跳过。
    for anchor in main.select(selector!(
        "section.method, section.associatedconstant, section.associatedtype",
    )) {
        if has_class(&anchor, "trait-impl") {
            continue;
        }
        let kind = member_kind_from_id(&anchor);
        if let Some(member) = build_member(&anchor, kind, parent_path, parent_name, parent_id) {
            members.push(member);
        }
    }

    members
}

/// 由成员锚点构建成员条目。
fn build_member(
    anchor: &ElementRef,
    kind: ItemKind,
    parent_path: &[String],
    parent_name: &str,
    parent_id: &ItemId,
) -> Option<DocItem> {
    // 成员名取自锚点 id 去掉类型前缀的部分：`method.new` → `new`。
    let anchor_id = anchor.value().attr("id")?;
    let name = anchor_id.split_once('.').map(|(_, n)| n)?.to_string();
    if name.is_empty() {
        return None;
    }

    // 方法 / 变体的签名在 `h3/h4.code-header`；字段的签名在 `code` 里。
    let signature = anchor
        .select(selector!("h3.code-header, h4.code-header"))
        .next()
        .map(|el| clean_signature(&signature_text(&el)))
        .filter(|text| !text.is_empty())
        .or_else(|| {
            anchor
                .select(selector!("code"))
                .next()
                .map(|el| clean_signature(&signature_text(&el)))
                .filter(|text| !text.is_empty())
        });

    let docs_md = find_member_doc(anchor).filter(|text| !text.is_empty());

    let mut path = parent_path.to_vec();
    path.push(parent_name.to_string());

    // 成员 id = 父 id + 「成员类型标记.成员名」，避免同名字段与方法冲突
    // （如 `LitBool` 的字段 `value` 与方法 `value()`）。
    let id = ItemId(format!("{}::{}.{}", parent_id.0, kind.kind_tag(), name));

    Some(DocItem {
        id,
        kind,
        name,
        crate_name: parent_path.first().cloned().unwrap_or_default(),
        path,
        signature,
        docs_md,
        source: None,
        sections: Vec::new(),
        members: Vec::new(),
        html_path: PathBuf::new(),
        rustdoc_version: None,
    })
}

/// 找一个成员锚点的文档。
///
/// 方法 / 关联项的文档在最近祖先 `details` 内；字段 / 变体的文档紧随后面。
fn find_member_doc(anchor: &ElementRef) -> Option<String> {
    // `ancestors()` 经 `Deref` 来自底层 `NodeRef`，需转回 `ElementRef`。
    for ancestor in anchor.ancestors() {
        let Some(element) = ElementRef::wrap(ancestor) else {
            continue;
        };
        if element.value().name() == "details" {
            return element
                .select(selector!(".docblock"))
                .next()
                .map(|db| docblock_to_md(&db));
        }
    }
    let sibling = ElementRef::wrap(anchor.next_sibling()?)?;
    has_class(&sibling, "docblock").then(|| docblock_to_md(&sibling))
}

/// 把 docblock 内部 HTML 转成 markdown。
///
/// 处理两处 rustdoc 特有噪声（见 `docs/lessons.md`）：
/// - `<a class="anchor">§</a>`：转换后表现为 `[§](#锚点)`，由 `strip_anchor_links` 删除；
/// - 代码块语言标注：htmd 默认不带语言，这里按 `<pre>` 的 class 补 `rust`。
fn docblock_to_md(docblock: &ElementRef) -> String {
    let mut html = docblock.inner_html();

    // 1) 用占位符抽出代码块，避免 htmd 丢掉语言信息。
    let mut code_blocks = Vec::new();
    for pre in docblock.select(selector!("pre.rust, pre.rust-example-rendered")) {
        let raw = pre.html();
        if !html.contains(&raw) {
            continue;
        }
        let placeholder = format!("MCPDOCSCODEBLOCK{}END", code_blocks.len());
        html = html.replace(&raw, &placeholder);
        code_blocks.push((code_language(&pre), pre.text().collect::<String>()));
    }

    // 2) 转换，并去掉 rustdoc 注入的 § 锚点链接。
    let converted = htmd::convert(&html).unwrap_or_default();
    let mut markdown = strip_anchor_links(&converted);

    // 3) 还原为带语言标注的围栏代码块。
    for (index, (language, code)) in code_blocks.into_iter().enumerate() {
        let placeholder = format!("MCPDOCSCODEBLOCK{index}END");
        let fenced = format!("```{language}\n{}\n```", code.trim_end());
        markdown = markdown.replace(&placeholder, &fenced);
    }

    markdown.trim().to_string()
}

/// 从 rustdoc 源码页（`src/.../*.rs.html`）提取指定行范围的源码文本。
///
/// 源码页结构为 `pre.rust code`，每一行行首有一个 `<a id="N">N</a>` 行号锚点；
/// 锚点之间的文本即该行内容。结构不符（非 1.98 风格）时返回 `None`，
/// 调用方可回退为「仅返回路径与行号」。
pub fn extract_source_lines(html: &str, start: u32, end: u32) -> Option<String> {
    let document = Html::parse_document(html);
    let code = document
        .select(selector!("pre.rust code"))
        .next()
        .or_else(|| document.select(selector!("pre.rust")).next())?;

    let mut lines: Vec<String> = Vec::new();
    let mut current: Option<usize> = None;
    collect_source_lines(&code, &mut lines, &mut current);
    if lines.is_empty() {
        return None;
    }

    let total = lines.len() as u32;
    let start = start.clamp(1, total);
    let end = end.min(total).max(start);
    let text = lines[(start - 1) as usize..end as usize]
        .iter()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    Some(text)
}

/// 递归收集源码行：行号锚点切换当前行，其余文本累积到当前行。
fn collect_source_lines(el: &ElementRef, lines: &mut Vec<String>, current: &mut Option<usize>) {
    for child in el.children() {
        match child.value() {
            Node::Text(text) => {
                if let Some(line_no) = *current {
                    if lines.len() < line_no {
                        lines.resize(line_no, String::new());
                    }
                    lines[line_no - 1].push_str(&text.text);
                }
            }
            Node::Element(inner) => {
                // 行号锚点：只取其 `id` 作为行号，跳过其文本（号码本身）。
                if inner.name() == "a" {
                    if let Some(line_no) = inner.attr("id").and_then(|id| id.parse::<usize>().ok())
                    {
                        *current = Some(line_no);
                        if lines.len() < line_no {
                            lines.resize(line_no, String::new());
                        }
                        continue;
                    }
                }
                if let Some(child) = ElementRef::wrap(child) {
                    collect_source_lines(&child, lines, current);
                }
            }
            _ => {}
        }
    }
}

/// 清理签名文本里的 rustdoc 装饰字符。
///
/// rustdoc 会在签名中插入可点击的提示图标（如 `ⓘ`），它们是纯噪声。
fn clean_signature(text: &str) -> String {
    text.replace('ⓘ', "").trim().to_string()
}

/// 收集签名文本，跳过 `details` 子树。
///
/// rustdoc 的 trait 签名会把方法列表折叠在 `<details>` 内，其 `<summary>`
/// 文本（如 `Show 29 methods`）不应出现在签名里；方法本身会作为成员单独列出。
fn signature_text(element: &ElementRef) -> String {
    let mut out = String::new();
    collect_text(element, &mut out);
    out
}

/// 递归收集文本；`details` 子树整体跳过。
fn collect_text(element: &ElementRef, out: &mut String) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => out.push_str(&text.text),
            Node::Element(inner) if inner.name() == "details" => {}
            _ => {
                if let Some(child) = ElementRef::wrap(child) {
                    collect_text(&child, out);
                }
            }
        }
    }
}

/// 由 `<pre>` 的 class 推断代码块语言，默认 `rust`。
fn code_language(pre: &ElementRef) -> String {
    let class = pre.value().attr("class").unwrap_or_default();
    for part in class.split_whitespace() {
        if let Some(language) = part.strip_prefix("language-") {
            return language.to_string();
        }
    }
    "rust".to_string()
}

/// 去掉 rustdoc 注入的 `[§](#锚点)` 链接文本。
///
/// rustdoc 在每个标题末尾插入 `<a class="anchor">§</a>`，转成 markdown 后
/// 表现为 `[§](#锚点)`。这里在输出层面整段删除。
fn strip_anchor_links(markdown: &str) -> String {
    const MARKER: &str = "[§](#";
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;

    while let Some(start) = rest.find(MARKER) {
        out.push_str(&rest[..start]);
        match rest[start..].find(')') {
            Some(close) => rest = &rest[start + close + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 由成员锚点 id 判断成员类型。
fn member_kind_from_id(anchor: &ElementRef) -> ItemKind {
    let id = anchor.value().attr("id").unwrap_or_default();
    if id.starts_with("tymethod.") {
        ItemKind::TyMethod
    } else if id.starts_with("associatedconstant.") {
        ItemKind::AssocConst
    } else if id.starts_with("associatedtype.") {
        ItemKind::AssocType
    } else {
        ItemKind::Method
    }
}

/// 成员类型对应的分节 id。
fn section_id_for(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Field => "fields",
        ItemKind::Variant => "variants",
        ItemKind::TyMethod => "required-methods",
        ItemKind::Method | ItemKind::AssocConst | ItemKind::AssocType => "implementations",
        _ => "",
    }
}

/// 该分节是否为默认剥离的噪声区块。
fn is_noise_section(id: &str) -> bool {
    matches!(id, "synthetic-implementations" | "blanket-implementations")
}

/// 是否是由 `h2.section-header` 标记的分节标题。
///
/// 必须限定 `h2`：字段的 `span` 也带 `section-header` class。
fn is_section_header(el: &ElementRef) -> bool {
    el.value().name() == "h2" && has_class(el, "section-header")
}

/// 取分节标题文本，去掉 rustdoc 注入的 `§` 锚点字符。
fn heading_text(heading: &ElementRef) -> String {
    heading
        .text()
        .collect::<String>()
        .replace('§', "")
        .trim()
        .to_string()
}

/// 元素是否含指定 class。
fn has_class(el: &ElementRef, class: &str) -> bool {
    el.value()
        .attr("class")
        .is_some_and(|classes| classes.split_whitespace().any(|c| c == class))
}
