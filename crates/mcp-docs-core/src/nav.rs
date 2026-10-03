//! 导航与关系：模块树、相关条目、trait 实现者解析。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::model::{ItemKind, ItemSummary};

/// 模块树节点（`module_tree` 的返回结构）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleNode {
    /// 模块名（根节点为 crate 名）。
    pub name: String,
    /// 模块完整路径（含 crate，`::` 分隔）。
    pub path: String,
    /// 该模块的直属条目数（不含子模块内部条目）。
    pub item_count: usize,
    /// 子模块。
    pub children: Vec<ModuleNode>,
}

/// 由扁平条目列表构建某个 crate 的模块树；crate 不存在时返回 `None`。
pub fn module_tree(items: &[ItemSummary], crate_name: &str) -> Option<ModuleNode> {
    let mut root = ModuleNode {
        name: crate_name.to_string(),
        path: crate_name.to_string(),
        item_count: 0,
        children: Vec::new(),
    };
    let mut found = false;
    for item in items
        .iter()
        .filter(|item| item.id.0.split("::").next() == Some(crate_name))
    {
        found = true;
        insert_into_tree(&mut root, item);
    }
    found.then_some(root)
}

/// 把一个条目插入模块树。
fn insert_into_tree(root: &mut ModuleNode, item: &ItemSummary) {
    // 定位到包含该条目的模块节点（路径去掉 crate 段）。
    let mut node = root;
    for segment in &item.path[1..] {
        node = child_of(node, segment);
    }
    node.item_count += 1;
    if item.kind == ItemKind::Module {
        // 即使子模块没有直属条目，也在树里占位。
        let _ = child_of(node, &item.name);
    }
}

/// 取（必要时创建）指定名字的子节点。
fn child_of<'a>(node: &'a mut ModuleNode, name: &str) -> &'a mut ModuleNode {
    if let Some(position) = node.children.iter().position(|child| child.name == name) {
        return &mut node.children[position];
    }
    let path = format!("{}::{name}", node.path);
    node.children.push(ModuleNode {
        name: name.to_string(),
        path,
        item_count: 0,
        children: Vec::new(),
    });
    node.children.last_mut().expect("刚插入的子节点必然存在")
}

/// 条目的相关条目（`get_related_items` 的返回结构）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatedItems {
    /// 父条目（成员条目才有，指向所属条目）。
    pub parent: Option<ItemSummary>,
    /// 同模块的兄弟条目（顶层条目）。
    pub siblings: Vec<ItemSummary>,
    /// 子成员条目。
    pub children: Vec<ItemSummary>,
}

/// 由扁平条目列表推导某条目的相关条目；条目不存在时返回 `None`。
pub fn related_items(items: &[ItemSummary], id: &str) -> Option<RelatedItems> {
    let target = items.iter().find(|item| item.id.0 == id)?;

    let parent = target
        .parent_id
        .as_deref()
        .and_then(|parent_id| items.iter().find(|item| item.id.0 == parent_id))
        .cloned();

    let children: Vec<ItemSummary> = items
        .iter()
        .filter(|item| item.parent_id.as_deref() == Some(id))
        .cloned()
        .collect();

    let siblings: Vec<ItemSummary> = items
        .iter()
        .filter(|item| {
            item.id.0 != id
                && item.parent_id.is_none()
                && !item.kind.is_member()
                && item.path == target.path
        })
        .cloned()
        .collect();

    Some(RelatedItems {
        parent,
        siblings,
        children,
    })
}

/// 一条 trait 实现记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraitImpl {
    /// 实现该 trait 的 crate。
    pub crate_name: String,
    /// 实现签名（已去除 HTML 标签的纯文本）。
    pub text: String,
}

/// 由 rustdoc 的 `trait.impl/.../trait.<Name>.js` 内容解析实现者清单。
///
/// 文件形如 `Object.fromEntries([["crate",[["impl <a ...>..</a> for ...",0], ...]]])`，
/// 参数是合法 JSON 数组；这里先按字符串/括号配对抽取数组字面量，再反序列化。
pub fn parse_trait_impls(js: &str) -> Vec<TraitImpl> {
    let Some(array) = extract_array_literal(js) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(array) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let Some(groups) = value.as_array() else {
        return out;
    };
    for group in groups {
        let Some(pair) = group.as_array() else {
            continue;
        };
        let crate_name = pair
            .first()
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let Some(impls) = pair.get(1).and_then(|value| value.as_array()) else {
            continue;
        };
        for entry in impls {
            if let Some(text) = entry
                .as_array()
                .and_then(|array| array.first())
                .and_then(|value| value.as_str())
            {
                out.push(TraitImpl {
                    crate_name: crate_name.clone(),
                    text: strip_tags(text),
                });
            }
        }
    }
    out
}

/// 由 trait 页面的相对 HTML 路径推导其实现者清单的 js 路径。
///
/// `bitflags/traits/trait.Flags.html` → `trait.impl/bitflags/traits/trait.Flags.js`。
pub fn trait_impl_rel_path(html_path: &Path) -> PathBuf {
    Path::new("trait.impl").join(html_path).with_extension("js")
}

/// 查找某个 trait 的实现清单文件。
///
/// rustdoc 把实现清单放在 trait 的**定义模块**下，而条目 `html_path` 是文档页路径；
/// 对 re-export 的 trait（如 `serde::Serialize` 的实现实际在 `serde_core`）两者不一致，
/// 故精确路径未命中时按文件名回退搜索（见 `docs/issues.md` E-2.1）。
pub fn find_trait_impl_paths(doc_dir: &Path, html_path: &Path) -> Vec<PathBuf> {
    let exact = doc_dir.join(trait_impl_rel_path(html_path));
    if exact.is_file() {
        return vec![exact];
    }

    let Some(file_name) = html_path
        .file_name()
        .map(|name| Path::new(name).with_extension("js"))
    else {
        return Vec::new();
    };
    let root = doc_dir.join("trait.impl");

    // 优先在同一 crate 目录下搜索，避免命中其它 crate 的同名 trait。
    if let Some(crate_name) = html_path.components().next() {
        let hits = collect_impl_files(&root.join(crate_name.as_os_str()), file_name.as_os_str());
        if !hits.is_empty() {
            return hits;
        }
    }
    collect_impl_files(&root, file_name.as_os_str())
}

/// 递归收集 `dir` 下文件名为 `file_name` 的文件（目录不存在时返回空）。
fn collect_impl_files(dir: &Path, file_name: &std::ffi::OsStr) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == file_name)
        .map(|entry| entry.into_path())
        .collect()
}

/// 抽取 `Object.fromEntries(` 后的数组字面量，按字符串与括号配对定位结尾。
fn extract_array_literal(js: &str) -> Option<&str> {
    const PREFIX: &str = "Object.fromEntries(";
    let start = js.find(PREFIX)? + PREFIX.len();
    let rest = &js[start..];
    let open = rest.find('[')?;

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in rest.as_bytes().iter().enumerate().skip(open) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[open..=offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// 去掉 HTML 标签并规整空白。
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
