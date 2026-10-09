//! 缓存、指纹与增量构建测试。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mcp_docs_core::{
    BuildOptions, DocCache, Granularity, ParseOptions, build, fingerprint_doc_root, is_stale,
};

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

    // 改动一个页面（内容与 mtime 都变化）。选 `enum.Kind.html` 而非
    // `struct.Demo.html`：后者被一个重导出别名引用，改它会连带重解析别名条目，
    // 无法用 `parsed == 1` 隔离验证增量逻辑。
    let target = doc.join("doc_probe").join("enum.Kind.html");
    std::thread::sleep(std::time::Duration::from_millis(20));
    let html = fs::read_to_string(&target).unwrap();
    fs::write(
        &target,
        html.replace("A demo enum.", "A changed demo enum."),
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

/// 重导出别名与目标共享同一 HTML：改动该页会连带重解析别名条目。
#[test]
fn reexport_alias_reparsed_with_its_target() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("doc");
    let out = tmp.path().join("out");
    copy_dir(&fixture_root(), &doc);

    build(&doc, &out, &full(true)).unwrap();

    // `struct.Demo.html` 被 `pub use …::{Demo} as DemoAlias;` 引用。
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
    // 目标条目与引用它的别名条目都会重解析。
    assert_eq!(report.parsed, 2, "目标页及其别名应一并重解析");
}

#[test]
fn granularity_item_skips_member_files() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let opts = BuildOptions {
        write_markdown: true,
        persist: true,
        granularity: Granularity::Item,
        ..Default::default()
    };
    let report = build(&fixture_root(), &out, &opts).unwrap();

    // 只写顶层条目文件，成员不落盘。别名条目与目标共享同一文件、内容相同，
    // 不重复落盘，故 `written` 等于非成员条目涉及的不同文件数。
    let tops: std::collections::HashSet<&str> = report
        .index
        .items
        .iter()
        .filter(|item| !item.kind.is_member())
        .map(|item| item.file.as_str())
        .collect();
    assert_eq!(report.written, tops.len());
    assert!(!out.join("doc_probe/struct.Demo.method.new.md").exists());

    // 成员摘要的 `file` 指向父条目文件。
    let member = report
        .index
        .items
        .iter()
        .find(|item| item.id.0 == "doc_probe::struct.Demo::method.new")
        .unwrap();
    assert_eq!(member.file, "doc_probe/struct.Demo.md");
}

#[test]
fn granularity_change_forces_full_rebuild() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("doc");
    let out = tmp.path().join("out");
    copy_dir(&fixture_root(), &doc);

    // 先按默认（member）粒度建索引并落盘。
    build(&doc, &out, &full(true)).unwrap();

    // 切换到 item 粒度 + 增量：粒度不一致，应全量重建、零复用。
    let opts = BuildOptions {
        persist: true,
        incremental: true,
        granularity: Granularity::Item,
        ..Default::default()
    };
    let report = build(&doc, &out, &opts).unwrap();
    assert_eq!(report.reused, 0);
    assert_eq!(report.index.granularity, Granularity::Item);
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

#[test]
fn doc_cache_evicts_beyond_capacity() {
    let cache = DocCache::with_capacity(1);
    let opts = ParseOptions::default();
    let demo = PathBuf::from("doc_probe/struct.Demo.html");
    let kind = PathBuf::from("doc_probe/enum.Kind.html");

    let first = cache.get_or_parse(&fixture_root(), &demo, &opts).unwrap();
    cache.get_or_parse(&fixture_root(), &kind, &opts).unwrap();
    assert_eq!(cache.len(), 1, "超出容量应淘汰最久未访问的条目");

    // 被淘汰后重新取用应得到新的解析结果。
    let again = cache.get_or_parse(&fixture_root(), &demo, &opts).unwrap();
    assert!(!Arc::ptr_eq(&first, &again), "淘汰后应重新解析");
    assert_eq!(cache.len(), 1);
}
