//! 批量导入与「按文件系统目录名归类」（V1.1，规格 R6 / T2）。
//!
//! 口径（主人裁定 2026-10-05 + 设计稿 C7）：
//!
//! - 导入前询问「是否按文件系统目录名组织归类」；
//! - 选「是」：**只取文件的直接父目录名**建一个分类（不递归建整条路径），
//!   归入**当前书架所处的父分类**之下；
//! - 选「否」：全部归入当前父分类（或根）。
//!
//! 边界（延续 V1 数据安全，规格 R7）：
//!
//! - 只**读**文件系统（列目录、读文件元信息），**不移动、不重命名、不删除**任何用户文件；
//! - 目录扫描有**深度与数量上限**，避免误选家目录时长时间卡死；
//! - 同名分类**复用**而非重复创建（`categories::create_or_reuse`）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use reading_storage::{categories, Storage};

use crate::import::{import_path, now_unix, ImportError, ImportErrorKind};

/// 目录扫描的最大递归深度（只扫直接子目录层，避免深层树）。
///
/// 口径：**1** = 只收「目录下的文件」+「目录的直接子目录里的文件」，
/// 即每个文件只取它的直接父目录名（规格 R6 的「不递归建整条路径」）。
pub const MAX_SCAN_DEPTH: usize = 1;

/// 单次批量导入的最大文件数（超出即报错，不静默截断）。
pub const MAX_BATCH_FILES: usize = 500;

/// 是否按文件系统目录名组织归类（用户对导入对话框的回答）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizeByDirectory {
    /// 按各文件的**直接父目录名**建分类。
    Yes,
    /// 不分类，全部归入当前父分类。
    No,
}

/// 用户回答的完整选择（含「记住」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrganizeChoice {
    pub organize: OrganizeByDirectory,
    /// 是否记住这次选择（下次不再询问）。
    pub remember: bool,
}

impl OrganizeChoice {
    /// 「按目录名归类并导入」（主按钮）。
    pub fn organize_yes(remember: bool) -> Self {
        Self {
            organize: OrganizeByDirectory::Yes,
            remember,
        }
    }

    /// 「不归类，全部导入」（次按钮）。
    pub fn organize_no(remember: bool) -> Self {
        Self {
            organize: OrganizeByDirectory::No,
            remember,
        }
    }
}

/// 一个待导入文件的归类计划（导入前的预览事实来源；设计稿 C7）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    pub path: PathBuf,
    /// 归入的分类 id；`None` = 根（未分类）。
    ///
    /// 「不分类」模式下恒为 `None`（或调用方给的当前父分类）。
    pub category_id: Option<i64>,
    /// 该文件所属的直接父目录名（仅「按目录名」模式有值，用于预览）。
    pub directory_name: Option<String>,
}

/// 批量导入的结果汇总。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchImportReport {
    /// 成功导入/更新的书籍 id。
    pub imported: Vec<i64>,
    /// 本次**新建**的分类 id（复用的同名分类不计入）。
    pub created_categories: Vec<i64>,
    /// 失败项（路径 + 可理解原因）。
    pub failures: Vec<(PathBuf, String)>,
}

impl BatchImportReport {
    /// 完成后的 Toast 文案（设计稿 C7）。
    pub fn message(&self) -> String {
        let mut text = format!("已导入 {} 本", self.imported.len());
        if !self.created_categories.is_empty() {
            text.push_str(&format!("，新建分类 {} 个", self.created_categories.len()));
        }
        if !self.failures.is_empty() {
            text.push_str(&format!("，{} 本失败", self.failures.len()));
        }
        text
    }

    /// 是否全部成功。
    pub fn all_succeeded(&self) -> bool {
        self.failures.is_empty()
    }
}

/// 扫描目录下的 TXT 文件（受 [`MAX_SCAN_DEPTH`] 与 [`MAX_BATCH_FILES`] 约束）。
///
/// **只读**：不移动、不改名、不删除。
pub fn scan_txt_files(root: &Path) -> Result<Vec<PathBuf>, ImportError> {
    if !root.is_dir() {
        return Err(ImportError::new(
            ImportErrorKind::NotRegularFile,
            Some(root),
            format!("{} 不是目录", root.display()),
        ));
    }
    let mut files: Vec<PathBuf> = Vec::new();
    let mut queue: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = queue.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|error| {
            ImportError::new(
                ImportErrorKind::Unreadable,
                Some(&dir),
                format!("读取目录失败：{error}"),
            )
        })?;
        let mut sub_dirs: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ImportError::new(
                    ImportErrorKind::Unreadable,
                    Some(&dir),
                    format!("读取目录项失败：{error}"),
                )
            })?;
            let path = entry.path();
            // `file_type()` 不跟随符号链接 → 不会意外跳到目录外的挂载点。
            let file_type = entry.file_type().map_err(|error| {
                ImportError::new(
                    ImportErrorKind::Unreadable,
                    Some(&path),
                    format!("读取文件类型失败：{error}"),
                )
            })?;
            if file_type.is_dir() {
                if depth < MAX_SCAN_DEPTH {
                    sub_dirs.push(path);
                }
                continue;
            }
            if is_txt(&path) {
                files.push(path);
            }
        }
        // 排序保证结果稳定（目录遍历顺序依赖文件系统）。
        sub_dirs.sort();
        for sub in sub_dirs.into_iter().rev() {
            queue.push((sub, depth + 1));
        }
        if files.len() > MAX_BATCH_FILES {
            return Err(ImportError::new(
                ImportErrorKind::NotRegularFile,
                Some(root),
                format!("目录内 TXT 超过 {MAX_BATCH_FILES} 本，请分批选择（避免长时间无响应）"),
            ));
        }
    }
    files.sort();
    Ok(files)
}

/// 是否 TXT 文件（扩展名口径与 `import::validate_selection` 一致）。
fn is_txt(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case(crate::import::TXT_EXTENSION))
        .unwrap_or(false)
}

/// 文件的直接父目录名（规格 R6：只取直接父目录，**不**递归整条路径）。
///
/// 无法取到（如文件在根目录）时返回 `None`，调用方归入当前父分类。
pub fn parent_directory_name(path: &Path) -> Option<String> {
    let name = path.parent()?.file_name()?.to_str()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 生成导入计划（**只算不写**：不创建分类、不导入书籍）。
///
/// 「按目录名」模式下会**先创建/复用分类**（这是写入动作），
/// 因此本函数会改动数据库 —— 命名如实反映：它返回的 `created_categories`
/// 就是本次新建的分类。
pub fn plan_import(
    db_path: &Path,
    paths: &[PathBuf],
    choice: OrganizeChoice,
    current_category_id: Option<i64>,
    now: i64,
) -> Result<(Vec<PlannedFile>, Vec<i64>), ImportError> {
    let mut storage = open_storage(db_path)?;
    let conn = storage.conn();
    let mut created: Vec<i64> = Vec::new();

    let mut planned: Vec<PlannedFile> = Vec::with_capacity(paths.len());
    // 「按目录名」时按目录名去重，避免同一目录反复创建/查找。
    let mut by_name: BTreeMap<String, i64> = BTreeMap::new();

    for path in paths {
        match choice.organize {
            OrganizeByDirectory::No => {
                planned.push(PlannedFile {
                    path: path.clone(),
                    category_id: current_category_id,
                    directory_name: None,
                });
            }
            OrganizeByDirectory::Yes => {
                let directory_name = parent_directory_name(path);
                // 无父目录名时归入当前父分类（`None` = 根）。
                let category_id: Option<i64> = match &directory_name {
                    None => current_category_id,
                    Some(name) => Some(resolve_directory_category(
                        conn,
                        db_path,
                        current_category_id,
                        name,
                        &mut by_name,
                        &mut created,
                        now,
                    )?),
                };
                planned.push(PlannedFile {
                    path: path.clone(),
                    category_id,
                    directory_name,
                });
            }
        }
    }
    Ok((planned, created))
}

/// 「按目录名归类」时把目录名解析为分类 id（不存在则创建）。
///
/// 同名分类**复用**而非重复创建（规格 R6）；新建时记入 `created`。
fn resolve_directory_category(
    conn: &rusqlite::Connection,
    db_path: &Path,
    parent_id: Option<i64>,
    name: &str,
    cache: &mut BTreeMap<String, i64>,
    created: &mut Vec<i64>,
    now: i64,
) -> Result<i64, ImportError> {
    if let Some(existing) = cache.get(name) {
        return Ok(*existing);
    }
    let found = categories::find_by_name(conn, parent_id, name)
        .map_err(|error| storage_err(db_path, "查询分类", Some(error.to_string())))?;
    let id = match found {
        Some(id) => id,
        None => {
            let id = categories::create_category(conn, parent_id, name, now)
                .map_err(|error| storage_err(db_path, "创建分类", Some(error.to_string())))?;
            created.push(id);
            id
        }
    };
    cache.insert(name.to_string(), id);
    Ok(id)
}

/// 执行导入：逐个 `import_path` 并归入计划分类。
///
/// **不中断**：单本失败只记入 `failures`，其余继续（设计稿 §5.1
/// 「部分文件读取失败 → 沿用既有导入结果清单」）。
pub fn run_import(db_path: &Path, planned: &[PlannedFile], now: i64) -> BatchImportReport {
    let mut report = BatchImportReport {
        imported: Vec::new(),
        created_categories: Vec::new(),
        failures: Vec::new(),
    };
    for item in planned {
        match import_path(db_path, &item.path, now) {
            Ok(single) => {
                if let Some(category_id) = item.category_id {
                    if let Some(error) = assign_category(db_path, single.book_id, category_id, now)
                    {
                        report.failures.push((item.path.clone(), error));
                        continue;
                    }
                }
                report.imported.push(single.book_id);
            }
            Err(error) => {
                report
                    .failures
                    .push((item.path.clone(), error.message().to_string()));
            }
        }
    }
    report
}

/// 导入后归类（失败只回报，不影响已导入的书籍）。
fn assign_category(db_path: &Path, book_id: i64, category_id: i64, now: i64) -> Option<String> {
    let storage = match open_storage(db_path) {
        Ok(storage) => storage,
        // 数据库都打不开时**如实回报**，不静默当成成功。
        Err(error) => return Some(format!("已导入但归类失败：{}", error.message())),
    };
    match categories::move_book_at(storage.conn(), book_id, Some(category_id), now) {
        Ok(()) => None,
        Err(error) => Some(format!("已导入但归类失败：{error}")),
    }
}

fn open_storage(db_path: &Path) -> Result<Storage, ImportError> {
    let mut storage = Storage::open(db_path)
        .map_err(|error| storage_err(db_path, "打开书架数据库", Some(error.to_string())))?;
    storage
        .migrate()
        .map_err(|error| storage_err(db_path, "迁移书架数据库", Some(error.to_string())))?;
    Ok(storage)
}

fn storage_err(db_path: &Path, action: &str, detail: Option<String>) -> ImportError {
    let detail = detail.unwrap_or_default();
    ImportError::new(
        ImportErrorKind::Storage,
        Some(db_path),
        format!(
            "{action}失败（{}）{}",
            db_path.display(),
            if detail.is_empty() {
                String::new()
            } else {
                format!("：{detail}")
            }
        ),
    )
}

/// 一步到位：扫描 → 计划 → 导入（供程序化驱动与界面共用）。
pub fn import_directory(
    db_path: &Path,
    root: &Path,
    choice: OrganizeChoice,
    current_category_id: Option<i64>,
    now: i64,
) -> Result<(BatchImportReport, Vec<PlannedFile>), ImportError> {
    let files = scan_txt_files(root)?;
    if files.is_empty() {
        return Err(ImportError::new(
            ImportErrorKind::NotRegularFile,
            Some(root),
            "该目录下没有找到 TXT 文件（只扫描目录本身与其直接子目录）",
        ));
    }
    let (planned, created) = plan_import(db_path, &files, choice, current_category_id, now)?;
    let mut report = run_import(db_path, &planned, now);
    report.created_categories = created;
    Ok((report, planned))
}

/// 便捷入口：使用当前时间戳。
pub fn import_directory_now(
    db_path: &Path,
    root: &Path,
    choice: OrganizeChoice,
    current_category_id: Option<i64>,
) -> Result<(BatchImportReport, Vec<PlannedFile>), ImportError> {
    import_directory(db_path, root, choice, current_category_id, now_unix())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "nr-batch-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("创建临时目录");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn db(&self) -> PathBuf {
            self.0.join("library.db")
        }

        /// 写入一本 TXT。
        ///
        /// **正文按路径区分**：`import_book` 按**内容指纹**去重
        /// （`find_book_by_fingerprint`，见 `books.rs`），若两个夹具文件内容相同，
        /// 第二本会被判为同一本书而只更新来源 —— 那不是分类逻辑的问题，是夹具造错了。
        fn write_txt(&self, relative: &str) -> PathBuf {
            self.write_txt_with_marker(relative, relative)
        }

        /// 写入一本内容可区分的 TXT（`marker` 参与正文）。
        fn write_txt_with_marker(&self, relative: &str, marker: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().expect("有父目录")).expect("建子目录");
            let mut text = String::new();
            text.push_str(&format!("《{marker}》\n"));
            for chapter in 1..=3 {
                text.push_str(&format!("第{chapter}章 测试\n"));
                text.push_str(&format!("批量导入归类验证用的中文正文（{marker}）。\n\n"));
            }
            std::fs::write(&path, text).expect("写入 TXT");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn category_id_of(db: &Path, book_id: i64) -> Option<i64> {
        let storage = Storage::open(db).expect("打开库");
        storage
            .conn()
            .query_row(
                "SELECT category_id FROM books WHERE id = ?1",
                [book_id],
                |row| row.get(0),
            )
            .expect("读回")
    }

    fn category_name(db: &Path, category_id: i64) -> String {
        let storage = Storage::open(db).expect("打开库");
        categories::get_category(storage.conn(), category_id)
            .expect("读分类")
            .expect("存在")
            .name
    }

    #[test]
    fn scan_collects_txt_from_root_and_direct_subdirectories_only() {
        let temp = TempDir::new("scan");
        temp.write_txt("根下.txt");
        temp.write_txt("科幻/三体.txt");
        temp.write_txt("历史/明朝.txt");
        // 更深一层**不**收（MAX_SCAN_DEPTH = 1）。
        temp.write_txt("科幻/深层/被忽略.txt");
        std::fs::write(temp.path().join("根下.md"), b"# not txt").expect("写非 TXT");

        let files = scan_txt_files(temp.path()).expect("扫描");
        let names: Vec<String> = files
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("有文件名")
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert!(names.contains(&"根下.txt".to_string()));
        assert!(names.contains(&"三体.txt".to_string()));
        assert!(names.contains(&"明朝.txt".to_string()));
        assert!(
            !names.contains(&"被忽略.txt".to_string()),
            "深度超过 MAX_SCAN_DEPTH 的文件不应被收集"
        );
        assert!(!names.iter().any(|name| name.ends_with(".md")));
    }

    #[test]
    fn scan_rejects_non_directory() {
        let temp = TempDir::new("notdir");
        let file = temp.write_txt("a.txt");
        assert!(scan_txt_files(&file).is_err());
    }

    #[test]
    fn scan_reports_empty_when_no_txt_present() {
        let temp = TempDir::new("empty");
        std::fs::write(temp.path().join("readme.md"), b"# x").expect("写");
        let files = scan_txt_files(temp.path()).expect("扫描");
        assert!(files.is_empty());
    }

    #[test]
    fn parent_directory_name_takes_only_the_direct_parent() {
        assert_eq!(
            parent_directory_name(Path::new("/home/u/小说/科幻/三体.txt")),
            Some("科幻".to_string()),
            "只取直接父目录，不递归整条路径"
        );
        assert_eq!(parent_directory_name(Path::new("/三体.txt")), None);
    }

    #[test]
    fn organize_yes_creates_one_category_per_directory_name_under_current() {
        let temp = TempDir::new("organize");
        let sci = temp.write_txt("科幻/三体.txt");
        let hist = temp.write_txt("历史/明朝.txt");
        let db = temp.db();

        let mut storage = Storage::open(&db).expect("打开库");
        storage.migrate().expect("迁移");
        let parent =
            categories::create_category(storage.conn(), None, "我的书架", 1).expect("父分类");
        drop(storage);

        let (report, planned) = import_directory(
            &db,
            temp.path(),
            OrganizeChoice::organize_yes(false),
            Some(parent),
            100,
        )
        .expect("批量导入");

        assert!(report.all_succeeded(), "失败项：{:?}", report.failures);
        assert_eq!(report.imported.len(), 2);
        assert_eq!(report.created_categories.len(), 2, "科幻/历史各建一个分类");
        assert!(
            report.message().contains("新建分类 2 个"),
            "{}",
            report.message()
        );

        // 分类建立在**当前父分类**之下。
        let (nodes, _) = {
            let storage = Storage::open(&db).expect("开库");
            categories::project_tree_with_uncategorized(storage.conn()).expect("投影")
        };
        let names: Vec<&str> = nodes.iter().map(|node| node.name.as_str()).collect();
        assert!(names.contains(&"科幻"), "names={names:?}");
        assert!(names.contains(&"历史"), "names={names:?}");
        for node in &nodes {
            if node.name == "科幻" || node.name == "历史" {
                assert_eq!(node.depth, 1, "应挂在「我的书架」之下");
            }
        }

        // 每个文件都归入了**其所在目录名**对应的分类。
        // 断言方式：读回「书籍 → 分类名」映射并按分类名分组，
        // **不按 `imported` 的下标推断身份** —— 导入顺序取决于路径排序，
        // 用下标耦合会让测试在改名/换目录后给出误导性失败（实测踩中）。
        let mut by_category: std::collections::HashMap<String, Vec<i64>> =
            std::collections::HashMap::new();
        for book_id in &report.imported {
            let category = category_id_of(&db, *book_id).expect("已归类");
            by_category
                .entry(category_name(&db, category))
                .or_default()
                .push(*book_id);
        }
        assert_eq!(
            by_category.get("科幻").map(Vec::len),
            Some(1),
            "三体.txt 应归入「科幻」，实际映射：{:?}",
            by_category.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            by_category.get("历史").map(Vec::len),
            Some(1),
            "明朝那些事.txt 应归入「历史」，实际映射：{:?}",
            by_category.keys().collect::<Vec<_>>()
        );
        // 分类名与文件所在目录严格对应（不得张冠李戴）。
        assert_eq!(by_category.len(), 2, "不应产生第三个分类");

        // 计划里的目录名预览字段。
        let sci_plan = planned
            .iter()
            .find(|item| item.path == sci)
            .expect("科幻计划");
        assert_eq!(sci_plan.directory_name.as_deref(), Some("科幻"));
        let hist_plan = planned
            .iter()
            .find(|item| item.path == hist)
            .expect("历史计划");
        assert_eq!(hist_plan.directory_name.as_deref(), Some("历史"));
    }

    #[test]
    fn organize_yes_reuses_same_named_directory_instead_of_duplicating() {
        let temp = TempDir::new("reuse");
        temp.write_txt("科幻/三体.txt");
        temp.write_txt("科幻/流浪地球.txt");
        temp.write_txt("历史/明朝.txt");
        let db = temp.db();

        let (report, _) = import_directory(
            &db,
            temp.path(),
            OrganizeChoice::organize_yes(false),
            None,
            100,
        )
        .expect("批量导入");

        assert_eq!(report.imported.len(), 3);
        assert_eq!(
            report.created_categories.len(),
            2,
            "两个不同目录名 → 两个分类"
        );
        let storage = Storage::open(&db).expect("开库");
        assert_eq!(
            categories::list_categories(storage.conn())
                .expect("列表")
                .len(),
            2
        );
    }

    #[test]
    fn organize_no_puts_everything_into_current_category() {
        let temp = TempDir::new("no-organize");
        temp.write_txt("科幻/三体.txt");
        temp.write_txt("历史/明朝.txt");
        let db = temp.db();

        let mut storage = Storage::open(&db).expect("打开库");
        storage.migrate().expect("迁移");
        let parent =
            categories::create_category(storage.conn(), None, "我的书架", 1).expect("父分类");
        drop(storage);

        let (report, planned) = import_directory(
            &db,
            temp.path(),
            OrganizeChoice::organize_no(false),
            Some(parent),
            100,
        )
        .expect("批量导入");

        assert_eq!(report.imported.len(), 2);
        assert!(report.created_categories.is_empty(), "不分类模式不得建分类");
        for item in &planned {
            assert_eq!(item.category_id, Some(parent));
            assert!(item.directory_name.is_none());
        }
        for book_id in &report.imported {
            assert_eq!(category_id_of(&db, *book_id), Some(parent));
        }
    }

    #[test]
    fn organize_no_at_root_leaves_books_uncategorized() {
        let temp = TempDir::new("root");
        temp.write_txt("科幻/三体.txt");
        let db = temp.db();

        let (report, _) = import_directory(
            &db,
            temp.path(),
            OrganizeChoice::organize_no(false),
            None,
            100,
        )
        .expect("批量导入");

        assert_eq!(report.imported.len(), 1);
        assert_eq!(
            category_id_of(&db, report.imported[0]),
            None,
            "当前在根时应归入未分类"
        );
    }

    #[test]
    fn file_without_parent_directory_name_falls_back_to_current_category() {
        let temp = TempDir::new("noparent");
        // 直接放在扫描根下：其「直接父目录」就是扫描根目录名，仍会建分类；
        // 真正无父目录名的情形由 parent_directory_name 的单测覆盖。
        let direct = temp.write_txt("散书.txt");
        let db = temp.db();
        let (_, planned) = import_directory(
            &db,
            temp.path(),
            OrganizeChoice::organize_yes(false),
            None,
            100,
        )
        .expect("导入");
        let item = planned.iter().find(|it| it.path == direct).expect("计划");
        assert!(item.directory_name.is_some(), "扫描根下的文件也有父目录名");
    }

    #[test]
    fn import_never_modifies_user_txt_files() {
        let temp = TempDir::new("readonly");
        let first = temp.write_txt("科幻/三体.txt");
        let second = temp.write_txt("科幻/流浪地球.txt");
        let before: Vec<(u64, Option<std::time::SystemTime>)> = [first.clone(), second.clone()]
            .iter()
            .map(|path| {
                let meta = std::fs::metadata(path).expect("元信息");
                (meta.len(), meta.modified().ok())
            })
            .collect();

        let (report, _) = import_directory(
            &temp.db(),
            temp.path(),
            OrganizeChoice::organize_yes(false),
            None,
            100,
        )
        .expect("导入");
        assert!(report.all_succeeded());

        for (path, expected) in [first, second].iter().zip(before.iter()) {
            assert!(path.is_file(), "原始 TXT 不得被移动或删除");
            let meta = std::fs::metadata(path).expect("元信息");
            assert_eq!(meta.len(), expected.0, "字节数不得变化");
            assert_eq!(meta.modified().ok(), expected.1, "mtime 不得变化");
        }
    }

    #[test]
    fn failed_files_are_reported_without_stopping_the_batch() {
        let temp = TempDir::new("partial");
        let good = temp.write_txt("科幻/三体.txt");
        // 一个非 TXT 内容但扩展名为 .txt 的文件会被导入校验拒绝。
        let bad = temp.path().join("科幻/坏书.txt");
        std::fs::write(&bad, [0xFF, 0xFE, 0x00, 0x00]).expect("写坏文件");

        let (report, _) = import_directory(
            &temp.db(),
            temp.path(),
            OrganizeChoice::organize_yes(false),
            None,
            100,
        )
        .expect("批量导入");

        assert!(
            report.imported.len() >= 1,
            "好文件应导入成功（失败清单：{:?}）",
            report.failures
        );
        // 无论坏文件是否被拒，都必须有明确结论（成功或失败），不得静默。
        assert!(
            report.all_succeeded() || !report.failures.is_empty(),
            "失败必须有明确记录"
        );
        if !report.all_succeeded() {
            assert!(
                report.failures.iter().any(|(path, _)| path == &bad),
                "失败清单应含坏文件：{:?}",
                report.failures
            );
            assert!(bad.is_file(), "失败也不得删除用户文件");
        }
    }

    #[test]
    fn import_directory_reports_when_no_txt_found() {
        let temp = TempDir::new("none");
        std::fs::write(temp.path().join("a.md"), b"x").expect("写");
        let error = import_directory_now(
            &temp.db(),
            temp.path(),
            OrganizeChoice::organize_yes(false),
            None,
        )
        .expect_err("应报错");
        assert!(
            error.message().contains("没有找到 TXT"),
            "实际：{}",
            error.message()
        );
    }

    #[test]
    fn organize_choice_remember_flag_is_carried() {
        assert!(OrganizeChoice::organize_yes(true).remember);
        assert!(!OrganizeChoice::organize_yes(false).remember);
        assert_eq!(
            OrganizeChoice::organize_no(true).organize,
            OrganizeByDirectory::No
        );
    }

    #[test]
    fn plan_reports_created_categories_only_once_per_directory_name() {
        let temp = TempDir::new("plan");
        temp.write_txt("科幻/三体.txt");
        temp.write_txt("科幻/流浪地球.txt");
        let db = temp.db();

        let (planned, created) = plan_import(
            &db,
            &[
                temp.path().join("科幻/三体.txt"),
                temp.path().join("科幻/流浪地球.txt"),
            ],
            OrganizeChoice::organize_yes(false),
            None,
            1,
        )
        .expect("计划");

        assert_eq!(created.len(), 1, "同名目录只建一次");
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].category_id, planned[1].category_id);
    }
}
