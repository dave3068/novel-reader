//! 字段校验：把「数据库只存元数据、索引、位置与短摘要」这条边界变成可断言的前置检查。
//!
//! 正文绝不允许入库；因此所有短文本字段（书名、章节名、附近文本指纹、路径/URI、元信息值）
//! 都有显式字节上限。超限一律 [`crate::StorageError::InvalidArgument`]，
//! 而不是静默截断（静默截断会让调用方把正文当摘要写进来却毫无察觉）。

use crate::error::{Result, StorageError};

/// 书名 / 原始文件名上限（字节）。
pub const MAX_NAME_BYTES: usize = 512;
/// 章节名、书签名上限（字节，短摘要）。
pub const MAX_TITLE_BYTES: usize = 256;
/// 来源定位符（桌面路径 / Android URI）上限（字节）。
pub const MAX_LOCATOR_BYTES: usize = 4096;
/// 附近文本指纹上限（字节）：与 reading-index `ANCHOR_MAX_QUOTE_BYTES`（24 字符 × 4）对齐。
pub const MAX_FINGERPRINT_BYTES: usize = 96;
/// 编码名上限（字节）。
pub const MAX_ENCODING_BYTES: usize = 32;
/// 布局签名文本上限（字节）。
pub const MAX_SIGNATURE_BYTES: usize = 256;
/// `app_meta` 键上限（字节）。
pub const MAX_META_KEY_BYTES: usize = 128;
/// `app_meta` 值上限（字节）—— 元信息同样不得藏正文。
pub const MAX_META_VALUE_BYTES: usize = 4096;

pub fn ensure_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(StorageError::invalid(format!("{field} 不得为空")));
    }
    Ok(())
}

pub fn ensure_max_len(field: &str, value: &str, max: usize) -> Result<()> {
    let len = value.len();
    if len > max {
        return Err(StorageError::invalid(format!(
            "{field} 长度 {len} 字节超过上限 {max} 字节（数据库不得保存正文，只允许短摘要）"
        )));
    }
    Ok(())
}

pub fn ensure_short_text(field: &str, value: &str, max: usize) -> Result<()> {
    ensure_non_empty(field, value)?;
    ensure_max_len(field, value, max)
}

pub fn ensure_non_negative(field: &str, value: i64) -> Result<()> {
    if value < 0 {
        return Err(StorageError::out_of_range(format!(
            "{field} 不得为负：{value}"
        )));
    }
    Ok(())
}

pub fn ensure_one_of(field: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        return Ok(());
    }
    Err(StorageError::invalid(format!(
        "{field} 取值 `{value}` 不在允许集合 {allowed:?} 内"
    )))
}

/// 仅允许 `[A-Za-z0-9_]`：用于把表名拼进 SQL 前的白名单检查。
pub fn ensure_identifier(name: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StorageError::invalid(format!("非法 SQL 标识符: `{name}`")));
    }
    Ok(())
}

/// `app_meta` 键：允许 `[A-Za-z0-9_.-]`（如 `migration.0001_init.applied_at`），
/// 但仍受长度上限约束，且必须是 ASCII（避免把正文片段塞进键）。
pub fn ensure_meta_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(StorageError::invalid("app_meta.key 不得为空"));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(StorageError::invalid(format!("非法 app_meta 键: `{key}`")));
    }
    ensure_max_len("app_meta.key", key, MAX_META_KEY_BYTES)?;
    Ok(())
}
