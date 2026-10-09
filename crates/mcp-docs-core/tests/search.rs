//! 索引构建与检索测试。

use std::path::{Path, PathBuf};

use mcp_docs_core::{
    BuildOptions, Granularity, Index, MatchMode, SearchQuery, build_index, load_index, search,
    search_page, write_index,
};

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
    assert!(ids.contains(&"doc_probe::struct.Demo"));
    assert!(ids.contains(&"doc_probe::struct.Demo::method.new"));
    assert!(ids.contains(&"doc_probe::struct.Demo::field.field"));
    assert!(ids.contains(&"doc_probe::enum.Kind::variant.A"));
    assert!(ids.contains(&"doc_probe::inner::struct.Nested"));
}

#[test]
fn summary_files_point_to_markdown() {
    let index = build();
    let find = |id: &str| index.items.iter().find(|item| item.id.0 == id).unwrap();

    // `file` 是相对输出根目录的路径。
    assert_eq!(
        find("doc_probe::struct.Demo").file,
        "doc_probe/struct.Demo.md"
    );
    assert_eq!(
        find("doc_probe::struct.Demo::method.new").file,
        "doc_probe/struct.Demo.method.new.md"
    );
    assert_eq!(
        find("doc_probe::inner::struct.Nested").file,
        "doc_probe/inner/struct.Nested.md"
    );
}

#[test]
fn search_finds_method_by_name() {
    let index = build();
    let hits = search(&index, &SearchQuery::new("new"));
    assert!(
        hits.iter()
            .any(|hit| hit.item.id.0 == "doc_probe::struct.Demo::method.new"),
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
        Some("doc_probe::struct.Demo")
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
    assert!(
        hits.iter()
            .all(|hit| hit.item.kind == mcp_docs_core::ItemKind::Method)
    );

    // 过滤到不存在的 crate 时无结果。
    let mut none = SearchQuery::new("n");
    none.crate_name = Some("不存在的crate".to_string());
    assert!(search(&index, &none).is_empty());
}

#[test]
fn index_json_roundtrips() {
    let index = build();
    let restored = write_then_load(&index);
    assert_eq!(index, restored);
}

/// 写盘再读回：落盘为 gzip，`path` / `name` / `parent_id` / `file` 加载后重建。
fn write_then_load(index: &Index) -> Index {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.json.gz");
    write_index(index, &path).unwrap();
    load_index(&path).unwrap()
}

/// 索引文件应为 gzip 压缩（魔数 `1f 8b`），而非明文 JSON。
#[test]
fn index_file_is_gzip_compressed() {
    let index = build();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.json.gz");
    write_index(&index, &path).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..2], &[0x1f, 0x8b], "索引应以 gzip 魔数开头");
    assert_eq!(load_index(&path).unwrap(), index);
}

/// 成员派生字段在加载后重建正确（`Item` 粒度下成员 `file` 指向父文件）。
#[test]
fn load_rebuilds_member_derived_fields() {
    let opts = BuildOptions {
        granularity: Granularity::Item,
        ..Default::default()
    };
    let index = mcp_docs_core::build(&fixture_root(), Path::new("out"), &opts)
        .unwrap()
        .index;
    let restored = write_then_load(&index);
    assert_eq!(index, restored);

    let member = restored
        .items
        .iter()
        .find(|item| item.id.0 == "doc_probe::struct.Demo::method.new")
        .expect("应有成员 method.new");
    assert!(member.kind.is_member());
    assert_eq!(member.name, "new");
    assert_eq!(
        member.path,
        vec!["doc_probe".to_string(), "Demo".to_string()]
    );
    assert_eq!(member.parent_id.as_deref(), Some("doc_probe::struct.Demo"));
    // Item 粒度：成员不单独落盘，`file` 指向父条目文件。
    assert_eq!(member.file, "doc_probe/struct.Demo.md");

    // Member 粒度：成员单独落盘。
    let member_index =
        mcp_docs_core::build(&fixture_root(), Path::new("out"), &BuildOptions::default())
            .unwrap()
            .index;
    let member_member = write_then_load(&member_index)
        .items
        .into_iter()
        .find(|item| item.id.0 == "doc_probe::struct.Demo::method.new")
        .unwrap();
    assert_eq!(member_member.file, "doc_probe/struct.Demo.method.new.md");
}

/// 查询词 `e` 的命中 id 列表（不截断）。
fn all_hits() -> Vec<String> {
    let index = build();
    let mut query = SearchQuery::new("e");
    query.limit = 1000;
    search(&index, &query)
        .iter()
        .map(|hit| hit.item.id.0.clone())
        .collect()
}

#[test]
fn search_page_total_is_pre_pagination() {
    let index = build();
    let total = all_hits().len();
    assert!(total >= 3, "需要足够多的命中来测试分页");

    let mut query = SearchQuery::new("e");
    query.limit = 2;
    query.offset = 1;
    let outcome = search_page(&index, &query);
    assert_eq!(outcome.total, total, "total 应为分页前的命中总数");
    assert_eq!(outcome.hits.len(), 2);
}

#[test]
fn offset_paginates_without_overlap() {
    let index = build();
    let full = all_hits();

    let page = |offset: usize, limit: usize| {
        let mut query = SearchQuery::new("e");
        query.offset = offset;
        query.limit = limit;
        search(&index, &query)
            .iter()
            .map(|hit| hit.item.id.0.clone())
            .collect::<Vec<_>>()
    };

    let mut paged = page(0, 2);
    paged.extend(page(2, 2));
    assert_eq!(paged, full[..4].to_vec(), "分页应与整体结果一致且无重叠");
}

#[test]
fn offset_past_end_and_zero_limit_are_empty() {
    let index = build();

    let mut past = SearchQuery::new("e");
    past.offset = 9999;
    assert!(search(&index, &past).is_empty());

    let mut zero = SearchQuery::new("e");
    zero.limit = 0;
    assert!(search(&index, &zero).is_empty());
}

#[test]
fn multi_word_requires_all_terms() {
    let index = build();

    // 两个词都命中的条目才会返回。
    let hits = search(&index, &SearchQuery::new("demo struct"));
    assert!(!hits.is_empty());
    for hit in &hits {
        let hay =
            format!("{} {} {}", hit.item.name, hit.item.id.0, hit.item.one_line).to_lowercase();
        assert!(
            hay.contains("demo") && hay.contains("struct"),
            "多词 AND 不成立：{}",
            hit.item.id.0
        );
    }

    // 含一个不存在的词时无结果。
    assert!(search(&index, &SearchQuery::new("demo zzzz")).is_empty());
}

#[test]
fn snippet_present_on_one_line_hit() {
    let index = build();
    let hits = search(&index, &SearchQuery::new("demo struct"));
    let demo = hits
        .iter()
        .find(|hit| hit.item.id.0 == "doc_probe::struct.Demo")
        .expect("应命中 Demo");
    let snippet = demo.snippet.as_ref().expect("摘要命中应附带 snippet");
    assert!(
        snippet.to_lowercase().contains("demo"),
        "snippet: {snippet}"
    );
}
