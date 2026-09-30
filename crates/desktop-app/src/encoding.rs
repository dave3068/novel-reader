//! 内容编码的手动覆盖（落库 + 内容索引失效）。
//!
//! 分层：本模块只做「持久化 + 版本失效」；界面（`ui.rs`）负责调用它并重建目录与分页会话，
//! 分页语义仍在 `pagination.rs`（界面不复制任何阅读业务）。
//!
//! 契约：覆盖只改「如何解释文件字节」，**永不写回原始文件**；写入后必须递增内容索引版本，
//! 否则目录缓存与分页边界仍会按旧编码口径复用（那就是「改了编码但正文没变」的成因）。

use std::path::Path;

use reading_source::TextEncoding;
use reading_storage::{books, meta, Storage};

/// 覆盖结果（供状态文案与证据日志使用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideReport {
    /// 生效的编码。
    pub encoding: TextEncoding,
    /// 递增后的内容索引版本（目录与分页缓存的失效依据）。
    pub index_version: i64,
}

/// 把用户选定的内容编码写入书目记录，并递增内容索引版本。
///
/// - `confidence` 记 1000（用户指定即为确定）；
/// - 标记 `encoding_user_override = 1`，界面据此显示「（手动）」；
/// - 书目不存在、编码名非法或数据库不可写时如实返回错误（不静默降级）。
pub fn override_encoding(
    db_path: &Path,
    book_id: i64,
    encoding: TextEncoding,
    now: i64,
) -> Result<OverrideReport, String> {
    let mut storage =
        Storage::open(db_path).map_err(|error| format!("无法打开书架数据库：{error}"))?;
    storage
        .migrate()
        .map_err(|error| format!("书架数据库迁移失败：{error}"))?;
    books::set_encoding(storage.conn(), book_id, &encoding.to_string(), 1_000, now)
        .map_err(|error| format!("写入编码失败：{error}"))?;
    let index_version = meta::bump_content_index_version(storage.conn(), now)
        .map_err(|error| format!("递增内容索引版本失败：{error}"))?;
    Ok(OverrideReport {
        encoding,
        index_version,
    })
}
