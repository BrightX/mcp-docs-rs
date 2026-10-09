//! 构建索引，并可选地渲染 markdown、落盘 index.json.gz 与 meta.json。

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rayon::prelude::*;

use crate::cache::{Meta, fingerprint_doc_root, path_mtime, write_meta};
use crate::codec;
use crate::discover;
use crate::error::Result;
use crate::markdown::{LinkStyle, RenderOptions, render_item, render_member_item, rewrite_links};
use crate::model::{
    CrateSummary, DiscoveredItem, DocItem, Granularity, INDEX_SCHEMA_VERSION, Index, ItemId,
    ItemKind, ItemSummary,
};
use crate::parse::{self, ParseOptions};
use crate::shared;
use crate::store::{atomic_write_fast, item_output_path, member_output_path};

/// 索引落盘文件名（JSON + gzip）。
pub const INDEX_FILE_NAME: &str = "index.json.gz";

/// 构建选项。
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// 渲染并写入 markdown 文件。
    pub write_markdown: bool,
    /// 复用未变化条目（增量）。需要 `out_root/index.json` 存在。
    pub incremental: bool,
    /// 把 `index.json.gz` 与 `meta.json` 落盘。
    pub persist: bool,
    /// 只处理指定 crate。
    pub crate_filter: Option<String>,
    /// 导出粒度：控制成员是否单独落盘。
    pub granularity: Granularity,
    /// 共享库根目录。`Some` 时启用跨项目复用（解析渲染结果按 crate 身份键入库）；
    /// `None` 时行为与不使用共享库完全一致。
    pub store: Option<PathBuf>,
}

/// 构建结果。
#[derive(Debug, Clone)]
pub struct BuildReport {
    /// 构建出的索引。
    pub index: Index,
    /// 重新解析的条目数。
    pub parsed: usize,
    /// 复用旧索引或共享库的条目数。
    pub reused: usize,
    /// 写入的 markdown 文件数。
    pub written: usize,
    /// 命中共享库而整体复用的 crate 数。
    pub shared_hits: usize,
    /// 本次写入共享库的 crate 数。
    pub shared_written: usize,
    /// 解析失败而跳过的条目（`id` 与原因）。
    ///
    /// rustdoc 会为宏重导出生成 `macro.foo!.html` 之类的**重定向页**，
    /// 这类页面没有正文；单个条目失败不应中断整体构建。
    pub skipped: Vec<String>,
}

/// 构建索引。
///
/// `out_root` 用于推导条目的 markdown 路径；`opts.persist` 为真时还会写出
/// `index.json.gz` 与 `meta.json`。
pub fn build(doc_root: &Path, out_root: &Path, opts: &BuildOptions) -> Result<BuildReport> {
    let parse_opts = ParseOptions::default();
    let render_opts = RenderOptions::default();

    // 增量：读取上次索引，用于复用未变化条目。
    // 仅在 schema 与导出粒度都一致时才复用，否则按当前选项全量重建
    // （切换粒度会改变成员的 `file`，且不会补写/删除成员文件）。
    let previous = if opts.incremental {
        load_index(&out_root.join(INDEX_FILE_NAME))
            .ok()
            .filter(|index| {
                index.schema_version == INDEX_SCHEMA_VERSION
                    && index.granularity == opts.granularity
            })
    } else {
        None
    };
    // rustdoc 版本来自首个解析到的页面；并行构建下用 `OnceLock` 采集。
    let rustdoc_version_lock = OnceLock::new();
    if let Some(version) = previous
        .as_ref()
        .and_then(|index| index.rustdoc_version.clone())
    {
        let _ = rustdoc_version_lock.set(version);
    }
    let previous_items: Vec<ItemSummary> = previous.map(|index| index.items).unwrap_or_default();
    // 预建索引，避免增量时对每个条目线性扫描整份旧索引（O(n×m)）。
    let previous_by_id: HashMap<&str, &ItemSummary> = previous_items
        .iter()
        .filter(|summary| !summary.kind.is_member())
        .map(|summary| (summary.id.0.as_str(), summary))
        .collect();
    let mut previous_members: HashMap<&str, Vec<&ItemSummary>> = HashMap::new();
    for summary in previous_items.iter().filter(|s| s.kind.is_member()) {
        if let Some(parent_id) = summary.parent_id.as_deref() {
            previous_members.entry(parent_id).or_default().push(summary);
        }
    }

    // 1) 串行发现：确定待处理的 crate 与条目，保持稳定顺序（并行后需按序汇总）。
    //    同时逐 crate 探测共享库：命中则整体复用，不再发现条目。
    let store_root = opts.store.as_deref();
    let mut crate_names: Vec<String> = Vec::new();
    let mut plans: Vec<Option<shared::CratePlan>> = Vec::new();
    let mut reused_crates: Vec<Option<Vec<ItemSummary>>> = Vec::new();
    let mut jobs: Vec<(usize, DiscoveredItem)> = Vec::new();
    for crate_name in discover::list_crates(doc_root)? {
        if let Some(filter) = &opts.crate_filter
            && &crate_name != filter
        {
            continue;
        }
        let crate_index = crate_names.len();

        let planned = match store_root {
            Some(store) => Some(shared::plan(
                store,
                doc_root,
                &crate_name,
                opts.granularity,
            )?),
            None => None,
        };
        if let Some(plan) = &planned
            && !plan.meta.rustdoc_version.is_empty()
            && rustdoc_version_lock.get().is_none()
        {
            let _ = rustdoc_version_lock.set(plan.meta.rustdoc_version.clone());
        }

        let reused = planned.as_ref().and_then(|plan| plan.reused.clone());
        if reused.is_none() {
            for entry in discover::discover_crate(doc_root, &crate_name)? {
                jobs.push((crate_index, entry));
            }
        }
        crate_names.push(crate_name);
        reused_crates.push(reused);
        plans.push(planned);
    }

    // store 启用时，未命中 crate 的 md 直接写入共享库对应目录。
    let store_md_dirs: Vec<Option<PathBuf>> = plans
        .iter()
        .map(|plan| plan.as_ref().map(|plan| plan.md_dir.clone()))
        .collect();

    // 2) 并行解析 / 渲染 / 落盘：条目之间彼此独立，是构建的主要瓶颈。
    let results: Vec<ProcessResult> = jobs
        .par_iter()
        .map(|(crate_index, entry)| {
            process_entry(
                doc_root,
                out_root,
                entry,
                *crate_index,
                &parse_opts,
                &render_opts,
                opts,
                &previous_by_id,
                &previous_members,
                &rustdoc_version_lock,
                store_md_dirs[*crate_index].as_deref(),
            )
        })
        .collect::<Result<Vec<_>>>()?;

    // 3) 串行汇总：按 crate 顺序拼接「复用条目」与「新建条目」。
    let mut per_crate: Vec<Vec<ItemSummary>> = vec![Vec::new(); crate_names.len()];
    let (mut parsed, mut reused, mut written) = (0usize, 0usize, 0usize);
    let mut skipped = Vec::new();
    for result in results {
        per_crate[result.crate_index].extend(result.summaries);
        parsed += result.parsed;
        reused += result.reused;
        written += result.written;
        if let Some(entry) = result.skipped {
            skipped.push(entry);
        }
    }

    let mut shared_hits = 0usize;
    let mut shared_written = 0usize;
    let mut crates: Vec<CrateSummary> = Vec::with_capacity(crate_names.len());
    let mut items: Vec<ItemSummary> = Vec::new();
    for (index, name) in crate_names.into_iter().enumerate() {
        let crate_items = match reused_crates[index].take() {
            // 命中共享库：直接复用条目，并按需把 md 物化到项目目录。
            Some(reuse_items) => {
                if opts.write_markdown
                    && let Some(plan) = &plans[index]
                {
                    let files: BTreeSet<String> = reuse_items
                        .iter()
                        .map(|item| item.file.clone())
                        .filter(|file| !file.is_empty())
                        .collect();
                    shared::materialize_crate(&plan.md_dir, out_root, files)?;
                }
                reused += reuse_items.len();
                shared_hits += 1;
                reuse_items
            }
            // 未命中：用构建结果，并写入共享库。
            None => {
                let built = std::mem::take(&mut per_crate[index]);
                if let (Some(store), Some(plan)) = (store_root, plans[index].as_ref()) {
                    shared::write_entry(store, plan, &built)?;
                    shared_written += 1;
                }
                built
            }
        };
        let item_count = crate_items.len();
        items.extend(crate_items);
        crates.push(CrateSummary { name, item_count });
    }

    let rustdoc_version = rustdoc_version_lock.into_inner();

    let index = Index {
        schema_version: INDEX_SCHEMA_VERSION,
        rustdoc_version,
        generated_at: unix_now(),
        target_doc: doc_root.to_string_lossy().into_owned(),
        granularity: opts.granularity,
        crates,
        items,
    };

    if opts.persist {
        write_index(&index, &out_root.join(INDEX_FILE_NAME))?;
        let meta = Meta {
            schema_version: INDEX_SCHEMA_VERSION,
            rustdoc_version: index.rustdoc_version.clone(),
            generated_at: index.generated_at,
            fingerprint: fingerprint_doc_root(doc_root),
        };
        write_meta(&meta, &out_root.join("meta.json"))?;
    }

    Ok(BuildReport {
        index,
        parsed,
        reused,
        written,
        shared_hits,
        shared_written,
        skipped,
    })
}

/// 单个条目的处理结果（用于并行构建后串行汇总）。
struct ProcessResult {
    /// 父条目摘要 + 成员摘要（父在前）。
    summaries: Vec<ItemSummary>,
    parsed: usize,
    reused: usize,
    written: usize,
    /// 解析失败而跳过的条目（`id：原因`）。
    skipped: Option<String>,
    /// 所属 crate 在 crate 列表里的下标。
    crate_index: usize,
}

impl ProcessResult {
    /// 空结果（产物缺失等静默跳过场景）。
    fn empty(crate_index: usize) -> Self {
        Self {
            summaries: Vec::new(),
            parsed: 0,
            reused: 0,
            written: 0,
            skipped: None,
            crate_index,
        }
    }
}

/// 处理单个条目：增量复用或解析、渲染、落盘，并产出摘要。
///
/// 该函数在并行迭代器内执行，因此只读共享外部状态、不做跨条目协调。
///
/// `store_md_dir` 为共享库内本 crate 的 md 目录：`Some` 时渲染结果写入共享库
/// （且恒渲染，保证库内 md 完整可复用），并按 `write_markdown` 决定是否再物化
/// 到项目输出目录。
#[allow(clippy::too_many_arguments)]
fn process_entry(
    doc_root: &Path,
    out_root: &Path,
    entry: &DiscoveredItem,
    crate_index: usize,
    parse_opts: &ParseOptions,
    render_opts: &RenderOptions,
    opts: &BuildOptions,
    previous_by_id: &HashMap<&str, &ItemSummary>,
    previous_members: &HashMap<&str, Vec<&ItemSummary>>,
    rustdoc_version: &OnceLock<String>,
    store_md_dir: Option<&Path>,
) -> Result<ProcessResult> {
    let html_path = doc_root.join(&entry.html_path);
    let mtime = path_mtime(&html_path);

    // 增量：父条目未变则整体复用（含其成员）。
    let unchanged = previous_by_id
        .get(entry.id.0.as_str())
        .filter(|summary| summary.src_mtime.is_some() && summary.src_mtime == mtime);
    if let Some(previous) = unchanged {
        let mut summaries = vec![(*previous).clone()];
        // 成员的 `path`（含 crate）正好等于父条目 id。
        if let Some(members) = previous_members.get(entry.id.0.as_str()) {
            summaries.extend(members.iter().map(|member| (*member).clone()));
        }
        let reused = summaries.len();
        return Ok(ProcessResult {
            summaries,
            parsed: 0,
            reused,
            written: 0,
            skipped: None,
            crate_index,
        });
    }

    // 产物缺失时跳过，不影响整体索引。
    let Ok(html) = fs::read_to_string(&html_path) else {
        return Ok(ProcessResult::empty(crate_index));
    };
    if rustdoc_version.get().is_none()
        && let Some((version, _)) = parse::parse_rustdoc_meta(&html)
    {
        let _ = rustdoc_version.set(version);
    }

    let item = match parse::parse_item_html(&html, &entry.html_path, parse_opts) {
        Ok(item) => item,
        Err(err) => {
            // 重定向页等异常产物：跳过该条目，不影响整体构建。
            return Ok(ProcessResult {
                skipped: Some(format!("{}：{err}", entry.id)),
                ..ProcessResult::empty(crate_index)
            });
        }
    };
    let parsed = 1;

    // 别名条目（crate 内部重导出）与目标共享同一 HTML，但身份取自发现阶段：
    // 用别名 id / 名字 / 路径覆盖解析结果，否则会与目标条目撞 id（重复）。
    // 别名只是指向目标的指针，不继承目标的成员（成员 id 带目标前缀，会重复）。
    let is_alias = item.id != entry.id;
    let (summary_id, summary_name, summary_path, summary_kind, members): (
        ItemId,
        String,
        Vec<String>,
        ItemKind,
        &[DocItem],
    ) = if is_alias {
        (
            entry.id.clone(),
            entry.name.clone(),
            entry.path.clone(),
            entry.kind,
            &[],
        )
    } else {
        (
            item.id.clone(),
            item.name.clone(),
            item.path.clone(),
            item.kind,
            &item.members,
        )
    };

    // 相对输出根的路径（共享库与项目目录逐字符同构，故与具体根无关）。
    let item_rel = item_output_path(Path::new(""), &entry.html_path);
    let item_file = rel_to_slash(&item_rel);

    // 渲染目标：store 启用时优先写共享库，且恒渲染以保证库内 md 完整可复用。
    // 别名条目与目标共享同一 `html_path`、渲染内容也相同，跳过写盘：
    // 目标条目必然被常规发现产出并写入该文件；避免两个条目并发写同一文件，
    // 在 Windows 上触发「文件被占用」（os error 32）。
    let should_render = (opts.write_markdown || store_md_dir.is_some()) && !is_alias;
    let primary = store_md_dir.unwrap_or(out_root);
    let mut written = 0;
    if should_render {
        let target = primary.join(&item_rel);
        atomic_write_fast(&target, render_item(&item, render_opts).as_bytes())?;
        written += 1;
        if opts.write_markdown && store_md_dir.is_some() {
            shared::materialize_file(&target, &out_root.join(&item_rel))?;
        }
    }

    // 无真实文档时，rustdoc 生成的 `<meta name="description">` 是占位文本
    // （如 "API documentation for the Rust `X` struct ..."），不应作为摘要返回。
    let has_docs = item.docs_md.is_some();
    let one_line = if has_docs {
        // 摘要可能含 rustdoc 生成的 markdown 链接，统一重写为 `.md`。
        rewrite_links(
            &parse::parse_one_line(&html).unwrap_or_default(),
            LinkStyle::Relative,
        )
    } else {
        String::new()
    };

    let mut summaries = Vec::with_capacity(members.len() + 1);
    summaries.push(ItemSummary {
        id: summary_id,
        kind: summary_kind,
        name: summary_name,
        path: summary_path,
        one_line,
        signature: item.signature.clone(),
        has_docs,
        has_members: !members.is_empty(),
        file: item_file.clone(),
        html_path: rel_string(doc_root, &entry.html_path),
        parent_id: None,
        src_mtime: mtime,
    });

    for member in members {
        let member_rel =
            member_output_path(Path::new(""), &entry.html_path, member.kind, &member.name);
        // 成员文件仅在「成员粒度」下写出；条目粒度时只内联在父文件里。
        if should_render && opts.granularity == Granularity::Member {
            let target = primary.join(&member_rel);
            atomic_write_fast(&target, render_member_item(member, render_opts).as_bytes())?;
            written += 1;
            if opts.write_markdown && store_md_dir.is_some() {
                shared::materialize_file(&target, &out_root.join(&member_rel))?;
            }
        }
        // 条目粒度下成员没有独立文件，`file` 指向父条目文件。
        let file = match opts.granularity {
            Granularity::Member => rel_to_slash(&member_rel),
            Granularity::Item => item_file.clone(),
        };
        summaries.push(ItemSummary {
            id: member.id.clone(),
            kind: member.kind,
            name: member.name.clone(),
            path: member.path.clone(),
            one_line: rewrite_links(&first_line(member.docs_md.as_deref()), LinkStyle::Relative),
            signature: member.signature.clone(),
            has_docs: member.docs_md.is_some(),
            has_members: false,
            file,
            html_path: rel_string(doc_root, &entry.html_path),
            parent_id: Some(item.id.0.clone()),
            src_mtime: mtime,
        });
    }

    Ok(ProcessResult {
        summaries,
        parsed,
        reused: 0,
        written,
        skipped: None,
        crate_index,
    })
}

/// 把路径转成统一以 `/` 分隔的字符串。
fn rel_to_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 扫描 `doc_root` 构建索引（不写出任何文件）。
pub fn build_index(doc_root: &Path, out_root: &Path) -> Result<Index> {
    Ok(build(doc_root, out_root, &BuildOptions::default())?.index)
}

/// 把索引写入文件（JSON + gzip，原子写）。
///
/// 索引动辄数万条目，紧凑 JSON 仍可达数 MB；gzip 后通常降到约 1/10。
pub fn write_index(index: &Index, path: &Path) -> Result<()> {
    codec::write_json_gz(path, index)
}

/// 从文件读取索引，并重建不落盘的派生字段。
pub fn load_index(path: &Path) -> Result<Index> {
    let mut index: Index = codec::read_json_gz(path)?;
    let granularity = index.granularity;
    for item in &mut index.items {
        item.rebuild_derived(granularity);
    }
    Ok(index)
}

/// 取相对 `out_root` 的路径字符串，统一用 `/` 分隔。
fn rel_string(out_root: &Path, path: &Path) -> String {
    path.strip_prefix(out_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 文档的首个非空行，作为成员的一行摘要。
fn first_line(docs: Option<&str>) -> String {
    docs.and_then(|docs| docs.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or_default()
        .to_string()
}

/// 当前 Unix 时间戳（秒）。
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
