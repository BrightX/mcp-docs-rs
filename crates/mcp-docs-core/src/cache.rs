//! 缓存与指纹：判断产物是否变化，并缓存解析结果。

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::error::Result;
use crate::model::{DocItem, ItemId, INDEX_SCHEMA_VERSION};
use crate::parse::{self, ParseOptions};
use crate::store::atomic_write;

/// 产物指纹，用于判断索引是否需要重建。
///
/// 统计方式是「遍历 + stat」，只关注文件数量、最新修改时间与总大小，
/// 不读取内容，因此对大产物目录也是 O(文件数)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// 参与统计的 html / js 文件数。
    pub file_count: u64,
    /// 最新修改时间（Unix 毫秒）。
    pub max_mtime: u64,
    /// 文件大小总和（字节）。
    pub size_sum: u64,
    /// `crates.js` 的内容哈希。
    pub crates_js_hash: u32,
}

/// 落盘的元信息（`meta.json`）。
///
/// 与 `index.json` 分开存放，使「是否需要重建」的判断不必反序列化整个索引。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// 索引结构版本。
    pub schema_version: u32,
    /// 生成索引时的 rustdoc 版本。
    pub rustdoc_version: Option<String>,
    /// 生成时间（Unix 秒）。
    pub generated_at: u64,
    /// 产物指纹。
    pub fingerprint: Fingerprint,
}

/// 计算 `doc_root` 的指纹。
pub fn fingerprint_doc_root(doc_root: &Path) -> Fingerprint {
    let mut file_count = 0u64;
    let mut max_mtime = 0u64;
    let mut size_sum = 0u64;

    for entry in WalkDir::new(doc_root).into_iter().flatten() {
        if !entry.file_type().is_file() || !is_tracked(entry.path()) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        file_count += 1;
        size_sum += metadata.len();
        max_mtime = max_mtime.max(metadata_mtime(&metadata));
    }

    Fingerprint {
        file_count,
        max_mtime,
        size_sum,
        crates_js_hash: file_hash(&doc_root.join("crates.js")),
    }
}

/// 判断索引是否已过期（需要重建）。
///
/// 缺少 `meta.json`、schema 版本不符、或指纹变化都视为过期。
pub fn is_stale(doc_root: &Path, meta_path: &Path) -> Result<bool> {
    let Ok(meta) = read_meta(meta_path) else {
        return Ok(true);
    };
    if meta.schema_version != INDEX_SCHEMA_VERSION {
        return Ok(true);
    }
    Ok(fingerprint_doc_root(doc_root) != meta.fingerprint)
}

/// 写入元信息（原子写）。
pub fn write_meta(meta: &Meta, path: &Path) -> Result<()> {
    // 紧凑 JSON，避免缩进空白放大文件与解析成本。
    let json = serde_json::to_vec(meta)?;
    atomic_write(path, &json)
}

/// 读取元信息。
pub fn read_meta(path: &Path) -> Result<Meta> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// 取文件修改时间（Unix 毫秒），失败返回 0。
pub fn path_mtime(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()
        .map(|metadata| metadata_mtime(&metadata))
}

/// 解析缓存默认容量：足以覆盖一次典型的多工具调用，又不至于常驻过多正文。
const DEFAULT_CACHE_CAPACITY: usize = 512;

/// 条目解析结果的内存缓存（`design.md` §7 的二级缓存）。
///
/// 供 MCP server 按需解析并复用；CLI 一次性运行用不到。
/// 容量有上限，超出后按「最久未访问」近似 LRU 淘汰，避免大 crate 下无界增长。
pub struct DocCache {
    entries: Mutex<HashMap<ItemId, Entry>>,
    capacity: usize,
    /// 单调递增的访问计数，用作近似的 LRU 时间戳。
    tick: AtomicU64,
}

/// 缓存条目：解析结果 + 最近访问时间戳。
struct Entry {
    item: Arc<DocItem>,
    stamp: u64,
}

impl Default for DocCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CACHE_CAPACITY)
    }
}

impl DocCache {
    /// 新建空缓存（默认容量）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 用指定容量新建空缓存（测试小容量淘汰行为用）。
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            capacity,
            tick: AtomicU64::new(0),
        }
    }

    /// 取缓存中的条目；缺失时解析并写入缓存。
    pub fn get_or_parse(
        &self,
        doc_root: &Path,
        rel_path: &Path,
        opts: &ParseOptions,
    ) -> Result<Arc<DocItem>> {
        let (_, path, name) = parse::path_to_identity(rel_path)?;
        let mut id_parts = path;
        id_parts.push(name);
        let id = ItemId(id_parts.join("::"));

        // 命中则刷新时间戳并直接返回。
        if let Some(cached) = self.touch(&id) {
            return Ok(cached);
        }

        // 未命中：在锁外解析，避免长时间持锁阻塞其他访问。
        let html = fs::read_to_string(doc_root.join(rel_path))?;
        let item = Arc::new(parse::parse_item_html(&html, rel_path, opts)?);

        let mut entries = self.lock();
        let stamp = self.next_tick();
        entries.insert(
            id,
            Entry {
                item: Arc::clone(&item),
                stamp,
            },
        );
        self.evict(&mut entries);
        Ok(item)
    }

    /// 清空缓存。
    pub fn invalidate(&self) {
        self.lock().clear();
    }

    /// 当前缓存的条目数。
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// 缓存是否为空。
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// 命中时刷新访问时间戳并返回条目。
    fn touch(&self, id: &ItemId) -> Option<Arc<DocItem>> {
        let mut entries = self.lock();
        let stamp = self.next_tick();
        let entry = entries.get_mut(id)?;
        entry.stamp = stamp;
        Some(Arc::clone(&entry.item))
    }

    /// 取下一个时间戳。
    fn next_tick(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::Relaxed)
    }

    /// 超出容量时淘汰最久未访问的条目。
    fn evict(&self, entries: &mut HashMap<ItemId, Entry>) {
        while entries.len() > self.capacity {
            let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.stamp)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            entries.remove(&oldest);
        }
    }

    /// 加锁；锁中毒时直接取出内部数据，避免因他人 panic 而连锁失败。
    fn lock(&self) -> MutexGuard<'_, HashMap<ItemId, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl std::fmt::Debug for DocCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocCache")
            .field("len", &self.len())
            .finish()
    }
}

/// 是否为需要纳入指纹的产物文件。
fn is_tracked(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("html" | "js")
    )
}

/// 从元数据取修改时间（Unix 毫秒）。
fn metadata_mtime(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 文件内容的 FNV-1a 哈希；读取失败返回 0。
fn file_hash(path: &Path) -> u32 {
    fs::read(path)
        .map(|bytes| crate::fnv1a(&bytes))
        .unwrap_or(0)
}
