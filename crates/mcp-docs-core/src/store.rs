//! 落盘：文件名编码、输出路径推导、原子写入。

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::error::Result;
use crate::model::ItemKind;

/// 清洗文件名，保证跨平台（尤其 Windows）合法。
///
/// - 非法字符 `< > : " / \ | ? *` 与控制字符替换为 `_`；
/// - Windows 保留名（`CON` / `PRN` / `AUX` / `NUL` / `COM1..9` / `LPT1..9`）加 `_` 前缀；
/// - 去掉结尾的 `.` 与空格；
/// - 过长（超过 200 字节）时截断并追加内容哈希。
pub fn encode_fs_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();

    out = out.trim_end_matches(['.', ' ']).to_string();
    if out.is_empty() {
        return "_".to_string();
    }

    if is_reserved(&out) {
        out.insert(0, '_');
    }

    if out.len() > 200 {
        let hash = crate::fnv1a(out.as_bytes());
        let mut truncated: String = out.chars().take(180).collect();
        truncated.push_str(&format!("-{hash:08x}"));
        out = truncated;
    }

    out
}

/// 条目 markdown 的输出路径：镜像 HTML 目录结构，扩展名换成 `.md`。
pub fn item_output_path(out_root: &Path, html_rel: &Path) -> PathBuf {
    out_root.join(sanitize_rel(&html_rel.with_extension("md")))
}

/// 成员 markdown 的输出路径：父文件名 + `.` + 成员类型 + `.` + 成员名 + `.md`。
///
/// 带成员类型是为了避免同名冲突 —— 例如 `syn::LitBool` 同时有字段 `value`
/// 与方法 `value()`，只用名字会互相覆盖。
///
/// 例如父文件 `syn/struct.LitBool.html` 的字段 `value`
/// → `<out_root>/syn/struct.LitBool.field.value.md`。
pub fn member_output_path(
    out_root: &Path,
    parent_html_rel: &Path,
    member_kind: ItemKind,
    member_name: &str,
) -> PathBuf {
    let dir = parent_html_rel.parent().unwrap_or_else(|| Path::new(""));
    let stem = parent_html_rel
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = format!(
        "{stem}.{}.{}.md",
        member_kind.kind_tag(),
        encode_fs_name(member_name)
    );
    out_root.join(sanitize_rel(dir)).join(file)
}

/// 原子写入：先写临时文件再 rename，避免读到写了一半的文件。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 对相对路径的每一段做文件名清洗。
fn sanitize_rel(rel: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in rel.components() {
        if let Component::Normal(name) = component {
            out.push(encode_fs_name(&name.to_string_lossy()));
        } else {
            out.push(component.as_os_str());
        }
    }
    out
}

/// 是否为 Windows 保留名。
fn is_reserved(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
        || numbered_reserved(stem, "COM")
        || numbered_reserved(stem, "LPT")
}

/// `COM1..COM9` / `LPT1..LPT9` 形式的保留名。
fn numbered_reserved(stem: &str, prefix: &str) -> bool {
    stem.strip_prefix(prefix)
        .is_some_and(|rest| matches!(rest, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
}
