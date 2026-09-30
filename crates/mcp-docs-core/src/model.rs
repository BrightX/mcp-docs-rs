//! 领域模型：条目类型、条目 id，以及发现阶段的条目。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 条目类型。
///
/// 与 rustdoc 的条目文件名前缀（如 `struct.Demo.html` 中的 `struct`）
/// 以及 `sidebar-items.js` 的 key 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    // —— 顶层条目 ——
    Module,
    Struct,
    Enum,
    Union,
    Trait,
    TraitAlias,
    Function,
    TypeAlias,
    Constant,
    Static,
    Macro,
    Primitive,
    Keyword,
    Derive,
    ProcMacro,
    // —— 成员子条目 ——
    Method,
    TyMethod,
    AssocConst,
    AssocType,
    Variant,
    Field,
    // —— 特殊 ——
    Impl,
    Unknown,
}

impl ItemKind {
    /// 解析 rustdoc 文件名前缀（单数形式），如 `"struct"` → [`ItemKind::Struct`]。
    pub fn from_file_prefix(prefix: &str) -> Option<Self> {
        let kind = match prefix {
            "mod" => Self::Module,
            "struct" => Self::Struct,
            "enum" => Self::Enum,
            "union" => Self::Union,
            "trait" => Self::Trait,
            "trait_alias" => Self::TraitAlias,
            "fn" => Self::Function,
            "type" => Self::TypeAlias,
            "constant" => Self::Constant,
            "static" => Self::Static,
            "macro" => Self::Macro,
            "primitive" => Self::Primitive,
            "keyword" => Self::Keyword,
            "derive" => Self::Derive,
            "proc_macro" => Self::ProcMacro,
            _ => return None,
        };
        Some(kind)
    }

    /// 解析 `sidebar-items.js` 的 key。
    ///
    /// 实测 rustdoc 1.95 的 key 为单数（与文件名前缀一致），
    /// 这里同时兼容旧版的复数形式（如 `"structs"`）。
    pub fn from_sidebar_key(key: &str) -> Option<Self> {
        if let Some(kind) = Self::from_file_prefix(key) {
            return Some(kind);
        }
        let kind = match key {
            "modules" => Self::Module,
            "structs" => Self::Struct,
            "enums" => Self::Enum,
            "unions" => Self::Union,
            "traits" => Self::Trait,
            "functions" => Self::Function,
            "types" => Self::TypeAlias,
            "constants" => Self::Constant,
            "statics" => Self::Static,
            "macros" => Self::Macro,
            "primitives" => Self::Primitive,
            "keywords" => Self::Keyword,
            "derives" => Self::Derive,
            "proc_macros" => Self::ProcMacro,
            _ => return None,
        };
        Some(kind)
    }

    /// 条目对应的 rustdoc 文件名前缀。
    pub fn file_prefix(&self) -> &'static str {
        match self {
            Self::Module => "mod",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Union => "union",
            Self::Trait => "trait",
            Self::TraitAlias => "trait",
            Self::Function => "fn",
            Self::TypeAlias => "type",
            Self::Constant => "constant",
            Self::Static => "static",
            Self::Macro => "macro",
            Self::Primitive => "primitive",
            Self::Keyword => "keyword",
            Self::Derive => "derive",
            Self::ProcMacro => "proc_macro",
            Self::Method | Self::TyMethod => "method",
            Self::AssocConst => "associatedconstant",
            Self::AssocType => "associatedtype",
            Self::Variant => "variant",
            Self::Field => "structfield",
            Self::Impl => "impl",
            Self::Unknown => "unknown",
        }
    }

    /// 是否为成员子条目（挂在父条目下，而非独立成页）。
    pub fn is_member(&self) -> bool {
        matches!(
            self,
            Self::Method
                | Self::TyMethod
                | Self::AssocConst
                | Self::AssocType
                | Self::Variant
                | Self::Field
        )
    }
}

/// 条目的稳定唯一标识，形如 `doc_probe::Demo::new`。
///
/// 注意：`::` 只存在于 id 中，**绝不用于文件名**（见 `docs/lessons.md` #2.1）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(pub String);

impl std::fmt::Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 发现阶段产出的条目（尚未解析正文）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredItem {
    /// 条目唯一标识。
    pub id: ItemId,
    /// 条目类型。
    pub kind: ItemKind,
    /// 条目名（不含路径）。
    pub name: String,
    /// 完整路径（含 crate，不含自身），如 `["doc_probe", "inner"]`。
    pub path: Vec<String>,
    /// 相对 `doc_root` 的 HTML 路径，如 `doc_probe/inner/struct.Nested.html`。
    pub html_path: PathBuf,
}
