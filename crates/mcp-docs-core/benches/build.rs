//! `build_index` 的基准：默认在 fixture 上跑；真实规模用环境变量门控。
//!
//! 设置 `MCP_DOCS_BENCH_DOC=/path/to/target/doc` 可测真实产物目录。

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{criterion_group, criterion_main, Criterion};
use mcp_docs_core::build_index;

/// fixture 根目录（等价于一个 `target/doc`）。
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_probe")
}

/// 基准：扫描并构建索引（不落盘）。
fn bench_build(c: &mut Criterion) {
    let doc_root = std::env::var_os("MCP_DOCS_BENCH_DOC")
        .map(PathBuf::from)
        .unwrap_or_else(fixture_root);

    c.bench_function("build_index", |b| {
        b.iter(|| build_index(black_box(&doc_root), Path::new("out")).unwrap());
    });
}

criterion_group!(benches, bench_build);
criterion_main!(benches);
