//! 链接解析：把 rustdoc HTML 里的 `href` 归类并归一化。

use std::path::{Component, Path, PathBuf};

/// 一个 `href` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// 页面内锚点，如 `#method.new`。
    InPage(String),
    /// 指向另一个 rustdoc 条目，`rel` 为相对 `doc_root` 的 HTML 路径。
    Item {
        /// 相对 `doc_root` 的 HTML 路径。
        rel: PathBuf,
        /// 目标锚点。
        anchor: Option<String>,
    },
    /// 指向源码页面。
    Source {
        /// 相对 `doc_root` 的源码 HTML 路径。
        rel: PathBuf,
        /// 目标锚点（如 `L10-L13`）。
        anchor: Option<String>,
    },
    /// 外部链接，原样保留。
    External(String),
}

/// 解析 `href`。
///
/// `current_rel` 为当前页面相对 `doc_root` 的路径，用于解析相对链接。
pub fn resolve_href(href: &str, current_rel: &Path) -> Resolved {
    let href = href.trim();

    if let Some(fragment) = href.strip_prefix('#') {
        return Resolved::InPage(fragment.to_string());
    }
    if is_external(href) {
        return Resolved::External(href.to_string());
    }

    let (path_part, anchor) = match href.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment.to_string())),
        None => (href, None),
    };

    // 相对当前页面的目录归一化，得到相对 `doc_root` 的路径。
    let base = current_rel.parent().unwrap_or_else(|| Path::new(""));
    let rel = normalize(&base.join(path_part));

    if rel.starts_with("src") {
        Resolved::Source { rel, anchor }
    } else {
        Resolved::Item { rel, anchor }
    }
}

/// 由 HTML 路径推导对应的 markdown 路径。
///
/// markdown 文件树镜像 HTML 目录结构，只把扩展名换成 `.md`，
/// 因此两者之间的相对链接关系保持一致。
pub fn html_to_md_relpath(html_rel: &Path) -> PathBuf {
    html_rel.with_extension("md")
}

/// 是否为外部链接。
fn is_external(href: &str) -> bool {
    href.starts_with("http://")
        || href.starts_with("https://")
        || href.starts_with("mailto:")
        || href.starts_with("//")
}

/// 归一化路径中的 `.` 与 `..`（纯字符串运算，不访问文件系统）。
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
