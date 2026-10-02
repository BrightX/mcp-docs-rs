//! 条目 id 的查找表，避免每次按 id 查找都线性扫描整份索引。

use std::collections::HashMap;

use crate::model::ItemSummary;

/// 预建的 id → 条目下标查找表。
///
/// - `exact`：完整的、带类型标记的 id（如 `tokio::task::fn.spawn`）。
/// - `stripped`：去掉类型标记的 id（如 `tokio::task::spawn`），同一键只保留
///   首次出现的条目（`or_insert` 首条胜出），与旧线性查找语义一致。
#[derive(Debug, Default, Clone)]
pub struct IdIndex {
    exact: HashMap<String, usize>,
    stripped: HashMap<String, usize>,
}

impl IdIndex {
    /// 依据条目列表构建查找表。
    pub fn build(items: &[ItemSummary]) -> Self {
        let mut exact = HashMap::with_capacity(items.len());
        let mut stripped = HashMap::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            exact.insert(item.id.0.clone(), index);
            // 首次出现的条目优先，避免被后出现的同义 id 覆盖。
            stripped.entry(item.id.without_kind_tags()).or_insert(index);
        }
        Self { exact, stripped }
    }

    /// 按 id 查找条目下标：先精确匹配，未命中时退化到省略类型标记的写法。
    pub fn find(&self, id: &str) -> Option<usize> {
        self.exact
            .get(id)
            .or_else(|| {
                let stripped = id
                    .split("::")
                    .map(|segment| {
                        segment
                            .split_once('.')
                            .map(|(_, name)| name)
                            .unwrap_or(segment)
                    })
                    .collect::<Vec<_>>()
                    .join("::");
                self.stripped.get(&stripped)
            })
            .copied()
    }
}
