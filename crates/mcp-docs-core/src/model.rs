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
    /// 属性宏（如 `#[tool_router]`），对应 `attr.*.html`。
    Attribute,
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
            "attr" => Self::Attribute,
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
            "attr" | "attributes" => Self::Attribute,
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
            Self::Attribute => "attr",
            Self::Method | Self::TyMethod => "method",
            Self::AssocConst => "associatedconstant",
            Self::AssocType => "associatedtype",
            Self::Variant => "variant",
            Self::Field => "structfield",
            Self::Impl => "impl",
            Self::Unknown => "unknown",
        }
    }

    /// 解析用户输入的条目类型名。
    ///
    /// 同时接受文件名前缀（`fn` / `attr`）、复数形式（`functions`）
    /// 与自然名单数（`function` / `method` / `field`），
    /// 便于 MCP 工具的 `kind` 参数按直觉传值。
    ///
    /// 自然名走 serde 反序列化，与 `index.json` 里的 `kind` 字段保持同源，
    /// 避免再维护一份映射表。
    pub fn parse_input(input: &str) -> Option<Self> {
        let key = input.trim().to_ascii_lowercase();
        Self::from_file_prefix(&key)
            .or_else(|| Self::from_sidebar_key(&key))
            .or_else(|| serde_json::from_str::<Self>(&format!("\"{key}\"")).ok())
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

    /// 条目类型在 id 与文件名里的短标记。
    ///
    /// 用于保证唯一性：同名但不同类型的条目很常见（如 `serde::Deserialize`
    /// 既是 trait 又是 derive 宏），也用于区分同名的字段与方法。
    pub fn kind_tag(&self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Method => "method",
            Self::TyMethod => "tymethod",
            Self::AssocConst => "assocconst",
            Self::AssocType => "assoctype",
            Self::Variant => "variant",
            other => other.file_prefix(),
        }
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

/// 源码位置引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// 源码文件路径，如 `../src/doc_probe/lib.rs.html`。
    pub file: String,
    /// 起始行号（1 起）。
    pub line_start: Option<u32>,
    /// 结束行号（1 起，含）。
    pub line_end: Option<u32>,
}

/// 条目页面上的一个分节（由 `h2.section-header` 切分）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// 分节锚点 id，如 `fields` / `implementations`。
    pub id: String,
    /// 分节标题，如 `Fields`。
    pub title: String,
    /// 分节内的散文内容（markdown）。M1 暂空，M2 补齐。
    pub body_md: String,
    /// 归属该分节的成员条目。
    pub members: Vec<DocItem>,
}

/// 一个已解析的文档条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocItem {
    /// 条目唯一标识。
    pub id: ItemId,
    /// 条目类型。
    pub kind: ItemKind,
    /// 条目名（不含路径）。
    pub name: String,
    /// 完整路径（含 crate，不含自身）。
    pub path: Vec<String>,
    /// 所属 crate。
    pub crate_name: String,
    /// 声明签名（`pre.rust.item-decl` 的纯文本）。
    pub signature: Option<String>,
    /// 主文档（`details.top-doc` 内的 docblock，markdown）。
    pub docs_md: Option<String>,
    /// 源码位置。
    pub source: Option<SourceRef>,
    /// 页面上的分节。
    pub sections: Vec<Section>,
    /// 全部成员条目（扁平汇总）。
    pub members: Vec<DocItem>,
    /// 相对 `doc_root` 的源 HTML 路径。
    pub html_path: PathBuf,
    /// 生成该页面的 rustdoc 版本。
    pub rustdoc_version: Option<String>,
}

/// 索引里的轻量条目摘要（不含正文），供检索与导航使用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemSummary {
    /// 条目唯一标识。
    pub id: ItemId,
    /// 条目类型。
    pub kind: ItemKind,
    /// 条目名（不含路径）。
    pub name: String,
    /// 完整路径（含 crate，不含自身）。
    pub path: Vec<String>,
    /// 一行摘要（页面 meta，成员取文档首行）。
    pub one_line: String,
    /// 是否有文档。
    pub has_docs: bool,
    /// 是否有成员。
    pub has_members: bool,
    /// 相对输出根目录的 markdown 路径，统一以 `/` 分隔。
    pub file: String,
    /// 相对 `doc_root` 的源 HTML 路径，统一以 `/` 分隔。
    #[serde(default)]
    pub html_path: String,
    /// 成员条目所属父条目的 id（非成员为 `None`）。
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 源 HTML 的修改时间（Unix 毫秒），用于增量判断。
    #[serde(default)]
    pub src_mtime: Option<u64>,
}

/// crate 摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateSummary {
    /// crate 名。
    pub name: String,
    /// crate 版本（暂未采集）。
    pub version: Option<String>,
    /// 条目数（含成员）。
    pub item_count: usize,
}

/// 全量索引。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// 索引结构版本。
    pub schema_version: u32,
    /// 生成索引时的 rustdoc 版本。
    pub rustdoc_version: Option<String>,
    /// 生成时间（Unix 秒）。
    pub generated_at: u64,
    /// 产物根目录。
    pub target_doc: String,
    /// crate 列表。
    pub crates: Vec<CrateSummary>,
    /// 全部条目（扁平）。
    pub items: Vec<ItemSummary>,
}

/// 当前索引结构版本。
///
/// 凡是**影响索引内容**的改动（字段增删、id 格式变化、解析逻辑变化）都要递增，
/// 否则 `is_stale` 只看产物指纹，会继续复用按旧逻辑生成的索引。
///
/// - 2：条目 id 改为带类型标记（`serde::trait.Deserialize`），并新增 `parent_id`。
/// - 3：`one_line` 摘要里的链接重写为 `.md`，源码路径归一化为相对 `doc_root`。
/// - 4：无真实文档的条目不再返回 rustdoc 占位摘要。
pub const INDEX_SCHEMA_VERSION: u32 = 4;
