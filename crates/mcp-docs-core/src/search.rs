//! 索引内的检索与排序。

use crate::model::{Index, ItemKind, ItemSummary};

/// 匹配模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MatchMode {
    /// 子串匹配（默认）。
    #[default]
    Substring,
    /// 前缀匹配。
    Prefix,
    /// 模糊子序列匹配。
    Fuzzy,
}

/// 检索条件。
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// 查询词。
    pub query: String,
    /// 匹配模式。
    pub mode: MatchMode,
    /// 限定条目类型（空表示不限）。
    pub kinds: Vec<ItemKind>,
    /// 限定 crate。
    pub crate_name: Option<String>,
    /// 返回上限。
    pub limit: usize,
    /// 跳过的命中数（分页）。
    pub offset: usize,
}

impl SearchQuery {
    /// 用查询词构造带默认值的条件。
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            mode: MatchMode::default(),
            kinds: Vec::new(),
            crate_name: None,
            limit: 20,
            offset: 0,
        }
    }
}

/// 一条检索结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// 命中的条目。
    pub item: ItemSummary,
    /// 相关度得分，越大越相关。
    pub score: i32,
}

/// 分页后的检索结果。
#[derive(Debug, Clone)]
pub struct SearchOutcome {
    /// 分页前的命中总数。
    pub total: usize,
    /// 当前页的命中。
    pub hits: Vec<SearchHit>,
}

/// 在索引中检索，返回当前页（考虑 `limit` / `offset`）。
pub fn search(index: &Index, query: &SearchQuery) -> Vec<SearchHit> {
    search_page(index, query).hits
}

/// 在索引中检索，返回命中总数与当前页。
pub fn search_page(index: &Index, query: &SearchQuery) -> SearchOutcome {
    let all = ranked_hits(index, query);
    let total = all.len();
    let hits = all
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect();
    SearchOutcome { total, hits }
}

/// 计算全部命中并排序（未分页）。
fn ranked_hits(index: &Index, query: &SearchQuery) -> Vec<SearchHit> {
    let needle = query.query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<SearchHit> = index
        .items
        .iter()
        .filter(|item| passes_filter(item, query))
        .filter_map(|item| {
            rank(item, &needle, query.mode).map(|score| SearchHit {
                item: item.clone(),
                score,
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.item.name.len().cmp(&b.item.name.len()))
            .then_with(|| a.item.id.0.cmp(&b.item.id.0))
    });
    hits
}

/// 按条目类型与 crate 过滤。
fn passes_filter(item: &ItemSummary, query: &SearchQuery) -> bool {
    if !query.kinds.is_empty() && !query.kinds.contains(&item.kind) {
        return false;
    }
    if let Some(crate_name) = &query.crate_name {
        if item.id.0.split("::").next() != Some(crate_name.as_str()) {
            return false;
        }
    }
    true
}

/// 计算相关度得分；不匹配返回 `None`。
///
/// 排序优先级：名字精确命中 > 名字前缀 > 名字子串 > 路径命中 > 摘要命中。
fn rank(item: &ItemSummary, query: &str, mode: MatchMode) -> Option<i32> {
    let name = item.name.to_lowercase();
    let name_hit = match mode {
        MatchMode::Substring => name.contains(query),
        MatchMode::Prefix => name.starts_with(query),
        MatchMode::Fuzzy => is_subsequence(&name, query),
    };
    if name_hit {
        let score = if name == query {
            100
        } else if name.starts_with(query) {
            80
        } else {
            60
        };
        return Some(score);
    }

    if item.id.0.to_lowercase().contains(query) {
        return Some(40);
    }
    if item.one_line.to_lowercase().contains(query) {
        return Some(20);
    }
    None
}

/// `needle` 是否为 `haystack` 的子序列（用于模糊匹配）。
fn is_subsequence(haystack: &str, needle: &str) -> bool {
    let mut candidates = haystack.chars();
    needle
        .chars()
        .all(|wanted| candidates.any(|candidate| candidate == wanted))
}
