//! 分类操作的动作层（V1.1，规格 R2–R5；设计稿 C3–C6）。
//!
//! 职责边界：
//!
//! - **判定与写入全在 Rust**：`categories.rs` 负责循环防护与同父同名，
//!   本模块负责「把用户的选择翻译成一次具体的存储调用」，并产出**可理解文案**；
//! - 界面只收集选择（弹窗输入 / 单选），**不推断**任何合法性；
//! - 「删除非空分类必须先选去向」（R3）：`DeleteChoice::None` 时本模块
//!   **直接拒绝**，不产生任何副作用 —— 界面只是把该拒绝显示出来。

use std::path::Path;

use reading_storage::categories::{self, DeleteDestination};
use reading_storage::Storage;

use crate::import::now_unix;

/// 书籍去向（规格 R3 的三个选项 + 未选 + 「正在选目标」中间态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteChoice {
    /// 未选择（**默认**；此时删除不得执行）。
    None,
    /// 移入另一分类（已选定目标）。
    MoveToCategory(i64),
    /// 一并移出书架。
    RemoveFromShelf,
    /// 移回根（未分类）。
    MoveToRoot,
}

impl DeleteChoice {
    /// 界面单选序号 → 去向。
    ///
    /// 口径（与 `DialogView::category_delete_destination` 一致）：
    /// `0`=未选、`1`=移入分类（`target_id` 为已选目标）、`2`=移出书架、`3`=移回根。
    /// **`4` = 「正在选择移入目标」的中间态**（用户点了「移入另一分类…」但还没选目标）：
    /// 此时**不得**执行删除（目标未定），与 `None` 同义。
    pub fn from_index(index: i32, target_id: i64) -> Self {
        match index {
            1 => DeleteChoice::MoveToCategory(target_id),
            2 => DeleteChoice::RemoveFromShelf,
            3 => DeleteChoice::MoveToRoot,
            // 0（未选）与 4（正在选目标）都**不得**执行删除。
            _ => DeleteChoice::None,
        }
    }

    /// 转为存储层去向；`None` 返回 `None`（**不构造默认值**，避免误删）。
    pub fn to_destination(self) -> Option<DeleteDestination> {
        match self {
            DeleteChoice::None => None,
            DeleteChoice::MoveToCategory(id) => {
                Some(DeleteDestination::MoveToCategory { target_id: id })
            }
            DeleteChoice::RemoveFromShelf => Some(DeleteDestination::RemoveFromShelf),
            DeleteChoice::MoveToRoot => Some(DeleteDestination::MoveToRoot),
        }
    }
}

/// 打开数据库并迁移（所有动作的公共入口）。
fn open(db_path: &Path) -> Result<Storage, String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    Ok(storage)
}

/// 新建分类（规格 R2；`parent_id = None` 表示根下）。
pub fn create_category(
    db_path: &Path,
    parent_id: Option<i64>,
    name: &str,
) -> Result<String, String> {
    let storage = open(db_path)?;
    categories::create_category(storage.conn(), parent_id, name, now_unix())
        .map_err(|error| error.to_string())?;
    Ok(format!("已新建分类「{name}」"))
}

/// 重命名分类（规格 R2）。
pub fn rename_category(db_path: &Path, id: i64, name: &str) -> Result<String, String> {
    let storage = open(db_path)?;
    categories::rename_category(storage.conn(), id, name, now_unix())
        .map_err(|error| error.to_string())?;
    Ok(format!("已重命名分类为「{name}」"))
}

/// 移动分类（规格 R4/R5；循环防护在存储层）。
pub fn move_category(
    db_path: &Path,
    id: i64,
    new_parent_id: Option<i64>,
) -> Result<String, String> {
    let storage = open(db_path)?;
    categories::move_category(storage.conn(), id, new_parent_id, now_unix())
        .map_err(|error| error.to_string())?;
    Ok(match new_parent_id {
        Some(_) => "已移动分类".to_string(),
        None => "已把分类移到根下".to_string(),
    })
}

/// 移动书籍到分类（规格 R4）。
pub fn move_book(db_path: &Path, book_id: i64, category_id: Option<i64>) -> Result<String, String> {
    let storage = open(db_path)?;
    categories::move_book_at(storage.conn(), book_id, category_id, now_unix())
        .map_err(|error| error.to_string())?;
    Ok(match category_id {
        Some(_) => "已移动到所选分类".to_string(),
        None => "已移到未分类".to_string(),
    })
}

/// 删除分类前的计划（删除浮层的计数来源；规格 R3）。
pub fn delete_plan(db_path: &Path, id: i64) -> Result<categories::DeletePlan, String> {
    let storage = open(db_path)?;
    categories::delete_plan(storage.conn(), id).map_err(|error| error.to_string())
}

/// 删除分类 —— **必须**显式给出书籍去向（规格 R3）。
///
/// `choice == None` 时**拒绝执行**并返回可理解错误，**不产生任何副作用**。
pub fn delete_category(db_path: &Path, id: i64, choice: DeleteChoice) -> Result<String, String> {
    let Some(destination) = choice.to_destination() else {
        return Err("请先选择书籍去向 —— 未选择时不会执行删除".to_string());
    };
    let mut storage = open(db_path)?;
    let report = categories::delete_category(storage.conn_mut(), id, destination, now_unix())
        .map_err(|error| error.to_string())?;
    Ok(report.message())
}

/// 行内菜单项（设计稿 C2：新建子分类 / 重命名 / 移动到分类 / 删除…（err 色，最后））。
pub fn menu_items() -> Vec<crate::app_state::ChoiceRow> {
    use crate::app_state::{category_action, ChoiceRow};
    vec![
        ChoiceRow {
            name: "新建子分类…".to_string(),
            current: false,
            enabled: true,
        },
        ChoiceRow {
            name: "重命名…".to_string(),
            current: false,
            enabled: true,
        },
        ChoiceRow {
            name: "移动到分类…".to_string(),
            current: false,
            enabled: true,
        },
        ChoiceRow {
            name: "删除…".to_string(),
            current: false,
            enabled: true,
        },
    ]
    .into_iter()
    .zip([
        category_action::ADD_CHILD,
        category_action::RENAME,
        category_action::MOVE,
        category_action::DELETE,
    ])
    .map(|(row, _)| row)
    .collect()
}

/// 「移动到分类」的目标列表（设计稿 C5：根不可作目标，「未分类」钉在顶部）。
///
/// 自身与全部子孙**置灰不可选**（规格 R5）。
///
/// ⚠️ 判定与顺序**全部来自共享存储层** [`categories::move_targets`] ——
/// 本函数只做「存储行 → 界面行」的文案装箱。**不得**在此重新遍历父子链：
/// 桌面与 Android 必须同源，否则两端「哪些目标可选」会各自演化。
pub fn move_targets(db_path: &Path, source_id: i64) -> Result<Vec<crate::app_state::ChoiceRow>, String> {
    use crate::app_state::ChoiceRow;
    let storage = open(db_path)?;
    let targets =
        categories::move_targets(storage.conn(), source_id).map_err(|error| error.to_string())?;

    Ok(targets
        .into_iter()
        .map(|target| {
            let indent = "　".repeat(target.depth.max(0) as usize);
            ChoiceRow {
                name: if target.selectable {
                    format!("{indent}{}（{} 本）", target.name, target.total_count)
                } else {
                    // 不可选标记**仅作提示**；真正的判定在 `enabled` 字段
                    // （界面读它，不解析文案）。
                    format!("{indent}{}（不可选）", target.name)
                },
                // 选中标记由界面按当前归属回填；这里只给名称与计数。
                current: false,
                enabled: target.selectable,
            }
        })
        .collect())
}

/// 「移动到分类」目标行的 id 列表（与 [`move_targets`] **同序**）。
///
/// ⚠️ 与 [`move_targets`] 共用同一来源（共享存储层），故「同序」是**结构性保证**，
/// 而不是靠两处实现碰巧一致（此前两处各自算一遍，属隐式约定）。
pub fn move_target_ids(db_path: &Path, source_id: i64) -> Result<Vec<Option<i64>>, String> {
    let storage = open(db_path)?;
    let targets =
        categories::move_targets(storage.conn(), source_id).map_err(|error| error.to_string())?;
    Ok(targets
        .into_iter()
        .map(|target| if target.id < 0 { None } else { Some(target.id) })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reading_storage::books::{import_book, BookImport, KIND_DESKTOP_PATH};

    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "nr-catops-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("建目录");
            Self(dir)
        }
        fn db(&self) -> std::path::PathBuf {
            self.0.join("library.db")
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    use std::path::PathBuf;

    fn seed(db: &Path, locator: &str, marker: &str) -> i64 {
        let mut storage = Storage::open(db).expect("开库");
        storage.migrate().expect("迁移");
        import_book(
            storage.conn(),
            &BookImport {
                display_name: marker.to_string(),
                original_name: "book.txt".to_string(),
                source_kind: KIND_DESKTOP_PATH.to_string(),
                source_locator: locator.to_string(),
                source_size: 32,
                source_modified_hint: None,
                source_fingerprint: None,
                encoding: "utf-8".to_string(),
                encoding_confidence: 1000,
                encoding_user_override: false,
            },
            1,
        )
        .expect("导入")
        .book_id
    }

    #[test]
    fn create_rename_and_move_report_readable_text() {
        let temp = Temp::new("crud");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        let storage = open(&db).expect("开库");
        let sci = categories::list_categories(storage.conn()).expect("列表")[0].id;
        drop(storage);

        create_category(&db, Some(sci), "中国科幻").expect("子分类");
        let storage = open(&db).expect("开库");
        let cn = categories::list_categories(storage.conn())
            .expect("列表")
            .into_iter()
            .find(|item| item.name == "中国科幻")
            .expect("子分类")
            .id;
        drop(storage);

        let text = rename_category(&db, cn, "中国科幻文学").expect("重命名");
        assert!(text.contains("已重命名"), "{text}");
        let text = move_category(&db, cn, None).expect("移到根");
        assert!(text.contains("根"), "{text}");
    }

    #[test]
    fn duplicate_name_surfaces_readable_error_without_writing() {
        let temp = Temp::new("dup");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        let error = create_category(&db, None, "科幻").expect_err("同名应失败");
        assert!(error.contains("已存在同名分类"), "实际：{error}");
        // 失败不得留下第二条记录。
        let storage = open(&db).expect("开库");
        assert_eq!(
            categories::list_categories(storage.conn())
                .expect("列表")
                .len(),
            1
        );
    }

    #[test]
    fn delete_without_destination_is_refused_and_changes_nothing() {
        let temp = Temp::new("nodest");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        let storage = open(&db).expect("开库");
        let sci = categories::list_categories(storage.conn()).expect("列表")[0].id;
        let book = seed(&db, "/tmp/a.txt", "甲");
        categories::move_book_at(storage.conn(), book, Some(sci), 2).expect("归类");
        drop(storage);

        let error = delete_category(&db, sci, DeleteChoice::None).expect_err("未选去向应拒");
        assert!(error.contains("请先选择书籍去向"), "实际：{error}");

        // 关键：分类与书籍归属都**未被改动**。
        let storage = open(&db).expect("开库");
        assert!(
            categories::get_category(storage.conn(), sci)
                .expect("读")
                .is_some(),
            "未选去向时分类不得被删除"
        );
        let category: Option<i64> = storage
            .conn()
            .query_row(
                "SELECT category_id FROM books WHERE id = ?1",
                [book],
                |row| row.get(0),
            )
            .expect("读回");
        assert_eq!(category, Some(sci), "书籍归属不得变化");
    }

    #[test]
    fn delete_with_each_destination_applies_the_users_choice() {
        // 移回根。
        let temp = Temp::new("root");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        let storage = open(&db).expect("开库");
        let sci = categories::list_categories(storage.conn()).expect("列表")[0].id;
        let book = seed(&db, "/tmp/a.txt", "甲");
        categories::move_book_at(storage.conn(), book, Some(sci), 2).expect("归类");
        drop(storage);

        let text = delete_category(&db, sci, DeleteChoice::MoveToRoot).expect("删除");
        assert!(text.contains("移回未分类"), "{text}");
        let storage = open(&db).expect("开库");
        let category: Option<i64> = storage
            .conn()
            .query_row(
                "SELECT category_id FROM books WHERE id = ?1",
                [book],
                |row| row.get(0),
            )
            .expect("读回");
        assert_eq!(category, None);

        // 移出书架。
        let temp = Temp::new("remove");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        let storage = open(&db).expect("开库");
        let sci = categories::list_categories(storage.conn()).expect("列表")[0].id;
        let book = seed(&db, "/tmp/b.txt", "乙");
        categories::move_book_at(storage.conn(), book, Some(sci), 2).expect("归类");
        drop(storage);
        let text = delete_category(&db, sci, DeleteChoice::RemoveFromShelf).expect("删除");
        assert!(text.contains("移出书架"), "{text}");
        let storage = open(&db).expect("开库");
        let status: String = storage
            .conn()
            .query_row("SELECT status FROM books WHERE id = ?1", [book], |row| {
                row.get(0)
            })
            .expect("读回");
        assert_eq!(status, "removed");

        // 移入另一分类。
        let temp = Temp::new("move");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        create_category(&db, None, "历史").expect("新建");
        let storage = open(&db).expect("开库");
        let all = categories::list_categories(storage.conn()).expect("列表");
        let sci = all
            .iter()
            .find(|item| item.name == "科幻")
            .expect("科幻")
            .id;
        let hist = all
            .iter()
            .find(|item| item.name == "历史")
            .expect("历史")
            .id;
        let book = seed(&db, "/tmp/c.txt", "丙");
        categories::move_book_at(storage.conn(), book, Some(sci), 2).expect("归类");
        drop(storage);
        delete_category(&db, sci, DeleteChoice::MoveToCategory(hist)).expect("删除");
        let storage = open(&db).expect("开库");
        let category: Option<i64> = storage
            .conn()
            .query_row(
                "SELECT category_id FROM books WHERE id = ?1",
                [book],
                |row| row.get(0),
            )
            .expect("读回");
        assert_eq!(category, Some(hist));
    }

    #[test]
    fn delete_choice_index_round_trip_and_none_maps_to_no_destination() {
        assert_eq!(
            DeleteChoice::from_index(0, 0).to_destination(),
            None,
            "未选必须映射为 None（不得落到任何默认去向）"
        );
        assert!(DeleteChoice::from_index(0, 5).to_destination().is_none());
        assert_eq!(
            DeleteChoice::from_index(1, 7).to_destination(),
            Some(DeleteDestination::MoveToCategory { target_id: 7 })
        );
        assert_eq!(
            DeleteChoice::from_index(2, 0).to_destination(),
            Some(DeleteDestination::RemoveFromShelf)
        );
        assert_eq!(
            DeleteChoice::from_index(3, 0).to_destination(),
            Some(DeleteDestination::MoveToRoot)
        );
    }

    #[test]
    fn move_targets_exclude_self_and_descendants() {
        let temp = Temp::new("targets");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        let storage = open(&db).expect("开库");
        let sci = categories::list_categories(storage.conn()).expect("列表")[0].id;
        drop(storage);
        create_category(&db, Some(sci), "中国科幻").expect("子");
        create_category(&db, None, "历史").expect("兄弟");
        let history_id = {
            let storage = open(&db).expect("开库");
            categories::list_categories(storage.conn())
                .expect("列表")
                .into_iter()
                .find(|node| node.name == "历史")
                .expect("历史")
                .id
        };

        let rows = move_targets(&db, sci).expect("目标");
        let text: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert!(text[0].starts_with("未分类"), "未分类钉在顶部：{text:?}");
        assert!(
            text.iter()
                .any(|name| name.contains("不可选") && name.contains("科幻")),
            "自身应不可选：{text:?}"
        );
        assert!(
            text.iter()
                .any(|name| name.contains("不可选") && name.contains("中国科幻")),
            "子孙应不可选：{text:?}"
        );
        assert!(
            text.iter()
                .any(|name| name.contains("历史") && !name.contains("不可选")),
            "兄弟可选：{text:?}"
        );

        // ⚠️ 两个列表必须**逐下标对齐**：界面用名称列表的 0 基下标去查 id 列表
        // （`ui.rs` 移动态）。此前 `move_target_ids` 把被阻断项**剔除**，
        // 于是「被阻断行之后的目标」全部错位 —— 夹具下「历史」在下标 3，
        // 而旧 id 列表只有 2 项 ⇒ `rows.get(3)` 取不到 ⇒ **根本选不中**。
        let ids = move_target_ids(&db, sci).expect("目标 id");
        assert_eq!(
            ids.len(),
            rows.len(),
            "id 列表必须与名称列表等长（否则下标错位）"
        );
        assert_eq!(ids[0], None, "首个是未分类（`None` 口径）");
        // 被阻断项**仍在**列表里（标记不可选，而非剔除）—— 下标因此保持一致。
        let sci_index = text.iter().position(|name| name.contains("科幻") && name.contains("不可选"));
        let history_index = text.iter().position(|name| name.contains("历史"));
        assert_eq!(ids[sci_index.expect("自身行")], Some(sci), "自身仍在列表里，只是不可选");
        // 关键回归：**被阻断行之后**的目标必须仍能被下标取到
        // （旧实现剔除阻断项 ⇒ 这里的下标会落到别的分类上，甚至越界取不到）。
        assert_eq!(
            ids[history_index.expect("历史行")],
            Some(history_id),
            "被阻断行之后的目标必须仍能被下标取到"
        );
        assert!(
            history_index.unwrap() > sci_index.unwrap(),
            "夹具应保证「历史」排在自身之后（否则这条回归测不到）"
        );
    }

    #[test]
    fn move_book_to_root_and_category() {
        let temp = Temp::new("movebook");
        let db = temp.db();
        create_category(&db, None, "科幻").expect("新建");
        let book = seed(&db, "/tmp/a.txt", "甲");
        let storage = open(&db).expect("开库");
        let sci = categories::list_categories(storage.conn()).expect("列表")[0].id;
        drop(storage);

        move_book(&db, book, Some(sci)).expect("移入");
        let text = move_book(&db, book, None).expect("移回");
        assert!(text.contains("未分类"), "{text}");
    }

    #[test]
    fn menu_items_are_four_and_delete_is_last() {
        let items = menu_items();
        assert_eq!(items.len(), 4);
        assert!(items[0].name.contains("新建子分类"));
        assert!(items[1].name.contains("重命名"));
        assert!(items[2].name.contains("移动到分类"));
        assert!(items[3].name.contains("删除"), "删除必须是最后一项");
    }
}
