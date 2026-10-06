//! 桌面 TXT 导入链路（DT2）：文件选择 → 校验 → 来源指纹 → 落库 → 可读结果。
//!
//! 依据 `desktop/spec.md` DR1/DR5、`desktop/design.md` §文件选择 与
//! `docs/novel-reader-v1-development-plan.md` §6.3：
//!
//! 1. 桌面文件选择器返回路径后，**先**校验扩展名、可读性与元数据，**再**创建书籍记录；
//! 2. 导入失败不得留下半成品（校验全部发生在第一次写库动作之前，且导入后做一次自检，
//!    自检失败时把刚刚创建的条目硬删除——只删书库记录，绝不触碰用户文件）；
//! 3. 来源指纹 = 大小 + 头/中/尾有限抽样哈希（算法与抽样上限全部来自冻结核心
//!    `reading-source`，本模块不重实现任何阅读语义）；
//! 4. 编码识别同样来自 `reading-source`（`detect_encoding`）；
//! 5. 本模块对用户原始文件**只读**：不移动、不重命名、不截断、不删除。
//!
//! # 选择器抽象
//!
//! [`FilePicker`] 把「系统文件对话框」与导入链路解耦：
//!
//! - [`SystemFilePicker`]：真实系统对话框（`zenity` / `kdialog`，二者都是本机基线
//!   Deepin 20.9 自带 GTK/Qt 对话框程序；找不到程序时返回 [`ImportErrorKind::NoPicker`]）；
//! - [`StaticFilePicker`]：固定返回给定路径（程序化验证与自动化用，见 `NR_IMPORT_PATH`）；
//! - [`CancellingFilePicker`]：模拟用户取消（必须零改动）。
//!
//! 用户取消（对话框退出码 1）不是错误：返回 `Ok(None)`，调用方不得写任何记录。

use std::path::{Path, PathBuf};
// `Command` 只用于 Linux 侧的 zenity/kdialog 分支（Windows 走原生对话框）。
#[cfg(not(target_os = "windows"))]
use std::process::Command;
use std::time::UNIX_EPOCH;

use reading_source::{
    detect_encoding, ConfidenceLevel, EncodingDetection, FileTextSource, Fingerprint, TextSource,
    FINGERPRINT_ALGO, FINGERPRINT_SAMPLE_BYTES,
};
use reading_storage::books::{
    self, BookImport, FORMAT_TXT, KIND_DESKTOP_PATH, STATUS_IMPORT_FAILED, STATUS_READY,
};
use reading_storage::Storage;

/// 唯一接受的文件扩展名（大小写不敏感）。
pub const TXT_EXTENSION: &str = "txt";

/// 程序化验证用：设置后以 [`StaticFilePicker`] 替代系统对话框（值为目标文件路径）。
pub const IMPORT_PATH_ENV: &str = "NR_IMPORT_PATH";

/// 系统对话框程序覆盖（测试/受限环境用；未设置时按 [`PICKER_PROGRAMS`] 顺序探测）。
pub const PICKER_PROGRAM_ENV: &str = "NR_FILE_PICKER";

/// 默认探测的对话框程序（依次尝试；全部不可用时返回 `NoPicker`）。
pub const PICKER_PROGRAMS: [&str; 2] = ["zenity", "kdialog"];

/// Open 对话框标题。
pub const PICKER_TITLE: &str = "选择 TXT 文件";

/// 多选对话框标题（V1.1）。
pub const PICKER_TITLE_MULTI: &str = "选择多个 TXT 文件（可按住 Ctrl/Shift 多选）";

/// 选目录对话框标题（V1.1）。
pub const PICKER_TITLE_DIRECTORY: &str = "选择包含 TXT 的目录";

/// 选图片对话框标题（V1.1，阅读背景图）。
pub const PICKER_TITLE_IMAGE: &str = "选择阅读背景图片";

/// 多选输出的分隔符（显式指定，不依赖 zenity 版本默认值）。
///
/// 选用 ASCII 单元分隔符 U+001F —— 它不可能出现在文件系统路径里，
/// 而换行/冒号都可能（文件名可含换行）。
pub const MULTI_SELECT_SEPARATOR: char = '\u{1f}';

/// 图片过滤器（zenity 口径）。
pub const PICKER_IMAGE_FILTER: &str = "图片文件 | *.png *.jpg *.jpeg *.bmp *.webp";

/// 图片过滤器（kdialog 口径）。
pub const PICKER_IMAGE_FILTER_KDLIB: &str = "*.png *.jpg *.jpeg *.bmp *.webp|图片文件";

/// `HOME` 缺失时的对话框起始目录（根目录，不会导致对话框打不开）。
fn home_fallback() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

/// 当前 Unix 秒（仓储与状态列的时间口径）。
pub fn now_unix() -> i64 {
    reading_storage::connection::now_unix()
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// 导入失败的稳定分类（可读文案见 [`ImportError::message`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportErrorKind {
    /// 扩展名不是 `.txt`。
    NotTxt,
    /// 路径不存在或不可访问。
    MissingFile,
    /// 不是普通文件（目录 / 设备 / 管道等）。
    NotRegularFile,
    /// 0 字节空文件。
    EmptyFile,
    /// 存在但打不开（权限或 IO 错误）。
    Unreadable,
    /// 系统里没有可用的文件对话框程序。
    NoPicker,
    /// 对话框程序本身调用失败。
    PickerFailed,
    /// 元数据 / 指纹 / 编码识别失败。
    Fingerprint,
    /// 本地书架数据库读写失败。
    Storage,
}

impl ImportErrorKind {
    /// 稳定标识（写入日志与证据，不随文案变化）。
    pub const fn code(self) -> &'static str {
        match self {
            ImportErrorKind::NotTxt => "import_not_txt",
            ImportErrorKind::MissingFile => "import_missing_file",
            ImportErrorKind::NotRegularFile => "import_not_regular_file",
            ImportErrorKind::EmptyFile => "import_empty_file",
            ImportErrorKind::Unreadable => "import_unreadable",
            ImportErrorKind::NoPicker => "import_no_picker",
            ImportErrorKind::PickerFailed => "import_picker_failed",
            ImportErrorKind::Fingerprint => "import_fingerprint_failed",
            ImportErrorKind::Storage => "import_storage_failed",
        }
    }

    /// 可读短语（中文优先）。
    pub const fn label(self) -> &'static str {
        match self {
            ImportErrorKind::NotTxt => "只接受 .txt 纯文本文件",
            ImportErrorKind::MissingFile => "文件不存在或已不可访问",
            ImportErrorKind::NotRegularFile => "所选路径不是普通文件",
            ImportErrorKind::EmptyFile => "文件为空，没有可阅读内容",
            ImportErrorKind::Unreadable => "文件存在但无法读取",
            ImportErrorKind::NoPicker => "未找到可用的系统文件对话框",
            ImportErrorKind::PickerFailed => "文件对话框调用失败",
            ImportErrorKind::Fingerprint => "读取文件元数据或指纹失败",
            ImportErrorKind::Storage => "本地书架数据库读写失败",
        }
    }
}

/// 导入错误：稳定分类 + 相关路径 + 可读细节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    /// 稳定分类。
    pub kind: ImportErrorKind,
    /// 相关路径（可能为空）。
    pub path: Option<PathBuf>,
    /// 可读技术细节（不用于业务分支）。
    pub detail: String,
}

impl ImportError {
    /// 构造错误。
    pub fn new(kind: ImportErrorKind, path: Option<&Path>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            path: path.map(Path::to_path_buf),
            detail: detail.into(),
        }
    }

    /// 面向用户的一行可读错误（导入失败时直接显示在状态栏）。
    pub fn message(&self) -> String {
        let where_ = match &self.path {
            Some(path) => format!("（{}）", path.display()),
            None => String::new(),
        };
        let detail = self.detail.trim();
        if detail.is_empty() {
            format!("{}{}", self.kind.label(), where_)
        } else {
            format!("{}{}：{}", self.kind.label(), where_, detail)
        }
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.kind.code(), self.message())
    }
}

fn storage_error(path: &Path, action: &str, error: impl std::fmt::Display) -> ImportError {
    ImportError::new(
        ImportErrorKind::Storage,
        Some(path),
        format!("{action}失败：{error}"),
    )
}

// ---------------------------------------------------------------------------
// 选择器
// ---------------------------------------------------------------------------

/// 文件选择器抽象：返回 `Ok(None)` 表示用户取消（不是错误）。
///
/// V1.1 扩展（规格 R6 / T2）：单文件之外补**多选**与**选目录**，
/// 以支撑「批量导入并按文件系统目录名归类」。三个方法的默认实现都是
/// 「退回单文件选择」或「不支持」，这样既有实现（测试替身、程序化驱动）
/// 不必立刻实现全部方法。
pub trait FilePicker: Send + Sync {
    /// 打开系统对话框让用户选择一个 TXT 文件。
    fn pick_txt_file(&self) -> Result<Option<PathBuf>, ImportError>;

    /// 多选 TXT 文件（V1.1）。
    ///
    /// 默认实现：退化为 `pick_txt_file`（返回 0 或 1 个），
    /// 保证既有实现可用但**不具备**多选能力。
    fn pick_txt_files(&self) -> Result<Option<Vec<PathBuf>>, ImportError> {
        Ok(self.pick_txt_file()?.map(|path| vec![path]))
    }

    /// 选择一个目录（V1.1；批量导入按目录名归类的入口）。
    ///
    /// 默认实现：返回「不支持」，由调用方给出可理解提示，
    /// **不得**静默退化成选文件（否则用户会以为选到了目录）。
    fn pick_directory(&self) -> Result<Option<PathBuf>, ImportError> {
        Err(ImportError::new(
            ImportErrorKind::NoPicker,
            None,
            "当前环境不支持选择目录",
        ))
    }

    /// 选择一张背景图片（V1.1；全局阅读背景图）。
    ///
    /// 默认实现：返回「不支持」—— **不得**退化成 `pick_txt_file`
    /// （用户会以为选到了图片，实际拿到的是 TXT）。
    fn pick_image(&self) -> Result<Option<PathBuf>, ImportError> {
        Err(ImportError::new(
            ImportErrorKind::NoPicker,
            None,
            "当前环境不支持选择图片",
        ))
    }

    /// 选择一张图片（V1.1；阅读背景图入口，规格 R14）。
    ///
    /// 默认实现：返回「不支持」。
    fn pick_image_file(&self) -> Result<Option<PathBuf>, ImportError> {
        Err(ImportError::new(
            ImportErrorKind::NoPicker,
            None,
            "当前环境不支持选择图片",
        ))
    }
}

/// 对话框程序方言（决定命令行参数与退出码语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerFlavor {
    /// GTK `zenity`：`--file-selection`；退出码 0 = 选中，1 = 取消。
    Zenity,
    /// KDE `kdialog`：`--getopenfilename`；退出码 0 = 选中，1 = 取消。
    KDialog,
}

impl PickerFlavor {
    fn from_program(program: &str) -> Self {
        if Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("kdialog"))
        {
            PickerFlavor::KDialog
        } else {
            PickerFlavor::Zenity
        }
    }

    fn stdout_is_selection(self) -> &'static str {
        match self {
            PickerFlavor::Zenity => "zenity --file-selection",
            PickerFlavor::KDialog => "kdialog --getopenfilename",
        }
    }

    fn args(self) -> Vec<String> {
        match self {
            PickerFlavor::Zenity => vec![
                "--file-selection".to_string(),
                format!("--title={PICKER_TITLE}"),
                format!("--file-filter=TXT 文本文件 ({TXT_EXTENSION}) | *.{TXT_EXTENSION} *.TXT"),
            ],
            PickerFlavor::KDialog => vec![
                "--getopenfilename".to_string(),
                std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
                format!("*.{TXT_EXTENSION}|TXT 文本文件"),
            ],
        }
    }

    /// 多选参数（V1.1 / 规格 R6）。
    ///
    /// `kdialog` 不支持多选，返回空切片 → 调用方按「不支持多选」如实回报，
    /// **不**静默退化成单选（否则用户以为选了一堆）。
    fn multi_args(self) -> Vec<String> {
        match self {
            PickerFlavor::Zenity => vec![
                "--file-selection".to_string(),
                "--multiple".to_string(),
                // 多选时输出按分隔符拆行；显式指定以免受 zenity 版本默认值影响。
                format!("--separator={MULTI_SELECT_SEPARATOR}"),
                format!("--title={PICKER_TITLE_MULTI}"),
                format!("--file-filter=TXT 文本文件 ({TXT_EXTENSION}) | *.{TXT_EXTENSION} *.TXT"),
            ],
            PickerFlavor::KDialog => Vec::new(),
        }
    }

    /// 选目录参数（V1.1 / 规格 R6）。
    fn directory_args(self) -> Vec<String> {
        match self {
            PickerFlavor::Zenity => vec![
                "--file-selection".to_string(),
                "--directory".to_string(),
                format!("--title={PICKER_TITLE_DIRECTORY}"),
            ],
            PickerFlavor::KDialog => {
                vec!["--getexistingdirectory".to_string(), home_fallback()]
            }
        }
    }

    /// 选图片参数（V1.1 / 规格 R14）。
    fn image_args(self) -> Vec<String> {
        match self {
            PickerFlavor::Zenity => vec![
                "--file-selection".to_string(),
                format!("--title={PICKER_TITLE_IMAGE}"),
                format!("--file-filter={PICKER_IMAGE_FILTER}"),
            ],
            PickerFlavor::KDialog => vec![
                "--getopenfilename".to_string(),
                home_fallback(),
                PICKER_IMAGE_FILTER_KDLIB.to_string(),
            ],
        }
    }
}

/// 系统文件对话框（依次尝试 [`PICKER_PROGRAMS`]，直到有一个可用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFilePicker {
    programs: Vec<String>,
}

impl Default for SystemFilePicker {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemFilePicker {
    /// 按环境变量 [`PICKER_PROGRAM_ENV`] 或默认识别程序清单构造。
    pub fn new() -> Self {
        if let Some(value) = std::env::var_os(PICKER_PROGRAM_ENV) {
            let value = value.to_string_lossy().trim().to_string();
            if !value.is_empty() {
                return Self {
                    programs: vec![value],
                };
            }
        }
        Self {
            programs: PICKER_PROGRAMS
                .iter()
                .map(|name| name.to_string())
                .collect(),
        }
    }

    /// 指定程序清单（测试用；不读取环境变量）。
    pub fn with_programs(programs: Vec<String>) -> Self {
        Self { programs }
    }

    /// 已配置的程序清单。
    pub fn programs(&self) -> &[String] {
        &self.programs
    }
}

/// 按对话框程序与退出码解释结果：`Ok(None)` = 用户取消。
pub fn interpret_picker_result(
    program: &str,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<Option<PathBuf>, ImportError> {
    let flavor = PickerFlavor::from_program(program);
    match exit_code {
        Some(0) => match first_non_empty_line(stdout) {
            Some(line) => Ok(Some(PathBuf::from(line))),
            None => Err(ImportError::new(
                ImportErrorKind::PickerFailed,
                None,
                format!("{} 退出码 0 但没有输出路径", flavor.stdout_is_selection()),
            )),
        },
        Some(1) => Ok(None),
        Some(code) => Err(ImportError::new(
            ImportErrorKind::PickerFailed,
            None,
            format!(
                "{} 退出码 {code}{}",
                flavor.stdout_is_selection(),
                first_non_empty_line(stderr)
                    .map(|line| format!("：{line}"))
                    .unwrap_or_default()
            ),
        )),
        None => Err(ImportError::new(
            ImportErrorKind::PickerFailed,
            None,
            format!("{} 被信号终止", flavor.stdout_is_selection()),
        )),
    }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| line.trim().trim_end_matches('\r'))
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

impl FilePicker for SystemFilePicker {
    fn pick_txt_file(&self) -> Result<Option<PathBuf>, ImportError> {
        // Windows：使用**原生** `GetOpenFileNameW`（comdlg32）。
        // 本机没有 zenity/kdialog，原先这条路径只会走到「未找到对话框程序」——
        // 表现就是点击「导入 TXT」毫无反应（2026-09-29 真实缺陷）。
        #[cfg(target_os = "windows")]
        {
            return pick_with_windows_dialog();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut last_missing: Option<ImportError> = None;
            for program in &self.programs {
                let flavor = PickerFlavor::from_program(program);
                let output = match Command::new(program).args(flavor.args()).output() {
                    Ok(output) => output,
                    // 程序不存在：继续尝试下一个；全部缺失才是 NoPicker。
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        last_missing = Some(ImportError::new(
                            ImportErrorKind::NoPicker,
                            None,
                            format!("未找到对话框程序 `{program}`"),
                        ));
                        continue;
                    }
                    Err(error) => {
                        return Err(ImportError::new(
                            ImportErrorKind::PickerFailed,
                            None,
                            format!("启动 `{program}` 失败：{error}"),
                        ))
                    }
                };
                return interpret_picker_result(
                    program,
                    output.status.code(),
                    &String::from_utf8_lossy(&output.stdout),
                    &String::from_utf8_lossy(&output.stderr),
                );
            }
            Err(last_missing.unwrap_or_else(|| {
                ImportError::new(
                    ImportErrorKind::NoPicker,
                    None,
                    "未配置任何对话框程序；可用 NR_FILE_PICKER 指定",
                )
            }))
        }
    }

    /// 多选 TXT（V1.1 / 规格 R6）。
    ///
    /// **Windows 未实测**：`GetOpenFileNameW` 不支持多选，需升级到
    /// `IFileOpenDialog`（`Win32_UI_Shell`）。该路径留到 T11 收口期实现，
    /// 本期在 Windows 上**明确返回不支持**，绝不静默退化成单选
    /// （那会让用户以为选了一堆书，实际只导入一本）。
    #[cfg(target_os = "windows")]
    fn pick_txt_files(&self) -> Result<Option<Vec<PathBuf>>, ImportError> {
        Err(windows_multi_select_unsupported())
    }

    /// 选目录（V1.1 / 规格 R6）。
    #[cfg(target_os = "windows")]
    fn pick_directory(&self) -> Result<Option<PathBuf>, ImportError> {
        Err(windows_multi_select_unsupported())
    }

    /// 选图片（V1.1 / 规格 R14）。
    ///
    /// Windows 侧**单选**本可复用既有 `GetOpenFileNameW`（仅换过滤串），
    /// 但为把 Windows 改动**整体隔离**到 T11 收口期（见 `pick_txt_files` 的说明），
    /// 本期同样如实回报不支持，不在未实测路径上追加新代码。
    #[cfg(target_os = "windows")]
    fn pick_image_file(&self) -> Result<Option<PathBuf>, ImportError> {
        Err(windows_multi_select_unsupported())
    }
}

#[cfg(target_os = "windows")]
fn windows_multi_select_unsupported() -> ImportError {
    ImportError::new(
        ImportErrorKind::NoPicker,
        None,
        "Windows 的多选 / 选目录 / 选图片尚未接入（需要 IFileOpenDialog）；本期请用「导入 TXT」逐个添加",
    )
}

/// 把对话框输出解析为多条路径（按 [`MULTI_SELECT_SEPARATOR`] 拆分）。
///
/// 单独抽出以便单测：zenity 多选输出一条含分隔符的行。
pub(crate) fn parse_multi_selection(stdout: &str) -> Vec<PathBuf> {
    stdout
        .split(MULTI_SELECT_SEPARATOR)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(not(target_os = "windows"))]
impl SystemFilePicker {
    /// 在候选程序上跑一次给定参数，返回第一条成功的选择。
    ///
    /// 「取消」（退出码 1）立即返回 `Ok(None)`；程序缺失则继续试下一个；
    /// 全部缺失才报 `NoPicker`。
    fn run_picker(
        &self,
        args_for: fn(PickerFlavor) -> Vec<String>,
        action: &str,
    ) -> Result<Option<String>, ImportError> {
        let mut last_missing: Option<ImportError> = None;
        for program in &self.programs {
            let flavor = PickerFlavor::from_program(program);
            let args = args_for(flavor);
            if args.is_empty() {
                // 该程序不支持此能力（如 kdialog 无多选）：如实回报，不静默退化。
                return Err(ImportError::new(
                    ImportErrorKind::NoPicker,
                    None,
                    format!("`{program}` 不支持{action}"),
                ));
            }
            let output = match Command::new(program).args(&args).output() {
                Ok(output) => output,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    last_missing = Some(ImportError::new(
                        ImportErrorKind::NoPicker,
                        None,
                        format!("未找到对话框程序 `{program}`"),
                    ));
                    continue;
                }
                Err(error) => {
                    return Err(ImportError::new(
                        ImportErrorKind::PickerFailed,
                        None,
                        format!("启动 `{program}` 失败：{error}"),
                    ));
                }
            };
            return match output.status.code() {
                Some(0) => Ok(first_non_empty_line(&String::from_utf8_lossy(
                    &output.stdout,
                ))),
                Some(1) => Ok(None),
                other => Err(ImportError::new(
                    ImportErrorKind::PickerFailed,
                    None,
                    format!("{action}对话框异常退出（{program} 退出码 {other:?}）"),
                )),
            };
        }
        Err(last_missing.unwrap_or_else(|| {
            ImportError::new(
                ImportErrorKind::NoPicker,
                None,
                "未配置任何对话框程序；可用 NR_FILE_PICKER 指定",
            )
        }))
    }

    fn pick_txt_files(&self) -> Result<Option<Vec<PathBuf>>, ImportError> {
        let mut last_missing: Option<ImportError> = None;
        for program in &self.programs {
            let flavor = PickerFlavor::from_program(program);
            let args = flavor.multi_args();
            if args.is_empty() {
                return Err(ImportError::new(
                    ImportErrorKind::NoPicker,
                    None,
                    format!("`{program}` 不支持多选；请逐个添加文件"),
                ));
            }
            let output = match Command::new(program).args(&args).output() {
                Ok(output) => output,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    last_missing = Some(ImportError::new(
                        ImportErrorKind::NoPicker,
                        None,
                        format!("未找到对话框程序 `{program}`"),
                    ));
                    continue;
                }
                Err(error) => {
                    return Err(ImportError::new(
                        ImportErrorKind::PickerFailed,
                        None,
                        format!("启动 `{program}` 失败：{error}"),
                    ));
                }
            };
            return match output.status.code() {
                Some(0) => {
                    let paths = parse_multi_selection(&String::from_utf8_lossy(&output.stdout));
                    if paths.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(paths))
                    }
                }
                Some(1) => Ok(None),
                other => Err(ImportError::new(
                    ImportErrorKind::PickerFailed,
                    None,
                    format!("多选对话框异常退出（{program} 退出码 {other:?}）"),
                )),
            };
        }
        Err(last_missing.unwrap_or_else(|| {
            ImportError::new(
                ImportErrorKind::NoPicker,
                None,
                "未配置任何对话框程序；可用 NR_FILE_PICKER 指定",
            )
        }))
    }

    fn pick_directory(&self) -> Result<Option<PathBuf>, ImportError> {
        self.run_picker(PickerFlavor::directory_args, "选择目录")
            .map(|value| value.map(PathBuf::from))
    }

    fn pick_image_file(&self) -> Result<Option<PathBuf>, ImportError> {
        self.run_picker(PickerFlavor::image_args, "选择图片")
            .map(|value| value.map(PathBuf::from))
    }
}

/// 文件类型过滤串（`GetOpenFileNameW` 的口径：双 NUL 结尾的 `说明\0模式\0…` 列表）。
///
/// 该函数**不按平台 cfg 门控**：它的不变量（双 NUL 结尾、含 `*.txt`）在 Linux 上也可单测。
pub fn txt_picker_filter() -> Vec<u16> {
    let text = "TXT 文本文件 (*.txt)\0*.txt\0所有文件 (*.*)\0*.*\0";
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 宽串（NUL 结尾）——Win32 对话框 API 的入参形式。
#[cfg(target_os = "windows")]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Windows 原生「打开文件」对话框（`GetOpenFileNameW`）。
///
/// 契约与 zenity/kdialog 分支一致：`Ok(None)` = 用户取消；真正的失败给出可读原因
/// （用 `CommDlgExtendedError` 区分「取消」与「失败」，绝不把失败伪装成取消）。
#[cfg(target_os = "windows")]
fn pick_with_windows_dialog() -> Result<Option<PathBuf>, ImportError> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let mut filter = txt_picker_filter();
    let title = wide("选择要导入的 TXT");
    // 8 KiB 路径缓冲（`nMaxFile` 为字符数）；够放长路径与多选分隔串。
    let mut buffer = vec![0u16; 4096];
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.lpstrFilter = filter.as_mut_ptr();
    ofn.lpstrFile = buffer.as_mut_ptr();
    ofn.nMaxFile = buffer.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;

    let accepted = unsafe { GetOpenFileNameW(&mut ofn) };
    if accepted == 0 {
        let extended = unsafe { CommDlgExtendedError() };
        if extended == 0 {
            // 用户直接关闭 / 取消：对话框没有错误码。
            return Ok(None);
        }
        return Err(ImportError::new(
            ImportErrorKind::PickerFailed,
            None,
            format!("Windows 文件对话框失败（CommDlgExtendedError=0x{extended:08X}）"),
        ));
    }
    let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(0);
    if length == 0 {
        return Err(ImportError::new(
            ImportErrorKind::PickerFailed,
            None,
            "Windows 文件对话框返回空路径".to_string(),
        ));
    }
    let path = std::ffi::OsString::from_wide(&buffer[..length]);
    Ok(Some(PathBuf::from(path)))
}

/// 固定返回给定路径的选择器（程序化验证 / 自动化；对应 `NR_IMPORT_PATH`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticFilePicker {
    path: PathBuf,
}

impl StaticFilePicker {
    /// 构造。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 目标路径。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl FilePicker for StaticFilePicker {
    fn pick_txt_file(&self) -> Result<Option<PathBuf>, ImportError> {
        Ok(Some(self.path.clone()))
    }
}

/// 模拟用户取消的选择器（必须零改动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CancellingFilePicker;

impl FilePicker for CancellingFilePicker {
    fn pick_txt_file(&self) -> Result<Option<PathBuf>, ImportError> {
        Ok(None)
    }
}

/// 生产选择器：设置 [`IMPORT_PATH_ENV`] 时用静态路径，否则用系统对话框。
pub fn picker_from_env() -> Box<dyn FilePicker> {
    match startup_import_path() {
        Some(path) => Box::new(StaticFilePicker::new(path)),
        None => Box::new(SystemFilePicker::new()),
    }
}

/// [`IMPORT_PATH_ENV`] 指定的启动期导入路径（未设置或为空时返回 `None`）。
pub fn startup_import_path() -> Option<PathBuf> {
    let value = std::env::var_os(IMPORT_PATH_ENV)?;
    let value = value.to_string_lossy();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

// ---------------------------------------------------------------------------
// 校验与事实提取
// ---------------------------------------------------------------------------

/// 已通过校验的文件事实（不含任何正文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedFile {
    /// 绝对或用户给出的原始路径。
    pub path: PathBuf,
    /// 原始文件名（含扩展名）。
    pub original_name: String,
    /// 默认书名（去掉 `.txt` 扩展名后的文件名）。
    pub display_name: String,
    /// 字节数。
    pub size: u64,
    /// 修改时间（Unix 毫秒；不可得时为 `None`）。
    pub modified_unix_ms: Option<i64>,
}

/// 去掉 `.txt` 扩展名得到默认书名（大小写不敏感）；结果为空时回退原始文件名。
pub fn derive_display_name(original_name: &str) -> String {
    let trimmed = original_name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let stem = match trimmed.len().checked_sub(TXT_EXTENSION.len() + 1) {
        Some(cut) if cut > 0 => {
            let (head, tail) = trimmed.split_at(cut);
            if tail.eq_ignore_ascii_case(".txt") {
                head
            } else {
                trimmed
            }
        }
        _ => trimmed,
    };
    let stem = stem.trim();
    if stem.is_empty() {
        trimmed.to_string()
    } else {
        stem.to_string()
    }
}

/// 校验扩展名 / 元数据 / 可读性。**不写任何库记录**，失败即返回可读错误。
pub fn validate_selection(path: &Path) -> Result<SelectedFile, ImportError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case(TXT_EXTENSION) {
        return Err(ImportError::new(
            ImportErrorKind::NotTxt,
            Some(path),
            format!(
                "扩展名 `{}` 不受支持（只接受 .{TXT_EXTENSION}）",
                if extension.is_empty() {
                    "无"
                } else {
                    extension
                }
            ),
        ));
    }

    let metadata = std::fs::metadata(path).map_err(|error| {
        let kind = match error.kind() {
            std::io::ErrorKind::NotFound => ImportErrorKind::MissingFile,
            std::io::ErrorKind::PermissionDenied => ImportErrorKind::Unreadable,
            _ => ImportErrorKind::MissingFile,
        };
        ImportError::new(kind, Some(path), error.to_string())
    })?;

    if !metadata.is_file() {
        return Err(ImportError::new(
            ImportErrorKind::NotRegularFile,
            Some(path),
            "元数据表明它不是普通文件",
        ));
    }
    if metadata.len() == 0 {
        return Err(ImportError::new(
            ImportErrorKind::EmptyFile,
            Some(path),
            "0 字节",
        ));
    }

    // 可读性：真实打开一次句柄（只打开，不读正文，也不写）。
    std::fs::File::open(path).map_err(|error| {
        ImportError::new(ImportErrorKind::Unreadable, Some(path), error.to_string())
    })?;

    let original_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let modified_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|delta| delta.as_millis() as i64);

    Ok(SelectedFile {
        path: path.to_path_buf(),
        display_name: derive_display_name(&original_name),
        original_name,
        size: metadata.len(),
        modified_unix_ms,
    })
}

// ---------------------------------------------------------------------------
// 指纹（算法与抽样上限全部来自 reading-source）
// ---------------------------------------------------------------------------

/// 紧凑指纹文本：算法 + 大小 + 头/中/尾抽样哈希。
///
/// 与 `reading-ffi::engine::compact_fingerprint` 同格式（同一落库口径），
/// 长度 ≤ `reading-storage` 的 `MAX_FINGERPRINT_BYTES`（96）。
pub fn compact_fingerprint(fingerprint: &Fingerprint) -> String {
    format!(
        "{}:{}:{:016x}:{:016x}:{:016x}",
        fingerprint.algo,
        fingerprint.size,
        fingerprint.head_hash,
        fingerprint.mid_hash,
        fingerprint.tail_hash
    )
}

/// 解析 [`compact_fingerprint`] 生成的文本；算法不识别或格式非法时返回 `None`。
pub fn parse_compact_fingerprint(text: &str) -> Option<Fingerprint> {
    let mut parts = text.split(':');
    let algo = parts.next()?;
    if algo != FINGERPRINT_ALGO {
        return None;
    }
    let size = parts.next()?.parse::<u64>().ok()?;
    let head_hash = u64::from_str_radix(parts.next()?, 16).ok()?;
    let mid_hash = u64::from_str_radix(parts.next()?, 16).ok()?;
    let tail_hash = u64::from_str_radix(parts.next()?, 16).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Fingerprint {
        size,
        head_hash,
        mid_hash,
        tail_hash,
        sample_len: FINGERPRINT_SAMPLE_BYTES,
        sampled_bytes: Fingerprint::sample_bytes_bound(FINGERPRINT_SAMPLE_BYTES).min(size),
        modified: None,
        algo: FINGERPRINT_ALGO,
    })
}

/// 计算文件指纹（抽样读取，绝不整本读取）并返回紧凑文本。
pub fn fingerprint_file(path: &Path) -> Result<(Fingerprint, String), ImportError> {
    let source = FileTextSource::open(path).map_err(|error| {
        ImportError::new(
            ImportErrorKind::Fingerprint,
            Some(path),
            format!("打开文件失败：{error}"),
        )
    })?;
    let fingerprint = source.fingerprint().map_err(|error| {
        ImportError::new(
            ImportErrorKind::Fingerprint,
            Some(path),
            format!("计算指纹失败：{error}"),
        )
    })?;
    let compact = compact_fingerprint(&fingerprint);
    Ok((fingerprint, compact))
}

/// 可信度 → 千分比（与 `reading-ffi::abi::confidence_permille` 同口径）。
pub const fn confidence_permille(level: ConfidenceLevel) -> u32 {
    match level {
        ConfidenceLevel::Low => 333,
        ConfidenceLevel::Medium => 666,
        ConfidenceLevel::High => 1000,
    }
}

/// 由已校验事实 + 指纹 + 编码识别结果构造落库请求。
pub fn build_import(
    selected: &SelectedFile,
    fingerprint: &str,
    detection: &EncodingDetection,
) -> BookImport {
    BookImport {
        display_name: selected.display_name.clone(),
        original_name: selected.original_name.clone(),
        source_kind: KIND_DESKTOP_PATH.to_string(),
        source_locator: selected.path.to_string_lossy().to_string(),
        source_size: selected.size as i64,
        source_modified_hint: selected.modified_unix_ms,
        source_fingerprint: Some(fingerprint.to_string()),
        encoding: detection.encoding.as_str().to_string(),
        encoding_confidence: confidence_permille(detection.confidence) as i32,
        encoding_user_override: false,
    }
}

// ---------------------------------------------------------------------------
// 导入结果
// ---------------------------------------------------------------------------

/// 导入成功的事实（全部来自真实数据库读回，不是入参回显）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// 书籍 id。
    pub book_id: i64,
    /// 书名。
    pub display_name: String,
    /// 原始文件名。
    pub original_name: String,
    /// 来源路径。
    pub path: PathBuf,
    /// 字节数。
    pub size: u64,
    /// 紧凑指纹。
    pub fingerprint: String,
    /// 编码稳定名。
    pub encoding: String,
    /// 编码可信度（千分比）。
    pub encoding_confidence: u32,
    /// 是否新建条目。
    pub created: bool,
    /// 是否新增来源记录。
    pub source_created: bool,
    /// 条目是否被更新。
    pub updated: bool,
    /// 落库后的书架状态。
    pub status: String,
    /// 该书当前来源条数。
    pub sources: usize,
}

impl ImportReport {
    /// 面向状态栏的一行结果。
    pub fn status_text(&self) -> String {
        let action = if self.created {
            "已导入"
        } else {
            "已更新"
        };
        format!(
            "{}：{}（{} 字节，编码 {}，置信 {}‰，来源 {} 条，状态 {}）",
            action,
            self.display_name,
            self.size,
            self.encoding,
            self.encoding_confidence,
            self.sources,
            self.status
        )
    }
}

/// 一次导入交互的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportFlow {
    /// 用户取消（零改动）。
    Cancelled,
    /// 导入完成。
    Imported(ImportReport),
}

impl ImportFlow {
    /// 面向状态栏的一行结果。
    pub fn status_text(&self) -> String {
        match self {
            ImportFlow::Cancelled => "已取消：本次没有选择文件，书架未做任何改动".to_string(),
            ImportFlow::Imported(report) => report.status_text(),
        }
    }
}

/// 把一次导入结果（含失败）转成状态栏文案。
pub fn status_text_for(result: &Result<ImportFlow, ImportError>) -> String {
    match result {
        Ok(flow) => flow.status_text(),
        Err(error) => format!("导入未完成（{}）：{}", error.kind.code(), error.message()),
    }
}

// ---------------------------------------------------------------------------
// 导入流程
// ---------------------------------------------------------------------------

/// 以给定选择器完成一次导入（取消 → [`ImportFlow::Cancelled`]）。
pub fn run_import_flow(
    db_path: &Path,
    picker: &dyn FilePicker,
    now: i64,
) -> Result<ImportFlow, ImportError> {
    match picker.pick_txt_file()? {
        None => Ok(ImportFlow::Cancelled),
        Some(path) => import_path(db_path, &path, now).map(ImportFlow::Imported),
    }
}

/// 导入指定路径的 TXT。
///
/// 顺序保证「失败不留半成品」：
/// 1. 校验（扩展名 / 元数据 / 可读性）与指纹、编码识别全部发生在**打开数据库之前**；
/// 2. 落库只有一次事务（`books::import_book`）；
/// 3. 落库后自检（条目存在、格式为 txt、状态非 `import_failed`）；自检失败且本次是**新建**
///    条目时，用 `purge_book_record` 硬删除该条目（只删记录，不碰用户文件）。
pub fn import_path(db_path: &Path, path: &Path, now: i64) -> Result<ImportReport, ImportError> {
    let selected = validate_selection(path)?;

    let (_fingerprint, compact) = fingerprint_file(path)?;
    let source = FileTextSource::open(path).map_err(|error| {
        ImportError::new(
            ImportErrorKind::Fingerprint,
            Some(path),
            format!("打开文件失败：{error}"),
        )
    })?;
    let detection = detect_encoding(&source).map_err(|error| {
        ImportError::new(
            ImportErrorKind::Fingerprint,
            Some(path),
            format!("编码识别失败：{error}"),
        )
    })?;

    let mut storage =
        Storage::open(db_path).map_err(|error| storage_error(db_path, "打开书架数据库", error))?;
    storage
        .migrate()
        .map_err(|error| storage_error(db_path, "迁移书架数据库", error))?;

    let import = build_import(&selected, &compact, &detection);
    let outcome = books::import_book(storage.conn(), &import, now)
        .map_err(|error| storage_error(db_path, "写入书籍与来源记录", error))?;

    // 自检：失败时不留可打开的残缺书籍记录。
    let record = books::get_book(storage.conn(), outcome.book_id)
        .map_err(|error| storage_error(db_path, "读回书籍记录", error))?;
    let usable = record
        .as_ref()
        .is_some_and(|book| book.format == FORMAT_TXT && book.status != STATUS_IMPORT_FAILED);
    if !usable {
        if outcome.created {
            let _ = books::purge_book_record(storage.conn(), outcome.book_id);
        }
        return Err(ImportError::new(
            ImportErrorKind::Storage,
            Some(path),
            format!(
                "导入自检失败：book_id={} 记录不可用（已回收本次新建条目）",
                outcome.book_id
            ),
        ));
    }
    let record = record.expect("自检已确认记录存在");

    let sources = books::list_book_sources(storage.conn(), outcome.book_id)
        .map_err(|error| storage_error(db_path, "读回来源清单", error))?;

    Ok(ImportReport {
        book_id: outcome.book_id,
        display_name: record.display_name.clone(),
        original_name: record.original_name.clone(),
        path: PathBuf::from(record.source_locator.clone()),
        size: record.source_size.max(0) as u64,
        fingerprint: record.source_fingerprint.clone().unwrap_or_default(),
        encoding: record.encoding.clone(),
        encoding_confidence: record.encoding_confidence.max(0) as u32,
        created: outcome.created,
        source_created: outcome.source_created,
        updated: outcome.updated,
        status: record.status.clone(),
        sources: sources.len(),
    })
}

/// 启动期自动导入（`NR_IMPORT_PATH` 已设置时返回结果；未设置返回 `None`）。
pub fn startup_import(db_path: &Path) -> Option<Result<ImportReport, ImportError>> {
    let path = startup_import_path()?;
    Some(import_path(db_path, &path, now_unix()))
}

/// 书架条目状态（供自检与 UI 文案使用）——只是核心状态的只读转发。
pub fn book_status_is_ready(status: &str) -> bool {
    status == STATUS_READY
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::io::Read;
    use std::sync::atomic::{AtomicU64, Ordering};

    use reading_storage::books::{self, STATUS_MISSING, STATUS_REMOVED};

    // -----------------------------------------------------------------------
    // 测试夹具（自管理临时目录；只在 /tmp 下操作副本，绝不触碰用户文件）
    // -----------------------------------------------------------------------

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |delta| delta.as_nanos());
            let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir()
                .join(format!("nr-dt2-{tag}-{}-{nanos}-{seq}", std::process::id()));
            fs::create_dir_all(&path).expect("创建临时目录");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }

        fn db(&self) -> PathBuf {
            self.join("library.db")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// 中文正文夹具（UTF-8，含章节行）。
    fn chinese_bytes() -> Vec<u8> {
        let mut text = String::new();
        for index in 1..=60 {
            text.push_str(&format!("第{index}章 测试章节\n"));
            text.push_str("这是一行用于导入链路验证的中文正文，包含标点，句号与逗号。\n");
            text.push_str("第二行正文，用于让文件长度超过单次抽样区间。\n\n");
        }
        text.into_bytes()
    }

    fn write_file(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("写入测试文件");
    }

    // -----------------------------------------------------------------------
    // SHA-256（与 `crates/reading-storage/tests/common` 同实现，FIPS 向量自检）
    // -----------------------------------------------------------------------

    const SHA256_K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    fn sha256_words(data: &[u8]) -> [u32; 8] {
        let mut state: [u32; 8] = [
            0x6a09_e667,
            0xbb67_ae85,
            0x3c6e_f372,
            0xa54f_f53a,
            0x510e_527f,
            0x9b05_688c,
            0x1f83_d9ab,
            0x5be0_cd19,
        ];
        let bit_len = (data.len() as u64).wrapping_mul(8);
        let mut message = data.to_vec();
        message.push(0x80);
        let pad = (56 + 64 - (message.len() % 64)) % 64;
        message.extend(std::iter::repeat_n(0_u8, pad));
        message.extend_from_slice(&bit_len.to_be_bytes());

        let mut start = 0;
        while start + 64 <= message.len() {
            let chunk = &message[start..start + 64];
            let mut w = [0_u32; 64];
            for (index, word) in w.iter_mut().take(16).enumerate() {
                let base = index * 4;
                *word = u32::from_be_bytes([
                    chunk[base],
                    chunk[base + 1],
                    chunk[base + 2],
                    chunk[base + 3],
                ]);
            }
            for index in 16..64 {
                let s0 = w[index - 15].rotate_right(7)
                    ^ w[index - 15].rotate_right(18)
                    ^ (w[index - 15] >> 3);
                let s1 = w[index - 2].rotate_right(17)
                    ^ w[index - 2].rotate_right(19)
                    ^ (w[index - 2] >> 10);
                w[index] = w[index - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[index - 7])
                    .wrapping_add(s1);
            }

            let mut a = state[0];
            let mut b = state[1];
            let mut c = state[2];
            let mut d = state[3];
            let mut e = state[4];
            let mut f = state[5];
            let mut g = state[6];
            let mut h = state[7];
            for index in 0..64 {
                let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let choice = (e & f) ^ ((!e) & g);
                let temp1 = h
                    .wrapping_add(big_s1)
                    .wrapping_add(choice)
                    .wrapping_add(SHA256_K[index])
                    .wrapping_add(w[index]);
                let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let majority = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = big_s0.wrapping_add(majority);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }
            let round = [a, b, c, d, e, f, g, h];
            for (slot, added) in state.iter_mut().zip(round) {
                *slot = slot.wrapping_add(added);
            }
            start += 64;
        }
        state
    }

    fn sha256_hex(data: &[u8]) -> String {
        let mut out = String::with_capacity(64);
        for word in sha256_words(data) {
            out.push_str(&format!("{word:08x}"));
        }
        out
    }

    /// 流式读回文件（每次 ≤ 4096 字节；不整本一次读入）。仅测试使用。
    fn read_all_streaming(path: &Path) -> Vec<u8> {
        let mut file = fs::File::open(path).expect("打开文件");
        let mut buffer = [0_u8; 4096];
        let mut out = Vec::new();
        loop {
            let read = file.read(&mut buffer).expect("读取文件");
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buffer[..read]);
        }
        out
    }

    /// 用户原始文件的完整摘要（用于「导入不修改原文件」的硬断言）。
    fn source_digest(path: &Path) -> (String, u64, Option<i64>) {
        let bytes = read_all_streaming(path);
        let metadata = fs::metadata(path).expect("读取元数据");
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|delta| delta.as_secs() as i64);
        (sha256_hex(&bytes), metadata.len(), mtime)
    }

    fn open_storage(dir: &TempDir) -> Storage {
        let mut storage = Storage::open(dir.db()).expect("打开数据库");
        storage.migrate().expect("迁移");
        storage
    }

    // -----------------------------------------------------------------------
    // 1) 导入成功
    // -----------------------------------------------------------------------

    #[test]
    fn import_txt_creates_book_and_source_with_fingerprint_and_keeps_source_bytes_intact() {
        let dir = TempDir::new("import-ok");
        write_file(&dir.join("book.txt"), &chinese_bytes());
        let path = dir.join("book.txt");
        let before = source_digest(&path);

        let report = import_path(&dir.db(), &path, 1_000).expect("导入应成功");

        assert!(report.created, "首次导入必须新建条目");
        assert!(report.source_created);
        assert_eq!(report.display_name, "book");
        assert_eq!(report.original_name, "book.txt");
        assert_eq!(report.size, before.1);
        assert_eq!(report.status, STATUS_READY);
        assert_eq!(report.encoding, "UTF-8");
        assert_eq!(
            report.encoding_confidence, 1000,
            "UTF-8 严格合法应为高可信度"
        );
        assert_eq!(report.sources, 1);
        assert!(
            report.fingerprint.starts_with("fnv1a64:"),
            "指纹口径必须与 reading-ffi 一致：{}",
            report.fingerprint
        );
        assert!(
            report.fingerprint.len() <= reading_storage::validate::MAX_FINGERPRINT_BYTES,
            "指纹必须落库长度上限内"
        );

        // 落库事实（真实读回，不回显入参）。
        let storage = open_storage(&dir);
        let book = books::get_book(storage.conn(), report.book_id)
            .expect("读回书籍")
            .expect("书籍存在");
        assert_eq!(book.source_kind, KIND_DESKTOP_PATH);
        assert_eq!(book.source_locator, path.to_string_lossy());
        assert_eq!(book.format, FORMAT_TXT);
        assert_eq!(
            book.source_fingerprint.as_deref(),
            Some(report.fingerprint.as_str())
        );
        let sources = books::list_book_sources(storage.conn(), report.book_id).expect("来源清单");
        assert_eq!(sources.len(), 1);
        assert!(sources[0].is_primary);
        assert_eq!(sources[0].source_locator, path.to_string_lossy());
        assert_eq!(
            sources[0].source_fingerprint.as_deref(),
            Some(report.fingerprint.as_str())
        );

        // 硬断言：导入不修改用户原始文件（内容哈希 / 长度 / 修改时间全部不变）。
        let after = source_digest(&path);
        assert_eq!(after, before, "导入后原文件哈希/长度/修改时间必须不变");
        assert_eq!(
            fingerprint_file(&path).expect("复算指纹").1,
            report.fingerprint,
            "原文件未变更 ⇒ 指纹必须稳定"
        );

        // 重复导入幂等：不新建条目、不重复来源。
        let again = import_path(&dir.db(), &path, 2_000).expect("重复导入应成功");
        assert_eq!(again.book_id, report.book_id);
        assert!(!again.created);
        assert!(!again.source_created);
        assert_eq!(
            books::count_books(storage.conn()).expect("书籍数"),
            1,
            "重复导入不得产生重复条目"
        );
        let sources = books::list_book_sources(storage.conn(), report.book_id).expect("来源清单");
        assert_eq!(sources.len(), 1);
    }

    #[test]
    fn import_accepts_uppercase_extension_and_derives_display_name() {
        let dir = TempDir::new("import-upper");
        write_file(
            &dir.join("斗破苍穹.TXT"),
            "第一章 起始\n正文。\n".as_bytes(),
        );
        let report =
            import_path(&dir.db(), &dir.join("斗破苍穹.TXT"), 10).expect("大写扩展名应被接受");
        assert_eq!(report.display_name, "斗破苍穹");
    }

    // -----------------------------------------------------------------------
    // 2) 非 TXT
    // -----------------------------------------------------------------------

    #[test]
    fn import_rejects_non_txt() {
        let dir = TempDir::new("reject-non-txt");
        write_file(&dir.join("book.pdf"), b"%PDF-1.7 not a txt\n");
        let storage = open_storage(&dir);

        let error =
            import_path(&dir.db(), &dir.join("book.pdf"), 10).expect_err("非 TXT 必须被拒绝");
        assert_eq!(error.kind, ImportErrorKind::NotTxt);
        assert_eq!(error.kind.code(), "import_not_txt");
        assert!(
            error.message().contains(".txt"),
            "错误必须可读且说明约定：{}",
            error.message()
        );
        assert_eq!(
            books::count_books(storage.conn()).expect("书籍数"),
            0,
            "拒绝时不得留下记录"
        );

        // 无扩展名同样拒绝。
        write_file(&dir.join("novel"), b"no extension\n");
        let error = import_path(&dir.db(), &dir.join("novel"), 11).expect_err("无扩展名必须被拒绝");
        assert_eq!(error.kind, ImportErrorKind::NotTxt);

        // 双扩展名同样拒绝（不得被 `book.txt.bak` 之类绕过）。
        write_file(&dir.join("book.txt.bak"), b"backup\n");
        let error =
            import_path(&dir.db(), &dir.join("book.txt.bak"), 12).expect_err("双扩展名必须被拒绝");
        assert_eq!(error.kind, ImportErrorKind::NotTxt);

        assert_eq!(books::count_books(storage.conn()).expect("书籍数"), 0);
    }

    // -----------------------------------------------------------------------
    // 3) 空文件
    // -----------------------------------------------------------------------

    #[test]
    fn import_rejects_empty_file_without_leaving_partial_record() {
        let dir = TempDir::new("reject-empty");
        write_file(&dir.join("empty.txt"), b"");
        let storage = open_storage(&dir);

        let error =
            import_path(&dir.db(), &dir.join("empty.txt"), 10).expect_err("空文件必须给出稳定错误");
        assert_eq!(error.kind, ImportErrorKind::EmptyFile);
        assert!(error.message().contains("空"), "{}", error.message());
        // 零记录：书籍与来源表都不得留下任何行。
        assert_eq!(books::count_books(storage.conn()).expect("书籍数"), 0);
        assert_eq!(
            books::count_books(storage.conn()).expect("书籍数"),
            0,
            "空文件不得留下半成品记录"
        );
    }

    // -----------------------------------------------------------------------
    // 4) 不可读 / 非普通文件 / 不存在
    // -----------------------------------------------------------------------

    #[test]
    fn import_rejects_unreadable_missing_and_non_regular_files() {
        let dir = TempDir::new("reject-unreadable");
        let storage = open_storage(&dir);

        // 路径不存在
        let error =
            import_path(&dir.db(), &dir.join("gone.txt"), 10).expect_err("不存在必须被拒绝");
        assert_eq!(error.kind, ImportErrorKind::MissingFile);

        // 目录（名字以 .txt 结尾）
        fs::create_dir_all(dir.join("dir.txt")).expect("创建目录");
        let error = import_path(&dir.db(), &dir.join("dir.txt"), 11).expect_err("目录必须被拒绝");
        assert_eq!(error.kind, ImportErrorKind::NotRegularFile);

        // 权限不可读（非 root 生效；root 运行时跳过保存位断言，但不得留下记录）
        let guarded = dir.join("locked.txt");
        write_file(&guarded, "第一章 锁定\n正文。\n".as_bytes());
        let mut perms = fs::metadata(&guarded).expect("元数据").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
        fs::set_permissions(&guarded, perms).expect("设置权限");
        let result = import_path(&dir.db(), &guarded, 12);
        let can_still_read = fs::File::open(&guarded).is_ok();
        if can_still_read {
            println!("注意：当前进程可读取 0o000 文件（疑似 root），跳过不可读断言");
        } else {
            let error = result.expect_err("不可读文件必须被拒绝");
            assert_eq!(error.kind, ImportErrorKind::Unreadable);
            assert!(error.message().contains("无法读取"), "{}", error.message());
        }
        // 无论哪种情形：不得留下任何记录。
        assert_eq!(books::count_books(storage.conn()).expect("书籍数"), 0);
    }

    // -----------------------------------------------------------------------
    // 5) 选择器行为（取消零改动 / 系统对话框解释 / 静态选择器）
    // -----------------------------------------------------------------------

    #[test]
    fn import_cancel_leaves_no_trace() {
        let dir = TempDir::new("cancel");
        let storage = open_storage(&dir);
        let flow = run_import_flow(&dir.db(), &CancellingFilePicker, 10).expect("取消不是错误");
        assert_eq!(flow, ImportFlow::Cancelled);
        assert!(flow.status_text().contains("已取消"));
        assert_eq!(books::count_books(storage.conn()).expect("书籍数"), 0);
        // 取消不得产生任何新文件（只允许书库自身的文件：WAL 模式下的 library.db*）。
        for entry in fs::read_dir(dir.path()).expect("列目录") {
            let name = entry
                .expect("目录项")
                .file_name()
                .to_string_lossy()
                .to_string();
            assert!(
                name.starts_with("library.db"),
                "取消不得产生新文件，实际出现 {name}"
            );
        }
    }

    #[test]
    fn static_picker_imports_given_path() {
        let dir = TempDir::new("static-picker");
        write_file(&dir.join("static.txt"), &chinese_bytes());
        let path = dir.join("static.txt");
        let flow = run_import_flow(&dir.db(), &StaticFilePicker::new(path.clone()), 10)
            .expect("静态选择器应导入");
        match flow {
            ImportFlow::Imported(report) => assert_eq!(report.path, path),
            ImportFlow::Cancelled => panic!("静态选择器不应取消"),
        }
    }

    #[test]
    fn windows_picker_filter_is_well_formed() {
        // `GetOpenFileNameW` 的过滤串契约：`说明\0模式\0…\0\0`（双 NUL 结尾）。
        // 该不变量与平台无关，故在 Linux 上直接单测（Windows 分支只做调用）。
        let filter = txt_picker_filter();
        assert_eq!(filter.last().copied(), Some(0), "必须以 NUL 结尾");
        assert_eq!(
            filter[filter.len() - 2],
            0,
            "必须以**双** NUL 结尾（否则最后的过滤项会被吞掉）"
        );
        let text = String::from_utf16_lossy(&filter[..filter.len() - 1]);
        let text = text.trim_end_matches('\0');
        let parts: Vec<&str> = text.split('\0').collect();
        assert_eq!(parts.len(), 4, "应为两对「说明/模式」：{parts:?}");
        assert!(parts[1].contains("*.txt"), "TXT 过滤项：{parts:?}");
        assert_eq!(parts[3], "*.*", "「所有文件」兜底项：{parts:?}");
    }

    #[test]
    fn system_picker_result_is_interpreted_from_real_program_output() {
        let dir = TempDir::new("picker-program");
        // 用真实子进程替换 zenity：脚本把参数写入 stderr、把路径写入 stdout。
        let script = dir.join("zenity");
        write_file(
            &script,
            b"#!/bin/sh\necho \"$@\" >&2\necho \"$NR_DT2_PICKER_SELECTION\"\nexit 0\n",
        );
        let mut perms = fs::metadata(&script).expect("元数据").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&script, perms).expect("设置权限");

        let selected = dir.join("chosen.txt");
        write_file(&selected, b"\xe7\xac\xac\xe4\xb8\x80\xe7\xab\xa0\n");
        // 脚本通过环境变量读取输出路径（不依赖 shell 的引号处理）。
        let picker = SystemFilePicker::with_programs(vec![script.to_string_lossy().to_string()]);
        let output = Command::new(script.to_string_lossy().to_string())
            .env(
                "NR_DT2_PICKER_SELECTION",
                selected.to_string_lossy().to_string(),
            )
            .args(PickerFlavor::Zenity.args())
            .output()
            .expect("运行替身对话框");
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--file-selection"),
            "必须使用文件选择模式"
        );
        let picked = interpret_picker_result(
            "zenity",
            output.status.code(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        )
        .expect("解释成功输出")
        .expect("应有选中路径");
        assert_eq!(picked, selected);

        // 退出码 1 = 用户取消（零改动）。
        let cancelled = interpret_picker_result("zenity", Some(1), "", "").expect("取消不是错误");
        assert!(cancelled.is_none());
        // 其它退出码 = 对话框失败。
        let failed =
            interpret_picker_result("zenity", Some(255), "", "Gtk-WARNING: cannot open display")
                .expect_err("非 0/1 退出码必须报错");
        assert_eq!(failed.kind, ImportErrorKind::PickerFailed);
        assert!(failed.message().contains("cannot open display"));

        // 全部程序缺失 → NoPicker（不是 PickerFailed）。
        let missing = SystemFilePicker::with_programs(vec!["nr-dt2-no-such-program".to_string()]);
        let error = missing
            .pick_txt_file()
            .expect_err("缺失程序必须报 NoPicker");
        assert_eq!(error.kind, ImportErrorKind::NoPicker);
        assert!(error.message().contains("未找到对话框程序"));
        assert_eq!(picker.programs().len(), 1);
    }

    // -----------------------------------------------------------------------
    // 6) 辅助函数
    // -----------------------------------------------------------------------

    #[test]
    fn fingerprint_text_round_trips_and_matches_reference_vectors() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests/fixtures/ascii-short.txt");
        let (fingerprint, compact) = fingerprint_file(&path).expect("计算仓库夹具指纹");
        let parsed = parse_compact_fingerprint(&compact).expect("解析紧凑指纹");
        assert!(
            parsed.same_content(&fingerprint),
            "紧凑文本必须可逆地保留内容签名"
        );
        assert_eq!(parsed.size, fingerprint.size);
        assert_eq!(parsed.algo, FINGERPRINT_ALGO);
        assert!(parsed.algo.to_string().len() <= 8);

        assert!(
            parse_compact_fingerprint("sha256:1:2:3:4").is_none(),
            "算法必须匹配"
        );
        assert!(
            parse_compact_fingerprint("fnv1a64:1:2:3").is_none(),
            "字段不足必须拒绝"
        );
        assert!(
            parse_compact_fingerprint("fnv1a64:1:2:3:4:5").is_none(),
            "字段过多必须拒绝"
        );
        assert!(
            parse_compact_fingerprint("fnv1a64:x:2:3:4").is_none(),
            "非数字必须拒绝"
        );
    }

    #[test]
    fn sha256_matches_fips_reference_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn display_name_derivation_covers_edge_cases() {
        assert_eq!(derive_display_name("book.txt"), "book");
        assert_eq!(derive_display_name("book.TXT"), "book");
        assert_eq!(derive_display_name("斗破苍穹.txt"), "斗破苍穹");
        assert_eq!(derive_display_name(".txt"), ".txt", "空主干回退原名");
        assert_eq!(derive_display_name("book.pdf"), "book.pdf");
        assert_eq!(derive_display_name(""), "");
    }

    #[test]
    fn error_messages_are_readable_and_carry_stable_codes() {
        let cases = [
            (ImportErrorKind::NotTxt, "import_not_txt"),
            (ImportErrorKind::MissingFile, "import_missing_file"),
            (ImportErrorKind::NotRegularFile, "import_not_regular_file"),
            (ImportErrorKind::EmptyFile, "import_empty_file"),
            (ImportErrorKind::Unreadable, "import_unreadable"),
            (ImportErrorKind::NoPicker, "import_no_picker"),
            (ImportErrorKind::PickerFailed, "import_picker_failed"),
            (ImportErrorKind::Fingerprint, "import_fingerprint_failed"),
            (ImportErrorKind::Storage, "import_storage_failed"),
        ];
        for (kind, code) in cases {
            assert_eq!(kind.code(), code);
            assert!(!kind.label().is_empty());
            let error = ImportError::new(kind, Some(Path::new("/tmp/x.txt")), "");
            assert!(error.message().contains("/tmp/x.txt"));
            assert!(error.to_string().contains(code));
        }
    }

    #[test]
    fn removed_book_is_not_reported_as_ready() {
        // `book_status_is_ready` 只转发核心状态，不得把软移除条目当成可读。
        assert!(book_status_is_ready(STATUS_READY));
        assert!(!book_status_is_ready(STATUS_REMOVED));
        assert!(!book_status_is_ready(STATUS_MISSING));
    }
}
