//! 渲染、链接与落盘相关测试。

use std::path::PathBuf;

use mcp_docs_core::{
    encode_fs_name, item_output_path, member_output_path, parse_item_html, render_item,
    resolve_href, rewrite_links, DocItem, ItemKind, LinkStyle, ParseOptions, RenderOptions,
    Resolved,
};

/// fixture 根目录，等价于一个 `target/doc`。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 解析 fixture 中某个页面。
fn parse(rel: &str) -> DocItem {
    let rel_path = PathBuf::from(rel);
    let html = std::fs::read_to_string(fixture_root().join(&rel_path)).unwrap();
    parse_item_html(&html, &rel_path, &ParseOptions::default()).unwrap()
}

#[test]
fn encodes_illegal_and_reserved_names() {
    assert_eq!(encode_fs_name("Demo::new"), "Demo__new");
    assert_eq!(encode_fs_name("Foo<Bar>"), "Foo_Bar_");
    assert_eq!(encode_fs_name("CON"), "_CON");
    assert_eq!(encode_fs_name("com1.txt"), "_com1.txt");
    assert_eq!(encode_fs_name("trailing."), "trailing");
    assert_eq!(encode_fs_name(""), "_");
}

#[test]
fn resolves_href_variants() {
    let nested = PathBuf::from("doc_probe/inner/struct.Nested.html");

    assert_eq!(
        resolve_href("#method.new", &nested),
        Resolved::InPage("method.new".to_string())
    );

    assert_eq!(
        resolve_href("../struct.Demo.html", &nested),
        Resolved::Item {
            rel: PathBuf::from("doc_probe/struct.Demo.html"),
            anchor: None
        }
    );

    assert_eq!(
        resolve_href(
            "struct.Bar.html#method.baz",
            &PathBuf::from("doc_probe/struct.Demo.html")
        ),
        Resolved::Item {
            rel: PathBuf::from("doc_probe/struct.Bar.html"),
            anchor: Some("method.baz".to_string())
        }
    );

    assert!(matches!(
        resolve_href(
            "https://doc.rust-lang.org/1.95.0/std/primitive.u32.html",
            &nested
        ),
        Resolved::External(_)
    ));

    assert_eq!(
        resolve_href(
            "../src/doc_probe/lib.rs.html#15-18",
            &PathBuf::from("doc_probe/struct.Demo.html")
        ),
        Resolved::Source {
            rel: PathBuf::from("src/doc_probe/lib.rs.html"),
            anchor: Some("15-18".to_string())
        }
    );
}

#[test]
fn renders_struct_markdown_without_noise() {
    let item = parse("doc_probe/struct.Demo.html");
    let markdown = render_item(&item, &RenderOptions::default());

    // 代码块带语言标注。
    assert!(markdown.contains("```rust"), "缺少 rust 围栏：\n{markdown}");
    // 不含 § 锚点噪声。
    assert!(!markdown.contains("[§]"), "仍有锚点噪声：\n{markdown}");
    // 不含噪声区块与 UI 文案。
    assert!(!markdown.contains("Auto Trait Implementations"));
    assert!(!markdown.contains("Copy item path"));
    // 关键内容齐备。
    assert!(markdown.contains("pub struct Demo"));
    assert!(markdown.contains("## Fields"));
    assert!(markdown.contains("## Methods"));
    assert!(markdown.contains("### `new`"), "缺少方法小节：\n{markdown}");
    assert!(markdown.contains("build a demo"));
    // 链接已重写为 .md（含带 title 的 htmd 形式 `.html "标题")`）。
    assert!(
        !markdown.contains(".html)") && !markdown.contains(".html \""),
        "链接未重写为 .md：\n{markdown}"
    );
}

#[test]
fn rewrites_link_targets_keeping_title_and_anchor() {
    // htmd 会把 rustdoc 的 `<a title="...">` 转成带 title 的 markdown 链接，
    // 此时 `.html` 后面跟的是空格 + 引号，旧的 `.html)` 替换会漏掉。
    assert_eq!(
        rewrite_links(
            "[`JoinHandle`](struct.JoinHandle.html \"struct tokio::task::JoinHandle\")",
            LinkStyle::Relative
        ),
        "[`JoinHandle`](struct.JoinHandle.md \"struct tokio::task::JoinHandle\")"
    );
    // 带锚点的链接。
    assert_eq!(
        rewrite_links(
            "[`run`](../struct.Foo.html#method.run \"struct Foo\")",
            LinkStyle::Relative
        ),
        "[`run`](../struct.Foo.md#method.run \"struct Foo\")"
    );
    // 外链与正文里出现的 `.html` 不被误改。
    assert_eq!(
        rewrite_links(
            "见 https://example.com/a.html 与正文里的 `foo.html` 字样",
            LinkStyle::Relative
        ),
        "见 https://example.com/a.html 与正文里的 `foo.html` 字样"
    );
    // PlainPath 把链接压成纯文本。
    assert_eq!(
        rewrite_links(
            "[`Foo`](struct.Foo.html \"struct Foo\")",
            LinkStyle::PlainPath
        ),
        "`Foo`"
    );
}

#[test]
fn parses_kind_input_in_multiple_forms() {
    // 文件名前缀。
    assert_eq!(ItemKind::parse_input("fn"), Some(ItemKind::Function));
    assert_eq!(ItemKind::parse_input("struct"), Some(ItemKind::Struct));
    assert_eq!(ItemKind::parse_input("attr"), Some(ItemKind::Attribute));
    // 自然名单数与复数。
    assert_eq!(ItemKind::parse_input("function"), Some(ItemKind::Function));
    assert_eq!(ItemKind::parse_input("functions"), Some(ItemKind::Function));
    // 成员类型（`from_file_prefix` 不含这些，正是早先静默失效的原因）。
    assert_eq!(ItemKind::parse_input("method"), Some(ItemKind::Method));
    assert_eq!(ItemKind::parse_input("field"), Some(ItemKind::Field));
    assert_eq!(ItemKind::parse_input("variant"), Some(ItemKind::Variant));
    // 大小写不敏感；无法识别的返回 None。
    assert_eq!(ItemKind::parse_input("Field"), Some(ItemKind::Field));
    assert_eq!(ItemKind::parse_input("nope"), None);
}

#[test]
fn member_file_has_no_duplicate_heading() {
    let item = parse("doc_probe/struct.Demo.html");
    let member = item
        .members
        .iter()
        .find(|m| m.name == "new")
        .expect("应识别出 Demo::new");
    let markdown = mcp_docs_core::render_member_item(member, &RenderOptions::default());
    assert!(
        markdown.starts_with("# `doc_probe::struct.Demo::method.new`"),
        "{markdown}"
    );
    assert!(
        !markdown.contains("### `new`"),
        "独立成员文件不应再有子标题：\n{markdown}"
    );
    assert!(markdown.contains("build a demo"));
}

#[test]
fn member_and_item_output_paths() {
    let out = PathBuf::from("out");
    assert_eq!(
        member_output_path(
            &out,
            &PathBuf::from("doc_probe/struct.Demo.html"),
            ItemKind::Method,
            "new"
        ),
        PathBuf::from("out/doc_probe/struct.Demo.method.new.md")
    );
    assert_eq!(
        item_output_path(&out, &PathBuf::from("doc_probe/inner/struct.Nested.html")),
        PathBuf::from("out/doc_probe/inner/struct.Nested.md")
    );
    assert_eq!(
        item_output_path(&out, &PathBuf::from("doc_probe/inner/index.html")),
        PathBuf::from("out/doc_probe/inner/index.md")
    );
}
