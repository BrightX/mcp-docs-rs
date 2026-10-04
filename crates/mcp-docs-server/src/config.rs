//! MCP 服务的启动配置解析。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{ArgAction, Parser};

/// 一个项目的配置。
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// 项目名。
    pub name: String,
    /// rustdoc 产物目录。
    pub doc_dir: PathBuf,
    /// 输出目录（`index.json` / `meta.json` 的落盘根）。
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

/// server 启动参数。
#[derive(Debug, Parser)]
#[command(
    name = "mcp-docs-server",
    version,
    about = "以 MCP 服务形式检索本地 rustdoc 文档"
)]
pub struct Args {
    /// rustdoc 产物目录（单项目便捷写法，映射为名为 `default` 的项目）。
    #[arg(long, env = "MCP_DOCS_DIR")]
    doc_dir: Option<PathBuf>,

    /// 输出目录（单项目便捷写法）。
    #[arg(long, env = "MCP_DOCS_OUT")]
    out_dir: Option<PathBuf>,

    /// 追加一个项目：`NAME=DOC_DIR`，可重复（如 `--project core=./target/doc`）。
    #[arg(long = "project", value_parser = parse_project_arg, action = ArgAction::Append)]
    projects: Vec<ProjectArg>,

    /// 项目清单 JSON 文件（`{ default?, store?, projects: [{name, doc_dir, out_dir?}] }`）。
    #[arg(long)]
    projects_file: Option<PathBuf>,

    /// 共享库根目录（跨项目复用 crate 文档索引）；缺省用平台缓存目录。
    #[arg(long, env = "MCP_DOCS_STORE")]
    store: Option<PathBuf>,

    /// 缺省项目名。
    #[arg(long)]
    default_project: Option<String>,
}

/// 命令行上的单个项目（`--project NAME=DOC_DIR`）。
#[derive(Debug, Clone)]
pub struct ProjectArg {
    name: String,
    doc_dir: PathBuf,
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

    // 6) 共享库根：--store > 文件 store > 平台默认。
    let store = args
        .store
        .clone()
        .or_else(|| file_store.map(PathBuf::from))
        .or_else(mcp_docs_core::default_store_root);

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
