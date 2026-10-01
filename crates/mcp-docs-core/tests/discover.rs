//! 基于真实 rustdoc 产物 fixture 的发现与 sidebar 解析测试。

use std::fs;
use std::path::PathBuf;

use mcp_docs_core::{
    discover_all, discover_crate, list_crates, parse_all_str, parse_sidebar_str, ItemKind,
};

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
fn parses_rustdoc_198_macro_pairs() {
    // rustdoc 1.98 起，宏条目变成 `[名字, 标志]` 二元组，其余类型仍是纯字符串。
    let js = r#"window.SIDEBAR_ITEMS = {"macro":[["eprint",1],["println",1]],"struct":["Demo"]};"#;
    let groups = parse_sidebar_str(js).unwrap();
    assert_eq!(groups[&ItemKind::Macro], vec!["eprint", "println"]);
    assert_eq!(groups[&ItemKind::Struct], vec!["Demo"]);
}

#[test]
fn parse_all_str_maps_hrefs() {
    // 覆盖：普通条目 / 跨模块条目 / 模块 index.html / 带锚点 / 宏重定向页 `!` /
    // 外链与逃出 crate 的 `..`（应跳过）。
    let html = r#"<section id="main-content"><ul class="all-items">
        <li><a href="struct.Demo.html">Demo</a></li>
        <li><a href="inner/struct.Nested.html">inner::Nested</a></li>
        <li><a href="inner/index.html">inner</a></li>
        <li><a href="enum.Kind.html#variant.A">Kind</a></li>
        <li><a href="macro.record_all!.html">record_all</a></li>
        <li><a href="https://example.com/x.html">外链</a></li>
        <li><a href="../other/struct.Foo.html">越界</a></li>
    </ul></section>"#;

    let items = parse_all_str(html, "doc_probe");
    let ids: Vec<&str> = items.iter().map(|item| item.id.0.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "doc_probe::struct.Demo",
            "doc_probe::inner::struct.Nested",
            "doc_probe::mod.inner",
            "doc_probe::enum.Kind",
            "doc_probe::macro.record_all",
        ]
    );
    // `!` 重定向页应规整为真实文件。
    assert_eq!(
        items[4].html_path,
        PathBuf::from("doc_probe/macro.record_all.html")
    );
}

#[test]
fn all_html_supplements_missing_submodule() {
    // 构造一个 sidebar 未列、目录扫描也不会下钻的子模块条目，
    // 只能由 all.html 兜底补全。
    let tmp = tempfile::tempdir().unwrap();
    let doc_root = tmp.path();
    let krate = doc_root.join("doc_probe");
    fs::create_dir_all(krate.join("ghostmod")).unwrap();
    fs::write(
        krate.join("sidebar-items.js"),
        r#"window.SIDEBAR_ITEMS = {"struct":["Demo"]};"#,
    )
    .unwrap();
    fs::write(krate.join("struct.Demo.html"), "<html></html>").unwrap();
    fs::write(
        krate.join("ghostmod").join("struct.Ghost.html"),
        "<html></html>",
    )
    .unwrap();
    fs::write(
        krate.join("all.html"),
        r#"<section id="main-content"><ul class="all-items">
            <li><a href="struct.Demo.html">Demo</a></li>
            <li><a href="ghostmod/struct.Ghost.html">ghostmod::Ghost</a></li>
        </ul></section>"#,
    )
    .unwrap();

    let items = discover_crate(doc_root, "doc_probe").unwrap();
    let ids: Vec<&str> = items.iter().map(|item| item.id.0.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "doc_probe::struct.Demo",
            "doc_probe::ghostmod::struct.Ghost"
        ]
    );
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
            "doc_probe::mod.inner",
            "doc_probe::struct.Demo",
            "doc_probe::enum.Kind",
            "doc_probe::trait.DoIt",
            "doc_probe::fn.free_fn",
            "doc_probe::inner::struct.Nested",
        ]
    );
}
