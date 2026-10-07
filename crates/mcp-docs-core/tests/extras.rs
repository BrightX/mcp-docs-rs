//! M10 新增能力的测试：示例抽取、源码行抽取、模块树、相关条目、trait 实现者。

use std::path::{Path, PathBuf};

use mcp_docs_core::{
    Index, build_index, extract_code_blocks, extract_source_lines, module_tree, parse_trait_impls,
    related_items, trait_impl_rel_path,
};

/// fixture 根目录，等价于一个 `target/doc`。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 构建 fixture 的索引。
fn build() -> Index {
    build_index(&fixture_root(), Path::new("out")).unwrap()
}

#[test]
fn extracts_rust_code_blocks() {
    let markdown = "# x\n\n```rust\nlet a = 1;\n```\n\ntext\n\n```text\nno\n```\n";
    let blocks = extract_code_blocks(markdown);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].0, "rust");
    assert_eq!(blocks[0].1, "let a = 1;");
    assert_eq!(blocks[1].0, "text");
}

#[test]
fn extracts_source_lines_from_src_page() {
    let html = r##"<html><body><pre class="rust"><code><a href="#1" id="1">1</a>let a = 1;
<a href="#2" id="2">2</a>let b = 2;
<a href="#3" id="3">3</a>let c = 3;</code></pre></body></html>"##;
    let text = extract_source_lines(html, 2, 3).unwrap();
    assert_eq!(text, "let b = 2;\nlet c = 3;");
}

#[test]
fn module_tree_contains_submodule() {
    let tree = module_tree(&build().items, "doc_probe").unwrap();
    assert_eq!(tree.name, "doc_probe");
    assert!(tree.children.iter().any(|child| child.name == "inner"));
    assert!(module_tree(&build().items, "不存在").is_none());
}

#[test]
fn related_items_of_member_points_to_parent() {
    let index = build();
    let related = related_items(&index.items, "doc_probe::struct.Demo::method.new").unwrap();
    assert_eq!(
        related.parent.as_ref().unwrap().id.0,
        "doc_probe::struct.Demo"
    );
    // 成员条目的兄弟是同父下的其它成员（E-2.2）。
    assert!(!related.siblings.is_empty(), "成员应有兄弟成员");
    assert!(
        related
            .siblings
            .iter()
            .all(|sibling| sibling.parent_id.as_deref() == Some("doc_probe::struct.Demo"))
    );

    // 父条目应含子成员。
    let parent = related_items(&index.items, "doc_probe::struct.Demo").unwrap();
    assert!(
        parent
            .children
            .iter()
            .any(|child| child.id.0 == "doc_probe::struct.Demo::method.new")
    );
}

#[test]
fn index_summary_has_signature() {
    let index = build();
    let demo = index
        .items
        .iter()
        .find(|item| item.id.0 == "doc_probe::struct.Demo")
        .unwrap();
    assert!(
        demo.signature
            .as_deref()
            .is_some_and(|signature| signature.contains("struct Demo")),
        "signature: {:?}",
        demo.signature
    );
}

#[test]
fn parses_trait_impls_from_js() {
    let js = r#"(function() {
    const implementors = Object.fromEntries([["selectors",[["impl <a class=\"trait\" href=\"x\">Flags</a> for <a class=\"struct\" href=\"y\">Elem</a>",0]]]]);
})()"#;
    let impls = parse_trait_impls(js);
    assert_eq!(impls.len(), 1);
    assert_eq!(impls[0].crate_name, "selectors");
    assert_eq!(impls[0].text, "impl Flags for Elem");
}

#[test]
fn trait_impl_path_mirrors_page_path() {
    let path = trait_impl_rel_path(Path::new("bitflags/traits/trait.Flags.html"));
    assert_eq!(
        path.to_string_lossy().replace('\\', "/"),
        "trait.impl/bitflags/traits/trait.Flags.js"
    );
}
