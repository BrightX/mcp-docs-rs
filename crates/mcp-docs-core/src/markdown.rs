//! 把解析出的条目渲染成 markdown。

use crate::model::{DocItem, ItemKind, SourceRef};

/// 链接输出风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkStyle {
    /// 写成指向目标 markdown 的相对路径（默认）。
    #[default]
    Relative,
    /// 一律写成纯文本路径，最省 token。
    PlainPath,
    /// 保留原始 HTML 链接（调试用）。
    KeepOriginal,
}

/// 渲染选项。
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// 链接风格。
    pub link_style: LinkStyle,
    /// 是否把成员内联进父文件（成员同时也会单独落盘）。
    pub include_members_inline: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            link_style: LinkStyle::default(),
            include_members_inline: true,
        }
    }
}

/// 渲染一个条目为 markdown。
pub fn render_item(item: &DocItem, opts: &RenderOptions) -> String {
    let mut out = String::new();

    out.push_str(&format!("# `{}`\n\n", item.id));
    out.push_str(kind_label(item.kind));
    if let Some(source) = &item.source {
        out.push_str(&format!(" · 源码 `{}`", source_label(source)));
    }
    out.push_str("\n\n");

    if let Some(signature) = &item.signature {
        out.push_str(&format!("```rust\n{}\n```\n\n", signature.trim()));
    }

    if let Some(docs) = &item.docs_md {
        let docs = rewrite_links(docs.trim(), opts.link_style);
        if !docs.is_empty() {
            out.push_str(&docs);
            out.push_str("\n\n");
        }
    }

    if opts.include_members_inline {
        for (kind, members) in group_members(&item.members) {
            out.push_str(&format!("## {}\n\n", group_title(kind)));
            for member in members {
                render_member(&mut out, member, opts);
            }
        }
    }

    format!("{}\n", out.trim_end())
}

/// 渲染单个成员为独立 markdown 文件的内容。
pub fn render_member_item(member: &DocItem, opts: &RenderOptions) -> String {
    let mut out = format!("# `{}`\n\n", member.id);
    out.push_str(kind_label(member.kind));
    out.push_str("\n\n");
    render_member_body(&mut out, member, opts);
    format!("{}\n", out.trim_end())
}

/// 渲染一个成员小节（父文件内联用）。
fn render_member(out: &mut String, member: &DocItem, opts: &RenderOptions) {
    out.push_str(&format!("### `{}`\n\n", member.name));
    render_member_body(out, member, opts);
}

/// 渲染成员的签名与文档正文。
fn render_member_body(out: &mut String, member: &DocItem, opts: &RenderOptions) {
    if let Some(signature) = &member.signature {
        out.push_str(&format!("```rust\n{}\n```\n\n", signature.trim()));
    }
    if let Some(docs) = &member.docs_md {
        let docs = rewrite_links(docs.trim(), opts.link_style);
        if !docs.is_empty() {
            out.push_str(&docs);
            out.push_str("\n\n");
        }
    }
}

/// 按成员类型分组，保持首次出现的顺序。
fn group_members(members: &[DocItem]) -> Vec<(ItemKind, Vec<&DocItem>)> {
    let mut groups: Vec<(ItemKind, Vec<&DocItem>)> = Vec::new();
    for member in members {
        match groups.iter_mut().find(|(kind, _)| *kind == member.kind) {
            Some((_, list)) => list.push(member),
            None => groups.push((member.kind, vec![member])),
        }
    }
    groups
}

/// 成员分组标题。
fn group_title(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Method => "Methods",
        ItemKind::TyMethod => "Required Methods",
        ItemKind::AssocConst => "Associated Constants",
        ItemKind::AssocType => "Associated Types",
        ItemKind::Variant => "Variants",
        ItemKind::Field => "Fields",
        _ => "Members",
    }
}

/// 条目类型的人类可读名。
fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Module => "module",
        ItemKind::Struct => "struct",
        ItemKind::Enum => "enum",
        ItemKind::Union => "union",
        ItemKind::Trait => "trait",
        ItemKind::TraitAlias => "trait alias",
        ItemKind::Function => "function",
        ItemKind::TypeAlias => "type alias",
        ItemKind::Constant => "constant",
        ItemKind::Static => "static",
        ItemKind::Macro => "macro",
        ItemKind::Primitive => "primitive",
        ItemKind::Keyword => "keyword",
        ItemKind::Derive => "derive macro",
        ItemKind::ProcMacro => "proc macro",
        ItemKind::Attribute => "attribute macro",
        ItemKind::Method => "method",
        ItemKind::TyMethod => "required method",
        ItemKind::AssocConst => "associated constant",
        ItemKind::AssocType => "associated type",
        ItemKind::Variant => "variant",
        ItemKind::Field => "field",
        ItemKind::Impl => "impl",
        ItemKind::Unknown => "item",
    }
}

/// 源码位置的可读形式，如 `lib.rs:15-18`。
fn source_label(source: &SourceRef) -> String {
    match (source.line_start, source.line_end) {
        (Some(start), Some(end)) if start == end => format!("{}:{}", source.file, start),
        (Some(start), Some(end)) => format!("{}:{}-{}", source.file, start, end),
        _ => source.file.clone(),
    }
}

/// 按输出风格重写 markdown 中的链接。
///
/// 对外可见，供 `one_line` 摘要等复用（见 `docs/lessons.md` #1.22）。
pub fn rewrite_links(markdown: &str, style: LinkStyle) -> String {
    match style {
        LinkStyle::KeepOriginal => markdown.to_string(),
        LinkStyle::Relative => rewrite_link_targets(markdown),
        LinkStyle::PlainPath => strip_links(markdown),
    }
}

/// 把 markdown 链接目标里的 `.html` 换成 `.md`。
///
/// 只处理 `](目标)` 里的目标，因此正文里出现的 `.html` 不会被误改。
/// htmd 生成的目标可能带 title（如 `struct.Foo.html "struct Foo"`）
/// 或锚点（`struct.Foo.html#method.bar`），这些都需保留。
fn rewrite_link_targets(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;

    while let Some(open) = rest.find("](") {
        out.push_str(&rest[..open + 2]);
        let body = &rest[open + 2..];
        match body.find(')') {
            Some(close) => {
                out.push_str(&rewrite_one_target(&body[..close]));
                out.push(')');
                rest = &body[close + 1..];
            }
            None => {
                out.push_str(body);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 重写单个链接目标：路径换扩展名，title 原样保留。
fn rewrite_one_target(target: &str) -> String {
    // title 紧跟一个空白（`path "title"`），路径本身不含空白。
    let (path, rest) = match target.find(char::is_whitespace) {
        Some(idx) => (&target[..idx], &target[idx..]),
        None => (target, ""),
    };
    format!("{}{rest}", rewrite_path_ext(path))
}

/// 把路径结尾的 `.html` 换成 `.md`，锚点原样保留。
fn rewrite_path_ext(path: &str) -> String {
    let (before, anchor) = match path.split_once('#') {
        Some((head, anchor)) => (head, Some(anchor)),
        None => (path, None),
    };
    let rewritten = match before.strip_suffix(".html") {
        Some(stem) => format!("{stem}.md"),
        None => before.to_string(),
    };
    match anchor {
        Some(anchor) => format!("{rewritten}#{anchor}"),
        None => rewritten,
    }
}

/// 把 markdown 链接 `[文本](目标)` 简化为纯文本 `文本`。
fn strip_links(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;

    loop {
        let Some(open) = rest.find('[') else {
            out.push_str(rest);
            break;
        };
        let after = &rest[open + 1..];
        if let Some(bracket) = after.find("](") {
            if let Some(paren) = after[bracket + 2..].find(')') {
                out.push_str(&rest[..open]);
                out.push_str(&after[..bracket]);
                rest = &after[bracket + 2 + paren + 1..];
                continue;
            }
        }
        out.push_str(&rest[..=open]);
        rest = after;
    }

    out
}
