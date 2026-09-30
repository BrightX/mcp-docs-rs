//! 基于真实 rustdoc 产物 fixture 的发现与 sidebar 解析测试。

use std::path::PathBuf;

use mcp_docs_core::{discover_all, list_crates, parse_sidebar_str, ItemKind};

/// fixture 根目录，等价于一个 `target/doc`。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

#[test]
fn parses_single_and_plural_sidebar_keys() {
    // 单数 key：rustdoc 1.95 实测形式。
    let js = r#"window.SIDEBAR_ITEMS = {"enum":["Kind"],"fn":["free_fn"],"mod":["inner"],"struct":["Demo"],"trait":["DoIt"]};"#;
    let groups = parse_sidebar_str(js).unwrap();
    assert_eq!(groups[&ItemKind::Module], vec!["inner"]);
    assert_eq!(groups[&ItemKind::Struct], vec!["Demo"]);
    assert_eq!(groups[&ItemKind::Enum], vec!["Kind"]);
    assert_eq!(groups[&ItemKind::Trait], vec!["DoIt"]);
    assert_eq!(groups[&ItemKind::Function], vec!["free_fn"]);

    // 复数 key：兼容旧版 rustdoc。
    let js = r#"window.SIDEBAR_ITEMS = {"structs":["A"],"functions":["f"]};"#;
    let groups = parse_sidebar_str(js).unwrap();
    assert_eq!(groups[&ItemKind::Struct], vec!["A"]);
    assert_eq!(groups[&ItemKind::Function], vec!["f"]);
}

#[test]
fn ignores_unknown_sidebar_keys() {
    let js = r#"window.SIDEBAR_ITEMS = {"struct":["Demo"],"whatever":["x"]};"#;
    let groups = parse_sidebar_str(js).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[&ItemKind::Struct], vec!["Demo"]);
}

#[test]
fn errors_on_missing_sidebar_assignment() {
    assert!(parse_sidebar_str("var x = 1;").is_err());
}

#[test]
fn lists_crates_from_fixture() {
    let crates = list_crates(&fixture_root()).unwrap();
    assert_eq!(crates, vec!["doc_probe".to_string()]);
}

#[test]
fn discovers_all_items_in_fixture() {
    let items = discover_all(&fixture_root()).unwrap();
    let ids: Vec<&str> = items.iter().map(|item| item.id.0.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "doc_probe::inner",
            "doc_probe::Demo",
            "doc_probe::Kind",
            "doc_probe::DoIt",
            "doc_probe::free_fn",
            "doc_probe::inner::Nested",
        ]
    );
}
