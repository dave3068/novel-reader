//! 统一错误模型。UI 端（桌面 Slint / Android Kotlin）不得重实现该模型，
//! 只能消费错误码与消息。

/// 错误码，C ABI 与直接 Rust API 共用。
///
/// 数值与 `shared/ffi/include/novel_reader_probe.h` 中的 `NR_ERR_*` 必须一致。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// 成功。
    Ok = 0,
    /// 空指针、长度为 0、非法句柄等参数错误。
    InvalidArgument = 1,
    /// 偏移/索引越界。
    OutOfRange = 2,
    /// 偏移未落在 UTF-8 合法边界。
    Utf8Boundary = 3,
    /// 书籍不存在。
    BookNotFound = 4,
    /// 尚未打开书籍。
    NoOpenBook = 5,
    /// 数据源 I/O 失败。
    IoError = 6,
    /// 请求已被取消（部分成功）。
    Cancelled = 7,
    /// 布局标识无登记边界。
    LayoutNotFound = 8,
    /// 引擎已销毁。
    EngineDestroyed = 9,
    /// 内部错误（panic 拦截等）。
    Internal = 10,
    /// 章节越界。
    ChapterOutOfRange = 11,
}

impl ErrorCode {
    /// 稳定的英文短名，用于结果与日志。
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Ok => "Ok",
            ErrorCode::InvalidArgument => "InvalidArgument",
            ErrorCode::OutOfRange => "OutOfRange",
            ErrorCode::Utf8Boundary => "Utf8Boundary",
            ErrorCode::BookNotFound => "BookNotFound",
            ErrorCode::NoOpenBook => "NoOpenBook",
            ErrorCode::IoError => "IoError",
            ErrorCode::Cancelled => "Cancelled",
            ErrorCode::LayoutNotFound => "LayoutNotFound",
            ErrorCode::EngineDestroyed => "EngineDestroyed",
            ErrorCode::Internal => "Internal",
            ErrorCode::ChapterOutOfRange => "ChapterOutOfRange",
        }
    }

    /// 对应的 C 错误码。
    pub fn as_c(self) -> i32 {
        self as i32
    }
}

/// 共享错误模型：错误码 + 人类可读消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NrError {
    pub code: ErrorCode,
    pub message: String,
}

impl NrError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for NrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for NrError {}

/// 共享 Result 别名。
pub type NrResult<T> = Result<T, NrError>;

/// 常用错误构造快捷函数。
pub fn invalid_argument(msg: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::InvalidArgument, msg)
}
pub fn out_of_range(msg: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::OutOfRange, msg)
}
pub fn no_open_book() -> NrError {
    NrError::new(ErrorCode::NoOpenBook, "尚未打开书籍")
}
pub fn io_error(msg: impl Into<String>) -> NrError {
    NrError::new(ErrorCode::IoError, msg)
}
pub fn book_not_found(book_id: u64) -> NrError {
    NrError::new(ErrorCode::BookNotFound, format!("书籍不存在: {book_id}"))
}
pub fn chapter_out_of_range(index: u32) -> NrError {
    NrError::new(ErrorCode::ChapterOutOfRange, format!("章节越界: {index}"))
}
