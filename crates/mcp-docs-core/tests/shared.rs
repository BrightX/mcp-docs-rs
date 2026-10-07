//! 共享库（跨项目复用）测试。

use std::fs;
use std::path::{Path, PathBuf};

use mcp_docs_core::{
    BuildOptions, CrateStat, Granularity, ItemId, ItemKind, ItemSummary, build, crate_scan,
    key_dir_name, materialize_file, parse_crate_version, plan, write_entry,
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

/// 构造一个最小条目摘要。
fn summary(id: &str, file: &str, html: &str) -> ItemSummary {
    ItemSummary {
        id: ItemId(id.to_string()),
        kind: ItemKind::Struct,
        name: "x".to_string(),
        path: vec!["doc_probe".to_string()],
        one_line: String::new(),
        signature: None,
        has_docs: false,
        has_members: false,
        file: file.to_string(),
        html_path: html.to_string(),
        parent_id: None,
        src_mtime: None,
    }
}

#[test]
fn key_reflects_identity_fields() {
    let stat = CrateStat {
        file_count: 3,
        size_sum: 100,
    };
    let base = key_dir_name("tokio", "1.51.0", "1.95.0", Granularity::Member, &stat);

    // 相同身份 → 相同键。
    assert_eq!(
        base,
        key_dir_name("tokio", "1.51.0", "1.95.0", Granularity::Member, &stat)
    );
    // 版本 / rustdoc 版本 / 粒度 / stat 指纹 任一变化 → 不同键。
    assert_ne!(
        base,
        key_dir_name("tokio", "1.51.1", "1.95.0", Granularity::Member, &stat)
    );
    assert_ne!(
        base,
        key_dir_name("tokio", "1.51.0", "1.96.0", Granularity::Member, &stat)
    );
    assert_ne!(
        base,
        key_dir_name("tokio", "1.51.0", "1.95.0", Granularity::Item, &stat)
    );
    assert_ne!(
        base,
        key_dir_name(
            "tokio",
            "1.51.0",
            "1.95.0",
            Granularity::Member,
            &CrateStat {
                file_count: 4,
                size_sum: 100
            }
        )
    );
}

#[test]
fn crate_scan_collects_stat_and_mtimes() {
    let (stat, mtimes) = crate_scan(&fixture_root(), "doc_probe");
    assert!(stat.file_count > 0, "应统计到 html/js 文件");
    assert!(stat.size_sum > 0);
    assert!(mtimes.contains_key("doc_probe/struct.Demo.html"));
}

#[test]
fn crate_version_is_parsed() {
    let html = fs::read_to_string(fixture_root().join("doc_probe/index.html")).unwrap();
    assert_eq!(parse_crate_version(&html).as_deref(), Some("0.1.0"));
}

#[test]
fn plan_write_reuse_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("doc");
    copy_dir(&fixture_root(), &doc);
    let store = tmp.path().join("store");

    // 首次：共享库为空，未命中。
    let planned = plan(&store, &doc, "doc_probe", Granularity::Member).unwrap();
    assert!(!planned.is_hit());

    let items = vec![summary(
        "doc_probe::struct.Demo",
        "doc_probe/struct.Demo.md",
        "doc_probe/struct.Demo.html",
    )];
    write_entry(&store, &planned, &items).unwrap();

    // 再次：命中，条目复用，且 src_mtime 按本项目 html 回填。
    let reused = plan(&store, &doc, "doc_probe", Granularity::Member).unwrap();
    assert!(reused.is_hit());
    let items = reused.reused.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].file, "doc_probe/struct.Demo.md");
    assert!(items[0].src_mtime.is_some(), "应回填本项目 mtime");
}

#[test]
fn changed_size_invalidates_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = tmp.path().join("doc");
    copy_dir(&fixture_root(), &doc);
    let store = tmp.path().join("store");

    let planned = plan(&store, &doc, "doc_probe", Granularity::Member).unwrap();
    write_entry(&store, &planned, &[]).unwrap();
    assert!(
        plan(&store, &doc, "doc_probe", Granularity::Member)
            .unwrap()
            .is_hit()
    );

    // 改动某个 html 的字节数 → stat 指纹变化 → 不再命中。
    let target = doc.join("doc_probe").join("struct.Demo.html");
    let html = fs::read_to_string(&target).unwrap();
    fs::write(&target, format!("{html}<!--pad-->")).unwrap();

    assert!(
        !plan(&store, &doc, "doc_probe", Granularity::Member)
            .unwrap()
            .is_hit()
    );
}

#[test]
fn materialize_copies_contents_and_overwrites() {
    let tmp = tempfile::tempdir().unwrap();
    let from = tmp.path().join("a/src.md");
    let to = tmp.path().join("b/dst.md");
    fs::create_dir_all(from.parent().unwrap()).unwrap();
    fs::write(&from, b"hello").unwrap();

    materialize_file(&from, &to).unwrap();
    assert_eq!(fs::read_to_string(&to).unwrap(), "hello");

    // 再次物化应覆盖旧内容。
    fs::write(&from, b"world").unwrap();
    materialize_file(&from, &to).unwrap();
    assert_eq!(fs::read_to_string(&to).unwrap(), "world");
}

/// 带共享库、渲染 md 的构建选项。
fn store_opts(store: &Path) -> BuildOptions {
    BuildOptions {
        write_markdown: true,
        persist: true,
        store: Some(store.to_path_buf()),
        ..Default::default()
    }
}

/// 第二个项目应命中第一个项目写入共享库的条目，实现跨项目复用。
#[test]
fn cross_project_reuse_hits_store() {
    let tmp = tempfile::tempdir().unwrap();
    let doc_a = tmp.path().join("a/doc");
    let doc_b = tmp.path().join("b/doc");
    copy_dir(&fixture_root(), &doc_a);
    copy_dir(&fixture_root(), &doc_b);
    let out_a = tmp.path().join("a/out");
    let out_b = tmp.path().join("b/out");
    let store = tmp.path().join("store");

    let report_a = build(&doc_a, &out_a, &store_opts(&store)).unwrap();
    assert_eq!(report_a.shared_hits, 0, "首次应无命中");
    assert!(report_a.shared_written >= 1, "首次应写入共享库");
    assert!(report_a.parsed > 0);

    let report_b = build(&doc_b, &out_b, &store_opts(&store)).unwrap();
    assert_eq!(report_b.parsed, 0, "第二次应全部复用共享库");
    assert!(report_b.shared_hits >= 1);
    assert!(report_b.reused > 0);
    assert_eq!(report_b.index.items.len(), report_a.index.items.len());

    // md 物化到 B 项目目录，且条目路径相对 B 项目。
    assert!(out_b.join("doc_probe/struct.Demo.md").is_file());
    let demo = report_b
        .index
        .items
        .iter()
        .find(|item| item.id.0 == "doc_probe::struct.Demo")
        .unwrap();
    assert_eq!(demo.file, "doc_probe/struct.Demo.md");
    assert_eq!(demo.html_path, "doc_probe/struct.Demo.html");
}

/// 粒度不同不应命中（成员 `file` 语义不同）。
#[test]
fn granularity_change_misses_store() {
    let tmp = tempfile::tempdir().unwrap();
    let doc_a = tmp.path().join("a/doc");
    let doc_b = tmp.path().join("b/doc");
    copy_dir(&fixture_root(), &doc_a);
    copy_dir(&fixture_root(), &doc_b);
    let store = tmp.path().join("store");

    let a = BuildOptions {
        granularity: Granularity::Member,
        ..store_opts(&store)
    };
    build(&doc_a, &tmp.path().join("a/out"), &a).unwrap();

    let b = BuildOptions {
        granularity: Granularity::Item,
        ..store_opts(&store)
    };
    let report = build(&doc_b, &tmp.path().join("b/out"), &b).unwrap();
    assert_eq!(report.shared_hits, 0, "粒度不同不应命中");
    assert!(report.parsed > 0);
}

/// 复用条目里的 `src_mtime` 应按本项目回填，否则本项目增量构建会全部重解析。
#[test]
fn reused_entry_keeps_incremental_working() {
    let tmp = tempfile::tempdir().unwrap();
    let doc_a = tmp.path().join("a/doc");
    let doc_b = tmp.path().join("b/doc");
    copy_dir(&fixture_root(), &doc_a);
    copy_dir(&fixture_root(), &doc_b);
    let store = tmp.path().join("store");
    let out_b = tmp.path().join("b/out");

    build(&doc_a, &tmp.path().join("a/out"), &store_opts(&store)).unwrap();
    let first_b = build(&doc_b, &out_b, &store_opts(&store)).unwrap();
    assert!(first_b.shared_hits >= 1);

    // 关闭共享库、开增量：若 src_mtime 未正确回填，会全部重新解析。
    let incremental = BuildOptions {
        incremental: true,
        persist: true,
        ..Default::default()
    };
    let second_b = build(&doc_b, &out_b, &incremental).unwrap();
    assert_eq!(second_b.parsed, 0, "src_mtime 未正确回填会重新解析");
    assert!(second_b.reused > 0);
}
