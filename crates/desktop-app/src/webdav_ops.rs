//! WebDAV 同步编排（阶段 1）：远程每设备状态文件 + 共享书籍 blob。
//!
//! 远程布局：
//! ```text
//! <base>/
//!   state-<device_id>.json   # 每台设备只写自己的文件
//!   books/<sha256>.txt       # 内容寻址、只增不删
//! ```
//!
//! 合并语义完全复用 `reading_storage::sync`；本模块只负责：
//! 列远端状态 → 下载状态与引用到的书籍 → 逐设备 import → 导出本机状态 → 上传。
//!
//! 阶段 1 支持三种方向：
//! - [`SyncMode::Pull`]：只把远端读物/进度合并到本地；
//! - [`SyncMode::Push`]：只把本地读物/状态上传到服务器；
//! - [`SyncMode::Both`]：先拉取合并，再上传本机状态。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use reading_storage::sync::{self, ExportOptions, ImportOptions, SyncReport, SyncStore};
use reading_storage::Storage;

use crate::webdav::{WebDavClient, WebDavConfig};

/// 同步方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// 远端 → 本地（读物/进度/书签/分类/设置）。
    Pull,
    /// 仅远端阅读进度（不下载新书、不合并书签/设置）。
    PullProgressOnly,
    /// 仅远端书籍文件（不合并进度/书签/设置）。
    PullBooksOnly,
    /// 本地 → 远端（读物/状态）。
    Push,
    /// 双向。
    Both,
}

impl SyncMode {
    fn pulls(self) -> bool {
        matches!(
            self,
            Self::Pull | Self::PullProgressOnly | Self::PullBooksOnly | Self::Both
        )
    }

    fn import_options(self) -> (bool, bool, bool, bool) {
        match self {
            Self::PullProgressOnly => (false, true, false, false),
            Self::PullBooksOnly => (true, false, false, false),
            _ => (true, true, true, true),
        }
    }

    fn pushes(self) -> bool {
        matches!(self, Self::Push | Self::Both)
    }
}

/// 一次 WebDAV 同步的结果（用于状态栏）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebDavSyncReport {
    pub remote_states: usize,
    pub downloaded_books: usize,
    pub uploaded_books: usize,
    pub imported: SyncReport,
}

impl WebDavSyncReport {
    pub fn summary(&self) -> String {
        let skipped = self.imported.progress_skipped_remote_newer;
        let skipped_note = if skipped > 0 {
            format!("；保留本地进度的书 {skipped} 本")
        } else {
            String::new()
        };
        format!(
            "远端设备状态 {}，下载书 {}，上传书 {}；{}{}",
            self.remote_states,
            self.downloaded_books,
            self.uploaded_books,
            self.imported.summary(),
            skipped_note
        )
    }
}

/// 执行一次双向同步。
pub fn sync(db_path: &Path, config: &WebDavConfig) -> Result<WebDavSyncReport, String> {
    sync_mode(db_path, config, SyncMode::Both)
}

/// 按指定方向同步。
pub fn sync_mode(
    db_path: &Path,
    config: &WebDavConfig,
    mode: SyncMode,
) -> Result<WebDavSyncReport, String> {
    let client = WebDavClient::new(config)?;
    // 先做一次只读 PROPFIND：401/404 能比 MKCOL 更清楚地指出认证或根地址问题。
    client.check_connection()?;
    client.ensure_dir("books")?;
    // 状态文件放在子目录：部分 WebDAV 挂载点允许子目录写入但不允许在根写文件。
    client.ensure_dir("states")?;

    let temp = temp_dir(db_path);
    if temp.exists() {
        let _ = fs::remove_dir_all(&temp);
    }
    fs::create_dir_all(temp.join("books"))
        .map_err(|error| format!("创建同步临时目录失败：{error}"))?;
    let result = run_sync(&client, db_path, &temp, mode);
    let _ = fs::remove_dir_all(&temp);
    result
}

fn run_sync(
    client: &WebDavClient,
    db_path: &Path,
    temp: &Path,
    mode: SyncMode,
) -> Result<WebDavSyncReport, String> {
    let mut report = WebDavSyncReport::default();
    if mode.pulls() {
        pull_remote(client, db_path, temp, &mut report, mode.import_options())?;
    }
    if mode.pushes() {
        push_local(client, db_path, temp, &mut report)?;
    }
    Ok(report)
}

fn pull_remote(
    client: &WebDavClient,
    db_path: &Path,
    temp: &Path,
    report: &mut WebDavSyncReport,
    options: (bool, bool, bool, bool),
) -> Result<(), String> {
    let own = SyncStore::open(Some(db_path))
        .map_err(|error| format!("读取同步边车失败：{error}"))?
        .device_id()
        .to_string();
    let own_file = format!("state-{own}.json");
    let policy = crate::sync_config::load(db_path).progress_policy();

    let mut state_files: Vec<(String, String)> = Vec::new();
    for name in client.list_names("states")? {
        if name.starts_with("state-") && name.ends_with(".json") {
            state_files.push(("states".to_string(), name));
        }
    }
    // 兼容旧布局：根目录下已有的 state-*.json 仍然读取。
    for name in client.list_names("")? {
        if name.starts_with("state-")
            && name.ends_with(".json")
            && !state_files.iter().any(|(_, existing)| existing == &name)
        {
            state_files.push((String::new(), name));
        }
    }
    for (directory, name) in state_files {
        if name == own_file {
            continue;
        }
        let remote_path = if directory.is_empty() {
            name.clone()
        } else {
            format!("{directory}/{name}")
        };
        let bytes = client.get(&remote_path)?;
        let state_dir = temp.join(format!("remote-{}", sanitize(&name)));
        fs::create_dir_all(&state_dir).map_err(|error| format!("创建远端状态目录失败：{error}"))?;
        fs::write(state_dir.join("state.json"), &bytes)
            .map_err(|error| format!("写入远端状态失败：{error}"))?;

        // 只下载该状态引用到的书籍；已下载的按哈希复用。
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("远端状态不是合法 JSON（{name}）：{error}"))?;
        if options.0 {
            for hash in referenced_hashes(&value) {
                let target = temp.join("books").join(format!("{hash}.txt"));
                if target.is_file() {
                    continue;
                }
                let bytes = client.get(&format!("books/{hash}.txt"))?;
                fs::write(&target, &bytes).map_err(|error| format!("写入书籍缓存失败：{error}"))?;
                report.downloaded_books += 1;
            }
        }

        let storage = open_db(db_path)?;
        let mut store =
            SyncStore::open(Some(db_path)).map_err(|error| format!("读取同步边车失败：{error}"))?;
        let imported = sync::import_bundle(
            &mut store,
            storage.conn(),
            &state_dir,
            &ImportOptions {
                library_dir: None,
                books_dir: Some(temp.join("books")),
                progress_policy: policy,
                import_books: options.0,
                apply_progress: options.1,
                apply_bookmarks: options.2,
                apply_settings: options.3,
            },
        )
        .map_err(|error| format!("合并远端状态失败（{name}）：{error}"))?;
        merge_report(&mut report.imported, imported);
        report.remote_states += 1;
    }
    Ok(())
}

fn push_local(
    client: &WebDavClient,
    db_path: &Path,
    temp: &Path,
    report: &mut WebDavSyncReport,
) -> Result<(), String> {
    let own = SyncStore::open(Some(db_path))
        .map_err(|error| format!("读取同步边车失败：{error}"))?
        .device_id()
        .to_string();
    let own_file = format!("state-{own}.json");

    let export_dir = temp.join("export");
    fs::create_dir_all(&export_dir).map_err(|error| format!("创建导出临时目录失败：{error}"))?;
    let storage = open_db(db_path)?;
    let mut store =
        SyncStore::open(Some(db_path)).map_err(|error| format!("读取同步边车失败：{error}"))?;
    sync::export_bundle(
        &mut store,
        storage.conn(),
        &export_dir,
        &ExportOptions::default(),
    )
    .map_err(|error| format!("导出本机同步包失败：{error}"))?;

    let books_dir = export_dir.join(sync::BUNDLE_BOOKS_DIR);
    if let Ok(entries) = fs::read_dir(&books_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let remote = format!("books/{name}");
            if !client.exists(&remote)? {
                let bytes = fs::read(&path) // scope-allow: 同步包文件，非正文通道
                    .map_err(|error| format!("读取待上传书籍失败：{error}"))?;
                client.put(&remote, &bytes)?;
                report.uploaded_books += 1;
            }
        }
    }

    let state = fs::read(export_dir.join(sync::BUNDLE_STATE_FILE)) // scope-allow: 同步包状态，非正文
        .map_err(|error| format!("读取本机同步状态失败：{error}"))?;
    client.put(&format!("states/{own_file}"), &state)?;
    Ok(())
}

fn open_db(db_path: &Path) -> Result<Storage, String> {
    let mut storage = Storage::open(db_path).map_err(|error| format!("打开数据库失败：{error}"))?;
    storage
        .migrate()
        .map_err(|error| format!("数据库迁移失败：{error}"))?;
    Ok(storage)
}

fn referenced_hashes(value: &serde_json::Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(books) = value.get("books").and_then(|books| books.as_array()) {
        for book in books {
            if let Some(hash) = book.get("content_hash").and_then(|hash| hash.as_str()) {
                if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    out.insert(hash.to_string());
                }
            }
        }
    }
    out
}

fn temp_dir(db_path: &Path) -> PathBuf {
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |delta| delta.as_nanos());
    parent.join(format!("webdav-tmp-{}-{stamp}", std::process::id()))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn merge_report(target: &mut SyncReport, other: SyncReport) {
    target.books_created += other.books_created;
    target.books_updated += other.books_updated;
    target.books_missing += other.books_missing;
    target.progress_applied += other.progress_applied;
    target.progress_skipped_local_newer += other.progress_skipped_local_newer;
    target.progress_skipped_remote_newer += other.progress_skipped_remote_newer;
    target.bookmarks_created += other.bookmarks_created;
    target.bookmarks_updated += other.bookmarks_updated;
    target.categories_created += other.categories_created;
    target.categories_updated += other.categories_updated;
    target.settings_applied |= other.settings_applied;
    target.conflicts.extend(other.conflicts);
    target.books.extend(other.books);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_hashes_only_accepts_sha256() {
        let value = serde_json::json!({
            "books": [
                {"content_hash": "a".repeat(64)},
                {"content_hash": "../escape"},
                {"content_hash": null}
            ]
        });
        let hashes = referenced_hashes(&value);
        assert_eq!(hashes.len(), 1);
        assert!(hashes.iter().next().unwrap().chars().all(|ch| ch == 'a'));
    }

    #[test]
    fn sanitize_keeps_state_file_names_safe() {
        assert_eq!(sanitize("state-abc.json"), "state-abc_json");
        assert_eq!(sanitize("../../etc"), "______etc");
    }
}
