//! reading-core 统一错误模型（V1 契约冻结项）。
//!
//! 错误码数值与 C 头文件 `NR_ERR_*` 宏、Kotlin 声明必须一致；任何不兼容变更都必须
//! 递增 [`crate::ABI_VERSION`] 并同步头文件、Kotlin 声明与契约测试（reading-core NFR-C4）。
//! 冻结依据：`results/gates/gate-0-baseline.md` §2.2（12 项错误码，0 为成功）。
//!
//! 使用规则（`design.md` §4）：UI 只做本地化文案映射，禁止用错误文本判断业务分支；
//! 业务分支一律比较 [`ErrorCode`] 数值。

/// 稳定错误码：`0` 表示成功，`1..=11` 表示失败。
///
/// `#[repr(i32)]` 保证枚举判别值本身就是跨 ABI 的数值，可直接写入 C 结构体与 JNI 返回值。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorCode {
    /// 成功。
    Ok = 0,
    /// 空指针、长度为 0、非法句柄等参数错误。
    InvalidArgument = 1,
    /// 偏移/索引越界。
    OutOfRange = 2,
    /// 偏移未落在合法 UTF-8 边界。
    Utf8Boundary = 3,
    /// 书籍不存在。
    BookNotFound = 4,
    /// 尚未打开书籍。
    NoOpenBook = 5,
    /// 数据源 I/O 失败。
    IoError = 6,
    /// 请求被取消（部分成功）。
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

/// 错误码总数（含成功码 `Ok`）。V1 冻结为 12。
pub const ERROR_CODE_COUNT: usize = 12;

impl ErrorCode {
    /// 全部错误码，按数值升序排列。
    pub const ALL: [ErrorCode; ERROR_CODE_COUNT] = [
        ErrorCode::Ok,
        ErrorCode::InvalidArgument,
        ErrorCode::OutOfRange,
        ErrorCode::Utf8Boundary,
        ErrorCode::BookNotFound,
        ErrorCode::NoOpenBook,
        ErrorCode::IoError,
        ErrorCode::Cancelled,
        ErrorCode::LayoutNotFound,
        ErrorCode::EngineDestroyed,
        ErrorCode::Internal,
        ErrorCode::ChapterOutOfRange,
    ];

    /// 由跨 ABI 数值还原错误码；未登记数值返回 `None`（调用方按 [`ErrorCode::Internal`] 处理）。
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(ErrorCode::Ok),
            1 => Some(ErrorCode::InvalidArgument),
            2 => Some(ErrorCode::OutOfRange),
            3 => Some(ErrorCode::Utf8Boundary),
            4 => Some(ErrorCode::BookNotFound),
            5 => Some(ErrorCode::NoOpenBook),
            6 => Some(ErrorCode::IoError),
            7 => Some(ErrorCode::Cancelled),
            8 => Some(ErrorCode::LayoutNotFound),
            9 => Some(ErrorCode::EngineDestroyed),
            10 => Some(ErrorCode::Internal),
            11 => Some(ErrorCode::ChapterOutOfRange),
            _ => None,
        }
    }

    /// 稳定的英文短名，用于日志与结果记录；不得作为业务分支依据。
    pub const fn as_str(self) -> &'static str {
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

    /// 跨 ABI 的错误码数值（与 C 头文件 `NR_ERR_*` 一致）。
    pub const fn as_c(self) -> i32 {
        self as i32
    }

    /// 是否为成功码。
    pub const fn is_ok(self) -> bool {
        matches!(self, ErrorCode::Ok)
    }
}

impl core::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ErrorCode> for i32 {
    fn from(code: ErrorCode) -> Self {
        code.as_c()
    }
}

impl TryFrom<i32> for ErrorCode {
    type Error = NrError;

    fn try_from(code: i32) -> Result<Self, Self::Error> {
        ErrorCode::from_code(code)
            .ok_or_else(|| NrError::new(ErrorCode::Internal, format!("未登记的错误码数值: {code}")))
    }
}

/// 共享错误模型：稳定错误码 + 人类可读详情。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NrError {
    /// 稳定错误码；业务分支只允许比较该字段。
    pub code: ErrorCode,
    /// 可读详情，仅用于日志与用户提示映射。
    pub message: String,
}

impl NrError {
    /// 构造错误。
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 错误码。
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// 可读详情。
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for NrError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for NrError {}

/// 数据源 I/O 失败统一映射为 [`ErrorCode::IoError`]，避免各层各自定义错误类型。
impl From<std::io::Error> for NrError {
    fn from(err: std::io::Error) -> Self {
        NrError::new(ErrorCode::IoError, err.to_string())
    }
}

/// 核心层统一结果类型。
pub type NrResult<T> = Result<T, NrError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_frozen_numeric_values() {
        // 冻结基线：results/gates/gate-0-baseline.md §2.2（与 C 头文件 NR_ERR_* 一致）。
        assert_eq!(ErrorCode::Ok.as_c(), 0);
        assert_eq!(ErrorCode::InvalidArgument.as_c(), 1);
        assert_eq!(ErrorCode::OutOfRange.as_c(), 2);
        assert_eq!(ErrorCode::Utf8Boundary.as_c(), 3);
        assert_eq!(ErrorCode::BookNotFound.as_c(), 4);
        assert_eq!(ErrorCode::NoOpenBook.as_c(), 5);
        assert_eq!(ErrorCode::IoError.as_c(), 6);
        assert_eq!(ErrorCode::Cancelled.as_c(), 7);
        assert_eq!(ErrorCode::LayoutNotFound.as_c(), 8);
        assert_eq!(ErrorCode::EngineDestroyed.as_c(), 9);
        assert_eq!(ErrorCode::Internal.as_c(), 10);
        assert_eq!(ErrorCode::ChapterOutOfRange.as_c(), 11);
    }

    #[test]
    fn error_codes_are_complete_and_contiguous() {
        assert_eq!(ErrorCode::ALL.len(), ERROR_CODE_COUNT);
        assert_eq!(ERROR_CODE_COUNT, 12);
        for (index, code) in ErrorCode::ALL.iter().enumerate() {
            assert_eq!(
                code.as_c(),
                index as i32,
                "错误码必须从 0 起连续且无空洞: {code:?}",
            );
        }
    }

    #[test]
    fn error_codes_names_are_stable_and_unique() {
        let mut names: Vec<&str> = ErrorCode::ALL.iter().map(|c| c.as_str()).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "错误码短名必须唯一");
        assert_eq!(ErrorCode::Ok.as_str(), "Ok");
        assert_eq!(ErrorCode::Utf8Boundary.as_str(), "Utf8Boundary");
        assert_eq!(ErrorCode::ChapterOutOfRange.as_str(), "ChapterOutOfRange");
        assert_eq!(ErrorCode::Ok.to_string(), "Ok");
    }

    #[test]
    fn error_codes_round_trip_across_abi_boundary() {
        for code in ErrorCode::ALL {
            let raw = code.as_c();
            assert_eq!(
                ErrorCode::from_code(raw),
                Some(code),
                "错误码 {raw} 必须可跨 ABI 往返",
            );
            assert_eq!(i32::from(code), raw);
            assert_eq!(ErrorCode::try_from(raw), Ok(code));
        }
    }

    #[test]
    fn error_codes_reject_unregistered_values() {
        assert_eq!(ErrorCode::from_code(-1), None);
        assert_eq!(ErrorCode::from_code(12), None);
        assert_eq!(ErrorCode::from_code(i32::MAX), None);
        let err = ErrorCode::try_from(42).expect_err("未登记数值必须报错");
        assert_eq!(err.code(), ErrorCode::Internal);
    }

    #[test]
    fn error_codes_only_ok_is_success() {
        for code in ErrorCode::ALL {
            assert_eq!(code.is_ok(), code == ErrorCode::Ok);
        }
    }

    #[test]
    fn error_codes_io_mapping_is_stable() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err = NrError::from(io);
        assert_eq!(err.code(), ErrorCode::IoError);
        assert!(err.to_string().starts_with("IoError: "));
        let explicit = NrError::new(ErrorCode::BookNotFound, "book 7 不存在");
        assert_eq!(explicit.code(), ErrorCode::BookNotFound);
        assert_eq!(explicit.message(), "book 7 不存在");
    }
}
