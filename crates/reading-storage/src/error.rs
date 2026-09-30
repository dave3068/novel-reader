//! `reading-storage` 错误模型。
//!
//! 本 crate 位于 `reading-core` **下层**（`reading-core` 依赖 `reading-storage`），
//! 因此不得引用 `reading_core::ErrorCode`；此处定义同数值的本地错误码，数值与
//! `results/gates/gate-0-baseline.md` §2.2 的 12 项冻结错误码逐项对齐，
//! 并由 `src/lib.rs` 的内建单测把数值钉死（防漂移）。

use std::fmt;
use std::path::PathBuf;

/// 冻结错误码（gate-0 §2.2）在本 crate 内的数值口径。
pub const ERR_OK: i32 = 0;
pub const ERR_INVALID_ARGUMENT: i32 = 1;
pub const ERR_OUT_OF_RANGE: i32 = 2;
pub const ERR_UTF8_BOUNDARY: i32 = 3;
pub const ERR_BOOK_NOT_FOUND: i32 = 4;
pub const ERR_NO_OPEN_BOOK: i32 = 5;
pub const ERR_IO: i32 = 6;
pub const ERR_CANCELLED: i32 = 7;
pub const ERR_LAYOUT_NOT_FOUND: i32 = 8;
pub const ERR_ENGINE_DESTROYED: i32 = 9;
pub const ERR_INTERNAL: i32 = 10;
pub const ERR_CHAPTER_OUT_OF_RANGE: i32 = 11;

/// 存储层错误（本地错误码载体）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// 参数非法：空字段、未知枚举值、超长短摘要、非法表名等。
    InvalidArgument(String),
    /// 数值越界：负偏移、负序号、超出 i64 表示范围等。
    OutOfRange(String),
    /// 书籍不存在（书架上没有该记录）。
    BookNotFound { book_id: i64 },
    /// 文件 I/O 失败（打开 / 备份 / 恢复数据库文件）。
    Io { path: PathBuf, detail: String },
    /// SQLite 迁移失败；`backup` 为迁移前备份路径（据此可恢复到迁移前状态）。
    MigrationFailed {
        version: i64,
        name: String,
        backup: Option<PathBuf>,
        detail: String,
    },
    /// 内部错误：SQL 失败、约束冲突、schema 不一致等。
    Internal(String),
}

impl StorageError {
    /// 映射到 gate-0 §2.2 冻结错误码。
    pub fn code(&self) -> i32 {
        match self {
            StorageError::InvalidArgument(_) => ERR_INVALID_ARGUMENT,
            StorageError::OutOfRange(_) => ERR_OUT_OF_RANGE,
            StorageError::BookNotFound { .. } => ERR_BOOK_NOT_FOUND,
            StorageError::Io { .. } => ERR_IO,
            StorageError::MigrationFailed { .. } | StorageError::Internal(_) => ERR_INTERNAL,
        }
    }

    /// 是否为「迁移失败且带备份」的可恢复错误。
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            StorageError::MigrationFailed {
                backup: Some(_),
                ..
            }
        )
    }

    /// 迁移备份路径（若存在）。
    pub fn backup_path(&self) -> Option<&PathBuf> {
        match self {
            StorageError::MigrationFailed { backup, .. } => backup.as_ref(),
            _ => None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        StorageError::InvalidArgument(message.into())
    }

    pub fn out_of_range(message: impl Into<String>) -> Self {
        StorageError::OutOfRange(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        StorageError::Internal(message.into())
    }

    pub fn io(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        StorageError::Io {
            path: path.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            StorageError::OutOfRange(m) => write!(f, "out of range: {m}"),
            StorageError::BookNotFound { book_id } => write!(f, "book not found: {book_id}"),
            StorageError::Io { path, detail } => {
                write!(f, "io error: {} ({detail})", path.display())
            }
            StorageError::MigrationFailed {
                version,
                name,
                backup,
                detail,
            } => write!(
                f,
                "migration {version} ({name}) failed: {detail}; backup={}",
                backup
                    .as_ref()
                    .map_or_else(|| "<none>".to_string(), |p| p.display().to_string())
            ),
            StorageError::Internal(m) => write!(f, "internal: {m}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        StorageError::Internal(format!("sqlite: {error}"))
    }
}

/// 本 crate 的 `Result` 别名。
pub type Result<T> = std::result::Result<T, StorageError>;
