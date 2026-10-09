//! 索引落盘的压缩编解码：JSON + gzip。
//!
//! 索引动辄数万条目，紧凑 JSON 仍可达数 MB；gzip 后通常降到约 1/10。
//! 文件名保持 `*.json`（内容为 gzip），调用方无需改动路径。

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Result;
use crate::store::atomic_write;

/// 序列化为 JSON、gzip 压缩后原子写入。
pub(crate) fn write_json_gz<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_vec(value)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json)?;
    let compressed = encoder.finish()?;
    atomic_write(path, &compressed)
}

/// 读取 gzip 压缩的 JSON 文件并反序列化。
pub(crate) fn read_json_gz<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)?;
    let mut decoder = GzDecoder::new(&bytes[..]);
    let mut json = Vec::new();
    decoder.read_to_end(&mut json)?;
    Ok(serde_json::from_slice(&json)?)
}
