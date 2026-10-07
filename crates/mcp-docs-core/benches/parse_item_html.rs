//! `parse_item_html` 的基准：解析单个 rustdoc 页面。

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use mcp_docs_core::{ParseOptions, parse_item_html};

/// fixture 根目录（等价于一个 `target/doc`）。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 基准：解析一个带成员与文档的结构体页面。
fn bench_parse(c: &mut Criterion) {
    let rel = PathBuf::from("doc_probe/struct.Demo.html");
    let html = std::fs::read_to_string(fixture_root().join(&rel)).unwrap();
    let opts = ParseOptions::default();

    c.bench_function("parse_item_html/struct.Demo", |b| {
        b.iter(|| parse_item_html(black_box(&html), black_box(&rel), &opts).unwrap());
    });
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
