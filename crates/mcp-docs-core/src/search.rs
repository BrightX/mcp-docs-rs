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
    /// 命中摘要片段（仅当结果页条目的 `one_line` 命中时给出）。
    pub snippet: Option<String>,
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
///
/// 排序后仅克隆当前页条目，避免把全部命中都克隆一遍（大索引下差异显著）。
pub fn search_page(index: &Index, query: &SearchQuery) -> SearchOutcome {
    let all = ranked_hits(index, query);
    let total = all.len();
    let needle = query.query.trim().to_lowercase();
    let hits = all
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .map(|(index_of, score)| {
            let item = index.items[index_of].clone();
            // 摘要命中时附带上下文片段，便于调用方判断相关性。
            let snippet =
                contains_ci(&item.one_line, &needle).then(|| make_snippet(&item.one_line, &needle));
            SearchHit {
                item,
                score,
                snippet,
            }
        })
        .collect();
    SearchOutcome { total, hits }
}

/// 计算全部命中并排序（未分页），返回 `(条目下标, 得分)`。
fn ranked_hits(index: &Index, query: &SearchQuery) -> Vec<(usize, i32)> {
    let needle = query.query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let terms: Vec<&str> = needle.split_whitespace().collect();

    let mut hits: Vec<(usize, i32)> = index
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| passes_filter(item, query))
        .filter_map(|(index_of, item)| {
            score_item(item, &terms, query.mode).map(|score| (index_of, score))
        })
        .collect();

    // 排序三级键：得分降序 → 名字长度升序 → id 升序（与旧实现逐字节一致）。
    hits.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| {
                index.items[a.0]
                    .name
                    .len()
                    .cmp(&index.items[b.0].name.len())
            })
            .then_with(|| index.items[a.0].id.0.cmp(&index.items[b.0].id.0))
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

/// 计算一个条目对整条查询的得分。
///
/// - 单词查询：沿用原有打分（名字 100/80/60、路径 40、摘要 20）。
/// - 多词查询：要求每个词都命中，得分是各词得分之和（AND 语义）。
fn score_item(item: &ItemSummary, terms: &[&str], mode: MatchMode) -> Option<i32> {
    match terms {
        [] => None,
        [term] => rank(item, term, mode),
        _ => {
            let mut total = 0;
            for term in terms {
                total += rank(item, term, mode)?;
            }
            Some(total)
        }
    }
}

/// 计算单词语的相关度得分；不匹配返回 `None`。
///
/// 排序优先级：名字精确命中 > 名字前缀 > 名字子串 > 路径命中 > 摘要命中。
fn rank(item: &ItemSummary, query: &str, mode: MatchMode) -> Option<i32> {
    let name = item.name.as_str();
    let name_hit = match mode {
        MatchMode::Substring => contains_ci(name, query),
        MatchMode::Prefix => starts_with_ci(name, query),
        MatchMode::Fuzzy => is_subsequence(&item.name.to_lowercase(), query),
    };
    if name_hit {
        let score = if name.eq_ignore_ascii_case(query) {
            100
        } else if starts_with_ci(name, query) {
            80
        } else {
            60
        };
        return Some(score);
    }

    if contains_ci(&item.id.0, query) {
        return Some(40);
    }
    if contains_ci(&item.one_line, query) {
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

/// ASCII 大小写不敏感的子串匹配（不分配）。
fn contains_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hay = haystack.as_bytes();
    let target = needle.as_bytes();
    if target.len() > hay.len() {
        return false;
    }
    hay.windows(target.len())
        .any(|window| window.eq_ignore_ascii_case(target))
}

/// ASCII 大小写不敏感的前缀匹配。
fn starts_with_ci(haystack: &str, prefix: &str) -> bool {
    let hay = haystack.as_bytes();
    let target = prefix.as_bytes();
    hay.len() >= target.len() && hay[..target.len()].eq_ignore_ascii_case(target)
}

/// 取命中位置前后各 40 个字符的上下文片段（按字符边界）。
fn make_snippet(text: &str, needle: &str) -> String {
    const WINDOW: usize = 40;
    let chars: Vec<char> = text.chars().collect();
    let center = find_ci(text, needle).unwrap_or(0);
    let start = center.saturating_sub(WINDOW);
    let end = (center + WINDOW).min(chars.len());

    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(chars[start..end].iter().copied());
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// ASCII 大小写不敏感地查找 `needle`，返回匹配起点的字符下标。
fn find_ci(text: &str, needle: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let target: Vec<char> = needle.chars().collect();
    if target.is_empty() || target.len() > chars.len() {
        return None;
    }
    (0..=chars.len() - target.len()).find(|&start| {
        chars[start..start + target.len()]
            .iter()
            .zip(&target)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}
