//! 侧栏分类树的**投影**（V1.1，规格 R1/R4/R5）。
//!
//! 职责边界（规格 R18、设计稿 §7.2「语义零下沉」）：
//!
//! - 本模块负责：树形组装、折叠可见性、直属/总计计数、选中标记、
//!   「移动分类」时的不可选标记（自身与全部子孙）；
//! - `.slint` 只消费本投影并按 `depth` 缩进、按 `expanded`/`selected` 上色，
//!   **不在界面里算层级、计数或循环可达性**；
//! - 「最近阅读」**不在侧栏**（规格 Q5 已裁定），它从书架页头的菜单项进入。

use std::collections::{HashMap, HashSet};

use reading_storage::{categories, Storage};

use crate::app_state::CategoryRow;

/// 侧栏行分类：根 / 未分类 / 真实分类。
pub const KIND_ALL: i32 = 0;
/// 未分类（`category_id IS NULL`）。
pub const KIND_UNCATEGORIZED: i32 = 1;
/// 真实分类。
pub const KIND_CATEGORY: i32 = 2;

/// 「未分类」在 `Option<i64>` 里的哨兵值。
///
/// 口径：`None` = 全部书籍；`Some(-1)` = 仅未分类（因为「未分类」本身就是
/// `category_id IS NULL`，无法用一个真实 id 表示）。**真实分类 id 从 1 开始**，
/// 故 `-1` 不会是合法分类 id。
pub const UNCATEGORIZED_SENTINEL: i64 = -1;

/// 根节点的 id 约定（空串 = 未选中任何分类）。
pub const ROOT_SELECTED_ID: &str = "";

/// 侧栏树投影的输入（全部来自 Rust，不来自界面）。
#[derive(Debug, Clone)]
pub struct SidebarInput {
    /// 当前选中的分类 id（`None` = 根「全部书籍」；`Some(UNCATEGORIZED_SENTINEL)` = 未分类）。
    pub selected: Option<i64>,
    /// 折叠的分类 id 集合（已折叠的节点不展开其子节点）。
    pub collapsed: HashSet<i64>,
    /// 「移动分类」操作的目标源分类 id；该节点及其全部子孙标记为不可选。
    pub move_source_id: Option<i64>,
    /// 侧栏是否可见（Rust 按断点决定，界面不自己算）。
    ///
    /// **默认可见**：`bool` 的 `Default` 是 `false`，直接 derive 会让「未显式传入」
    /// 的调用方拿到「侧栏不可见」这一意外状态，故手工实现 `Default`。
    pub visible: bool,
}

impl Default for SidebarInput {
    fn default() -> Self {
        Self {
            selected: None,
            collapsed: HashSet::new(),
            move_source_id: None,
            visible: true,
        }
    }
}

/// 侧栏树投影 + 未分类计数。
pub fn project(
    conn: &rusqlite::Connection,
    input: &SidebarInput,
) -> Result<(Vec<CategoryRow>, i64), String> {
    let (nodes, unclassified) =
        categories::project_tree_with_uncategorized(conn).map_err(|error| error.to_string())?;
    // `CategoryNode`（存储层投影）**不含** `parent_id` —— 那是「界面只消费投影」
    // 的刻意设计；侧栏需要父链来判折叠，故这里另取一次 `list_categories` 建映射。
    let all = categories::list_categories(conn).map_err(|error| error.to_string())?;
    let parent_of: HashMap<i64, Option<i64>> =
        all.iter().map(|node| (node.id, node.parent_id)).collect();
    // 「有子分类」由完整分类表判定（`CategoryNode` 只给计数，不给结构）。
    let parents: HashSet<i64> = all.iter().filter_map(|node| node.parent_id).collect();

    // 「移动源」的自身 + 全部子孙（规格 R5 的不可选集合）。
    let blocked: HashSet<i64> = match input.move_source_id {
        None => HashSet::new(),
        Some(source) => categories::descendant_ids(conn, source)
            .map_err(|error| error.to_string())?
            .into_iter()
            .collect(),
    };

    // 祖先被折叠时，其子孙不可见（逐层判断；界面不做这件事）。
    let is_visible = |node: &categories::CategoryNode| -> bool {
        // 父链从 `parent_of` 取（`CategoryNode` 不带 `parent_id`）。
        let mut cursor = parent_of.get(&node.id).copied().flatten();
        let mut guard = 0;
        while let Some(parent) = cursor {
            if input.collapsed.contains(&parent) {
                return false;
            }
            cursor = parent_of.get(&parent).copied().flatten();
            guard += 1;
            if guard > 64 {
                // 兜底：历史数据异常形成环时不再上溯（投影不挂死）。
                break;
            }
        }
        true
    };

    let total_all: i64 =
        nodes.iter().map(|node| node.direct_book_count).sum::<i64>() + unclassified;

    let mut rows: Vec<CategoryRow> = Vec::with_capacity(nodes.len() + 2);

    // ① 根：「全部书籍」（常驻，显示全部含未分类）。
    rows.push(CategoryRow {
        id: ROOT_SELECTED_ID.to_string(),
        name: "全部书籍".to_string(),
        depth: 0,
        direct_count: total_all.min(i32::MAX as i64) as i32,
        total_count: total_all.min(i32::MAX as i64) as i32,
        has_children: !nodes.is_empty(),
        expanded: true,
        selected: input.selected.is_none(),
        move_blocked: false,
        virtual_row: true,
        kind: KIND_ALL,
        parent_id: String::new(),
    });

    // ② 未分类：根下的过滤入口，**不是真实分类**（不可重命名 / 移动 / 删除）。
    rows.push(CategoryRow {
        id: "uncategorized".to_string(),
        name: "未分类".to_string(),
        depth: 1,
        direct_count: unclassified.min(i32::MAX as i64) as i32,
        total_count: unclassified.min(i32::MAX as i64) as i32,
        has_children: false,
        expanded: true,
        selected: false,
        move_blocked: false,
        virtual_row: true,
        kind: KIND_UNCATEGORIZED,
        parent_id: ROOT_SELECTED_ID.to_string(),
    });

    // ③ 真实分类树（尊重折叠）。
    for node in &nodes {
        if !is_visible(node) {
            continue;
        }
        let has_children = parents.contains(&node.id);
        let expanded = !input.collapsed.contains(&node.id);
        rows.push(CategoryRow {
            id: node.id.to_string(),
            name: node.name.clone(),
            depth: node.depth + 1,
            direct_count: node.direct_book_count.min(i32::MAX as i64) as i32,
            total_count: node.total_book_count.min(i32::MAX as i64) as i32,
            has_children,
            expanded,
            selected: input.selected == Some(node.id),
            move_blocked: blocked.contains(&node.id),
            virtual_row: false,
            kind: KIND_CATEGORY,
            parent_id: parent_of
                .get(&node.id)
                .copied()
                .flatten()
                .map(|value| value.to_string())
                .unwrap_or_default(),
        });
    }

    // 「未分类」被选中时打勾（Rust 判定，界面不推断）。
    // 必须**恰好一行** selected：根行在 `selected == Some(-1)` 时不打勾，
    // 未分类行补上勾 —— 否则会出现「0 行选中」或「2 行选中」。
    if input.selected == Some(UNCATEGORIZED_SENTINEL) {
        if let Some(row) = rows.iter_mut().find(|row| row.kind == KIND_UNCATEGORIZED) {
            row.selected = true;
        }
    }
    Ok((rows, unclassified))
}

/// 「未分类」选中态的判定入口（`selected == None` 且根不是选中项时用得上）。
///
/// 口径：`selected` 为「未分类」时用字符串约定（见 [`UNCATEGORIZED_SELECTED_ID`]）。
pub fn is_uncategorized_selected(selected: &str) -> bool {
    selected == UNCATEGORIZED_SELECTED_ID
}

/// 「未分类」在 `selected_category_id` 中的约定值。
pub const UNCATEGORIZED_SELECTED_ID: &str = "uncategorized";

/// 按 `selected_category_id` 还原过滤目标（`None` = 全部书籍）。
pub fn resolve_selected(selected: &str) -> Option<Option<i64>> {
    if selected.is_empty() {
        // 根：全部书籍。
        Some(None)
    } else if selected == UNCATEGORIZED_SELECTED_ID {
        // 未分类：用哨兵表示「仅未分类」（`Some(None)` 已被根占用）。
        Some(Some(UNCATEGORIZED_SENTINEL))
    } else {
        selected.parse::<i64>().ok().map(Some)
    }
}

/// 过滤后的书籍 id 集合（按当前分类作用域）。
///
/// `None` = 全部书籍；`Some(UNCATEGORIZED_SENTINEL)` = 仅未分类；`Some(id)` = 该分类的**直属**书籍
/// （**不含子孙分类**，规格 R1 与设计稿 C1 的计数口径一致）。
pub fn visible_book_ids(conn: &rusqlite::Connection, selected: &str) -> Result<Vec<i64>, String> {
    let Some(target) = resolve_selected(selected) else {
        return Err(format!("分类 id 无法解析：{selected}"));
    };
    match target {
        None => {
            let mut statement = conn
                .prepare("SELECT id FROM books WHERE status != 'removed'")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|error| error.to_string())?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|error| error.to_string())?);
            }
            Ok(ids)
        }
        Some(UNCATEGORIZED_SENTINEL) => {
            let mut statement = conn
                .prepare("SELECT id FROM books WHERE status != 'removed' AND category_id IS NULL")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|error| error.to_string())?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|error| error.to_string())?);
            }
            Ok(ids)
        }
        Some(category_id) => {
            let mut statement = conn
                .prepare("SELECT id FROM books WHERE status != 'removed' AND category_id = ?1")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([category_id], |row| row.get::<_, i64>(0))
                .map_err(|error| error.to_string())?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|error| error.to_string())?);
            }
            Ok(ids)
        }
    }
}

/// 面包屑（页头显示「分类：全部书籍 / 科幻 / 中国科幻」；规格 R1）。
pub fn breadcrumb(conn: &rusqlite::Connection, selected: &str) -> String {
    if selected.is_empty() {
        return "全部书籍".to_string();
    }
    if is_uncategorized_selected(selected) {
        return "未分类".to_string();
    }
    let Some(id) = selected.parse::<i64>().ok() else {
        return "全部书籍".to_string();
    };
    let Ok(all) = categories::list_categories(conn) else {
        return "全部书籍".to_string();
    };
    let mut chain: Vec<String> = Vec::new();
    let mut cursor = Some(id);
    let mut guard = 0;
    while let Some(current) = cursor {
        guard += 1;
        if guard > 64 {
            break;
        }
        let Some(node) = all.iter().find(|item| item.id == current) else {
            break;
        };
        chain.push(node.name.clone());
        cursor = node.parent_id;
    }
    chain.reverse();
    if chain.is_empty() {
        "全部书籍".to_string()
    } else {
        format!("全部书籍 / {}", chain.join(" / "))
    }
}

/// 分类作用域的**短标签**（页头 / 状态栏用；不含连接数据库，故只按 id 判定）。
///
/// 与 [`breadcrumb`] 的区别：后者需要连接（要查分类名），本函数供
/// 纯 UI 回调路径使用（此时锁已被占用，不能再拿连接）。
pub fn breadcrumb_scope_label(selected: &str) -> String {
    if selected.is_empty() {
        "全部书籍".to_string()
    } else if is_uncategorized_selected(selected) {
        "未分类".to_string()
    } else {
        format!("分类 {selected}")
    }
}

/// 打开数据库并投影（供 `shelf.rs` 调用）。
pub fn project_for_db(
    db_path: &std::path::Path,
    input: &SidebarInput,
) -> Result<(Vec<CategoryRow>, i64), String> {
    let mut storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    storage.migrate().map_err(|error| error.to_string())?;
    let (rows, unclassified) = project(storage.conn(), input)?;
    Ok((rows, unclassified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reading_storage::books::{import_book, BookImport, KIND_DESKTOP_PATH};
    use reading_storage::progress::{save_progress, ProgressInput};

    fn storage() -> Storage {
        let mut storage = Storage::open_in_memory().expect("内存库");
        storage.migrate().expect("迁移");
        storage
    }

    fn seed_book(conn: &rusqlite::Connection, locator: &str, now: i64) -> i64 {
        import_book(
            conn,
            &BookImport {
                display_name: locator.to_string(),
                original_name: "book.txt".to_string(),
                source_kind: KIND_DESKTOP_PATH.to_string(),
                source_locator: locator.to_string(),
                source_size: 64,
                source_modified_hint: None,
                source_fingerprint: None,
                encoding: "utf-8".to_string(),
                encoding_confidence: 1000,
                encoding_user_override: false,
            },
            now,
        )
        .expect("导入")
        .book_id
    }

    fn fixture() -> (Storage, i64, i64, i64, i64, i64) {
        let storage = storage();
        let conn = storage.conn();
        let sci = categories::create_category(conn, None, "科幻", 1).expect("科幻");
        let cn = categories::create_category(conn, Some(sci), "中国科幻", 2).expect("中国科幻");
        let west = categories::create_category(conn, Some(sci), "外国科幻", 3).expect("外国科幻");
        let hist = categories::create_category(conn, None, "历史", 4).expect("历史");

        let a = seed_book(conn, "/tmp/a.txt", 10);
        let b = seed_book(conn, "/tmp/b.txt", 11);
        let c = seed_book(conn, "/tmp/c.txt", 12);
        let loose = seed_book(conn, "/tmp/d.txt", 13);
        categories::move_book_at(conn, a, Some(sci), 20).expect("归类");
        categories::move_book_at(conn, b, Some(cn), 21).expect("归类");
        categories::move_book_at(conn, c, Some(west), 22).expect("归类");
        save_progress(conn, &ProgressInput::at(loose, 0, 10), 23).expect("进度");
        (storage, sci, cn, west, hist, loose)
    }

    fn names(rows: &[CategoryRow]) -> Vec<String> {
        rows.iter().map(|row| row.name.clone()).collect()
    }

    #[test]
    fn sidebar_starts_with_all_books_and_uncategorized() {
        let (storage, _, _, _, _, loose) = fixture();
        let (rows, unclassified) = project(storage.conn(), &SidebarInput::default()).expect("投影");
        assert_eq!(unclassified, 1, "d.txt 未归类");
        assert_eq!(names(&rows)[..3], ["全部书籍", "未分类", "科幻"]);
        let all = &rows[0];
        assert_eq!(all.kind, KIND_ALL);
        assert!(all.virtual_row, "全部书籍是虚拟节点，不可重命名/删除");
        assert!(all.selected, "默认选中根");
        assert_eq!(all.total_count, 4, "4 本全部在架");
        let uncat = &rows[1];
        assert_eq!(uncat.kind, KIND_UNCATEGORIZED);
        assert_eq!(uncat.direct_count, 1);
        assert!(!uncat.selected);
        assert_eq!(uncat.parent_id, ROOT_SELECTED_ID);
        let _ = loose;
    }

    #[test]
    fn sidebar_rows_are_depth_first_children_right_after_their_parent() {
        // 设计稿 C1：子分类**紧跟父分类**（深度优先）。
        // 早前投影用 `VecDeque::pop_front` + 子节点入队尾 ⇒ 广度优先，
        // 子分类被排到所有根兄弟之后（实测侧栏里「中国科幻/外国科幻」跑到「历史」后面），
        // 视觉上像「子项脱离了父项」。这条测试把顺序钉住。
        let (storage, _, _, _, _, _) = fixture();
        let (rows, _) = project(storage.conn(), &SidebarInput::default()).expect("投影");
        assert_eq!(
            names(&rows),
            ["全部书籍", "未分类", "科幻", "中国科幻", "外国科幻", "历史"],
            "侧栏必须是深度优先序（子分类紧跟父分类）"
        );
    }

    #[test]
    fn real_categories_carry_depth_and_counts() {
        let (storage, sci, cn, _, _, _) = fixture();
        let (rows, _) = project(storage.conn(), &SidebarInput::default()).expect("投影");
        let sci_row = rows
            .iter()
            .find(|row| row.id == sci.to_string())
            .expect("科幻");
        let cn_row = rows
            .iter()
            .find(|row| row.id == cn.to_string())
            .expect("中国科幻");
        assert_eq!(sci_row.kind, KIND_CATEGORY);
        assert!(!sci_row.virtual_row);
        assert_eq!(sci_row.depth, 1, "根下分类 depth=1");
        assert_eq!(cn_row.depth, 2, "子分类 depth=2");
        assert_eq!(sci_row.direct_count, 1, "科幻直属 1 本");
        assert_eq!(sci_row.total_count, 3, "科幻 + 全部子孙 = 3 本");
        assert_eq!(cn_row.direct_count, 1);
        assert_eq!(cn_row.total_count, 1);
        assert!(sci_row.has_children, "科幻有子分类");
        assert_eq!(cn_row.parent_id, sci.to_string());
    }

    #[test]
    fn collapsing_a_node_hides_its_descendants() {
        let (storage, sci, cn, _, _, _) = fixture();
        let mut collapsed = HashSet::new();
        collapsed.insert(sci);
        let (rows, _) = project(
            storage.conn(),
            &SidebarInput {
                collapsed,
                ..SidebarInput::default()
            },
        )
        .expect("投影");
        let list = names(&rows);
        assert!(list.contains(&"科幻".to_string()));
        assert!(
            !list.contains(&"中国科幻".to_string()),
            "折叠后子分类应隐藏"
        );
        assert!(!list.contains(&"外国科幻".to_string()));
        // 折叠节点自身标记为未展开。
        let sci_row = rows
            .iter()
            .find(|row| row.id == sci.to_string())
            .expect("科幻");
        assert!(!sci_row.expanded);
    }

    #[test]
    fn selected_category_is_marked() {
        let (storage, sci, cn, _, _, _) = fixture();
        let (rows, _) = project(
            storage.conn(),
            &SidebarInput {
                selected: Some(cn),
                ..SidebarInput::default()
            },
        )
        .expect("投影");
        let selected: Vec<&str> = rows
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(selected, vec!["中国科幻"], "有且只有一个选中行");
        let all_row = rows
            .iter()
            .find(|row| row.kind == KIND_ALL)
            .expect("全部书籍");
        assert!(!all_row.selected);
        let _ = sci;
    }

    #[test]
    fn move_mode_blocks_self_and_descendants() {
        let (storage, sci, cn, west, _, _) = fixture();
        let (rows, _) = project(
            storage.conn(),
            &SidebarInput {
                move_source_id: Some(sci),
                ..SidebarInput::default()
            },
        )
        .expect("投影");
        let blocked: Vec<&str> = rows
            .iter()
            .filter(|row| row.move_blocked)
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(
            blocked,
            vec!["科幻", "中国科幻", "外国科幻"],
            "自身与全部子孙不可选"
        );
        let hist_row = rows.iter().find(|row| row.name == "历史").expect("历史");
        assert!(!hist_row.move_blocked, "非子孙可选");
    }

    #[test]
    fn visible_book_ids_follow_the_selected_scope() {
        let (storage, sci, cn, _, _, _) = fixture();
        let conn = storage.conn();
        let all = visible_book_ids(conn, "").expect("全部");
        assert_eq!(all.len(), 4);
        let uncategorized = visible_book_ids(conn, UNCATEGORIZED_SELECTED_ID).expect("未分类");
        assert_eq!(uncategorized.len(), 1);
        let sci_books = visible_book_ids(conn, &sci.to_string()).expect("科幻");
        assert_eq!(sci_books.len(), 1, "只含直属，不含子孙");
        let cn_books = visible_book_ids(conn, &cn.to_string()).expect("中国科幻");
        assert_eq!(cn_books.len(), 1);
    }

    #[test]
    fn removed_books_are_not_visible_in_any_scope() {
        let storage = storage();
        let conn = storage.conn();
        let book = seed_book(conn, "/tmp/a.txt", 1);
        categories::move_book_at(conn, book, None, 2).expect("根");
        reading_storage::books::set_book_status(
            conn,
            book,
            reading_storage::books::STATUS_REMOVED,
            3,
        )
        .expect("软移除");
        assert!(visible_book_ids(conn, "").expect("全部").is_empty());
        assert!(visible_book_ids(conn, UNCATEGORIZED_SELECTED_ID)
            .expect("未分类")
            .is_empty());
    }

    #[test]
    fn breadcrumb_joins_the_ancestor_chain() {
        let (storage, _, cn, _, _, _) = fixture();
        assert_eq!(breadcrumb(storage.conn(), ""), "全部书籍");
        assert_eq!(
            breadcrumb(storage.conn(), UNCATEGORIZED_SELECTED_ID),
            "未分类"
        );
        assert_eq!(
            breadcrumb(storage.conn(), &cn.to_string()),
            "全部书籍 / 科幻 / 中国科幻"
        );
        assert_eq!(
            breadcrumb(storage.conn(), "999999"),
            "全部书籍",
            "未知 id 回落根"
        );
    }

    #[test]
    fn resolve_selected_distinguishes_root_from_uncategorized() {
        assert_eq!(resolve_selected(""), Some(None));
        assert_eq!(
            resolve_selected(UNCATEGORIZED_SELECTED_ID),
            Some(Some(UNCATEGORIZED_SENTINEL))
        );
        assert_eq!(resolve_selected("42"), Some(Some(42)));
        assert_eq!(resolve_selected("abc"), None, "非法 id 返回 None");
    }

    #[test]
    fn empty_library_still_shows_both_virtual_rows() {
        let storage = storage();
        let (rows, unclassified) = project(storage.conn(), &SidebarInput::default()).expect("投影");
        assert_eq!(unclassified, 0);
        assert_eq!(names(&rows), vec!["全部书籍", "未分类"]);
        assert_eq!(rows[0].total_count, 0);
    }

    #[test]
    fn sidebar_visibility_is_decided_by_rust() {
        let (storage, _, _, _, _, _) = fixture();
        let (rows, _) = project(
            storage.conn(),
            &SidebarInput {
                visible: false,
                ..SidebarInput::default()
            },
        )
        .expect("投影");
        // `visible` 不改变行内容，只由调用方决定是否展示（避免界面自己算断点）。
        assert!(!rows.is_empty());
        assert!(SidebarInput::default().visible, "默认可见");
    }

    #[test]
    fn deeply_nested_chain_projects_without_recursion_limit_issues() {
        let storage = storage();
        let conn = storage.conn();
        let mut parent = None;
        let mut ids = Vec::new();
        for level in 1..=12 {
            let id = categories::create_category(conn, parent, &format!("第{level}层"), level)
                .expect("建分类");
            ids.push(id);
            parent = Some(id);
        }
        let (rows, _) = project(conn, &SidebarInput::default()).expect("投影");
        let chain: Vec<i32> = rows
            .iter()
            .filter(|row| row.kind == KIND_CATEGORY)
            .map(|row| row.depth)
            .collect();
        assert_eq!(chain, (1..=12).collect::<Vec<_>>(), "每层深度 +1");
        // 折叠最顶层 → 只见 1 个真实分类。
        let mut collapsed = HashSet::new();
        collapsed.insert(ids[0]);
        let (rows, _) = project(
            conn,
            &SidebarInput {
                collapsed,
                ..SidebarInput::default()
            },
        )
        .expect("投影");
        assert_eq!(
            rows.iter().filter(|row| row.kind == KIND_CATEGORY).count(),
            1
        );
    }
}
