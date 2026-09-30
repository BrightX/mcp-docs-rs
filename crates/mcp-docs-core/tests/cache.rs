//! 缓存、指纹与增量构建测试。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mcp_docs_core::{build, fingerprint_doc_root, is_stale, BuildOptions, DocCache, ParseOptions};

/// fixture 根目录，等价于一个 `target/doc`。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 递归复制目录（测试用）。
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// 全量构建的默认选项。
fn full(persist: bool) -> BuildOptions {
    BuildOptions {
        persist,
        ..Default::default()
    }
}

#[test]
fn fingerprint_is_stable() {
    let root = fixture_root();
    assert_eq!(fingerprint_doc_root(&root), fingerprint_doc_root(&root));
}

#[test]
fn stale_detection_follows_meta() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path();
    let meta_path = out.join("meta.json");

    // 没有 meta.json 时视为过期。
    assert!(is_stale(&fixture_root(), &meta_path).unwrap());

    build(&fixture_root(), out, &full(true)).unwrap();
    // 构建后指纹一致，不再过期。
    assert!(!is_stale(&fixture_root(), &meta_path).unwrap());
}

#[test]
fn incremental_reuses_unchanged_items() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("doc");
    let out = tmp.path().join("out");
    copy_dir(&fixture_root(), &doc);

    let first = build(&doc, &out, &full(true)).unwrap();
    assert_eq!(first.reused, 0);
    assert!(first.parsed > 0);

    let incremental = BuildOptions {
        persist: true,
        incremental: true,
        ..Default::default()
    };
    let second = build(&doc, &out, &incremental).unwrap();
    assert_eq!(second.parsed, 0, "第二次应全部复用");
    assert_eq!(second.reused, first.index.items.len());
    assert_eq!(second.index.items.len(), first.index.items.len());
}

#[test]
fn only_changed_page_is_reparsed() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("doc");
    let out = tmp.path().join("out");
    copy_dir(&fixture_root(), &doc);

    build(&doc, &out, &full(true)).unwrap();

    // 改动一个页面（内容与 mtime 都变化）。
    let target = doc.join("doc_probe").join("struct.Demo.html");
    std::thread::sleep(std::time::Duration::from_millis(20));
    let html = fs::read_to_string(&target).unwrap();
    fs::write(
        &target,
        html.replace("A demo struct.", "A changed demo struct."),
    )
    .unwrap();

    let incremental = BuildOptions {
        persist: true,
        incremental: true,
        ..Default::default()
    };
    let report = build(&doc, &out, &incremental).unwrap();
    assert_eq!(report.parsed, 1, "应只重新解析被改动的页面");
    assert!(report.reused > 0);
}

#[test]
fn doc_cache_reuses_parsed_items() {
    let cache = DocCache::new();
    let rel = PathBuf::from("doc_probe/struct.Demo.html");
    let opts = ParseOptions::default();

    let first = cache.get_or_parse(&fixture_root(), &rel, &opts).unwrap();
    let second = cache.get_or_parse(&fixture_root(), &rel, &opts).unwrap();
    assert!(Arc::ptr_eq(&first, &second), "第二次应命中缓存");
    assert_eq!(cache.len(), 1);

    cache.invalidate();
    assert!(cache.is_empty());
}
