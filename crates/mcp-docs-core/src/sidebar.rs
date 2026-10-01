//! 解析 rustdoc 的 `sidebar-items.js`。
//!
//! 该文件每个模块目录各一份，是构建条目索引的主锚点。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::error::{Error, Result};
use crate::model::ItemKind;

/// 解析 `sidebar-items.js` 的文本内容，返回「条目类型 → 名字列表」。
///
/// 文件形如 `window.SIDEBAR_ITEMS = {"struct":["Demo"]};`，
/// 去掉 JS 赋值外壳后即为合法 JSON。
pub fn parse_sidebar_str(js: &str) -> Result<BTreeMap<ItemKind, Vec<String>>> {
    let json =
        crate::strip_js_assignment(js, "window.SIDEBAR_ITEMS").ok_or_else(|| Error::Parse {
            path: Path::new("sidebar-items.js").to_path_buf(),
            reason: "未找到 window.SIDEBAR_ITEMS 赋值".to_string(),
        })?;
    // 条目既可能是纯字符串，也可能是 `[名字, 标志]` 二元组，统一按 JSON 值解析。
    let raw: BTreeMap<String, Vec<serde_json::Value>> = serde_json::from_str(json)?;

    let mut groups: BTreeMap<ItemKind, Vec<String>> = BTreeMap::new();
    for (key, entries) in raw {
        // 未知 key 直接忽略，保证向前兼容。
        if let Some(kind) = ItemKind::from_sidebar_key(&key) {
            groups
                .entry(kind)
                .or_default()
                .extend(entries.into_iter().filter_map(sidebar_entry_name));
        }
    }
    Ok(groups)
}

/// 取出 sidebar 单个条目的名字。
///
/// rustdoc 1.98 起，宏条目变成 `[名字, 标志]` 二元组；其余类型仍是纯字符串。
/// 只取名字、丢弃标志位，两种形态都兼容（见 `docs/lessons.md` #1.24）。
fn sidebar_entry_name(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(name) => Some(name),
        serde_json::Value::Array(items) => items
            .into_iter()
            .next()
            .and_then(|first| first.as_str().map(str::to_string)),
        _ => None,
    }
}

/// 解析某个目录下的 `sidebar-items.js`。
///
/// 返回 `Ok(None)` 表示文件不存在（叶模块没有该文件，属正常情况）。
pub fn parse_sidebar_file(path: &Path) -> Result<Option<BTreeMap<ItemKind, Vec<String>>>> {
    match fs::read_to_string(path) {
        Ok(js) => Ok(Some(parse_sidebar_str(&js)?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Error::Io(err)),
    }
}
