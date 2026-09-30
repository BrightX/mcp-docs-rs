//! 索引构建与检索测试。

use std::path::{Path, PathBuf};

use mcp_docs_core::{build_index, search, Index, MatchMode, SearchQuery};

/// fixture 根目录，等价于一个 `target/doc`。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 构建 fixture 的索引（输出根仅用于推导路径，无需存在）。
fn build() -> Index {
    build_index(&fixture_root(), Path::new("out")).unwrap()
}

#[test]
fn index_contains_items_and_members() {
    let index = build();
    // 6 个顶层条目 + 5 个成员（Demo 的 field/new、Kind 的 A/B、DoIt 的 run）。
    assert_eq!(index.items.len(), 11, "{:#?}", index.items);

    let ids: Vec<&str> = index.items.iter().map(|item| item.id.0.as_str()).collect();
    assert!(ids.contains(&"doc_probe::Demo"));
    assert!(ids.contains(&"doc_probe::Demo::new"));
    assert!(ids.contains(&"doc_probe::Demo::field"));
    assert!(ids.contains(&"doc_probe::Kind::A"));
    assert!(ids.contains(&"doc_probe::inner::Nested"));
}

#[test]
fn summary_files_point_to_markdown() {
    let index = build();
    let find = |id: &str| index.items.iter().find(|item| item.id.0 == id).unwrap();

    // `file` 是相对输出根目录的路径。
    assert_eq!(find("doc_probe::Demo").file, "doc_probe/struct.Demo.md");
    assert_eq!(
        find("doc_probe::Demo::new").file,
        "doc_probe/struct.Demo.new.md"
    );
    assert_eq!(
        find("doc_probe::inner::Nested").file,
        "doc_probe/inner/struct.Nested.md"
    );
}

#[test]
fn search_finds_method_by_name() {
    let index = build();
    let hits = search(&index, &SearchQuery::new("new"));
    assert!(
        hits.iter()
            .any(|hit| hit.item.id.0 == "doc_probe::Demo::new"),
        "{:?}",
        hits.iter().map(|h| &h.item.id.0).collect::<Vec<_>>()
    );
}

#[test]
fn search_finds_item_by_description() {
    let index = build();
    let hits = search(&index, &SearchQuery::new("demo struct"));
    assert_eq!(
        hits.first().map(|hit| hit.item.id.0.as_str()),
        Some("doc_probe::Demo")
    );
}

#[test]
fn prefix_mode_only_matches_prefix() {
    let index = build();
    let mut query = SearchQuery::new("de");
    query.mode = MatchMode::Prefix;
    let hits = search(&index, &query);
    // 名字前缀命中得 80 分；id / 摘要兜底命中得分更低。
    let demo = hits
        .iter()
        .find(|hit| hit.item.name == "Demo")
        .expect("应命中 Demo");
    assert_eq!(demo.score, 80);
}

#[test]
fn crate_and_kind_filters_apply() {
    let index = build();

    let mut query = SearchQuery::new("n");
    query.crate_name = Some("doc_probe".to_string());
    query.kinds = vec![mcp_docs_core::ItemKind::Method];
    let hits = search(&index, &query);
    assert!(hits
        .iter()
        .all(|hit| hit.item.kind == mcp_docs_core::ItemKind::Method));

    // 过滤到不存在的 crate 时无结果。
    let mut none = SearchQuery::new("n");
    none.crate_name = Some("不存在的crate".to_string());
    assert!(search(&index, &none).is_empty());
}

#[test]
fn index_json_roundtrips() {
    let index = build();
    let json = serde_json::to_string(&index).unwrap();
    let restored: Index = serde_json::from_str(&json).unwrap();
    assert_eq!(index, restored);
}
