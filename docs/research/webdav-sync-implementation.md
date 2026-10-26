# 阶段 1 WebDAV 同步：实现记录与限制

> 日期：2026-10-25
> 状态：**代码已实现（桌面 + Android）**；**未对真实 WebDAV 服务做端到端验收**。
> 前置：`docs/research/sync-phase0-implementation.md`（目录包/合并语义）。

## 1. 远程布局

```text
<WebDAV 根目录>/
  states/state-<device_id>.json   # 每台设备只写自己的状态文件
  books/<sha256>.txt              # 内容寻址；只增不删
  # 旧布局根目录下的 state-*.json 仍会读取（兼容）
```

- 每台设备只写自己的 `state-<device_id>.json`，避免两端互相覆盖；
- 书籍 blob 不可变，按 SHA-256 命名；已存在则跳过上传；
- 合并语义完全复用 `reading-storage::sync`（与阶段 0 同一套校验/合并）；
- 远程目录可以是坚果云/InfiniCLOUD/Nextcloud 的任意子目录；应用只访问该根目录。

## 2. 同步流程

1. 确保根目录与 `books/` 存在（MKCOL；已存在视为成功）；
2. 先导出本机包到临时目录（得到 `device_id` 与本机状态）；
3. PROPFIND 根目录，列出 `state-*.json`；跳过自己的状态文件；
4. 对每个远端状态：
   - 下载 `state-<id>.json` 到独立临时目录；
   - 解析 `books[].content_hash`，只下载引用到的 `books/<hash>.txt`；
   - 调用核心 `nr_sync_import` 合并；
   - Android 把导入摘要中的书籍副本写入同步书目录，重启后重建书架；
5. 上传本机导出包中远端不存在的 `books/<hash>.txt`；
6. PUT 本机 `state-<device_id>.json`；
7. 删除临时目录。

## 3. 入口与凭证

### 桌面

设置页 →「同步 · WebDAV（阶段 1）」→「WebDAV 配置…」：

- 弹出一个独立对话框，输入：启用开关、HTTPS 地址、用户名、应用密码、远端进度策略；
- 输入值只在对话框本地保存，**保存配置时才写回 Rust/SQLite** —— 避免设置页逐键重投影导致滚动条跳顶/焦点丢失；
- 配置写入 `app_meta`（**密码明文存于本机 `library.db`**，阶段 1 已知限制）；
- 书架状态栏「同步…」菜单可选择：拉取远端读物到本地 / 上传本地读物到服务器 / 双向同步。

### Android

设置页 →「WebDAV 同步（阶段 1，仅 HTTPS）」：

- 「启用 WebDAV」开关；关闭时按普通离线模式运行；
- 地址 / 用户名 / 密码（应用密码）三个输入框；
- 「远端进度较新时使用远端」开关；
- 书架顶栏「同步」菜单：拉取远端读物到本地 / 上传本地读物到服务器 / 双向同步；
- 配置写入应用私有 SharedPreferences；**密码目前只做 Base64 存储，不是加密**，
  这是阶段 1 的已知限制，后续应迁移到 Android Keystore/EncryptedSharedPreferences；
- 需要 `INTERNET` 权限（已加入 Manifest）。

## 3.1 401 排查（MKCOL/PROPFIND 认证失败）

`HTTP 401` 表示服务端拒绝了 Basic Auth，不是目录已存在/无权限：

- 坚果云：地址应为 `https://dav.jianguoyun.com/dav/`；用户名是账号邮箱；密码必须是“第三方应用密码”，不是登录密码。
- InfiniCLOUD：使用账号页给出的 WebDAV URL；密码为 WebDAV 应用密码。
- Nextcloud：地址通常为 `https://host/remote.php/dav/files/<用户名>/`，用户名/应用密码区分大小写。
- 先用只读 `PROPFIND`（Depth 0）验证认证；客户端现在在 MKCOL 前先做该检查，401 会给出明确提示。
- 客户端会把根地址规范为以 `/` 结尾，避免 MKCOL 被 301 重定向后丢失 Authorization。
- 状态文件已改放 `states/` 子目录：若服务器允许子目录写入但拒绝在根创建文件（403/405），可绕过该限制；旧根目录 `state-*.json` 仍兼容读取。
- 仍失败时可用 `curl -u '用户:应用密码' -X PROPFIND -H 'Depth: 0' <URL>` 在终端复核；不要把密码写进 shell 历史/公开日志。

## 3.2 阅读进度如何同步

- 阅读进度在翻页/退出时已写入本地 `reading_progress`（按内容位置，不按页码）。
- **上传本地进度**：书架「同步」→「上传本地读物到服务器」会把本机 `state-<device>.json` 与书籍 blob 上传。
- **拉取远端进度**：选择「拉取远端读物到本地」或「双向同步」；仅“上传”不会把远端进度拉回。
- 远端进度较新时的处理：
  - 默认「保留本地进度」：不覆盖本机当前位置，摘要显示保留数量；
  - 在 WebDAV 配置里打开「远端进度较新时使用远端」后，才会用远端位置覆盖本地。
- 同步是**手动触发**；没有后台定时同步。两端都改过时目前按上述全局策略处理，不是逐本弹窗。

## 3.3 「仅本地」排除与选择性拉取

- 桌面书架行「…」→「同步」在“仅本地/恢复同步”之间切换；分类行「…」→「同步」同理。
- 被标记的书籍：导出时**不上传**书籍文件、元数据、进度、书签；导入时也**不合并**远端同名书籍/进度。
- 被标记的分类：其自身及全部子分类下的书籍一并排除；同样双向生效。
- 排除状态只存本机 `app_meta`，不会上传；服务端已有副本不会被删除（避免误删其他设备数据）。
- 书架「同步…」菜单提供：
  - 拉取远端读物到本地（全部）
  - 仅拉取远端阅读进度（不下载新书）
  - 仅下载远端书籍文件（不合并进度/书签/设置）
  - 上传本地读物到服务器
  - 双向同步
- 当前选择粒度是**整本书 / 整个分类**；尚未做“只同步某本书的进度但不同步其文件”的逐项复选框。

## 4. 安全边界

- 仅 HTTPS；不提供“忽略证书错误”开关；
- Basic Auth；坚果云/InfiniCLOUD 使用“应用密码”，可随时撤销；
- 不访问同步根目录之外的路径；不列父目录；
- 不解析正文，只搬运 `state-*.json` 与 `books/*.txt`；
- 原始 TXT 只读：同步只会下载/写入应用自己的副本；
- 未做端到端加密：WebDAV 服务端可见书籍与状态明文；
- 远端进度策略默认**保留本地**；只有用户显式选择“使用远端较新进度”才会覆盖本地位置。

## 5. 已验证 / 未验证

**已验证（2026-10-25 本机）**

- 核心同步包与合并：`cargo test --workspace --locked`；
- 桌面 WebDAV 代码编译/单测（URL 校验、XML href 解析、SHA-256 过滤）；
  另有**本机 loopback 最小 WebDAV 桩**（MKCOL/PROPFIND/PUT/HEAD/GET）验证客户端请求形态：
  `cargo test --manifest-path crates/desktop-app/Cargo.toml webdav`；
- Android 编译/单测（含 WebDAV 配置存储、PROPFIND href 解析）：
  `cd android && ./gradlew :app:testDebugUnitTest`；
- 门禁：`verify-core-scope`、`verify-abi --kotlin`、`verify-desktop-scope`、
  `verify-android-boundary`、`verify-android-scope`、`verify-android-v11-parity`。

**未验证**

- 真实坚果云/InfiniCLOUD/Nextcloud 的 PROPFIND/PUT/HEAD 兼容性（尤其 207 响应命名空间、
  405/301 行为、证书链）；
- 两台真实设备（桌面 + Android）经 WebDAV 的完整往返；
- 坚果云 600 请求/30 分钟限制下的首次大书库同步；
- Android 真机的 SAF/网络/后台行为；
- Windows/macOS 桌面构建。

## 6. 已知限制（下一轮）

1. **请求配额/退避**：当前没有 429/503 退避，也没有请求节流；首次同步大量书籍时
   可能触发坚果云 600 请求/30 分钟限制。建议：优先按远端状态跳过已存在书籍、
   加入指数退避与“稍后继续”。
2. **删除传播**：书籍 `removed` 会传播；书签/分类删除仍没有墓碑。
3. **进度选择**：目前是“保留本地 / 使用远端较新”整策略，不是逐本弹窗；冲突只记录在摘要里。
4. **凭证**：桌面密码明文存于 `library.db`；Android 密码未加密。
5. **长任务阻塞**：Android 的 `syncWebDav` 持有阅读串行门，网络期间阅读操作会等待；
   桌面在 UI 线程执行网络请求，窗口可能短暂无响应。后续应把网络移出 UI 线程/阅读门。
6. **无断点续传**：单文件 GET/PUT 一次性读写；TXT 体积通常可接受，但大文件/弱网需改进。
7. **无真实服务测试**：发布前必须按 `docs/android/device-runbook.md` 和桌面 runbook 做实测。

## 7. 代码位置

| 层 | 文件 |
|---|---|
| 桌面 WebDAV 协议 | `crates/desktop-app/src/webdav.rs` |
| 桌面同步编排 | `crates/desktop-app/src/webdav_ops.rs` |
| 桌面入口 | `crates/desktop-app/src/settings.rs` / `sync_config.rs` / `ui.rs`（设置页字段 + 书架菜单） |
| Android WebDAV 协议 | `android/.../source/WebDavClient.kt` |
| Android 配置存储 | `android/.../source/WebDavPrefsStore.kt` |
| Android 同步编排 | `android/.../reader/ReaderViewModel.kt`（`syncWebDav`） |
| Android 入口 | `android/.../ui/SettingsScreen.kt` / `NovelReaderApp.kt` / `MainActivity.kt` |
