//! 跨项目共享的 crate 文档索引库。
//!
//! 多个项目（多个仓库）常依赖同一批 crate（如都依赖 tokio 1.51），若各自重新
//! 解析渲染一遍就是纯浪费。这里按「crate 名 + 版本 + rustdoc 版本 + 粒度 + stat
//! 指纹」计算身份键，把每个 crate 的条目摘要与 markdown 在共享库中只存一份，
//! 各项目按需复用。
//!
//! 身份键**刻意不含 mtime**：各项目 `target/doc` 副本的 mtime 不同，含 mtime 会
//! 导致永远无法跨项目命中。相对地，stat 指纹只用文件数与总字节数，跨副本稳定。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::cache::{is_tracked, metadata_mtime};
use crate::error::Result;
use crate::model::{Granularity, INDEX_SCHEMA_VERSION, ItemSummary};
use crate::store::{atomic_write, encode_fs_name};
use crate::{fnv1a, parse};

/// 一个 crate 的 stat 级指纹：只 stat、不读内容。
///
/// 与 [`crate::Fingerprint`] 不同，这里**不含 mtime**，以便跨项目复用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateStat {
    /// 参与统计的 html / js 文件数。
    pub file_count: u64,
    /// 文件大小总和（字节）。
    pub size_sum: u64,
}

/// 共享库条目的元信息（`meta.json`）。
///
/// 写入顺序上**最后写**，作为该条目「已完整落盘」的提交标记；缺它即视为
/// 半成品条目，读取方直接忽略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreMeta {
    /// 索引结构版本。
    pub schema_version: u32,
    /// crate 名。
    pub name: String,
    /// crate 版本号（取不到时为 `unknown`）。
    pub version: String,
    /// 生成该 crate 文档的 rustdoc 版本。
    pub rustdoc_version: String,
    /// 导出粒度（成员是否单独落盘）。
    pub granularity: Granularity,
    /// html / js 文件数。
    pub file_count: u64,
    /// 文件大小总和（字节）。
    pub size_sum: u64,
    /// 写入时间（Unix 秒）。
    pub generated_at: u64,
}

/// 一个 crate 的规划结果：身份键、落盘位置，以及（命中时）可复用的条目。
#[derive(Debug, Clone)]
pub struct CratePlan {
    /// 身份键（共享库内的目录名）。
    pub key: String,
    /// 待写入/已存在的元信息（`generated_at` 由 [`write_entry`] 填写）。
    pub meta: StoreMeta,
    /// 该 crate 在共享库里的 markdown 根目录（等价于项目 `out_root`）。
    pub md_dir: PathBuf,
    /// 命中的可复用条目；`None` 表示未命中，需要正常构建。
    pub reused: Option<Vec<ItemSummary>>,
}

impl CratePlan {
    /// 是否命中共享库（命中则无需解析该 crate）。
    pub fn is_hit(&self) -> bool {
        self.reused.is_some()
    }
}

/// 推导共享库根目录的跨平台默认位置（不引入额外依赖）。
///
/// 优先取环境变量 `MCP_DOCS_STORE`；否则按平台约定：Windows 用
/// `%LOCALAPPDATA%\mcp-docs\store`，macOS 用 `~/Library/Caches/mcp-docs/store`，
/// 其余 Unix 用 `$XDG_CACHE_HOME`（缺省 `~/.cache`）下的 `mcp-docs/store`。
/// 无法确定时返回 `None`（调用方可据此关闭共享）。
pub fn default_store_root() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("MCP_DOCS_STORE")
        && !value.is_empty()
    {
        return Some(PathBuf::from(value));
    }

    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(|dir| PathBuf::from(dir).join("mcp-docs").join("store"));

    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library/Caches/mcp-docs")
            .join("store")
    });

    #[cfg(all(unix, not(target_os = "macos")))]
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(|xdg| PathBuf::from(xdg).join("mcp-docs").join("store"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".cache/mcp-docs").join("store"))
        });

    root
}

/// crate 身份键 → 共享库目录名（Windows 安全）。
///
/// 目录名保留 `名字-版本` 可读前缀，后缀 8 位哈希消歧并覆盖 rustdoc 版本、
/// 粒度与 stat 指纹——即真正决定「是否同一份文档」的字段全部参与哈希。
pub fn key_dir_name(
    name: &str,
    version: &str,
    rustdoc_version: &str,
    granularity: Granularity,
    stat: &CrateStat,
) -> String {
    // 用 `\0` 分隔，避免字段拼接产生歧义（如 name="a-b" 与 name="a", version="b"）。
    let identity = format!(
        "{name}\0{version}\0{rustdoc_version}\0{granularity:?}\0{}\0{}",
        stat.file_count, stat.size_sum
    );
    let hash = fnv1a(identity.as_bytes());
    format!(
        "{}-{}-{hash:08x}",
        encode_fs_name(name),
        encode_fs_name(version)
    )
}

/// 统计一个 crate 目录，并收集其 html 文件的相对路径 → mtime。
///
/// `html_path` 与 `ItemSummary.html_path` 同格式（相对 `doc_root`、`/` 分隔），
/// 便于命中共享库后按本项目 mtime 回填，保证后续 `--incremental` 生效。
pub fn crate_scan(doc_root: &Path, crate_name: &str) -> (CrateStat, HashMap<String, u64>) {
    let root = doc_root.join(crate_name);
    let mut stat = CrateStat {
        file_count: 0,
        size_sum: 0,
    };
    let mut mtimes = HashMap::new();

    for entry in WalkDir::new(&root).into_iter().flatten() {
        if !entry.file_type().is_file() || !is_tracked(entry.path()) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        stat.file_count += 1;
        stat.size_sum += metadata.len();

        if entry.path().extension().and_then(|ext| ext.to_str()) == Some("html")
            && let Ok(rel) = entry.path().strip_prefix(doc_root)
        {
            mtimes.insert(
                rel.to_string_lossy().replace('\\', "/"),
                metadata_mtime(&metadata),
            );
        }
    }

    (stat, mtimes)
}

/// 规划一个 crate：算身份键、探测共享库命中。
///
/// 命中则 `plan.reused` 为 `Some(条目)`（已按本项目 mtime 回填 `src_mtime`），
/// 调用方可直接复用，无需解析；未命中则 `reused` 为 `None`，调用方应正常构建，
/// 构建完成后用 [`write_entry`] 落库。
pub fn plan(
    store_root: &Path,
    doc_root: &Path,
    crate_name: &str,
    granularity: Granularity,
) -> Result<CratePlan> {
    let (version, rustdoc_version) = crate_identity(doc_root, crate_name);
    let (stat, mtimes) = crate_scan(doc_root, crate_name);
    let key = key_dir_name(crate_name, &version, &rustdoc_version, granularity, &stat);
    let md_dir = entry_dir(store_root, &key).join("md");
    let meta = StoreMeta {
        schema_version: INDEX_SCHEMA_VERSION,
        name: crate_name.to_string(),
        version,
        rustdoc_version,
        granularity,
        file_count: stat.file_count,
        size_sum: stat.size_sum,
        generated_at: 0,
    };
    let reused = load_reused(store_root, &key, &meta, &mtimes)?;

    Ok(CratePlan {
        key,
        meta,
        md_dir,
        reused,
    })
}

/// 把构建好的 crate 条目写入共享库（`items.json` + `meta.json`，meta 最后写）。
pub fn write_entry(store_root: &Path, plan: &CratePlan, items: &[ItemSummary]) -> Result<()> {
    let dir = entry_dir(store_root, &plan.key);
    fs::create_dir_all(&dir)?;

    let items_bytes = serde_json::to_vec(items)?;
    atomic_write(&dir.join("items.json"), &items_bytes)?;

    // meta.json 最后写：它存在即代表该条目已完整可复用。
    let mut meta = plan.meta.clone();
    meta.generated_at = unix_now();
    let meta_bytes = serde_json::to_vec(&meta)?;
    atomic_write(&dir.join("meta.json"), &meta_bytes)?;
    Ok(())
}

/// 物化单个 markdown：同卷硬链接（Windows 无需提权），跨卷回退复制。
///
/// 覆盖前先删目标（硬链接到已存在文件会失败）；`from == to` 时直接跳过。
pub fn materialize_file(from: &Path, to: &Path) -> Result<()> {
    if from == to {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = fs::remove_file(to);
    if fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    // 跨卷（共享库在系统盘、项目在其它盘）硬链接不可用，退回复制。
    fs::copy(from, to)?;
    Ok(())
}

/// 把一个 crate 的 markdown 从共享库物化到项目 `out_root`。
///
/// `files` 为该 crate 的条目 `file`（相对 `out_root`，形如 `<crate>/xxx.md`）集合，
/// 已按路径去重（`item` 粒度下多个成员共享同一父文件）。
pub fn materialize_crate(
    md_dir: &Path,
    out_root: &Path,
    files: impl IntoIterator<Item = String>,
) -> Result<()> {
    for file in files {
        if file.is_empty() {
            continue;
        }
        materialize_file(&md_dir.join(&file), &out_root.join(&file))?;
    }
    Ok(())
}

/// 共享库内某条目的目录。
fn entry_dir(store_root: &Path, key: &str) -> PathBuf {
    store_root.join("shared").join(key)
}

/// 读取 crate 首页，得到 (crate 版本, rustdoc 版本)；取不到时给安全缺省。
fn crate_identity(doc_root: &Path, crate_name: &str) -> (String, String) {
    let index_html = doc_root.join(crate_name).join("index.html");
    let Ok(html) = fs::read_to_string(&index_html) else {
        return ("unknown".to_string(), String::new());
    };
    let rustdoc_version = parse::parse_rustdoc_meta(&html)
        .map(|(version, _)| version)
        .unwrap_or_default();
    let version = parse::parse_crate_version(&html).unwrap_or_else(|| "unknown".to_string());
    (version, rustdoc_version)
}

/// 命中判定：读取共享库条目并校验元信息一致。
///
/// 未命中（`meta.json`/`items.json` 缺失、schema 或任一身份字段不符）返回 `None`。
fn load_reused(
    store_root: &Path,
    key: &str,
    expected: &StoreMeta,
    mtimes: &HashMap<String, u64>,
) -> Result<Option<Vec<ItemSummary>>> {
    let dir = entry_dir(store_root, key);

    let Ok(bytes) = fs::read(dir.join("meta.json")) else {
        return Ok(None);
    };
    let Ok(actual) = serde_json::from_slice::<StoreMeta>(&bytes) else {
        return Ok(None);
    };
    let same = actual.schema_version == expected.schema_version
        && actual.name == expected.name
        && actual.version == expected.version
        && actual.rustdoc_version == expected.rustdoc_version
        && actual.granularity == expected.granularity
        && actual.file_count == expected.file_count
        && actual.size_sum == expected.size_sum;
    if !same {
        return Ok(None);
    }

    let Ok(bytes) = fs::read(dir.join("items.json")) else {
        return Ok(None);
    };
    // 条目损坏（半成品/格式不符）时降级为未命中，交由调用方重建。
    let Ok(mut items) = serde_json::from_slice::<Vec<ItemSummary>>(&bytes) else {
        return Ok(None);
    };
    // 用本项目 mtime 回填，否则本项目后续 `--incremental` 永远判定为变化。
    for item in &mut items {
        if let Some(mtime) = mtimes.get(&item.html_path) {
            item.src_mtime = Some(*mtime);
        }
    }
    Ok(Some(items))
}

/// 当前 Unix 时间戳（秒）。
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
