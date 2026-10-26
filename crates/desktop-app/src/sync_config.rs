//! 桌面端跨设备同步配置（WebDAV 开关/地址/凭证/进度策略）。
//!
//! 存储位置：`app_meta` 的键值行（与 UI 交互模式同一层），**不新增数据库表/列**。
//! 已知限制：密码以明文存于 `library.db`；这是阶段 1 的临时方案，后续应迁移到
//! 系统密钥环/0600 凭证文件。未配置/未启用时应用完全按离线模式运行。

use std::path::Path;

use reading_storage::meta;
use reading_storage::sync::ProgressPolicy;
use reading_storage::Storage;

use crate::webdav::WebDavConfig;

pub const KEY_ENABLED: &str = "sync.webdav.enabled";
pub const KEY_URL: &str = "sync.webdav.url";
pub const KEY_USER: &str = "sync.webdav.user";
pub const KEY_PASSWORD: &str = "sync.webdav.password";
pub const KEY_PROGRESS: &str = "sync.progress_policy";

/// 进度策略字符串（UI 枚举值）。
pub const PROGRESS_KEEP_LOCAL: &str = "keep_local";
pub const PROGRESS_REMOTE_IF_NEWER: &str = "remote_if_newer";

/// 同步配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncConfig {
    pub enabled: bool,
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub progress: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            username: String::new(),
            password: String::new(),
            progress: PROGRESS_KEEP_LOCAL.to_string(),
        }
    }
}

impl SyncConfig {
    pub fn progress_policy(&self) -> ProgressPolicy {
        if self.progress == PROGRESS_REMOTE_IF_NEWER {
            ProgressPolicy::RemoteIfNewer
        } else {
            ProgressPolicy::KeepLocal
        }
    }

    /// 是否具备发起 WebDAV 请求的完整配置。
    pub fn is_ready(&self) -> bool {
        self.enabled
            && self.base_url.trim().starts_with("https://")
            && !self.username.trim().is_empty()
    }

    /// 转成客户端配置（调用前应检查 [`is_ready`]）。
    pub fn webdav(&self) -> WebDavConfig {
        WebDavConfig {
            base_url: self.base_url.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }
}

/// 从数据库连接读取配置；键缺失时尝试环境变量（兼容旧入口）。
pub fn load_from_conn(conn: &rusqlite::Connection) -> SyncConfig {
    let get = |key: &str| meta::get(conn, key).ok().flatten();
    let config = SyncConfig {
        enabled: get(KEY_ENABLED).as_deref() == Some("1"),
        base_url: get(KEY_URL).unwrap_or_default(),
        username: get(KEY_USER).unwrap_or_default(),
        password: get(KEY_PASSWORD).unwrap_or_default(),
        progress: get(KEY_PROGRESS).unwrap_or_else(|| PROGRESS_KEEP_LOCAL.to_string()),
    };
    config
}

pub fn load(db_path: &Path) -> SyncConfig {
    match Storage::open(db_path) {
        Ok(storage) => load_from_conn(storage.conn()),
        Err(_) => SyncConfig::default(),
    }
}

/// 写入配置（逐键 upsert；失败返回可读原因）。
pub fn save_to_conn(
    conn: &rusqlite::Connection,
    config: &SyncConfig,
    now: i64,
) -> Result<(), String> {
    let values = [
        (KEY_ENABLED, if config.enabled { "1" } else { "0" }),
        (KEY_URL, config.base_url.trim()),
        (KEY_USER, config.username.trim()),
        (KEY_PASSWORD, config.password.as_str()),
        (KEY_PROGRESS, config.progress.as_str()),
    ];
    for (key, value) in values {
        meta::set(conn, key, value, now).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn save(db_path: &Path, config: &SyncConfig, now: i64) -> Result<(), String> {
    let storage = Storage::open(db_path).map_err(|error| error.to_string())?;
    save_to_conn(storage.conn(), config, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn round_trips_config_in_app_meta() {
        let dir = std::env::temp_dir().join(format!(
            "nr-sync-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |delta| delta.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("建目录");
        let db = dir.join("library.db");
        let mut storage = Storage::open(&db).expect("打开");
        storage.migrate().expect("迁移");
        let config = SyncConfig {
            enabled: true,
            base_url: "https://dav.example.com/dav/".to_string(),
            username: "u".to_string(),
            password: "p".to_string(),
            progress: PROGRESS_REMOTE_IF_NEWER.to_string(),
        };
        save_to_conn(storage.conn(), &config, 1).expect("保存");
        assert_eq!(load_from_conn(storage.conn()), config);
        let _ = std::fs::remove_dir_all(PathBuf::from(dir));
    }
}
