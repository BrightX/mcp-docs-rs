//! 单条目解析测试，基于真实 rustdoc 产物 fixture。

use std::path::PathBuf;

use mcp_docs_core::{
    DocItem, ItemKind, ParseOptions, parse_item_html, parse_one_line, parse_rustdoc_meta,
};

/// fixture 根目录，等价于一个 `target/doc`。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 解析 fixture 中某个页面（参数为相对 `doc_root` 的路径）。
fn parse(rel: &str) -> DocItem {
    let rel_path = PathBuf::from(rel);
    let html = std::fs::read_to_string(fixture_root().join(&rel_path)).unwrap();
    parse_item_html(&html, &rel_path, &ParseOptions::default()).unwrap()
}

/// 把多行文本折叠成单行，便于断言签名。
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn parses_struct_signature_and_main_docs() {
    let item = parse("doc_probe/struct.Demo.html");
    assert_eq!(item.id.0, "doc_probe::struct.Demo");
    assert_eq!(item.kind, ItemKind::Struct);
    assert_eq!(
        collapse(item.signature.as_deref().unwrap()),
        "pub struct Demo { pub field: u32, }"
    );
    assert!(item.docs_md.as_deref().unwrap().contains("A demo struct."));
}

#[test]
fn parses_struct_method_and_field_members() {
    let item = parse("doc_probe/struct.Demo.html");

    let method = item
        .members
        .iter()
        .find(|m| m.id.0 == "doc_probe::struct.Demo::method.new")
        .expect("应识别出方法 Demo::new");
    assert_eq!(method.kind, ItemKind::Method);
    assert!(method.docs_md.as_deref().unwrap().contains("build a demo"));

    let field = item
        .members
        .iter()
        .find(|m| m.id.0 == "doc_probe::struct.Demo::field.field")
        .expect("应识别出字段 Demo::field");
    assert_eq!(field.kind, ItemKind::Field);
    assert!(field.docs_md.as_deref().unwrap().contains("the field"));
}

#[test]
fn parses_enum_variants() {
    let item = parse("doc_probe/enum.Kind.html");
    let ids: Vec<&str> = item.members.iter().map(|m| m.id.0.as_str()).collect();
    assert!(
        ids.contains(&"doc_probe::enum.Kind::variant.A"),
        "成员：{ids:?}"
    );
    assert!(
        ids.contains(&"doc_probe::enum.Kind::variant.B"),
        "成员：{ids:?}"
    );
}

#[test]
fn parses_trait_required_method_as_tymethod() {
    let item = parse("doc_probe/trait.DoIt.html");
    let method = item
        .members
        .iter()
        .find(|m| m.id.0 == "doc_probe::trait.DoIt::tymethod.run")
        .expect("应识别出 trait 必需方法 DoIt::run");
    assert_eq!(method.kind, ItemKind::TyMethod);
}

#[test]
fn splits_sections_and_skips_noise() {
    let item = parse("doc_probe/struct.Demo.html");
    let ids: Vec<&str> = item.sections.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"fields"));
    assert!(ids.contains(&"implementations"));
    // 文档分节（无 section-header class、位于 docblock 内）也要收集（ISSUE-1）。
    assert!(ids.contains(&"examples"));
    // 噪声区块默认剥离。
    assert!(!ids.contains(&"synthetic-implementations"));
    assert!(!ids.contains(&"blanket-implementations"));

    let examples = item
        .sections
        .iter()
        .find(|section| section.id == "examples")
        .expect("应收集到 examples 文档分节");
    assert!(
        examples.body_md.contains("doc_probe::Demo"),
        "examples 正文应包含示例代码，实际：{}",
        examples.body_md
    );
}

#[test]
fn parses_rustdoc_meta_and_one_line() {
    let html = std::fs::read_to_string(fixture_root().join("doc_probe/struct.Demo.html")).unwrap();
    let (version, krate) = parse_rustdoc_meta(&html).unwrap();
    assert!(version.starts_with("1."), "版本：{version}");
    assert_eq!(krate, "doc_probe");
    assert_eq!(parse_one_line(&html).unwrap(), "A demo struct.");
}

#[test]
fn normalizes_source_path_relative_to_doc_root() {
    // 页面里的源码链接是 `../src/doc_probe/lib.rs.html#10-13`（相对当前页面），
    // 归一化后应是相对 doc_root 的 `src/doc_probe/lib.rs.html`。
    let item = parse("doc_probe/struct.Demo.html");
    let source = item.source.expect("应识别出源码链接");
    assert_eq!(source.file, "src/doc_probe/lib.rs.html");
    assert_eq!(source.line_start, Some(10));
    assert_eq!(source.line_end, Some(13));
}
