//! 阶段 0 同步包：桌面端导出/导入入口。
//!
//! 只做「选择目录 → 调共享存储层 → 返回可读结果」；合并语义、哈希与稳定性
//! 全部在 `reading_storage::sync`，本模块不复制业务规则。

use std::path::{Path, PathBuf};

use reading_storage::sync::{self, ExportOptions, ImportOptions, SyncReport};
use reading_storage::Storage;

/// 导出包在目标目录下的固定子目录名（避免污染用户的网盘同步根目录）。
pub const BUNDLE_DIR_NAME: &str = "novel-reader-sync";

/// 打开数据库并确保迁移完成。
fn open(db_path: &Path) -> Result<Storage, String> {
    let mut storage = Storage::open(db_path).map_err(|error| format!("打开数据库失败：{error}"))?;
    storage
        .migrate()
        .map_err(|error| format!("数据库迁移失败：{error}"))?;
    Ok(storage)
}

/// 导出到 `selected/novel-reader-sync/`，返回实际包目录与统计。
pub fn export(db_path: &Path, selected: &Path) -> Result<(PathBuf, SyncReport), String> {
    let storage = open(db_path)?;
    let target = selected.join(BUNDLE_DIR_NAME);
    let mut store = sync::SyncStore::open(Some(db_path))
        .map_err(|error| format!("打开同步状态失败：{error}"))?;
    let report = sync::export_bundle(
        &mut store,
        storage.conn(),
        &target,
        &ExportOptions::default(),
    )
    .map_err(|error| format!("导出同步包失败：{error}"))?;
    Ok((target, report))
}

/// 从用户选择的目录导入：优先把该目录本身当作包；否则尝试其下的
/// `novel-reader-sync/` 子目录（与导出布局对称）。
pub fn import(db_path: &Path, selected: &Path) -> Result<(PathBuf, SyncReport), String> {
    let storage = open(db_path)?;
    let bundle = if selected.join(sync::BUNDLE_STATE_FILE).is_file() {
        selected.to_path_buf()
    } else {
        selected.join(BUNDLE_DIR_NAME)
    };
    if !bundle.join(sync::BUNDLE_STATE_FILE).is_file() {
        return Err(format!(
            "所选目录不是同步包：缺少 {}（也没有 {}/{}）",
            sync::BUNDLE_STATE_FILE,
            BUNDLE_DIR_NAME,
            sync::BUNDLE_STATE_FILE
        ));
    }
    let mut store = sync::SyncStore::open(Some(db_path))
        .map_err(|error| format!("打开同步状态失败：{error}"))?;
    let report = sync::import_bundle(
        &mut store,
        storage.conn(),
        &bundle,
        &ImportOptions {
            progress_policy: crate::sync_config::load(db_path).progress_policy(),
            ..ImportOptions::default()
        },
    )
    .map_err(|error| format!("导入同步包失败：{error}"))?;
    Ok((bundle, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reading_storage::books::{self, BookImport};

    #[test]
    fn export_then_import_into_empty_db_uses_bundle_subdir() {
        let root = std::env::temp_dir().join(format!(
            "nr-desktop-sync-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let source = root.join("source");
        let target = root.join("target");
        std::fs::create_dir_all(&source).expect("建源目录");
        std::fs::create_dir_all(&target).expect("建目标目录");
        let txt = source.join("book.txt");
        let bytes = b"chapter one\nhello\n";
        std::fs::write(&txt, bytes).expect("写书");

        let db_a = source.join("library.db");
        let mut storage_a = open(&db_a).expect("打开 A");
        books::import_book(
            storage_a.conn(),
            &BookImport {
                display_name: "同步书".to_string(),
                original_name: "book.txt".to_string(),
                source_kind: books::KIND_DESKTOP_PATH.to_string(),
                source_locator: txt.to_string_lossy().to_string(),
                source_size: bytes.len() as i64,
                source_modified_hint: None,
                source_fingerprint: Some("test-fingerprint".to_string()),
                encoding: "utf-8".to_string(),
                encoding_confidence: 1000,
                encoding_user_override: false,
            },
            1,
        )
        .expect("导入 A");
        let (bundle, report) = export(&db_a, &target).expect("导出");
        assert_eq!(report.books_exported, 1);
        assert!(bundle.join("state.json").is_file());

        let db_b = target.join("library.db");
        let (_, imported) = import(&db_b, &target).expect("导入 B");
        assert_eq!(imported.books_created, 1);
        let storage_b = open(&db_b).expect("打开 B");
        let shelf = books::list_shelf_books(storage_b.conn()).expect("B 书架");
        assert_eq!(shelf.len(), 1);
        assert_eq!(shelf[0].display_name, "同步书");
        let _ = std::fs::remove_dir_all(&root);
    }
}
