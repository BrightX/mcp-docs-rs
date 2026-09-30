//! 库内统一错误类型。

use std::path::PathBuf;

use thiserror::Error;

/// 库内统一错误类型。
#[derive(Debug, Error)]
pub enum Error {
    /// 文件读写失败。
    #[error("读写文件失败：{0}")]
    Io(#[from] std::io::Error),

    /// JSON 解析失败（`crates.js` / `sidebar-items.js` 内容异常）。
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),

    /// rustdoc 产物结构不符合预期。
    #[error("解析 {path} 失败：{reason}")]
    Parse {
        /// 出问题的文件路径。
        path: PathBuf,
        /// 具体原因。
        reason: String,
    },
}

/// 库内统一 `Result`。
pub type Result<T> = std::result::Result<T, Error>;
