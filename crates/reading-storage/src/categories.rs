//! `book_categories`：书架分类树（V1.1，规格 R1–R7）。
//!
//! 数据口径（`.hermes/specs/novel-reader-txt-v1.1/spec.md` §3.1）：
//!
//! - **邻接表**（`parent_id` 自引用），根下分类的 `parent_id IS NULL`；
//! - 书籍归属在 `books.category_id`，`NULL` = 未分类（与 V1 现行平铺效果一致）；
//! - 分类名**同父唯一**（`ux_categories_sibling_name` 兜底，应用层给出可理解错误）。
//!
//! 关键不变量（**必须由本模块保证，不得下沉到 UI**，变异测试 C17 盯着）：
//!
//! 1. **无环**：分类不得挂在自身或自己的子孙之下。SQL 触发器跨平台不可移植，故沿父链
//!    上溯校验（[`move_category`] / [`is_descendant_of`]）。
//! 2. **不丢书**：删除分类必须显式给出书籍去向（[`DeletePlan`]），不存在「顺手把书删了」。
//! 3. **不动原文件**：本模块只写 SQLite，**绝不**触碰用户原始 TXT（规格 R7）。

use std::collections::{HashMap, HashSet, VecDeque};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Result, StorageError};

/// 删除非空分类时，其下书籍的去向（规格 R3：用户必须显式选择）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteDestination {
    /// 移入另一分类（`target_id` 为目标分类 id）。
    MoveToCategory { target_id: i64 },
    /// 一并移出书架（`books.status = removed`；进度与书签保留，原始 TXT 不动）。
    RemoveFromShelf,
    /// 移回根（未分类，`category_id = NULL`）。
    MoveToRoot,
}

/// 分类记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort_index: i64,
}

/// 删除分类前的计划（用户确认界面的事实来源；设计稿 C4）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeletePlan {
    pub category_id: i64,
    pub category_name: String,
    /// 该分类 + 全部子孙分类的 id 集合。
    pub descendant_ids: Vec<i64>,
    /// 上述范围内的**直属**书籍数（计数口径 = 分类自身 + 子孙，规格 R3）。
    pub book_count: i64,
}

/// 名称规范化：trim；空名报可理解错误（规格 R2）。
fn normalize_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(StorageError::invalid("分类名称不能为空"));
    }
    if trimmed.chars().count() > 128 {
        return Err(StorageError::invalid("分类名称过长（最多 128 个字符）"));
    }
    Ok(trimmed.to_string())
}

/// 同父同名判定用：把 `parent_id` 归一为 i64（`None` → 0），与唯一索引口径一致。
fn parent_key(parent_id: Option<i64>) -> i64 {
    parent_id.unwrap_or(0)
}

/// 同父同名的可理解错误（规格 R2 / 设计稿 §5.2）。
fn duplicate_error(name: &str, parent_id: Option<i64>) -> StorageError {
    let where_ = match parent_id {
        None => "根目录下".to_string(),
        Some(id) => format!("分类 id={id} 下"),
    };
    StorageError::invalid(format!(
        "同一分类下已存在同名分类「{name}」（{where_}），请换一个名字"
    ))
}

/// 创建分类（规格 R2）。`parent_id = None` 表示根下。
pub fn create_category(
    conn: &Connection,
    parent_id: Option<i64>,
    name: &str,
    now: i64,
) -> Result<i64> {
    let name = normalize_name(name)?;
    if let Some(parent) = parent_id {
        if !category_exists(conn, parent)? {
            return Err(StorageError::invalid(format!(
                "父分类不存在（id={parent}）"
            )));
        }
    }
    if find_by_name(conn, parent_id, &name)?.is_some() {
        return Err(duplicate_error(&name, parent_id));
    }
    let sort_index = next_sort_index(conn, parent_id)?;
    conn.execute(
        "INSERT INTO book_categories (parent_id, name, sort_index, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![parent_id, name, sort_index, now],
    )
    .map_err(|error| match error {
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            duplicate_error(&name, parent_id)
        }
        other => StorageError::from(other),
    })?;
    Ok(conn.last_insert_rowid())
}

/// 同级末尾序号（界面本期只做「新建在末尾」，拖拽排序留待后续）。
///
/// 注意：根层（`parent_id = None`）的 SQL **不含占位符**，因此必须走 `params![]`；
/// 早前版本对根层仍传 `?1`，导致 `Wrong number of parameters`（实测踩中）。
fn next_sort_index(conn: &Connection, parent_id: Option<i64>) -> Result<i64> {
    let sql = match parent_id {
        Some(_) => {
            "SELECT IFNULL(MAX(sort_index), -1) + 1 FROM book_categories WHERE parent_id = ?1"
        }
        None => {
            "SELECT IFNULL(MAX(sort_index), -1) + 1 FROM book_categories WHERE parent_id IS NULL"
        }
    };
    let next = match parent_id {
        Some(parent) => conn.query_row(sql, params![parent], |row| row.get(0)),
        None => conn.query_row(sql, [], |row| row.get(0)),
    };
    next.map_err(StorageError::from)
}

/// 分类是否存在。
pub fn category_exists(conn: &Connection, id: i64) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM book_categories WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(StorageError::from)?;
    Ok(found.is_some())
}

/// 按 id 读取分类。
pub fn get_category(conn: &Connection, id: i64) -> Result<Option<Category>> {
    conn.query_row(
        "SELECT id, parent_id, name, sort_index FROM book_categories WHERE id = ?1",
        params![id],
        |row| {
            Ok(Category {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                sort_index: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(StorageError::from)
}

/// 同父同名查找（唯一性的应用层查询口径）。
pub fn find_by_name(conn: &Connection, parent_id: Option<i64>, name: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM book_categories WHERE IFNULL(parent_id, 0) = ?1 AND name = ?2",
        params![parent_key(parent_id), name],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(StorageError::from)
}

/// 「建分类或复用同名分类」：批量导入按目录名归类时用（规格 R6）。
pub fn create_or_reuse(
    conn: &Connection,
    parent_id: Option<i64>,
    name: &str,
    now: i64,
) -> Result<i64> {
    match find_by_name(conn, parent_id, name)? {
        Some(id) => Ok(id),
        None => create_category(conn, parent_id, name, now),
    }
}

/// 重命名分类（规格 R2）。不改父分类，也不改书籍。
pub fn rename_category(conn: &Connection, id: i64, name: &str, now: i64) -> Result<()> {
    let name = normalize_name(name)?;
    let current = get_category(conn, id)?
        .ok_or_else(|| StorageError::invalid(format!("分类不存在（id={id}）")))?;
    // 同名但**同 id** 视为未改动（否则会被唯一索引误判为冲突）。
    if name == current.name {
        return Ok(());
    }
    if find_by_name(conn, current.parent_id, &name)?.is_some() {
        return Err(duplicate_error(&name, current.parent_id));
    }
    let changed = conn
        .execute(
            "UPDATE book_categories SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, name, now],
        )
        .map_err(StorageError::from)?;
    if changed == 0 {
        return Err(StorageError::invalid(format!("分类不存在（id={id}）")));
    }
    Ok(())
}

/// 读取全部分类（按 sort_index, id 稳定排序；内存里组装成树）。
pub fn list_categories(conn: &Connection) -> Result<Vec<Category>> {
    let mut statement = conn
        .prepare(
            "SELECT id, parent_id, name, sort_index FROM book_categories ORDER BY sort_index, id",
        )
        .map_err(StorageError::from)?;
    let rows = statement
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                sort_index: row.get(3)?,
            })
        })
        .map_err(StorageError::from)?;
    let mut categories = Vec::new();
    for row in rows {
        categories.push(row.map_err(StorageError::from)?);
    }
    Ok(categories)
}

/// 树投影的一行（界面只接受本投影，**不做**任何父子/层级计算）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryNode {
    pub id: i64,
    pub name: String,
    /// 0 基深度（根下为 0）。
    pub depth: i32,
    /// 该分类**直属**书籍数（不含子孙；规格 R1 与设计稿 C1 的计数口径）。
    pub direct_book_count: i64,
    /// 该分类及全部子孙的书籍总数。
    pub total_book_count: i64,
}

/// 某分类的子孙 id 集合（含自身，规格 R5 的判定基础）。
pub fn descendant_ids(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let children = child_map(conn)?;
    let mut seen: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<i64> = VecDeque::new();
    queue.push_back(id);
    while let Some(current) = queue.pop_front() {
        if !seen.insert(current) {
            continue;
        }
        for child in children.get(&current).map(Vec::as_slice).unwrap_or(&[]) {
            queue.push_back(*child);
        }
    }
    Ok(seen.into_iter().collect())
}

/// `parent_id -> children` 邻接索引。
fn child_map(conn: &Connection) -> Result<HashMap<i64, Vec<i64>>> {
    let mut map: HashMap<i64, Vec<i64>> = HashMap::new();
    for category in list_categories(conn)? {
        if let Some(parent) = category.parent_id {
            map.entry(parent).or_default().push(category.id);
        }
    }
    Ok(map)
}

/// `candidate` 是否为 `ancestor` 的子孙（不含自身）。用于循环防护（规格 R5）。
pub fn is_descendant_of(conn: &Connection, candidate: i64, ancestor: i64) -> Result<bool> {
    if candidate == ancestor {
        return Ok(true);
    }
    Ok(descendant_ids(conn, ancestor)?.contains(&candidate))
}

/// 移动分类（规格 R4/R5）。拒绝移入自身或自己的子孙。
pub fn move_category(
    conn: &Connection,
    id: i64,
    new_parent_id: Option<i64>,
    now: i64,
) -> Result<()> {
    let category = get_category(conn, id)?
        .ok_or_else(|| StorageError::invalid(format!("分类不存在（id={id}）")))?;
    if let Some(target) = new_parent_id {
        if !category_exists(conn, target)? {
            return Err(StorageError::invalid(format!(
                "目标分类不存在（id={target}）"
            )));
        }
        if is_descendant_of(conn, target, id)? {
            // 规格 R5 / 设计稿 §5.2 固定文案；**不改挂到最近合法父级**。
            return Err(StorageError::invalid("不能把分类移动到它自己的下级分类中"));
        }
    }
    if category.parent_id == new_parent_id {
        return Ok(());
    }
    let sort_index = next_sort_index(conn, new_parent_id)?;
    let changed = conn
        .execute(
            "UPDATE book_categories SET parent_id = ?2, sort_index = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, new_parent_id, sort_index, now],
        )
        .map_err(StorageError::from)?;
    if changed == 0 {
        return Err(StorageError::invalid(format!("分类不存在（id={id}）")));
    }
    Ok(())
}

/// 把书籍移入分类（`None` = 未分类/根；规格 R4）。
pub fn move_book(conn: &Connection, book_id: i64, category_id: Option<i64>) -> Result<()> {
    if let Some(category) = category_id {
        if !category_exists(conn, category)? {
            return Err(StorageError::invalid(format!(
                "目标分类不存在（id={category}）"
            )));
        }
    }
    let changed = conn
        .execute(
            "UPDATE books SET category_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![book_id, category_id, unix_now_fallback(book_id)],
        )
        .map_err(StorageError::from)?;
    if changed == 0 {
        return Err(StorageError::invalid(format!("书籍不存在（id={book_id}）")));
    }
    Ok(())
}

/// 设定书籍分类的**显式时间戳**版本（导入路径用；[`move_book`] 走存储层默认）。
pub fn move_book_at(
    conn: &Connection,
    book_id: i64,
    category_id: Option<i64>,
    now: i64,
) -> Result<()> {
    if let Some(category) = category_id {
        if !category_exists(conn, category)? {
            return Err(StorageError::invalid(format!(
                "目标分类不存在（id={category}）"
            )));
        }
    }
    let changed = conn
        .execute(
            "UPDATE books SET category_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![book_id, category_id, now],
        )
        .map_err(StorageError::from)?;
    if changed == 0 {
        return Err(StorageError::invalid(format!("书籍不存在（id={book_id}）")));
    }
    Ok(())
}

/// `books.updated_at` 需要真实时间戳；此处避免为一个时间戳引入依赖，
/// 统一由调用方通过 [`move_book_at`] 传入。只有测试与无时间上下文的场合用本函数。
fn unix_now_fallback(_book_id: i64) -> i64 {
    0
}

/// 删除分类前的计划（该分类 + 全部子孙的书籍计数；规格 R3 / 设计稿 C4）。
pub fn delete_plan(conn: &Connection, id: i64) -> Result<DeletePlan> {
    let category = get_category(conn, id)?
        .ok_or_else(|| StorageError::invalid(format!("分类不存在（id={id}）")))?;
    let ids = descendant_ids(conn, id)?;
    let book_count = count_books_in(conn, &ids)?;
    Ok(DeletePlan {
        category_id: id,
        category_name: category.name,
        descendant_ids: ids,
        book_count,
    })
}

/// 计数：给定分类集合内的书籍数（含各集合的直接书籍）。
fn count_books_in(conn: &Connection, ids: &[i64]) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    // 分批 IN 查询（避免超过 SQLite 变量上限；分类数远小于上限，此处按 500 一批）。
    let mut total = 0;
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!("SELECT COUNT(*) FROM books WHERE category_id IN ({placeholders})");
        let count: i64 = conn
            .query_row(&sql, rusqlite::params_from_iter(chunk.iter()), |row| {
                row.get(0)
            })
            .map_err(StorageError::from)?;
        total += count;
    }
    Ok(total)
}

/// 删除分类，**必须**显式给出其下书籍去向（规格 R3：未选去向不得执行删除）。
///
/// 事务内完成「书籍按去向安置 + 删除分类行」；`books.category_id` 的外键为
/// `ON DELETE SET NULL`，此处显式安置是为了让用户的选择被**如实执行**，
/// 而不是依赖外键默认行为把书丢回根。
pub fn delete_category(
    conn: &mut Connection,
    id: i64,
    destination: DeleteDestination,
    now: i64,
) -> Result<DeleteReport> {
    let plan = delete_plan(conn, id)?;
    let ids = &plan.descendant_ids;
    let tx = conn.transaction().map_err(StorageError::from)?;

    match destination {
        DeleteDestination::MoveToCategory { target_id } => {
            if ids.contains(&target_id) {
                return Err(StorageError::invalid(
                    "目标分类不能是被删除分类自身或其下级分类",
                ));
            }
            // 借用 `tx` 而非 `conn`：事务已开启，对 `conn` 再取不可变借用会与
            // 可变事务借用冲突（E0502）。校验必须仍在事务内完成，避免 TOCTOU。
            if !category_exists(&tx, target_id)? {
                return Err(StorageError::invalid(format!(
                    "目标分类不存在（id={target_id}）"
                )));
            }
            relocate_books(&tx, ids, Some(target_id), now)?;
        }
        DeleteDestination::MoveToRoot => {
            relocate_books(&tx, ids, None, now)?;
        }
        DeleteDestination::RemoveFromShelf => {
            remove_books_from_shelf(&tx, ids, now)?;
        }
    }

    // 删除分类行及其子孙（无子孙书籍需要安置的情形下，子孙分类一并删除）。
    for descendant in ids {
        tx.execute(
            "DELETE FROM book_categories WHERE id = ?1",
            params![descendant],
        )
        .map_err(StorageError::from)?;
    }
    tx.commit().map_err(StorageError::from)?;

    Ok(DeleteReport {
        category_name: plan.category_name,
        moved_books: plan.book_count,
        deleted_categories: ids.len(),
        destination,
    })
}

/// 删除结果（用户语言；设计稿 §5.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteReport {
    pub category_name: String,
    pub moved_books: i64,
    pub deleted_categories: usize,
    pub destination: DeleteDestination,
}

impl DeleteReport {
    /// 面向用户的结果文案。
    pub fn message(&self) -> String {
        let books = match self.destination {
            DeleteDestination::MoveToCategory { target_id } => {
                format!("{} 本书已移入目标分类（id={target_id}）", self.moved_books)
            }
            DeleteDestination::MoveToRoot => {
                format!("{} 本书已移回未分类", self.moved_books)
            }
            DeleteDestination::RemoveFromShelf => {
                format!("{} 本书已移出书架（原始文件未改动）", self.moved_books)
            }
        };
        format!(
            "已删除分类「{}」（含 {} 个下级分类）；{books}",
            self.category_name, self.deleted_categories
        )
    }
}

/// 把给定分类集合内的书籍迁到目标分类。
fn relocate_books(conn: &Connection, ids: &[i64], target: Option<i64>, now: i64) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut moved = 0;
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "UPDATE books SET category_id = ?1, updated_at = ?2 WHERE category_id IN ({placeholders})"
        );
        let mut params: Vec<rusqlite::types::Value> = Vec::with_capacity(chunk.len() + 2);
        params.push(match target {
            Some(value) => value.into(),
            None => rusqlite::types::Value::Null,
        });
        params.push(now.into());
        for id in chunk {
            params.push((*id).into());
        }
        moved += conn
            .execute(&sql, rusqlite::params_from_iter(params.iter()))
            .map_err(StorageError::from)?;
    }
    Ok(moved)
}

/// 把给定分类集合内的书籍移出书架（软移除；进度与书签保留，原始 TXT 不动）。
fn remove_books_from_shelf(conn: &Connection, ids: &[i64], now: i64) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut moved = 0;
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "UPDATE books SET status = 'removed', updated_at = ?1 WHERE status != 'removed' AND category_id IN ({placeholders})"
        );
        let mut params: Vec<rusqlite::types::Value> = Vec::with_capacity(chunk.len() + 1);
        params.push(now.into());
        for id in chunk {
            params.push((*id).into());
        }
        moved += conn
            .execute(&sql, rusqlite::params_from_iter(params.iter()))
            .map_err(StorageError::from)?;
    }
    Ok(moved)
}

/// 分类树投影（含直属/总计计数；界面直接消费，**不在 UI 侧算层级**）。
///
/// 以 BFS 输出，天然按「父在子前」排序；`depth` 用于缩进。
pub fn project_tree(conn: &Connection) -> Result<Vec<CategoryNode>> {
    let categories = list_categories(conn)?;
    let known: HashSet<i64> = categories.iter().map(|item| item.id).collect();
    let mut direct: HashMap<i64, i64> = HashMap::new();
    let mut unclassified = 0i64;
    {
        let mut statement = conn
            .prepare(
                "SELECT COALESCE(category_id, 0), COUNT(*) FROM books GROUP BY COALESCE(category_id, 0)",
            )
            .map_err(StorageError::from)?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(StorageError::from)?;
        for row in rows {
            let (category_id, count) = row.map_err(StorageError::from)?;
            if category_id == 0 {
                unclassified = count;
            } else {
                direct.insert(category_id, count);
            }
        }
    }

    let children = child_map(conn)?;
    // 孤儿子节点（父分类不存在，理论上不应出现）按根处理，保证投影不丢节点、不死循环。
    let roots: Vec<&Category> = categories
        .iter()
        .filter(|item| match item.parent_id {
            None => true,
            Some(parent) => !known.contains(&parent),
        })
        .collect();

    let mut nodes: Vec<CategoryNode> = Vec::with_capacity(categories.len());
    let mut queue: VecDeque<(&Category, i32)> = roots
        .into_iter()
        .map(|item| (item, 0))
        .collect::<VecDeque<_>>();
    let mut visited: HashSet<i64> = HashSet::new();
    while let Some((category, depth)) = queue.pop_front() {
        if !visited.insert(category.id) {
            // 环已被 [`move_category`] 拦在写入侧；此处兜底避免投影死循环。
            continue;
        }
        let own = direct.get(&category.id).copied().unwrap_or(0);
        let descendant_total: i64 = descendants_of_in_memory(&children, category.id)
            .into_iter()
            .map(|id| direct.get(&id).copied().unwrap_or(0))
            .sum();
        nodes.push(CategoryNode {
            id: category.id,
            name: category.name.clone(),
            depth,
            direct_book_count: own,
            total_book_count: own + descendant_total,
        });
        if let Some(kids) = children.get(&category.id) {
            let by_id: HashMap<i64, &Category> =
                categories.iter().map(|item| (item.id, item)).collect();
            for child_id in kids {
                if let Some(child) = by_id.get(child_id) {
                    queue.push_back((child, depth + 1));
                }
            }
        }
    }

    // 未分类计数通过 `project_tree_with_uncategorized` 暴露；此处仅返回分类节点。
    let _ = unclassified;
    Ok(nodes)
}

/// 内存邻接表下的全部子孙（不含自身）。
fn descendants_of_in_memory(children: &HashMap<i64, Vec<i64>>, root: i64) -> Vec<i64> {
    let mut seen: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<i64> = VecDeque::new();
    queue.push_back(root);
    while let Some(current) = queue.pop_front() {
        for child in children.get(&current).map(Vec::as_slice).unwrap_or(&[]) {
            if seen.insert(*child) {
                queue.push_back(*child);
            }
        }
    }
    seen.into_iter().collect()
}

/// 树投影 + 未分类书籍数（侧栏根节点用；规格 R1）。
pub fn project_tree_with_uncategorized(conn: &Connection) -> Result<(Vec<CategoryNode>, i64)> {
    let nodes = project_tree(conn)?;
    let unclassified: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM books WHERE category_id IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(StorageError::from)?;
    Ok((nodes, unclassified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::books::{import_book, BookImport, KIND_DESKTOP_PATH};
    use crate::Storage;

    fn storage() -> Storage {
        let mut storage = Storage::open_in_memory().expect("内存库");
        storage.migrate().expect("迁移");
        storage
    }

    fn seed_book(conn: &Connection, locator: &str, now: i64) -> i64 {
        import_book(
            conn,
            &BookImport {
                display_name: locator.to_string(),
                original_name: "book.txt".to_string(),
                source_kind: KIND_DESKTOP_PATH.to_string(),
                source_locator: locator.to_string(),
                source_size: 128,
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

    fn book_category(conn: &Connection, book_id: i64) -> Option<i64> {
        conn.query_row(
            "SELECT category_id FROM books WHERE id = ?1",
            params![book_id],
            |row| row.get(0),
        )
        .optional()
        .expect("读回")
        .flatten()
    }

    #[test]
    fn database_rejects_same_named_sibling_even_when_bypassing_application_checks() {
        // C5 的守卫：唯一索引（`ux_categories_sibling_name`）是**数据库级兜底**，
        // 应用层报错可被绕过（例如将来新增的写入路径、或直接改库的迁移脚本）。
        // 早前版本的 `duplicate_name_under_same_parent...` 只验证应用层报错，
        // 删掉唯一索引后仍然全绿 —— 判据空转（实测踩中），故补这条直写库的测试。
        let storage = storage();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO book_categories (parent_id, name, sort_index, created_at, updated_at)
             VALUES (NULL, '科幻', 0, 1, 1)",
            [],
        )
        .expect("首个根下分类");

        // 直写库插入同名兄弟 → 必须被唯一索引拒绝。
        let bypassed = conn.execute(
            "INSERT INTO book_categories (parent_id, name, sort_index, created_at, updated_at)
             VALUES (NULL, '科幻', 1, 2, 2)",
            [],
        );
        assert!(
            bypassed.is_err(),
            "绕过应用层直写库时，同父同名仍必须被唯一索引拒绝（唯一索引已被删除？）"
        );

        // 不同父下同名 → 必须允许（唯一索引不能误伤）。
        let parent = create_category(conn, None, "小说", 3).expect("父分类");
        assert!(
            conn.execute(
                "INSERT INTO book_categories (parent_id, name, sort_index, created_at, updated_at)
                 VALUES (?1, '科幻', 0, 4, 4)",
                params![parent],
            )
            .is_ok(),
            "不同父下的同名分类应当允许"
        );
    }

    #[test]
    fn sibling_name_uniqueness_covers_root_level_null_parents() {
        // C6 的守卫：根层分类的 `parent_id IS NULL`，
        // 唯一索引用 `IFNULL(parent_id, 0)` 归一后才覆盖到根层。
        // 若写成裸 `parent_id`，SQLite 认为 `NULL != NULL` → 根层同名可共存（实测判据）。
        let storage = storage();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO book_categories (parent_id, name, sort_index, created_at, updated_at)
             VALUES (NULL, '历史', 0, 1, 1)",
            [],
        )
        .expect("首个根下分类");
        let duplicate = conn.execute(
            "INSERT INTO book_categories (parent_id, name, sort_index, created_at, updated_at)
             VALUES (NULL, '历史', 1, 2, 2)",
            [],
        );
        assert!(
            duplicate.is_err(),
            "根层同名分类必须互斥（唯一索引的 IFNULL 归一被去掉了？）"
        );
    }

    #[test]
    fn three_level_nesting_is_created_and_projected_in_parent_first_order() {
        let storage = storage();
        let conn = storage.conn();
        let root = create_category(conn, None, "科幻", 1).expect("根下分类");
        let child = create_category(conn, Some(root), "中国科幻", 2).expect("子分类");
        let grand = create_category(conn, Some(child), "科幻文学", 3).expect("孙分类");
        assert!(create_category(conn, None, "历史", 4).is_ok());

        let (nodes, unclassified) = project_tree_with_uncategorized(conn).expect("树投影");
        assert_eq!(unclassified, 0);
        assert_eq!(nodes.len(), 4);
        let ids: Vec<i64> = nodes.iter().map(|node| node.id).collect();
        let position = |id: i64| ids.iter().position(|item| *item == id).expect("存在");
        assert!(position(root) < position(child), "父必须在子之前");
        assert!(position(child) < position(grand), "父必须在孙之前");
        assert_eq!(
            nodes.iter().find(|n| n.id == grand).map(|n| n.depth),
            Some(2)
        );
    }

    #[test]
    fn duplicate_name_under_same_parent_is_rejected_with_readable_error() {
        let storage = storage();
        let conn = storage.conn();
        let root = create_category(conn, None, "科幻", 1).expect("分类");
        create_category(conn, Some(root), "中国科幻", 2).expect("子分类");

        let error = create_category(conn, Some(root), " 中国科幻 ", 3).expect_err("同名应失败");
        let message = error.to_string();
        assert!(message.contains("已存在同名分类"), "实际文案：{message}");
        assert!(message.contains("换一个名字"), "实际文案：{message}");
        // 不同父下同名允许。
        assert!(create_category(conn, None, "中国科幻", 4).is_ok());
    }

    #[test]
    fn moving_category_into_own_descendant_is_rejected() {
        let storage = storage();
        let conn = storage.conn();
        let a = create_category(conn, None, "A", 1).expect("A");
        let b = create_category(conn, Some(a), "B", 2).expect("B");
        let c = create_category(conn, Some(b), "C", 3).expect("C");

        for target in [a, b, c] {
            let error = move_category(conn, a, Some(target), 9).expect_err("循环应被拒");
            assert!(
                error
                    .to_string()
                    .contains("不能把分类移动到它自己的下级分类中"),
                "实际文案：{}",
                error
            );
        }
        // 结构不变。
        assert_eq!(
            get_category(conn, a)
                .expect("读 A")
                .expect("存在")
                .parent_id,
            None
        );
        assert_eq!(
            get_category(conn, b)
                .expect("读 B")
                .expect("存在")
                .parent_id,
            Some(a)
        );
    }

    #[test]
    fn moving_category_to_sibling_parent_is_allowed() {
        let storage = storage();
        let conn = storage.conn();
        let a = create_category(conn, None, "A", 1).expect("A");
        let b = create_category(conn, None, "B", 2).expect("B");
        let c = create_category(conn, Some(a), "C", 3).expect("C");

        move_category(conn, c, Some(b), 4).expect("移动到兄弟父下");
        assert_eq!(
            get_category(conn, c)
                .expect("读 C")
                .expect("存在")
                .parent_id,
            Some(b)
        );
    }

    #[test]
    fn rename_to_own_current_name_is_noop_not_conflict() {
        let storage = storage();
        let conn = storage.conn();
        let a = create_category(conn, None, "科幻", 1).expect("A");
        rename_category(conn, a, "科幻", 2).expect("同名同 id 应为无操作");
        rename_category(conn, a, "  科幻  ", 3).expect("trim 后同名仍为无操作");
        assert_eq!(
            get_category(conn, a).expect("读").expect("存在").name,
            "科幻"
        );
    }

    #[test]
    fn book_moves_between_categories_and_back_to_root() {
        let storage = storage();
        let conn = storage.conn();
        let book = seed_book(conn, "/tmp/a.txt", 1);
        let category = create_category(conn, None, "科幻", 2).expect("分类");
        let other = create_category(conn, None, "历史", 3).expect("分类");

        assert_eq!(book_category(conn, book), None);
        move_book_at(conn, book, Some(category), 4).expect("移入");
        assert_eq!(book_category(conn, book), Some(category));
        move_book_at(conn, book, Some(other), 5).expect("改移");
        assert_eq!(book_category(conn, book), Some(other));
        move_book_at(conn, book, None, 6).expect("移回根");
        assert_eq!(book_category(conn, book), None);
        assert!(
            move_book_at(conn, book, Some(9_999), 7).is_err(),
            "目标不存在应失败"
        );
    }

    #[test]
    fn delete_plan_counts_books_of_descendants() {
        let storage = storage();
        let conn = storage.conn();
        let a = create_category(conn, None, "A", 1).expect("A");
        let b = create_category(conn, Some(a), "B", 2).expect("B");
        let first = seed_book(conn, "/tmp/1.txt", 3);
        let second = seed_book(conn, "/tmp/2.txt", 4);
        move_book_at(conn, first, Some(a), 5).expect("归类");
        move_book_at(conn, second, Some(b), 6).expect("归类");

        let plan = delete_plan(conn, a).expect("删除计划");
        assert_eq!(plan.book_count, 2, "A + 其子孙 B 的书籍都应计入");
        assert_eq!(plan.descendant_ids.len(), 2);
    }

    #[test]
    fn delete_moves_books_to_chosen_category_only() {
        let mut storage = storage();
        let first = seed_book(storage.conn(), "/tmp/1.txt", 1);
        let second = seed_book(storage.conn(), "/tmp/2.txt", 2);
        let source = create_category(storage.conn(), None, "待删", 3).expect("分类");
        let target = create_category(storage.conn(), None, "目标", 4).expect("分类");
        move_book_at(storage.conn(), first, Some(source), 5).expect("归类");
        move_book_at(storage.conn(), second, Some(source), 6).expect("归类");

        let report = delete_category(
            storage.conn_mut(),
            source,
            DeleteDestination::MoveToCategory { target_id: target },
            7,
        )
        .expect("删除");
        assert_eq!(report.moved_books, 2);
        assert_eq!(book_category(storage.conn(), first), Some(target));
        assert_eq!(book_category(storage.conn(), second), Some(target));
        assert!(get_category(storage.conn(), source).expect("读").is_none());
    }

    #[test]
    fn delete_can_remove_books_from_shelf_without_touching_files() {
        let mut storage = storage();
        let book = seed_book(storage.conn(), "/tmp/1.txt", 1);
        let source = create_category(storage.conn(), None, "待删", 2).expect("分类");
        move_book_at(storage.conn(), book, Some(source), 3).expect("归类");

        delete_category(
            storage.conn_mut(),
            source,
            DeleteDestination::RemoveFromShelf,
            4,
        )
        .expect("删除");

        let status: String = storage
            .conn()
            .query_row(
                "SELECT status FROM books WHERE id = ?1",
                params![book],
                |row| row.get(0),
            )
            .expect("读状态");
        assert_eq!(status, "removed", "应为软移除而非硬删除");
        assert_eq!(book_category(storage.conn(), book), None);
    }

    #[test]
    fn delete_rejects_moving_into_own_descendant_as_destination() {
        let mut storage = storage();
        let a = create_category(storage.conn(), None, "A", 1).expect("A");
        let b = create_category(storage.conn(), Some(a), "B", 2).expect("B");
        assert!(delete_category(
            storage.conn_mut(),
            a,
            DeleteDestination::MoveToCategory { target_id: b },
            3
        )
        .is_err());
        assert!(
            get_category(storage.conn(), a).expect("读").is_some(),
            "失败后不应删分类"
        );
    }

    #[test]
    fn create_or_reuse_reuses_same_named_category() {
        let storage = storage();
        let conn = storage.conn();
        let first = create_or_reuse(conn, None, "科幻", 1).expect("建");
        let second = create_or_reuse(conn, None, "科幻", 2).expect("复用");
        assert_eq!(first, second);
        assert_eq!(list_categories(conn).expect("列表").len(), 1);
    }

    #[test]
    fn orphan_category_cannot_be_written_and_projection_tolerates_missing_parent() {
        let storage = storage();
        let conn = storage.conn();

        // 第一道：`foreign_keys = ON` 时，父指向不存在 id 的写入**必须**被拒
        // （连接层 PRAGMA 见 `connection.rs`），所以「孤儿」在正常路径上产生不了。
        let rejected = conn.execute(
            "INSERT INTO book_categories (id, parent_id, name, sort_index, created_at, updated_at)
             VALUES (99, 12345, '孤儿', 0, 1, 1)",
            [],
        );
        assert!(
            rejected.is_err(),
            "外键约束应阻止写出孤儿分类（parent 指向不存在的 id）"
        );

        // 第二道：即便绕过外键（历史遗留库可能存在这种行），投影也**不得丢节点**
        // 或死循环 —— 兜底为根下节点。
        conn.pragma_update(None, "foreign_keys", "OFF")
            .expect("临时关外键");
        conn.execute(
            "INSERT INTO book_categories (id, parent_id, name, sort_index, created_at, updated_at)
             VALUES (99, 12345, '孤儿', 0, 1, 1)",
            [],
        )
        .expect("关闭外键后插入孤儿");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("恢复外键");

        let nodes = project_tree(conn).expect("投影");
        assert_eq!(nodes.len(), 1, "孤儿节点不得从投影中消失");
        assert_eq!(nodes[0].depth, 0, "孤儿应被兜底为根下节点");
        assert_eq!(nodes[0].name, "孤儿");
    }

    #[test]
    fn direct_and_total_book_counts_differ_for_parents() {
        let storage = storage();
        let conn = storage.conn();
        let parent = create_category(conn, None, "科幻", 1).expect("父");
        let child = create_category(conn, Some(parent), "中国科幻", 2).expect("子");
        let first = seed_book(conn, "/tmp/1.txt", 3);
        let second = seed_book(conn, "/tmp/2.txt", 4);
        let third = seed_book(conn, "/tmp/3.txt", 5);
        move_book_at(conn, first, Some(parent), 6).expect("归类");
        move_book_at(conn, second, Some(child), 7).expect("归类");
        move_book_at(conn, third, Some(child), 8).expect("归类");

        let nodes = project_tree(conn).expect("投影");
        let parent_node = nodes.iter().find(|n| n.id == parent).expect("父节点");
        let child_node = nodes.iter().find(|n| n.id == child).expect("子节点");
        assert_eq!(parent_node.direct_book_count, 1);
        assert_eq!(parent_node.total_book_count, 3);
        assert_eq!(child_node.direct_book_count, 2);
        assert_eq!(child_node.total_book_count, 2);
    }
}

/// 作用域种类（V1.1；与 ABI `nr_list_books_in_scope` 的 `scope_kind` 一致）。
pub const SCOPE_ALL: i32 = 0;
pub const SCOPE_UNCATEGORIZED: i32 = 1;
pub const SCOPE_CATEGORY: i32 = 2;

/// 某作用域下**在架**书籍的 id 列表（V1.1；界面按分类筛选书架的**唯一**真相）。
///
/// - [`SCOPE_ALL`]：全部在架书籍（不过滤）；
/// - [`SCOPE_UNCATEGORIZED`]：`category_id IS NULL` 的在架书籍；
/// - [`SCOPE_CATEGORY`]：`category_id` 指定分类**及其全部子孙**下的在架书籍
///   —— 与侧栏「合计计数」同一口径。
///
/// **子树展开在核心侧完成**（复用 [`descendant_ids`]）：界面只做集合成员判断，
/// 不得自己遍历父子链 —— 否则桌面与 Android 的筛选口径会各自演化。
///
/// 顺序按 `id`：调用方自行建集合，不依赖顺序。
pub fn books_in_scope(conn: &Connection, scope_kind: i32, category_id: i64) -> Result<Vec<i64>> {
    match scope_kind {
        SCOPE_ALL => {
            let mut stmt = conn.prepare("SELECT id FROM books WHERE status <> ?1 ORDER BY id")?;
            let rows = stmt.query_map(params![crate::books::STATUS_REMOVED], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        }
        SCOPE_UNCATEGORIZED => {
            let mut stmt = conn.prepare(
                "SELECT id FROM books WHERE status <> ?1 AND category_id IS NULL ORDER BY id",
            )?;
            let rows = stmt.query_map(params![crate::books::STATUS_REMOVED], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        }
        SCOPE_CATEGORY => {
            // ⚠️ `descendant_ids` **已包含自身**（BFS 起点即 `id`）——
            // 不要再 push(category_id)，否则该分类会被查两次、书籍重复出现。
            let scope_ids = descendant_ids(conn, category_id)?;
            let mut ids = Vec::new();
            for id in scope_ids {
                let mut stmt = conn.prepare(
                    "SELECT id FROM books WHERE status <> ?1 AND category_id = ?2 ORDER BY id",
                )?;
                let rows =
                    stmt.query_map(params![crate::books::STATUS_REMOVED, id], |row| row.get(0))?;
                for row in rows {
                    ids.push(row?);
                }
            }
            // 去重 + 排序：同一本书只可能属于一个分类，但作用域集合本身不保证无重复。
            ids.sort_unstable();
            ids.dedup();
            Ok(ids)
        }
        _ => Err(StorageError::InvalidArgument(format!(
            "未知的作用域种类：{scope_kind}"
        ))),
    }
}

/// 移动 / 删除时的目标选项（V1.1）。
///
/// `id = -1` 表示「未分类」（不是一个真实分类）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveTarget {
    pub id: i64,
    pub name: String,
    /// 层级（用于缩进显示）。
    pub depth: i32,
    /// 是否**可选**：自身与全部子孙为 `false`（选了就会成环 ⇒ 树投影死循环）。
    ///
    /// 刻意**不隐藏**而只标记不可选 —— 让用户看见「为什么不能选」，
    /// 比整条消失更好解释（与桌面 `category_ops::move_targets` 同一口径）。
    pub selectable: bool,
    /// 含子孙的在架书籍数（仅展示用；未分类行是未分类书籍数）。
    pub total_count: i64,
}

/// 合法的移动 / 删除目标（V1.1；规格 R4/R5）。
///
/// 返回「未分类」+ **全部分类**，并把 `source_id` 自身与其全部子孙标为不可选。
/// **环判定在核心完成** —— 界面不得自行遍历父子链判断可达性。
pub fn move_targets(conn: &Connection, source_id: i64) -> Result<Vec<MoveTarget>> {
    let (nodes, unclassified) = project_tree_with_uncategorized(conn)?;
    let blocked: HashSet<i64> = descendant_ids(conn, source_id)?.into_iter().collect();

    let mut rows = Vec::with_capacity(nodes.len() + 1);
    rows.push(MoveTarget {
        id: -1,
        name: "未分类".to_string(),
        depth: 0,
        selectable: true,
        total_count: unclassified,
    });
    for node in &nodes {
        rows.push(MoveTarget {
            id: node.id,
            name: node.name.clone(),
            depth: node.depth,
            selectable: !blocked.contains(&node.id),
            total_count: node.total_book_count,
        });
    }
    Ok(rows)
}
