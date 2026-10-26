//! 阶段 0 跨设备同步：可导出/可合并的目录包（`state.json` + `books/<sha256>.txt`）。
//!
//! # 边界
//!
//! - 本模块**不联网**，只做本地文件与 SQLite 的交换；WebDAV 等传输层在后续阶段。
//! - **不直接同步 `library.db`**：SQLite 的 WAL、设备本地 `rowid`、布局缓存都不可跨设备
//!   合并。这里同步的是「逻辑记录」与「按内容哈希寻址的书籍副本」。
//! - 本地数据库仍是每台设备的真相；导入时通过既有仓储 API 写入，并保留用户原始 TXT
//!   的只读语义（只读源文件、只写应用自己的副本）。
//! - 稳定身份放在数据库同目录的 `sync-state.json` 边车文件中：设备 ID、单调序号、
//!   本地 id ↔ 同步 UUID / 内容哈希的映射。边车文件不是用户数据，丢失后可由内容哈希
//!   与指纹重建；不修改既有 schema，避免为同步功能改写冻结迁移。
//!
//! # 目录包
//!
//! ```text
//! <bundle>/
//!   state.json
//!   books/
//!     <sha256>.txt
//! ```
//!
//! `state.json` 里的书籍、进度、书签、分类、设置都是纯值，不含任何设备本地路径
//! （书籍文件用内容哈希引用）。导入方自己决定副本落在哪里。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{Result, StorageError};
use crate::validate::{
    ensure_max_len, ensure_non_negative, ensure_short_text, MAX_ENCODING_BYTES,
    MAX_FINGERPRINT_BYTES, MAX_NAME_BYTES, MAX_TITLE_BYTES,
};
use crate::{bookmarks, books, categories, connection, meta, progress, settings};

/// 边车状态文件名（与 `library.db` 同目录）。
pub const SYNC_STATE_FILE: &str = "sync-state.json";
/// 同步包状态文件名。
pub const BUNDLE_STATE_FILE: &str = "state.json";
/// 同步包书籍目录名。
pub const BUNDLE_BOOKS_DIR: &str = "books";
/// 同步包格式标识。
pub const BUNDLE_MAGIC: &str = "novel-reader-sync";
/// 同步包 schema 版本。
pub const SYNC_SCHEMA_VERSION: u32 = 1;
/// `state.json` 大小上限（防止误选大文件/恶意输入）。
pub const MAX_BUNDLE_STATE_BYTES: u64 = 16 * 1024 * 1024;
/// 单次同步的最大实体数（防止恶意/误操作把 UI 拖死）。
pub const MAX_BUNDLE_ENTITIES: usize = 100_000;

/// 导出选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    /// 是否把可读的书籍文件复制进同步包。默认 `true`。
    pub include_books: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_books: true,
        }
    }
}

/// 远端进度较新时的处理策略。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProgressPolicy {
    /// 保留本地进度（默认；避免同步静默覆盖读者当前位置）。
    #[default]
    KeepLocal,
    /// 使用远端较新的进度。
    RemoteIfNewer,
}

/// 导入选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    /// 新书副本落盘目录；`None` 表示数据库同目录下的 `sync-books/`。
    pub library_dir: Option<PathBuf>,
    /// 书籍包目录；`None` 表示 `dir/books/`。
    ///
    /// WebDAV 等传输层会把多台设备的 `state.json` 放在不同临时目录，但共享同一份
    /// 书籍缓存；此字段让合并逻辑复用同一份 `<sha256>.txt`。
    pub books_dir: Option<PathBuf>,
    /// 远端进度比本地新时的处理策略。
    pub progress_policy: ProgressPolicy,
    /// 是否下载/创建远端书籍副本（false = 只合并已有本地书的元数据/进度）。
    pub import_books: bool,
    /// 是否合并阅读进度。
    pub apply_progress: bool,
    /// 是否合并书签。
    pub apply_bookmarks: bool,
    /// 是否合并全局设置。
    pub apply_settings: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            library_dir: None,
            books_dir: None,
            progress_policy: ProgressPolicy::KeepLocal,
            import_books: true,
            apply_progress: true,
            apply_bookmarks: true,
            apply_settings: true,
        }
    }
}

/// 同步结果（同时作为 FFI/UI 的可读摘要）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    /// 导出/导入方的设备 ID（Android WebDAV 用它命名 `state-<device>.json`）。
    #[serde(default)]
    pub device_id: String,
    pub books_exported: usize,
    pub book_files_copied: usize,
    pub progress_exported: usize,
    pub bookmarks_exported: usize,
    pub categories_exported: usize,
    pub books_created: usize,
    pub books_updated: usize,
    pub books_missing: usize,
    /// 因本地「仅本地」策略而跳过的远端书籍数。
    #[serde(default)]
    pub books_skipped_excluded: usize,
    pub progress_applied: usize,
    pub progress_skipped_local_newer: usize,
    /// 远端更新，但按策略保留本地而跳过的条数。
    pub progress_skipped_remote_newer: usize,
    pub bookmarks_created: usize,
    pub bookmarks_updated: usize,
    pub bookmarks_skipped_local_newer: usize,
    pub categories_created: usize,
    pub categories_updated: usize,
    pub settings_applied: bool,
    /// 导入后本地可用的书籍摘要（Android 宿主用它把同步书加入书架；桌面可忽略）。
    #[serde(default)]
    pub books: Vec<SyncBookSummary>,
    pub conflicts: Vec<String>,
}

/// 导入结果中的书籍摘要（不携带正文；路径是应用自己的副本）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncBookSummary {
    pub id: i64,
    pub display_name: String,
    pub source_locator: String,
    pub source_size: i64,
    pub status: String,
}

impl SyncReport {
    /// 面向界面的单行摘要。
    pub fn summary(&self) -> String {
        format!(
            "书籍 +{} / 更新 {} / 缺失 {}；进度 {}；书签 +{} / 更新 {}；分类 +{}；设置{}",
            self.books_created,
            self.books_updated,
            self.books_missing,
            self.progress_applied,
            self.bookmarks_created,
            self.bookmarks_updated,
            self.categories_created,
            if self.settings_applied {
                "已更新"
            } else {
                "未变"
            }
        )
    }

    /// JSON 摘要（FFI/UI 共用；序列化失败时退化为 `{}`，不 panic）。
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// 边车中的书籍身份。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BookIdentity {
    sync_id: String,
    #[serde(default)]
    local_id: Option<i64>,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    fingerprint: Option<String>,
    #[serde(default)]
    locator: String,
}

/// 边车中的对端状态（目前只记录最后导入的序号）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PeerState {
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    imported_at: i64,
}

/// 边车状态。字段带默认值，便于后续加性演进。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalState {
    schema: u32,
    device_id: String,
    #[serde(default)]
    seq: u64,
    /// key = `id:<local book id>`；同内容多路径会复用同一 sync_id。
    #[serde(default)]
    books: BTreeMap<String, BookIdentity>,
    /// key = local category id；value = sync UUID。
    #[serde(default)]
    categories: BTreeMap<String, String>,
    /// key = local bookmark id；value = sync UUID。
    #[serde(default)]
    bookmarks: BTreeMap<String, String>,
    #[serde(default)]
    peers: BTreeMap<String, PeerState>,
}

impl LocalState {
    fn new(device_id: String) -> Self {
        Self {
            schema: SYNC_SCHEMA_VERSION,
            device_id,
            seq: 0,
            books: BTreeMap::new(),
            categories: BTreeMap::new(),
            bookmarks: BTreeMap::new(),
            peers: BTreeMap::new(),
        }
    }
}

/// 同步边车：设备身份 + 稳定 ID 映射。
#[derive(Debug)]
pub struct SyncStore {
    path: Option<PathBuf>,
    state: LocalState,
}

impl SyncStore {
    /// 从数据库路径推导边车路径；数据库不存在/内存库时使用无落盘状态。
    pub fn open(db_path: Option<&Path>) -> Result<Self> {
        let path = db_path
            .and_then(Path::parent)
            .map(|parent| parent.join(SYNC_STATE_FILE));
        let state = match &path {
            Some(path) if path.is_file() => {
                let bytes = fs::read(path).map_err(|error| {
                    StorageError::io(path, format!("读取同步边车失败: {error}"))
                })?;
                let mut state: LocalState = serde_json::from_slice(&bytes).map_err(|error| {
                    StorageError::invalid(format!("同步边车不是合法 JSON: {error}"))
                })?;
                if state.schema != SYNC_SCHEMA_VERSION {
                    return Err(StorageError::invalid(format!(
                        "同步边车 schema={}，当前只支持 {}",
                        state.schema, SYNC_SCHEMA_VERSION
                    )));
                }
                if state.device_id.trim().is_empty() {
                    state.device_id = Uuid::new_v4().to_string();
                }
                state
            }
            Some(_) => LocalState::new(Uuid::new_v4().to_string()),
            None => LocalState::new(Uuid::new_v4().to_string()),
        };
        Ok(Self { path, state })
    }

    /// 当前设备 ID。
    pub fn device_id(&self) -> &str {
        &self.state.device_id
    }

    /// 当前同步序号。
    pub fn seq(&self) -> u64 {
        self.state.seq
    }

    /// 落盘边车（原子写）。
    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_json_atomic(path, &self.state)
    }

    /// 为书籍分配/复用稳定 sync_id（供 UI 标记「仅本地」使用）。
    pub fn book_sync_id(&mut self, book: &books::BookRecord, content_hash: Option<&str>) -> String {
        self.ensure_book_identity(book, content_hash)
    }

    /// 为分类分配/复用稳定 sync_id。
    pub fn category_sync_id(&mut self, local_id: i64) -> String {
        self.ensure_category_sync_id(local_id)
    }

    fn ensure_category_sync_id(&mut self, local_id: i64) -> String {
        let key = local_id.to_string();
        if let Some(existing) = self.state.categories.get(&key) {
            return existing.clone();
        }
        let sync_id = Uuid::new_v4().to_string();
        self.state.categories.insert(key, sync_id.clone());
        sync_id
    }

    fn ensure_bookmark_sync_id(&mut self, local_id: i64) -> String {
        let key = local_id.to_string();
        if let Some(existing) = self.state.bookmarks.get(&key) {
            return existing.clone();
        }
        let sync_id = Uuid::new_v4().to_string();
        self.state.bookmarks.insert(key, sync_id.clone());
        sync_id
    }

    fn ensure_book_identity(
        &mut self,
        book: &books::BookRecord,
        content_hash: Option<&str>,
    ) -> String {
        // 1) 本地 id 精确命中（同一数据库内的正常路径）。
        let by_local = self
            .state
            .books
            .iter()
            .find(|(_, identity)| identity.local_id == Some(book.id))
            .map(|(key, identity)| (key.clone(), identity.clone()));
        // 2) 指纹命中（数据库重建、路径变化）。
        let by_fingerprint = || {
            book.source_fingerprint.as_ref().and_then(|fingerprint| {
                self.state.books.iter().find_map(|(key, identity)| {
                    (identity.fingerprint.as_deref() == Some(fingerprint.as_str()))
                        .then(|| (key.clone(), identity.clone()))
                })
            })
        };
        // 3) 定位符命中（无指纹的旧记录）。
        let by_locator = || {
            self.state.books.iter().find_map(|(key, identity)| {
                (identity.locator == book.source_locator).then(|| (key.clone(), identity.clone()))
            })
        };

        let found = by_local.or_else(by_fingerprint).or_else(by_locator);
        if let Some((key, mut identity)) = found {
            identity.local_id = Some(book.id);
            identity.content_hash = content_hash
                .map(ToOwned::to_owned)
                .or(identity.content_hash);
            identity.fingerprint = book.source_fingerprint.clone();
            identity.locator = book.source_locator.clone();
            let sync_id = identity.sync_id.clone();
            self.state.books.insert(key, identity);
            return sync_id;
        }

        let sync_id = Uuid::new_v4().to_string();
        let key = format!("id:{}", book.id);
        self.state.books.insert(
            key,
            BookIdentity {
                sync_id: sync_id.clone(),
                local_id: Some(book.id),
                content_hash: content_hash.map(ToOwned::to_owned),
                fingerprint: book.source_fingerprint.clone(),
                locator: book.source_locator.clone(),
            },
        );
        sync_id
    }

    fn local_id_for_sync_book(&self, sync_id: &str) -> Option<i64> {
        self.state.books.values().find_map(|identity| {
            (identity.sync_id == sync_id)
                .then_some(identity.local_id)
                .flatten()
        })
    }

    fn local_id_for_sync_category(&self, sync_id: &str) -> Option<i64> {
        self.state.categories.iter().find_map(|(local, remote)| {
            (remote == sync_id)
                .then(|| local.parse::<i64>().ok())
                .flatten()
        })
    }

    fn local_id_for_sync_bookmark(&self, sync_id: &str) -> Option<i64> {
        self.state.bookmarks.iter().find_map(|(local, remote)| {
            (remote == sync_id)
                .then(|| local.parse::<i64>().ok())
                .flatten()
        })
    }

    fn remember_book_identity(
        &mut self,
        local_id: i64,
        sync_id: &str,
        content_hash: Option<&str>,
        fingerprint: Option<&str>,
        locator: &str,
    ) {
        let key = format!("id:{local_id}");
        self.state.books.insert(
            key,
            BookIdentity {
                sync_id: sync_id.to_string(),
                local_id: Some(local_id),
                content_hash: content_hash.map(ToOwned::to_owned),
                fingerprint: fingerprint.map(ToOwned::to_owned),
                locator: locator.to_string(),
            },
        );
    }

    fn category_sync_id_or_none(&self, local_id: i64) -> Option<String> {
        self.state.categories.get(&local_id.to_string()).cloned()
    }
}

// ---------------------------------------------------------------------------
// 本地「仅本地」排除（不新增表：按 sync_id 存 app_meta 键）
// ---------------------------------------------------------------------------

/// 当前本地排除集合。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncExclusions {
    pub book_sync_ids: HashSet<String>,
    pub book_hashes: HashSet<String>,
    pub category_sync_ids: HashSet<String>,
}

impl SyncExclusions {
    pub fn is_empty(&self) -> bool {
        self.book_sync_ids.is_empty()
            && self.book_hashes.is_empty()
            && self.category_sync_ids.is_empty()
    }
}

/// 标记/取消「仅本地」书籍。`value` 记录内容哈希（可为空），供其他设备同内容识别。
pub fn set_book_excluded(
    store: &mut SyncStore,
    conn: &Connection,
    book_id: i64,
    excluded: bool,
) -> Result<()> {
    let book = books::get_book(conn, book_id)?.ok_or(StorageError::BookNotFound { book_id })?;
    let hash = readable_source_path(&book).and_then(|path| sha256_file(&path).ok());
    let sync_id = store.ensure_book_identity(&book, hash.as_deref());
    let key = format!("{EXCLUDE_BOOK_PREFIX}{sync_id}");
    if excluded {
        meta::set(
            conn,
            &key,
            hash.as_deref().unwrap_or(""),
            connection::now_unix(),
        )?;
    } else {
        meta::remove(conn, &key)?;
    }
    Ok(())
}

/// 当前书籍是否「仅本地」。
pub fn is_book_excluded(store: &SyncStore, conn: &Connection, book_id: i64) -> Result<bool> {
    let Some(sync_id) = store
        .state
        .books
        .values()
        .find(|identity| identity.local_id == Some(book_id))
        .map(|identity| identity.sync_id.clone())
    else {
        return Ok(false);
    };
    Ok(meta::get(conn, &format!("{EXCLUDE_BOOK_PREFIX}{sync_id}"))?.is_some())
}

/// 标记/取消「仅本地」分类（子分类在导出/导入时一并排除）。
pub fn set_category_excluded(
    store: &mut SyncStore,
    conn: &Connection,
    category_id: i64,
    excluded: bool,
) -> Result<()> {
    if !categories::category_exists(conn, category_id)? {
        return Err(StorageError::invalid(format!(
            "分类不存在（id={category_id}）"
        )));
    }
    let sync_id = store.ensure_category_sync_id(category_id);
    let key = format!("{EXCLUDE_CATEGORY_PREFIX}{sync_id}");
    if excluded {
        meta::set(conn, &key, "1", connection::now_unix())?;
    } else {
        meta::remove(conn, &key)?;
    }
    Ok(())
}

pub fn is_category_excluded(
    store: &SyncStore,
    conn: &Connection,
    category_id: i64,
) -> Result<bool> {
    let Some(sync_id) = store.category_sync_id_or_none(category_id) else {
        return Ok(false);
    };
    Ok(meta::get(conn, &format!("{EXCLUDE_CATEGORY_PREFIX}{sync_id}"))?.is_some())
}

/// 读取全部本地排除。
pub fn load_exclusions(conn: &Connection) -> Result<SyncExclusions> {
    let mut out = SyncExclusions::default();
    if !meta::table_exists(conn)? {
        return Ok(out);
    }
    let mut stmt = conn.prepare("SELECT key, value FROM app_meta WHERE key LIKE ?1")?;
    let rows = stmt.query_map([format!("{EXCLUDE_BOOK_PREFIX}%")], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (key, value) = row?;
        if let Some(sync_id) = key.strip_prefix(EXCLUDE_BOOK_PREFIX) {
            out.book_sync_ids.insert(sync_id.to_string());
            if !value.is_empty() {
                out.book_hashes.insert(value);
            }
        }
    }
    let mut stmt = conn.prepare("SELECT key FROM app_meta WHERE key LIKE ?1")?;
    let rows = stmt.query_map([format!("{EXCLUDE_CATEGORY_PREFIX}%")], |row| {
        row.get::<_, String>(0)
    })?;
    for row in rows {
        let key = row?;
        if let Some(sync_id) = key.strip_prefix(EXCLUDE_CATEGORY_PREFIX) {
            out.category_sync_ids.insert(sync_id.to_string());
        }
    }
    Ok(out)
}

/// 供界面投影：当前被排除的本地书籍 id。
pub fn excluded_book_ids(store: &SyncStore, conn: &Connection) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for identity in store.state.books.values() {
        let Some(local_id) = identity.local_id else {
            continue;
        };
        if meta::get(conn, &format!("{EXCLUDE_BOOK_PREFIX}{}", identity.sync_id))?.is_some() {
            out.push(local_id);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// 供界面投影：当前被排除的本地分类 id。
pub fn excluded_category_ids(store: &SyncStore, conn: &Connection) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for (local, sync_id) in &store.state.categories {
        let Ok(local_id) = local.parse::<i64>() else {
            continue;
        };
        if meta::get(conn, &format!("{EXCLUDE_CATEGORY_PREFIX}{sync_id}"))?.is_some() {
            out.push(local_id);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

// ---------------------------------------------------------------------------
// 同步包数据（纯值；可被未来 WebDAV 传输层直接复用）
// ---------------------------------------------------------------------------

/// 同步包根对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBundle {
    pub magic: String,
    pub schema: u32,
    pub device_id: String,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub exported_at: i64,
    #[serde(default)]
    pub books: Vec<SyncBook>,
    #[serde(default)]
    pub progress: Vec<SyncProgress>,
    #[serde(default)]
    pub bookmarks: Vec<SyncBookmark>,
    #[serde(default)]
    pub categories: Vec<SyncCategory>,
    #[serde(default)]
    pub settings: Option<SyncSettings>,
    /// 预留：删除传播。阶段 0 只导出/应用书籍的 `removed` 状态，不生成墓碑。
    #[serde(default)]
    pub tombstones: Vec<SyncTombstone>,
}

/// 同步包中的书籍。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBook {
    pub sync_id: String,
    #[serde(default)]
    pub content_hash: Option<String>,
    pub display_name: String,
    pub original_name: String,
    #[serde(default)]
    pub encoding: String,
    #[serde(default)]
    pub encoding_confidence: i32,
    #[serde(default)]
    pub encoding_user_override: bool,
    #[serde(default)]
    pub source_fingerprint: Option<String>,
    #[serde(default)]
    pub source_size: i64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub category_sync_id: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// 同步包中的阅读进度（不含页码；页码跨设备无效）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncProgress {
    pub book_sync_id: String,
    #[serde(default)]
    pub content_unit_id: i64,
    #[serde(default)]
    pub chapter_index: i64,
    #[serde(default)]
    pub byte_offset: i64,
    #[serde(default)]
    pub nearby_fingerprint: Option<String>,
    #[serde(default)]
    pub index_version: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub last_opened_at: i64,
}

/// 同步包中的书签。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBookmark {
    pub sync_id: String,
    pub book_sync_id: String,
    #[serde(default)]
    pub content_unit_id: i64,
    #[serde(default)]
    pub chapter_index: i64,
    #[serde(default)]
    pub byte_offset: i64,
    #[serde(default)]
    pub nearby_fingerprint: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// 同步包中的分类。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCategory {
    pub sync_id: String,
    #[serde(default)]
    pub parent_sync_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub sort_index: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// 同步包中的全局设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSettings {
    pub font_family: String,
    pub font_weight: i64,
    pub font_size_milli: i64,
    pub line_spacing_milli: i64,
    pub paragraph_spacing_milli: i64,
    pub first_line_indent_milli: i64,
    pub margin_x_milli: i64,
    pub margin_y_milli: i64,
    pub theme: String,
    pub operation_mode: String,
    pub default_encoding: String,
    pub updated_at: i64,
}

/// 本地「仅本地」书籍排除键前缀（value = 内容哈希或空）。
pub const EXCLUDE_BOOK_PREFIX: &str = "sync.exclude.book.";
/// 本地「仅本地」分类排除键前缀。
pub const EXCLUDE_CATEGORY_PREFIX: &str = "sync.exclude.category.";

/// 预留的墓碑（阶段 0 不生成，但导入端可安全忽略/应用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncTombstone {
    pub entity: String,
    pub sync_id: String,
    #[serde(default)]
    pub deleted_at: i64,
}

// ---------------------------------------------------------------------------
// 导出
// ---------------------------------------------------------------------------

/// 导出同步包到目录。
pub fn export_bundle(
    store: &mut SyncStore,
    conn: &Connection,
    dir: &Path,
    options: &ExportOptions,
) -> Result<SyncReport> {
    fs::create_dir_all(dir)
        .map_err(|error| StorageError::io(dir, format!("创建同步包目录失败: {error}")))?;
    let books_dir = dir.join(BUNDLE_BOOKS_DIR);
    fs::create_dir_all(&books_dir)
        .map_err(|error| StorageError::io(&books_dir, format!("创建书籍目录失败: {error}")))?;

    let mut report = SyncReport {
        device_id: store.device_id().to_string(),
        ..SyncReport::default()
    };
    let now = connection::now_unix();

    let exclusions = load_exclusions(conn)?;
    let category_rows = categories::list_categories(conn)?;
    let mut category_sync_ids: HashMap<i64, String> = HashMap::new();
    for category in &category_rows {
        category_sync_ids.insert(category.id, store.ensure_category_sync_id(category.id));
    }
    // 分类排除沿父子链向下继承。
    let mut excluded_categories: HashSet<i64> = category_rows
        .iter()
        .filter(|category| {
            category_sync_ids
                .get(&category.id)
                .is_some_and(|sync_id| exclusions.category_sync_ids.contains(sync_id))
        })
        .map(|category| category.id)
        .collect();
    loop {
        let mut added = false;
        for category in &category_rows {
            if let Some(parent) = category.parent_id {
                if excluded_categories.contains(&parent) && excluded_categories.insert(category.id)
                {
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }

    let mut categories_out = Vec::with_capacity(category_rows.len());
    for category in &category_rows {
        if excluded_categories.contains(&category.id) {
            continue;
        }
        let sync_id = category_sync_ids
            .get(&category.id)
            .cloned()
            .unwrap_or_default();
        let parent_sync_id = category
            .parent_id
            .and_then(|parent| category_sync_ids.get(&parent).cloned());
        categories_out.push(SyncCategory {
            sync_id,
            parent_sync_id,
            name: category.name.clone(),
            sort_index: category.sort_index,
            created_at: now,
            updated_at: now,
        });
    }
    report.categories_exported = categories_out.len();

    let book_categories: HashMap<i64, Option<i64>> =
        books::list_book_categories(conn)?.into_iter().collect();
    let book_rows = books::list_all_books(conn)?;
    let mut books_out = Vec::with_capacity(book_rows.len());
    let mut book_sync_ids: HashMap<i64, String> = HashMap::new();

    for book in &book_rows {
        let source_path = readable_source_path(book);
        let content_hash = source_path
            .as_ref()
            .and_then(|path| match sha256_file(path) {
                Ok(hash) => Some(hash),
                Err(error) => {
                    report
                        .conflicts
                        .push(format!("book-file:{}:{}", book.id, error));
                    None
                }
            });
        let sync_id = store.ensure_book_identity(book, content_hash.as_deref());
        let category_id = book_categories.get(&book.id).copied().flatten();
        let excluded = exclusions.book_sync_ids.contains(&sync_id)
            || content_hash
                .as_ref()
                .is_some_and(|hash| exclusions.book_hashes.contains(hash))
            || category_id.is_some_and(|id| excluded_categories.contains(&id));
        if excluded {
            continue;
        }
        if options.include_books {
            if let Some(path) = &source_path {
                match copy_book_to_bundle(path, &books_dir) {
                    Ok(_) => report.book_files_copied += 1,
                    Err(error) => report
                        .conflicts
                        .push(format!("book-file:{}:{}", book.id, error)),
                }
            }
        }
        book_sync_ids.insert(book.id, sync_id.clone());
        let category_sync_id = category_id.and_then(|id| category_sync_ids.get(&id).cloned());
        books_out.push(SyncBook {
            sync_id,
            content_hash,
            display_name: book.display_name.clone(),
            original_name: book.original_name.clone(),
            encoding: book.encoding.clone(),
            encoding_confidence: book.encoding_confidence,
            encoding_user_override: book.encoding_user_override,
            source_fingerprint: book.source_fingerprint.clone(),
            source_size: book.source_size,
            status: book.status.clone(),
            category_sync_id,
            created_at: book.created_at,
            updated_at: book.updated_at,
        });
    }
    report.books_exported = books_out.len();

    let mut progress_out = Vec::new();
    for row in list_all_progress(conn)? {
        if let Some(sync_id) = book_sync_ids.get(&row.book_id) {
            progress_out.push(SyncProgress {
                book_sync_id: sync_id.clone(),
                content_unit_id: row.content_unit_id,
                chapter_index: row.chapter_index,
                byte_offset: row.byte_offset,
                nearby_fingerprint: row.nearby_fingerprint,
                index_version: row.index_version,
                updated_at: row.updated_at,
                last_opened_at: row.last_opened_at,
            });
        }
    }
    report.progress_exported = progress_out.len();

    let mut bookmarks_out = Vec::new();
    for row in list_all_bookmarks(conn)? {
        let Some(book_sync_id) = book_sync_ids.get(&row.book_id) else {
            continue;
        };
        let sync_id = store.ensure_bookmark_sync_id(row.id);
        bookmarks_out.push(SyncBookmark {
            sync_id,
            book_sync_id: book_sync_id.clone(),
            content_unit_id: row.content_unit_id,
            chapter_index: row.chapter_index,
            byte_offset: row.byte_offset,
            nearby_fingerprint: row.nearby_fingerprint,
            title: row.title,
            created_at: row.created_at,
            updated_at: row.updated_at,
        });
    }
    report.bookmarks_exported = bookmarks_out.len();

    let current_settings = settings::load_settings(conn)?;
    let settings_out = SyncSettings {
        font_family: current_settings.font_family.clone(),
        font_weight: current_settings.font_weight,
        font_size_milli: current_settings.font_size_milli,
        line_spacing_milli: current_settings.line_spacing_milli,
        paragraph_spacing_milli: current_settings.paragraph_spacing_milli,
        first_line_indent_milli: current_settings.first_line_indent_milli,
        margin_x_milli: current_settings.margin_x_milli,
        margin_y_milli: current_settings.margin_y_milli,
        theme: current_settings.theme.clone(),
        operation_mode: current_settings.operation_mode.clone(),
        default_encoding: current_settings.default_encoding.clone(),
        updated_at: current_settings.updated_at,
    };

    let next_seq = store.state.seq.saturating_add(1);
    let bundle = SyncBundle {
        magic: BUNDLE_MAGIC.to_string(),
        schema: SYNC_SCHEMA_VERSION,
        device_id: store.state.device_id.clone(),
        seq: next_seq,
        exported_at: now,
        books: books_out,
        progress: progress_out,
        bookmarks: bookmarks_out,
        categories: categories_out,
        settings: Some(settings_out),
        tombstones: Vec::new(),
    };
    validate_bundle(&bundle)?;
    write_json_atomic(&dir.join(BUNDLE_STATE_FILE), &bundle)?;
    store.state.seq = next_seq;
    store.save()?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// 导入
// ---------------------------------------------------------------------------

/// 从目录包合并到本地数据库。
pub fn import_bundle(
    store: &mut SyncStore,
    conn: &Connection,
    dir: &Path,
    options: &ImportOptions,
) -> Result<SyncReport> {
    let bundle = read_bundle(dir)?;
    validate_bundle(&bundle)?;
    let now = connection::now_unix();
    let library_dir = options
        .library_dir
        .clone()
        .or_else(|| {
            store
                .path
                .as_ref()
                .and_then(|path| path.parent())
                .map(|parent| parent.join("sync-books"))
        })
        .unwrap_or_else(|| dir.join("imported-books"));
    fs::create_dir_all(&library_dir).map_err(|error| {
        StorageError::io(&library_dir, format!("创建书籍副本目录失败: {error}"))
    })?;
    let books_dir = options
        .books_dir
        .clone()
        .unwrap_or_else(|| dir.join(BUNDLE_BOOKS_DIR));

    let mut report = SyncReport {
        device_id: store.device_id().to_string(),
        ..SyncReport::default()
    };
    let exclusions = load_exclusions(conn)?;

    // ---- 分类（先解析父链，再复用/创建；最后按远端时间更新名称与父级）----
    let mut remote_categories: HashMap<&str, &SyncCategory> = HashMap::new();
    for category in &bundle.categories {
        if remote_categories
            .insert(category.sync_id.as_str(), category)
            .is_some()
        {
            return Err(StorageError::invalid(format!(
                "同步包分类 sync_id 重复: {}",
                category.sync_id
            )));
        }
    }
    let mut excluded_remote_categories: HashSet<String> = bundle
        .categories
        .iter()
        .filter(|category| exclusions.category_sync_ids.contains(&category.sync_id))
        .map(|category| category.sync_id.clone())
        .collect();
    loop {
        let mut added = false;
        for category in &bundle.categories {
            if let Some(parent) = &category.parent_sync_id {
                if excluded_remote_categories.contains(parent)
                    && excluded_remote_categories.insert(category.sync_id.clone())
                {
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }
    let mut category_map: BTreeMap<String, i64> = BTreeMap::new();
    let mut resolving: HashSet<String> = HashSet::new();
    for category in &bundle.categories {
        if excluded_remote_categories.contains(&category.sync_id) {
            continue;
        }
        resolve_category(
            store,
            conn,
            category,
            &remote_categories,
            &mut category_map,
            &mut resolving,
            &mut report,
            now,
        )?;
    }
    // 先落盘映射：即使后续书签/设置合并失败，重试也不会重复建分类。
    store.save()?;

    // ---- 书籍 ----
    let mut remote_books: HashMap<&str, &SyncBook> = HashMap::new();
    for book in &bundle.books {
        if remote_books.insert(book.sync_id.as_str(), book).is_some() {
            return Err(StorageError::invalid(format!(
                "同步包书籍 sync_id 重复: {}",
                book.sync_id
            )));
        }
    }
    let mut book_map: BTreeMap<String, i64> = BTreeMap::new();
    let mut content_conflicts: HashSet<String> = HashSet::new();
    for book in &bundle.books {
        let category_excluded = book
            .category_sync_id
            .as_ref()
            .is_some_and(|id| excluded_remote_categories.contains(id));
        if exclusions.book_sync_ids.contains(&book.sync_id)
            || book
                .content_hash
                .as_ref()
                .is_some_and(|hash| exclusions.book_hashes.contains(hash))
            || category_excluded
        {
            report.books_skipped_excluded += 1;
            continue;
        }
        match resolve_book(
            store,
            conn,
            &books_dir,
            &library_dir,
            book,
            &mut report,
            now,
            options.import_books,
        ) {
            Ok((Some(local_id), content_conflict)) => {
                if content_conflict {
                    content_conflicts.insert(book.sync_id.clone());
                }
                if let Some(category_sync_id) = &book.category_sync_id {
                    if let Some(category_id) = category_map.get(category_sync_id) {
                        let current = book_category_id(conn, local_id)?;
                        if current != Some(*category_id) {
                            categories::move_book_at(conn, local_id, Some(*category_id), now)?;
                        }
                    }
                }
                book_map.insert(book.sync_id.clone(), local_id);
            }
            Ok((None, _)) => {}
            Err(error) => {
                report
                    .conflicts
                    .push(format!("book:{}:{error}", book.sync_id));
            }
        }
    }
    // 同理：书籍映射先落盘，避免失败重试重复导入副本。
    store.save()?;

    // ---- 进度 ----
    for remote in &bundle.progress {
        if !options.apply_progress {
            break;
        }
        if content_conflicts.contains(&remote.book_sync_id) {
            report.conflicts.push(format!(
                "progress:{}:book-content-differs",
                remote.book_sync_id
            ));
            continue;
        }
        let Some(local_id) = book_map.get(&remote.book_sync_id).copied() else {
            report
                .conflicts
                .push(format!("progress:{}:missing-book", remote.book_sync_id));
            continue;
        };
        merge_progress(conn, local_id, remote, options.progress_policy, &mut report)?;
    }

    // ---- 书签 ----
    for remote in &bundle.bookmarks {
        if !options.apply_bookmarks {
            break;
        }
        if content_conflicts.contains(&remote.book_sync_id) {
            report
                .conflicts
                .push(format!("bookmark:{}:book-content-differs", remote.sync_id));
            continue;
        }
        let Some(local_id) = book_map.get(&remote.book_sync_id).copied() else {
            report
                .conflicts
                .push(format!("bookmark:{}:missing-book", remote.sync_id));
            continue;
        };
        merge_bookmark(store, conn, local_id, remote, &mut report)?;
    }

    // ---- 设置（逐字段独立同步；这里按整行时间戳取新，避免把设备相关布局强行套用）----
    if let Some(remote) = &bundle.settings {
        if !options.apply_settings {
            // 设置不同步时直接跳过（不影响书籍/进度合并）。
        } else {
            let local = settings::load_settings(conn)?;
            if remote.updated_at > local.updated_at {
                let next = settings::ReaderSettings {
                    font_family: remote.font_family.clone(),
                    font_weight: remote.font_weight,
                    font_size_milli: remote.font_size_milli,
                    line_spacing_milli: remote.line_spacing_milli,
                    paragraph_spacing_milli: remote.paragraph_spacing_milli,
                    first_line_indent_milli: remote.first_line_indent_milli,
                    margin_x_milli: remote.margin_x_milli,
                    margin_y_milli: remote.margin_y_milli,
                    theme: remote.theme.clone(),
                    operation_mode: remote.operation_mode.clone(),
                    default_encoding: remote.default_encoding.clone(),
                    updated_at: remote.updated_at,
                };
                next.validate()?;
                settings::save_settings(conn, &next, remote.updated_at.max(1))?;
                // `save_settings` 可能把 updated_at 写成传入时间；显式保持远端值。
                conn.execute(
                    "UPDATE reader_settings SET updated_at = ?2 WHERE id = ?1",
                    params![settings::SETTINGS_ROW_ID, remote.updated_at],
                )?;
                report.settings_applied = true;
            }
        }
    }

    // ---- 对端序号 ----
    store.state.peers.insert(
        bundle.device_id.clone(),
        PeerState {
            seq: bundle.seq,
            imported_at: now,
        },
    );
    store.save()?;

    Ok(report)
}

#[allow(clippy::too_many_arguments)] // 同步合并的上下文参数；拆结构体反而让调用点更难核对
fn resolve_category(
    store: &mut SyncStore,
    conn: &Connection,
    remote: &SyncCategory,
    remote_categories: &HashMap<&str, &SyncCategory>,
    category_map: &mut BTreeMap<String, i64>,
    resolving: &mut HashSet<String>,
    report: &mut SyncReport,
    now: i64,
) -> Result<i64> {
    if let Some(local_id) = category_map.get(&remote.sync_id) {
        return Ok(*local_id);
    }
    if !resolving.insert(remote.sync_id.clone()) {
        return Err(StorageError::invalid(format!(
            "同步包分类存在环: {}",
            remote.sync_id
        )));
    }

    let parent_id = match &remote.parent_sync_id {
        Some(parent_sync_id) => match remote_categories.get(parent_sync_id.as_str()) {
            // 父分类不在包内（例如另一端只导出了子集）：挂根，避免丢节点。
            None => None,
            Some(parent) => Some(resolve_category(
                store,
                conn,
                parent,
                remote_categories,
                category_map,
                resolving,
                report,
                now,
            )?),
        },
        None => None,
    };

    let existing_local = store
        .local_id_for_sync_category(&remote.sync_id)
        .filter(|id| categories::get_category(conn, *id).ok().flatten().is_some());
    let local_id = match existing_local {
        Some(id) => {
            let current = categories::get_category(conn, id)?
                .ok_or_else(|| StorageError::invalid(format!("分类不存在（id={id}）")))?;
            if current.name != remote.name {
                categories::rename_category(conn, id, &remote.name, now)?;
                report.categories_updated += 1;
            }
            if current.parent_id != parent_id {
                categories::move_category(conn, id, parent_id, now)?;
                report.categories_updated += 1;
            }
            id
        }
        None => {
            // 同父同名视为同一分类（两端各自创建过同名分类）。
            let same_name = categories::find_by_name(conn, parent_id, &remote.name)?;
            match same_name {
                Some(id) => id,
                None => {
                    let id = categories::create_category(conn, parent_id, &remote.name, now)?;
                    report.categories_created += 1;
                    id
                }
            }
        }
    };

    store
        .state
        .categories
        .insert(local_id.to_string(), remote.sync_id.clone());
    category_map.insert(remote.sync_id.clone(), local_id);
    resolving.remove(&remote.sync_id);
    Ok(local_id)
}

#[allow(clippy::too_many_arguments)] // 与 resolve_category 同理：显式传参让合并上下文在调用点可核对
fn resolve_book(
    store: &mut SyncStore,
    conn: &Connection,
    books_dir: &Path,
    library_dir: &Path,
    remote: &SyncBook,
    report: &mut SyncReport,
    now: i64,
    import_books: bool,
) -> Result<(Option<i64>, bool)> {
    // 1) 边车中已有的 sync_id（且本地行仍存在）。
    let mut local_id = store
        .local_id_for_sync_book(&remote.sync_id)
        .filter(|id| books::get_book(conn, *id).ok().flatten().is_some());

    // 2) 内容哈希命中（同一本书从另一台设备同步过）。
    if local_id.is_none() {
        if let Some(hash) = &remote.content_hash {
            local_id = store.state.books.values().find_map(|identity| {
                (identity.content_hash.as_deref() == Some(hash.as_str()))
                    .then_some(identity.local_id)
                    .flatten()
                    .filter(|id| books::get_book(conn, *id).ok().flatten().is_some())
            });
        }
    }

    // 3) 既有指纹命中（本机已导入过同一文件）。
    if local_id.is_none() {
        if let Some(fingerprint) = &remote.source_fingerprint {
            local_id = books::find_book_by_fingerprint(conn, fingerprint)?.map(|book| book.id);
        }
    }

    let bundle_book = remote
        .content_hash
        .as_ref()
        .map(|hash| books_dir.join(format!("{hash}.txt")))
        .filter(|path| path.is_file());

    let local_content_hash = store
        .state
        .books
        .values()
        .find(|identity| identity.sync_id == remote.sync_id)
        .and_then(|identity| identity.content_hash.clone());
    let content_conflict = matches!(
        (&remote.content_hash, &local_content_hash),
        (Some(remote_hash), Some(local_hash)) if remote_hash != local_hash
    );
    if content_conflict {
        report.conflicts.push(format!(
            "book:{}:content-hash-differs (local={}, remote={})",
            remote.sync_id,
            local_content_hash.as_deref().unwrap_or("<none>"),
            remote.content_hash.as_deref().unwrap_or("<none>")
        ));
    }

    let mut created = false;
    let mut local = match local_id {
        Some(id) => Some(
            books::get_book(conn, id)?
                .ok_or_else(|| StorageError::invalid(format!("书籍不存在（id={id}）")))?,
        ),
        None => None,
    };

    if local.is_none() && !import_books {
        return Ok((None, content_conflict));
    }
    if local.is_none() {
        if let Some(source) = &bundle_book {
            let hash = remote
                .content_hash
                .as_deref()
                .ok_or_else(|| StorageError::invalid("book 缺少 content_hash"))?;
            let dest = library_dir.join(format!("{hash}.txt"));
            copy_verified(source, &dest, hash)?;
            let import = books::BookImport {
                display_name: remote.display_name.clone(),
                original_name: remote.original_name.clone(),
                source_kind: books::KIND_PRIVATE_COPY.to_string(),
                source_locator: path_to_utf8(&dest)?,
                source_size: fs::metadata(&dest)
                    .map_err(|error| StorageError::io(&dest, error.to_string()))?
                    .len() as i64,
                source_modified_hint: fs::metadata(&dest)
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|delta| delta.as_secs() as i64),
                source_fingerprint: remote.source_fingerprint.clone(),
                encoding: non_empty_encoding(&remote.encoding),
                encoding_confidence: remote.encoding_confidence.clamp(0, 1000),
                encoding_user_override: remote.encoding_user_override,
            };
            let outcome = books::import_book(conn, &import, now)?;
            if remote.status == books::STATUS_REMOVED {
                books::remove_book_from_shelf(conn, outcome.book_id, now)?;
            } else {
                books::set_book_status(conn, outcome.book_id, books::STATUS_READY, now)?;
            }
            report.books_created += 1;
            created = true;
            local = books::get_book(conn, outcome.book_id)?;
        } else if remote.status != books::STATUS_REMOVED {
            // 没有书籍文件：建占位记录，保留进度/书签/分类；文件到达后可重新导入。
            let locator = library_dir.join(format!("{}.txt", remote.sync_id));
            let import = books::BookImport {
                display_name: remote.display_name.clone(),
                original_name: remote.original_name.clone(),
                source_kind: books::KIND_PRIVATE_COPY.to_string(),
                source_locator: path_to_utf8(&locator)?,
                source_size: remote.source_size.max(0),
                source_modified_hint: None,
                source_fingerprint: remote.source_fingerprint.clone(),
                encoding: non_empty_encoding(&remote.encoding),
                encoding_confidence: remote.encoding_confidence.clamp(0, 1000),
                encoding_user_override: remote.encoding_user_override,
            };
            let outcome = books::import_book(conn, &import, now)?;
            books::set_book_status(conn, outcome.book_id, books::STATUS_MISSING, now)?;
            report.books_missing += 1;
            report.books_created += 1;
            local = books::get_book(conn, outcome.book_id)?;
        }
    }

    let Some(book) = local else {
        // 远端就是 removed 且本地没有：不需要建记录。
        return Ok((None, content_conflict));
    };
    let effective_id = book.id;

    // 已有记录：只按远端更新时间更新展示名；状态/分类单独处理。
    if !created && remote.updated_at > book.updated_at && remote.display_name != book.display_name {
        books::rename_book(conn, effective_id, &remote.display_name, now)?;
        report.books_updated += 1;
    }
    if content_conflict {
        // 同一 sync_id 但内容不同：不按远端状态移除/恢复本地书，避免误删另一版本。
    } else if remote.status == books::STATUS_REMOVED && book.status != books::STATUS_REMOVED {
        books::remove_book_from_shelf(conn, effective_id, now)?;
        report.books_updated += 1;
    } else if remote.status != books::STATUS_REMOVED
        && book.status == books::STATUS_MISSING
        && bundle_book.is_some()
    {
        books::set_book_status(conn, effective_id, books::STATUS_READY, now)?;
        report.books_updated += 1;
    } else if remote.status != books::STATUS_REMOVED && book.status == books::STATUS_REMOVED {
        let next = if bundle_book.is_some() || readable_source_path(&book).is_some() {
            books::STATUS_READY
        } else {
            books::STATUS_MISSING
        };
        books::set_book_status(conn, effective_id, next, now)?;
        report.books_updated += 1;
    }

    let remote_hash = remote.content_hash.clone();
    let content_hash = if content_conflict {
        local_content_hash
    } else {
        remote_hash.or(local_content_hash)
    };
    store.remember_book_identity(
        effective_id,
        &remote.sync_id,
        content_hash.as_deref(),
        remote.source_fingerprint.as_deref(),
        &book.source_locator,
    );
    if !report.books.iter().any(|entry| entry.id == effective_id) {
        if let Some(current) = books::get_book(conn, effective_id)? {
            report.books.push(SyncBookSummary {
                id: effective_id,
                display_name: current.display_name.clone(),
                source_locator: current.source_locator.clone(),
                source_size: current.source_size,
                status: current.status.clone(),
            });
        }
    }
    Ok((Some(effective_id), content_conflict))
}

fn merge_progress(
    conn: &Connection,
    local_book_id: i64,
    remote: &SyncProgress,
    policy: ProgressPolicy,
    report: &mut SyncReport,
) -> Result<()> {
    let local = progress::load_progress(conn, local_book_id)?;
    if let Some(local) = &local {
        if remote.updated_at < local.updated_at {
            report.progress_skipped_local_newer += 1;
            return Ok(());
        }
        if remote.updated_at > local.updated_at && policy == ProgressPolicy::KeepLocal {
            report.progress_skipped_remote_newer += 1;
            return Ok(());
        }
        if remote.updated_at == local.updated_at
            && (remote.byte_offset != local.byte_offset
                || remote.chapter_index != local.chapter_index
                || remote.content_unit_id != local.content_unit_id)
        {
            report.conflicts.push(format!(
                "progress:{}:equal-timestamp-different-position",
                remote.book_sync_id
            ));
            return Ok(());
        }
        if remote.updated_at == local.updated_at {
            return Ok(());
        }
    }
    let book = books::get_book(conn, local_book_id)?;
    if let Some(book) = &book {
        if book.source_size > 0 && remote.byte_offset > book.source_size {
            report.conflicts.push(format!(
                "progress:{}:offset-out-of-range",
                remote.book_sync_id
            ));
            return Ok(());
        }
    }
    progress::save_progress(
        conn,
        &progress::ProgressInput {
            book_id: local_book_id,
            content_unit_id: remote.content_unit_id.max(0),
            chapter_index: remote.chapter_index.max(0),
            byte_offset: remote.byte_offset.max(0),
            nearby_fingerprint: remote.nearby_fingerprint.clone(),
            last_page_hint: None,
            index_version: remote.index_version.max(0),
        },
        remote.updated_at.max(1),
    )?;
    if remote.last_opened_at > 0 {
        conn.execute(
            "UPDATE reading_progress SET last_opened_at = ?2 WHERE book_id = ?1",
            params![local_book_id, remote.last_opened_at],
        )?;
    }
    report.progress_applied += 1;
    Ok(())
}

fn merge_bookmark(
    store: &mut SyncStore,
    conn: &Connection,
    local_book_id: i64,
    remote: &SyncBookmark,
    report: &mut SyncReport,
) -> Result<()> {
    ensure_max_len("bookmarks.title", &remote.title, MAX_TITLE_BYTES)?;
    if let Some(fingerprint) = &remote.nearby_fingerprint {
        ensure_max_len(
            "bookmarks.nearby_fingerprint",
            fingerprint,
            MAX_FINGERPRINT_BYTES,
        )?;
    }
    let existing = store
        .local_id_for_sync_bookmark(&remote.sync_id)
        .filter(|id| bookmarks::get_bookmark(conn, *id).ok().flatten().is_some());
    match existing {
        Some(id) => {
            let current = bookmarks::get_bookmark(conn, id)?
                .ok_or_else(|| StorageError::invalid(format!("书签不存在（id={id}）")))?;
            if remote.updated_at <= current.updated_at {
                report.bookmarks_skipped_local_newer += 1;
                return Ok(());
            }
            conn.execute(
                "UPDATE bookmarks SET content_unit_id = ?2, chapter_index = ?3, byte_offset = ?4, \
                 nearby_fingerprint = ?5, title = ?6, updated_at = ?7 WHERE id = ?1",
                params![
                    id,
                    remote.content_unit_id.max(0),
                    remote.chapter_index.max(0),
                    remote.byte_offset.max(0),
                    remote.nearby_fingerprint,
                    remote.title,
                    remote.updated_at
                ],
            )?;
            report.bookmarks_updated += 1;
        }
        None => {
            let id = bookmarks::create_bookmark(
                conn,
                &bookmarks::BookmarkInput {
                    book_id: local_book_id,
                    content_unit_id: remote.content_unit_id.max(0),
                    chapter_index: remote.chapter_index.max(0),
                    byte_offset: remote.byte_offset.max(0),
                    nearby_fingerprint: remote.nearby_fingerprint.clone(),
                    title: remote.title.clone(),
                },
                remote.created_at.max(1),
            )?;
            conn.execute(
                "UPDATE bookmarks SET created_at = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, remote.created_at.max(1), remote.updated_at.max(1)],
            )?;
            store
                .state
                .bookmarks
                .insert(id.to_string(), remote.sync_id.clone());
            report.bookmarks_created += 1;
        }
    }
    // 书签映射逐本落盘：失败重试不会重复创建同一远端书签。
    store.save()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 校验、哈希、I/O 辅助
// ---------------------------------------------------------------------------

fn read_bundle(dir: &Path) -> Result<SyncBundle> {
    let state_path = dir.join(BUNDLE_STATE_FILE);
    let metadata = fs::metadata(&state_path)
        .map_err(|error| StorageError::io(&state_path, format!("读取同步包状态失败: {error}")))?;
    if metadata.len() > MAX_BUNDLE_STATE_BYTES {
        return Err(StorageError::invalid(format!(
            "同步包 state.json 过大（{} 字节，上限 {}）",
            metadata.len(),
            MAX_BUNDLE_STATE_BYTES
        )));
    }
    let bytes = fs::read(&state_path)
        .map_err(|error| StorageError::io(&state_path, format!("读取同步包状态失败: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| StorageError::invalid(format!("同步包 state.json 不是合法 JSON: {error}")))
}

fn validate_bundle(bundle: &SyncBundle) -> Result<()> {
    if bundle.magic != BUNDLE_MAGIC {
        return Err(StorageError::invalid("同步包 magic 不匹配"));
    }
    if bundle.schema != SYNC_SCHEMA_VERSION {
        return Err(StorageError::invalid(format!(
            "同步包 schema={}，当前只支持 {}",
            bundle.schema, SYNC_SCHEMA_VERSION
        )));
    }
    if bundle.device_id.trim().is_empty() {
        return Err(StorageError::invalid("同步包缺少 device_id"));
    }
    for (name, count) in [
        ("books", bundle.books.len()),
        ("progress", bundle.progress.len()),
        ("bookmarks", bundle.bookmarks.len()),
        ("categories", bundle.categories.len()),
        ("tombstones", bundle.tombstones.len()),
    ] {
        if count > MAX_BUNDLE_ENTITIES {
            return Err(StorageError::invalid(format!(
                "同步包 {name} 数量 {count} 超过上限 {MAX_BUNDLE_ENTITIES}"
            )));
        }
    }

    let mut book_ids = HashSet::new();
    for book in &bundle.books {
        validate_sync_id("books.sync_id", &book.sync_id)?;
        if !book_ids.insert(book.sync_id.as_str()) {
            return Err(StorageError::invalid(format!(
                "同步包书籍 sync_id 重复: {}",
                book.sync_id
            )));
        }
        ensure_short_text("books.display_name", &book.display_name, MAX_NAME_BYTES)?;
        ensure_short_text("books.original_name", &book.original_name, MAX_NAME_BYTES)?;
        ensure_max_len("books.encoding", &book.encoding, MAX_ENCODING_BYTES)?;
        ensure_non_negative("books.source_size", book.source_size)?;
        if !book.status.is_empty() {
            crate::validate::ensure_one_of("books.status", &book.status, books::BOOK_STATUSES)?;
        }
        if !(0..=1000).contains(&book.encoding_confidence) {
            return Err(StorageError::invalid(format!(
                "books.encoding_confidence 必须在 0..=1000，实际 {}",
                book.encoding_confidence
            )));
        }
        if let Some(hash) = &book.content_hash {
            validate_content_hash(hash)?;
        }
        if let Some(fingerprint) = &book.source_fingerprint {
            ensure_max_len(
                "books.source_fingerprint",
                fingerprint,
                MAX_FINGERPRINT_BYTES,
            )?;
        }
    }

    let mut category_ids = HashSet::new();
    for category in &bundle.categories {
        validate_sync_id("categories.sync_id", &category.sync_id)?;
        if !category_ids.insert(category.sync_id.as_str()) {
            return Err(StorageError::invalid(format!(
                "同步包分类 sync_id 重复: {}",
                category.sync_id
            )));
        }
        if category.name.trim().is_empty() {
            return Err(StorageError::invalid("同步包分类名不得为空"));
        }
        ensure_max_len("categories.name", &category.name, MAX_NAME_BYTES)?;
        ensure_non_negative("categories.sort_index", category.sort_index)?;
        if let Some(parent) = &category.parent_sync_id {
            validate_sync_id("categories.parent_sync_id", parent)?;
        }
    }
    for category in &bundle.categories {
        if let Some(parent) = &category.parent_sync_id {
            if !category_ids.contains(parent.as_str()) {
                // 允许父节点缺失（挂根），但父 id 必须是格式合法的。
                continue;
            }
        }
    }

    for item in &bundle.progress {
        validate_sync_id("progress.book_sync_id", &item.book_sync_id)?;
        ensure_non_negative("progress.byte_offset", item.byte_offset)?;
        ensure_non_negative("progress.chapter_index", item.chapter_index)?;
        ensure_non_negative("progress.content_unit_id", item.content_unit_id)?;
        ensure_non_negative("progress.index_version", item.index_version)?;
        if let Some(fingerprint) = &item.nearby_fingerprint {
            ensure_max_len(
                "progress.nearby_fingerprint",
                fingerprint,
                MAX_FINGERPRINT_BYTES,
            )?;
        }
    }

    let mut bookmark_ids = HashSet::new();
    for item in &bundle.bookmarks {
        validate_sync_id("bookmarks.sync_id", &item.sync_id)?;
        validate_sync_id("bookmarks.book_sync_id", &item.book_sync_id)?;
        if !bookmark_ids.insert(item.sync_id.as_str()) {
            return Err(StorageError::invalid(format!(
                "同步包书签 sync_id 重复: {}",
                item.sync_id
            )));
        }
        ensure_max_len("bookmarks.title", &item.title, MAX_TITLE_BYTES)?;
        ensure_non_negative("bookmarks.byte_offset", item.byte_offset)?;
        ensure_non_negative("bookmarks.chapter_index", item.chapter_index)?;
        if let Some(fingerprint) = &item.nearby_fingerprint {
            ensure_max_len(
                "bookmarks.nearby_fingerprint",
                fingerprint,
                MAX_FINGERPRINT_BYTES,
            )?;
        }
    }
    Ok(())
}

fn validate_sync_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(StorageError::invalid(format!(
            "{field} 必须是 1..=128 字节的字母/数字/`-`/`_` 字符串"
        )));
    }
    Ok(())
}

fn validate_content_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StorageError::invalid(format!(
            "content_hash 必须是 64 位 SHA-256 十六进制: `{hash}`"
        )));
    }
    Ok(())
}

fn readable_source_path(book: &books::BookRecord) -> Option<PathBuf> {
    if book.source_kind == books::KIND_ANDROID_URI {
        return None;
    }
    let path = PathBuf::from(&book.source_locator);
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}

fn non_empty_encoding(encoding: &str) -> String {
    if encoding.trim().is_empty() {
        "utf-8".to_string()
    } else {
        encoding.to_string()
    }
}

fn path_to_utf8(path: &Path) -> Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| StorageError::invalid(format!("路径不是合法 UTF-8: {}", path.display())))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .map_err(|error| StorageError::io(path, format!("打开书籍失败: {error}")))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| StorageError::io(path, format!("读取书籍失败: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn copy_book_to_bundle(source: &Path, books_dir: &Path) -> Result<String> {
    let temp = books_dir.join(format!(".tmp-{}", Uuid::new_v4()));
    let hash = copy_and_hash(source, &temp)?;
    let dest = books_dir.join(format!("{hash}.txt"));
    if dest.is_file() {
        let _ = fs::remove_file(&temp);
    } else {
        fs::rename(&temp, &dest)
            .map_err(|error| StorageError::io(&dest, format!("落盘同步包书籍失败: {error}")))?;
    }
    Ok(hash)
}

fn copy_verified(source: &Path, dest: &Path, expected_hash: &str) -> Result<()> {
    let temp = dest.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let hash = copy_and_hash(source, &temp)?;
    if hash != expected_hash {
        let _ = fs::remove_file(&temp);
        return Err(StorageError::invalid(format!(
            "书籍哈希校验失败：期望 {expected_hash}，实际 {hash}"
        )));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| StorageError::io(parent, format!("创建目录失败: {error}")))?;
    }
    fs::rename(&temp, dest)
        .map_err(|error| StorageError::io(dest, format!("写入书籍副本失败: {error}")))?;
    Ok(())
}

fn copy_and_hash(source: &Path, dest: &Path) -> Result<String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| StorageError::io(parent, format!("创建目录失败: {error}")))?;
    }
    let mut input = File::open(source)
        .map_err(|error| StorageError::io(source, format!("打开源文件失败: {error}")))?;
    let mut output = File::create(dest)
        .map_err(|error| StorageError::io(dest, format!("创建目标文件失败: {error}")))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| StorageError::io(source, format!("读取源文件失败: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|error| StorageError::io(dest, format!("写入目标文件失败: {error}")))?;
    }
    output
        .sync_all()
        .map_err(|error| StorageError::io(dest, format!("刷新目标文件失败: {error}")))?;
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| StorageError::io(parent, format!("创建目录失败: {error}")))?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| StorageError::internal(format!("序列化同步状态失败: {error}")))?;
    let temp = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    fs::write(&temp, &bytes)
        .map_err(|error| StorageError::io(&temp, format!("写入同步状态失败: {error}")))?;
    fs::rename(&temp, path)
        .map_err(|error| StorageError::io(path, format!("替换同步状态失败: {error}")))?;
    Ok(())
}

fn book_category_id(conn: &Connection, book_id: i64) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT category_id FROM books WHERE id = ?1",
        [book_id],
        |row| row.get::<_, Option<i64>>(0),
    )
    .optional()
    .map_err(StorageError::from)
    .map(|value| value.flatten())
}

fn list_all_progress(conn: &Connection) -> Result<Vec<progress::ProgressRecord>> {
    let mut stmt = conn.prepare(
        "SELECT book_id, content_unit_id, chapter_index, byte_offset, nearby_fingerprint, \
         last_page_hint, index_version, updated_at, last_opened_at FROM reading_progress ORDER BY book_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(progress::ProgressRecord {
            book_id: row.get(0)?,
            content_unit_id: row.get(1)?,
            chapter_index: row.get(2)?,
            byte_offset: row.get(3)?,
            nearby_fingerprint: row.get(4)?,
            last_page_hint: row.get(5)?,
            index_version: row.get(6)?,
            updated_at: row.get(7)?,
            last_opened_at: row.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn list_all_bookmarks(conn: &Connection) -> Result<Vec<bookmarks::BookmarkRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, book_id, content_unit_id, chapter_index, byte_offset, nearby_fingerprint, \
         title, created_at, updated_at FROM bookmarks ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(bookmarks::BookmarkRecord {
            id: row.get(0)?,
            book_id: row.get(1)?,
            content_unit_id: row.get(2)?,
            chapter_index: row.get(3)?,
            byte_offset: row.get(4)?,
            nearby_fingerprint: row.get(5)?,
            title: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}
