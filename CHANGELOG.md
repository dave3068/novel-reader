# 变更日志

本文件记录用户可见变更、跨语言契约变更与发布口径调整。未发布内容归入 `[Unreleased]`，
发布时再补版本号与日期。更早的提交历史可用 `git log` 查阅；本文件从 0.3.0 起回溯登记。

## [Unreleased]

### 新增

- 可选跨设备同步（显式开启，关闭时按普通离线模式运行）：
  - **阶段 0 目录包**：桌面与 Android 设置页支持导出/导入
    `novel-reader-sync/`（`state.json` + `books/<sha256>.txt`），可放入任意由网盘客户端
    搬运的目录。导出只读取原 TXT，导入只写应用自己的 `sync-books/` 副本。
  - **阶段 1 WebDAV**：桌面与 Android 支持 HTTPS + Basic Auth 的 WebDAV 后端，
    远程布局为 `states/state-<device_id>.json` + 内容寻址的 `books/<sha256>.txt`；
    提供拉取远端、仅拉取进度、仅下载书籍、上传本地与双向同步入口。
  - 同步范围：阅读进度（按内容位置，不按页码）、书签、分类、阅读设置与书籍副本；
    阅读历史、章节索引与分页缓存仍只保留在各设备本地。
  - 合并语义：书籍按 `sync_id` → 内容哈希 → 源指纹匹配；进度按更新时间取较新值，
    默认**保留本地**，仅在用户显式选择后才用远端较新进度覆盖；分类按 `sync_id` 并集合并；
    远端标记为移除的书籍只做软移除，不删除任何原始 TXT。
  - 排除机制：书籍或分类可标记为「仅本地」，其自身及子分类在导出、导入与 WebDAV
    同步中均被跳过；排除状态只存本机。
  - Android 使用 SAF 目录选择器搬运同步包，并把导入的同步书持久化到书架；
    桌面同步入口位于设置页与书架「同步…」菜单。
- C ABI 新增 `nr_sync_export`、`nr_sync_import` 两个加性入口，入口总数由 40 增至 42；
  `NR_ABI_VERSION` 仍为 1。头文件、Rust 导出、JNI 与 Kotlin 声明同步更新。
- 新增同步实现记录与可行性调研：
  `docs/research/network-storage-sync-feasibility.md`、
  `docs/research/sync-phase0-implementation.md`、
  `docs/research/webdav-sync-implementation.md`。
- Android 真机 runbook 新增 S13–S15（目录包导出/导入、WebDAV 双向同步）待验证项。

### 变更

- Android 清单新增 `INTERNET` 权限；仅在启用 WebDAV 时产生网络访问。
- 桌面与 Android 设置页新增同步配置；书架新增「同步…」方向菜单。
- 刷新 0.5.0 Linux X11 与 Windows MSVC 出包证据，覆盖包含同步代码的构建。
- 已知限制 L5 更新为「跨设备同步尚未完成真实网盘/真机验收」，README 与总体方案同步标注。

### 安全与限制

- WebDAV 密码当前桌面端以明文存于本机 `library.db`，Android 端以 Base64 存于应用私有
  SharedPreferences，**均非加密**；后续应迁移到系统密钥环 / Android Keystore。
- 同步未做端到端加密：WebDAV 服务端与同步目录的持有者可读取书籍与状态明文。
- 尚未对真实坚果云 / InfiniCLOUD / Nextcloud 做端到端验收；请求配额退避、断点续传、
  书签/分类删除墓碑、逐本冲突处理与后台定时同步仍未实现。
- Android 真机上的 SAF 目录选择、网络与后台行为仍为「待验证」，不得据此宣称同步已验收。

### 验证（2026-10-26 本机）

- `cargo test --workspace --locked`：通过（含新增 `reading-storage/tests/sync.rs` 13 项）。
- `cargo test --manifest-path crates/desktop-app/Cargo.toml --locked`：通过（186 项单元测试
  及全部集成测试），含 WebDAV 协议、URL 校验、`href` 解析与本机 loopback 桩测试。
- `cd android && ./gradlew :app:testDebugUnitTest`：通过（本机增量构建，UP-TO-DATE）。
- `cargo fmt --all -- --check`：通过。
- 门禁：`verify-core-scope`、`verify-v1-structure`、`verify-abi --kotlin`、
  `verify-desktop-scope`、`verify-desktop-targets`、`verify-android-scope`、
  `verify-android-boundary`、`verify-android-v11-parity`、`verify-v1-release --desktop --android` 均通过。
- 未验证：真实 WebDAV 服务兼容性、双设备往返、Android 真机、Windows/macOS 同步构建。
- 说明：`cargo clippy --workspace --all-targets --locked -- -D warnings` 当前仍会因
  `reading-source` 的既有 lint（`is_multiple_of`、可合并 `if`、测试模块后的条目）失败，
  与本轮同步改动无关。

## [0.5.0] - 2026-10-09

### 新增

- 桌面 V1.1：分类侧栏、阅读历史面板、配色与背景设置、批量导入。
- Android V1.1：分类侧栏、阅读历史页、配色与背景设置，阅读页保持屏幕常亮。
- Android 书架改为文件管理器式导航：窄屏逐级下钻、宽屏左右分栏；分类过滤按直属书籍，
  并支持书籍移动到分类。
- C ABI 扩展分类、历史、配色与背景入口，并同步头文件、JNI 与 Kotlin 声明。

### 变更

- 桌面 V1.1 书架侧栏与移动浮层的 15 项界面修复；单文件导入归入当前分类。
- 设置页分组说明改为独立说明块，窄窗下不再被裁切；修正重复的分组命名。

## [0.3.0] - 2026-10-05

### 变更

- 设置页分组说明改为独立说明块并按实测内容定高，窄窗数值行控件自适应。
- 刷新桌面与 Windows MSVC 出包证据；Windows 官方口径四步验证通过。
