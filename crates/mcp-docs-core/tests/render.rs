//! 渲染、链接与落盘相关测试。

use std::path::PathBuf;

use mcp_docs_core::{
    encode_fs_name, item_output_path, member_output_path, parse_item_html, render_item,
    resolve_href, DocItem, ParseOptions, RenderOptions, Resolved,
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
    // 链接已重写为 .md。
    assert!(
        !markdown.contains(".html)"),
        "链接未重写为 .md：\n{markdown}"
    );
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
        markdown.starts_with("# `doc_probe::Demo::new`"),
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
        member_output_path(&out, &PathBuf::from("doc_probe/struct.Demo.html"), "new"),
        PathBuf::from("out/doc_probe/struct.Demo.new.md")
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
