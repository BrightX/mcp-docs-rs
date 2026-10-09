//! MCP 服务的启动配置解析（全部通过环境变量传入）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// 启动配置所用的环境变量名。
pub mod env {
    /// rustdoc 产物目录（单项目便捷写法，映射为名为 `default` 的项目）。
    pub const DOC_DIR: &str = "MCP_DOCS_DIR";
    /// 输出目录（单项目便捷写法）。
    pub const OUT_DIR: &str = "MCP_DOCS_OUT";
    /// 追加多个项目：分号分隔的 `NAME=DOC_DIR` 列表，如 `core=./target/doc;svc=./svc/target/doc`。
    pub const PROJECTS: &str = "MCP_DOCS_PROJECTS";
    /// 项目清单 JSON 文件（`{ default?, store?, projects: [{name, doc_dir, out_dir?}] }`）。
    pub const PROJECTS_FILE: &str = "MCP_DOCS_PROJECTS_FILE";
    /// 共享库根目录（跨项目复用 crate 文档索引）；缺省用平台缓存目录。
    pub const STORE: &str = "MCP_DOCS_STORE";
    /// 关闭共享库：非空取值（`1`/`true`/`yes`/`on`）时开启，不读也不写全局缓存。
    pub const NO_STORE: &str = "MCP_DOCS_NO_STORE";
    /// 缺省项目名。
    pub const DEFAULT_PROJECT: &str = "MCP_DOCS_DEFAULT_PROJECT";
}

/// 一个项目的配置。
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// 项目名。
    pub name: String,
    /// rustdoc 产物目录。
    pub doc_dir: PathBuf,
    /// 输出目录（`index.json.gz` / `meta.json` 的落盘根）。
    pub out_dir: PathBuf,
}

/// server 的完整配置。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// 全部项目。
    pub projects: Vec<ProjectConfig>,
    /// 缺省项目名（工具/资源未显式指定时使用）。
    pub default_project: String,
    /// 共享库根目录；`None` 表示不启用跨项目复用。
    pub store: Option<PathBuf>,
}

/// server 启动参数（全部来自环境变量，不解析命令行）。
#[derive(Debug, Clone, Default)]
pub struct Args {
    /// 单项目便捷写法的 rustdoc 产物目录。
    doc_dir: Option<PathBuf>,
    /// 单项目便捷写法的输出目录。
    out_dir: Option<PathBuf>,
    /// `MCP_DOCS_PROJECTS` 解析出的项目列表。
    projects: Vec<ProjectArg>,
    /// 项目清单 JSON 文件路径。
    projects_file: Option<PathBuf>,
    /// 共享库根目录。
    store: Option<PathBuf>,
    /// 是否关闭共享库。
    no_store: bool,
    /// 缺省项目名。
    default_project: Option<String>,
}

impl Args {
    /// 从进程环境变量加载启动参数。
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// 从任意「键 → 值」查找函数加载参数（便于测试注入）。
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let non_empty = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());

        let projects = match non_empty(env::PROJECTS) {
            Some(raw) => parse_projects(&raw)?,
            None => Vec::new(),
        };

        Ok(Self {
            doc_dir: non_empty(env::DOC_DIR).map(PathBuf::from),
            out_dir: non_empty(env::OUT_DIR).map(PathBuf::from),
            projects,
            projects_file: non_empty(env::PROJECTS_FILE).map(PathBuf::from),
            store: non_empty(env::STORE).map(PathBuf::from),
            no_store: lookup(env::NO_STORE)
                .map(|value| parse_bool(&value))
                .unwrap_or(false),
            default_project: non_empty(env::DEFAULT_PROJECT),
        })
    }
}

/// 命令行上的单个项目（`--project NAME=DOC_DIR`）。
#[derive(Debug, Clone)]
pub struct ProjectArg {
    name: String,
    doc_dir: PathBuf,
}

/// 解析 `MCP_DOCS_PROJECTS`：分号分隔的 `NAME=DOC_DIR` 列表，空段忽略。
fn parse_projects(raw: &str) -> anyhow::Result<Vec<ProjectArg>> {
    let mut projects = Vec::new();
    for entry in raw.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        projects.push(parse_project_arg(entry).map_err(anyhow::Error::msg)?);
    }
    Ok(projects)
}

/// 解析 `NAME=DOC_DIR`。
///
/// 用 `=` 作分隔符，避免 Windows 盘符里的 `:` 造成歧义。
fn parse_project_arg(value: &str) -> Result<ProjectArg, String> {
    let (name, doc) = value
        .split_once('=')
        .ok_or_else(|| "项目参数应为 `NAME=DOC_DIR`".to_string())?;
    let name = name.trim();
    let doc = doc.trim();
    if name.is_empty() {
        return Err("项目名不能为空".to_string());
    }
    if doc.is_empty() {
        return Err("DOC_DIR 不能为空".to_string());
    }
    Ok(ProjectArg {
        name: name.to_string(),
        doc_dir: PathBuf::from(doc),
    })
}

/// 解析布尔型环境变量：空串或 `1`/`true`/`yes`/`on`（忽略大小写）视为真。
fn parse_bool(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
}

/// `--projects-file` 的 JSON 结构。
#[derive(Debug, serde::Deserialize)]
struct ProjectsFile {
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    store: Option<String>,
    projects: Vec<ProjectEntry>,
}

/// 配置文件里的单个项目。
#[derive(Debug, serde::Deserialize)]
struct ProjectEntry {
    name: String,
    doc_dir: PathBuf,
    #[serde(default)]
    out_dir: Option<PathBuf>,
}

/// 把启动参数解析成 [`ServerConfig`]。
pub fn resolve_config(args: &Args) -> anyhow::Result<ServerConfig> {
    let mut projects: Vec<ProjectConfig> = Vec::new();
    let mut file_default: Option<String> = None;
    let mut file_store: Option<String> = None;

    // 1) 配置文件。
    if let Some(path) = &args.projects_file {
        let bytes =
            std::fs::read(path).with_context(|| format!("读取项目清单 {} 失败", path.display()))?;
        let file: ProjectsFile = serde_json::from_slice(&bytes)
            .with_context(|| format!("解析项目清单 {} 失败", path.display()))?;
        file_default = file.default;
        file_store = file.store;
        for entry in file.projects {
            let doc_dir = absolute(&entry.doc_dir);
            let out_dir = entry
                .out_dir
                .as_ref()
                .map(|path| absolute(path))
                .unwrap_or_else(|| default_out(&doc_dir));
            projects.push(ProjectConfig {
                name: entry.name,
                doc_dir,
                out_dir,
            });
        }
    }

    // 2) 可重复的 --project。
    for arg in &args.projects {
        let doc_dir = absolute(&arg.doc_dir);
        projects.push(ProjectConfig {
            name: arg.name.clone(),
            doc_dir: doc_dir.clone(),
            out_dir: default_out(&doc_dir),
        });
    }

    // 3) 旧参数（映射为单项目）。仅在没有多项目配置时生效。
    if projects.is_empty() {
        let doc = args
            .doc_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("target/doc"));
        let doc_dir = absolute(&doc);
        let name = args
            .default_project
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let out_dir = args
            .out_dir
            .as_ref()
            .map(|path| absolute(path))
            .unwrap_or_else(|| default_out(&doc_dir));
        projects.push(ProjectConfig {
            name,
            doc_dir,
            out_dir,
        });
    }

    // 4) 项目名去重。
    let mut seen = HashSet::new();
    for project in &projects {
        anyhow::ensure!(
            seen.insert(project.name.clone()),
            "项目名重复：`{}`",
            project.name
        );
    }

    // 5) 缺省项目：显式 > 文件 > 名为 default 的项目 > 第一个。
    if let Some(explicit) = &args.default_project {
        anyhow::ensure!(
            projects.iter().any(|project| &project.name == explicit),
            "缺省项目 `{explicit}` 不存在"
        );
    }
    let default_project = args
        .default_project
        .clone()
        .or(file_default)
        .filter(|name| projects.iter().any(|project| &project.name == name))
        .or_else(|| {
            projects
                .iter()
                .find(|project| project.name == "default")
                .map(|project| project.name.clone())
        })
        .or_else(|| projects.first().map(|project| project.name.clone()))
        .expect("projects 已确保非空");

    // 6) 共享库根：--no-store 关闭；否则 --store > 文件 store > 平台默认。
    let store = if args.no_store {
        None
    } else {
        args.store
            .clone()
            .or_else(|| file_store.map(PathBuf::from))
            .or_else(mcp_docs_core::default_store_root)
    };

    Ok(ServerConfig {
        projects,
        default_project,
        store,
    })
}

/// 规范化为绝对路径（与旧行为一致，不受客户端工作目录影响）。
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 由 `doc_dir` 推导默认输出目录：`<doc_dir>/../doc-search`。
///
/// 与旧默认（`target/doc` → `target/doc-search`）一致。
fn default_out(doc_dir: &Path) -> PathBuf {
    doc_dir
        .parent()
        .map(|parent| parent.join("doc-search"))
        .unwrap_or_else(|| doc_dir.join("doc-search"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用一组「键 → 值」构造 [`Args`]，模拟环境变量。
    fn args_from(pairs: &[(&str, &str)]) -> Args {
        Args::from_lookup(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_string())
        })
        .unwrap()
    }

    /// `MCP_DOCS_NO_STORE` 应关闭共享库（E-5.1）。
    #[test]
    fn no_store_disables_shared_store() {
        let args = args_from(&[(env::DOC_DIR, "target/doc"), (env::NO_STORE, "1")]);
        let cfg = resolve_config(&args).unwrap();
        assert!(
            cfg.store.is_none(),
            "设置 MCP_DOCS_NO_STORE 时不应启用共享库"
        );
    }

    /// 未设置 `MCP_DOCS_STORE` 时默认回落到平台缓存目录（平台无缓存目录时除外）。
    #[test]
    fn store_defaults_to_platform_cache() {
        let args = args_from(&[(env::DOC_DIR, "target/doc")]);
        let cfg = resolve_config(&args).unwrap();
        assert_eq!(
            cfg.store.is_some(),
            mcp_docs_core::default_store_root().is_some()
        );
    }

    /// `MCP_DOCS_PROJECTS` 应按分号拆出多个项目，缺省项取第一个。
    #[test]
    fn projects_env_parses_semicolon_list() {
        let args = args_from(&[(
            env::PROJECTS,
            "core=/repo/a/target/doc; svc=/repo/b/target/doc",
        )]);
        let cfg = resolve_config(&args).unwrap();
        assert_eq!(cfg.projects.len(), 2);
        assert_eq!(cfg.projects[0].name, "core");
        assert_eq!(cfg.projects[1].name, "svc");
        assert_eq!(cfg.default_project, "core");
    }
}
