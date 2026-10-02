//! `search` 的基准：在 fixture 索引上检索。

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{criterion_group, criterion_main, Criterion};
use mcp_docs_core::{build_index, search, SearchQuery};

/// fixture 根目录（等价于一个 `target/doc`）。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 基准：在已建好的索引上做一次单关键词检索。
fn bench_search(c: &mut Criterion) {
    let index = build_index(&fixture_root(), Path::new("out")).unwrap();
    let mut query = SearchQuery::new("e");
    query.limit = 20;

    c.bench_function("search/e", |b| {
        b.iter(|| search(black_box(&index), black_box(&query)));
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
