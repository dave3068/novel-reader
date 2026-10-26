# 阶段 0 同步：实现记录与限制

> 日期：2026-10-25
> 状态：**已实现（核心 + 桌面 + Android）**；尚未做真机验收。阶段 1 WebDAV 代码已另行实现，
> 见 `docs/research/webdav-sync-implementation.md`（同样未做真实服务/真机验收）。
> 前置调研：`docs/research/network-storage-sync-feasibility.md`

## 1. 用户可见行为

### 桌面

设置页 →「同步（阶段 0）」：

- **导出同步包…**：选择一个目录（例如百度网盘同步空间、坚果云同步目录），应用创建
  `<所选目录>/novel-reader-sync/`，写入 `state.json` 与 `books/<sha256>.txt`。
- **导入同步包…**：选择同步包目录（或它的父目录），应用合并到本地库；
  同步导入的书籍副本落在数据库同目录的 `sync-books/`。
- 原始 TXT **只读**：导出只读取源文件，导入只写应用自己的副本。

### Android

设置页 →「同步（阶段 0）」：

- **导出同步包…**：系统目录选择器（SAF）；应用先把包写到 app-private 暂存目录，
  再复制到 `<所选目录>/novel-reader-sync/`。
- **导入同步包…**：选择 `<...>/novel-reader-sync/` 目录；应用先复制到暂存目录，
  再交给核心合并。
- 导入的书籍副本写入 `files/state/sync-books/`，并记录到宿主侧同步书目录，
  重启后仍出现在书架；打开时走普通文件源（不经整本正文通道）。

## 2. 包格式

目录包（不是 ZIP，避免新增压缩依赖与解压路径安全问题）：

```text
novel-reader-sync/
  state.json
  books/
    <sha256>.txt
```

`state.json` 关键字段：

```json
{
  "magic": "novel-reader-sync",
  "schema": 1,
  "device_id": "…",
  "seq": 12,
  "exported_at": 1760000000,
  "books": [
    {
      "sync_id": "uuid",
      "content_hash": "sha256",
      "display_name": "…",
      "original_name": "….txt",
      "encoding": "utf-8",
      "encoding_confidence": 1000,
      "encoding_user_override": false,
      "source_fingerprint": "fnv1a64:…",
      "source_size": 12345,
      "status": "ready",
      "category_sync_id": "uuid|null",
      "created_at": 0,
      "updated_at": 0
    }
  ],
  "progress": [
    {
      "book_sync_id": "uuid",
      "content_unit_id": 0,
      "chapter_index": 3,
      "byte_offset": 4567,
      "nearby_fingerprint": "…",
      "index_version": 0,
      "updated_at": 0,
      "last_opened_at": 0
    }
  ],
  "bookmarks": [],
  "categories": [],
  "settings": {},
  "tombstones": []
}
```

设计要点：

- **不写页码**：`last_page_hint`、总页数都不跨设备；位置以 `content_unit_id + chapter_index + byte_offset + nearby_fingerprint` 为准。
- **书籍文件按 SHA-256 内容寻址**：同名不同内容不会互相覆盖；同内容不同路径去重。
- **路径不入包**：`sync_id` 是稳定 UUID（`[A-Za-z0-9_-]`，长度 1..=128）；书籍文件只以哈希引用，避免路径遍历与平台差异。
- **元数据上限**：`state.json` ≤ 16 MiB；每类实体 ≤ 100,000；超限拒绝导入。
- **严格校验**：magic/schema/sync_id/content_hash/名称/书签标题/指纹长度/负偏移；非法输入不会写到用户目录。

## 3. 本地稳定身份：边车文件

不新增数据库表/列（避免改动冻结迁移）。数据库同目录写 `sync-state.json`：

```json
{
  "schema": 1,
  "device_id": "uuid",
  "seq": 12,
  "books": {
    "id:42": {
      "sync_id": "uuid",
      "local_id": 42,
      "content_hash": "sha256",
      "fingerprint": "fnv1a64:…",
      "locator": "/…/book.txt"
    }
  },
  "categories": { "3": "uuid" },
  "bookmarks": { "7": "uuid" },
  "peers": { "other-device": { "seq": 9, "imported_at": 0 } }
}
```

- `device_id` 首次使用时生成并持久化；`seq` 每次导出递增。
- 本地 `INTEGER PRIMARY KEY` **从不**跨设备复用；同步层用 `sync_id` / 内容哈希映射。
- 边车丢失时可由 `source_fingerprint`、`source_locator`、内容哈希重建部分映射；
  最坏情况是同一本书被当成新书（不会丢原始文件），不会损坏数据库。
- 边车是元数据，不是用户数据；写入采用临时文件 + rename。

## 4. 合并规则（当前实现）

| 数据 | 规则 | 已知限制 |
|---|---|---|
| 书籍 | 优先按 `sync_id`；其次内容哈希；再次 `source_fingerprint`；都找不到且有文件则导入副本；无文件则建 `missing` 占位并保留进度/书签 | 内容哈希不同（同一 `sync_id`）会记冲突并**跳过该书进度/书签**，不覆盖本地书 |
| 进度 | 比较 `updated_at`，远端更新才写；相同时间但位置不同记冲突；偏移超出本地 `source_size` 记冲突 | 仍是时间戳 LWW，不是向量时钟；两端同时改且时钟异常时可能选错 |
| 书签 | 按 `sync_id` 并集；远端更新时间更新才覆盖 | **不传播删除**（阶段 0 没有书签墓碑） |
| 分类 | 按 `sync_id` 合并；父节点缺失挂根；同父同名复用；存在则改名/移动 | **不传播删除**；同名兄弟可能被合并 |
| 设置 | 整行 `updated_at` 更新才覆盖 | 设备相关布局（边距等）也会跟随；后续应改为字段白名单 |
| 书籍移除 | 远端 `status = removed` → 本地软移除（保留进度/书签，不删副本） | 仅书籍有移除传播；分类/书签没有 |
| 阅读历史 | 不同步 | 阶段 0 范围外 |
| 章节/分页缓存 | 不同步 | 每台设备按本地布局重建 |

导入后 `SyncReport` 附带 `books` 摘要（id/显示名/副本路径/状态），Android 用它把同步书
加入持久书架；桌面端忽略该字段。

## 5. 安全与数据安全

- **原始 TXT 只读**：导出只 `File::open/read`；导入只写 `sync-books/<hash>.txt`；
  测试断言源文件 sha256 不变。
- **路径安全**：`sync_id` 只允许字母/数字/`-`/`_`；`content_hash` 必须是 64 位十六进制；
  书籍包文件名只由哈希生成；拒绝 `..`/路径分隔符。
- **阶段 0 不联网**：只做本地目录包；WebDAV（阶段 1）是独立入口，不复用本节的目录选择流程。
- **未加密**：包内是明文 TXT 与 JSON；放进网盘意味着网盘可读。端到端加密未实现。
- **凭证**：阶段 0 不需要凭证；阶段 1 WebDAV 当前桌面明文存 `app_meta`、Android 以 Base64 存 SharedPreferences（都不是加密，待迁移系统密钥环/Keystore）。

## 6. 新增依赖与许可

阶段 0 在 `reading-storage` 新增：`serde` / `serde_json`（MIT OR Apache-2.0）、
`sha2`（MIT OR Apache-2.0）、`uuid`（Apache-2.0 OR MIT）。阶段 1 桌面端新增
`ureq`（MIT OR Apache-2.0）、`base64`（MIT OR Apache-2.0）、`serde_json`。
均为 GPL-3.0-only 项目可用的宽松许可；发布时仍需按发布清单登记。

## 7. 代码位置

| 层 | 文件 | 职责 |
|---|---|---|
| 共享核心 | `crates/reading-storage/src/sync.rs` | 包格式、边车、SHA-256、导出/导入/合并、校验 |
| 共享核心 | `crates/reading-storage/tests/sync.rs` | 往返、幂等、移除传播、仅元数据、路径安全 |
| FFI | `crates/reading-ffi/src/engine.rs` / `exports.rs` | `nr_sync_export` / `nr_sync_import`（JSON 摘要 + `nr_buffer`） |
| 头文件 | `include/novel_reader.h` | 两个加性入口（共 42 项） |
| 桌面 | `crates/desktop-app/src/sync_ops.rs` / `settings.rs` / `ui.rs` | 目录选择、导出/导入、状态提示 |
| Android | `android/.../source/SyncSafBridge.kt` / `SyncSafPlatform.java` / `SyncedBooksPrefsStore.kt` | SAF 暂存搬运、同步书持久化 |
| Android | `MainActivity.kt` / `ReaderViewModel.kt` / `SettingsScreen.kt` | 目录选择、核心调用、书架重建、设置入口 |

## 8. 已验证

在 2026-10-25 的本机环境实际执行：

- `cargo test --workspace --locked`：通过；
- `cargo test --manifest-path crates/desktop-app/Cargo.toml`：通过（186 项 + 测试二进制）；
- `cd android && ./gradlew :app:testDebugUnitTest`：通过（352 项，含新增同步书/WebDAV 配置与解析测试）；
- `python3 scripts/verify-abi.py --kotlin android/app/src/main/java`：通过（42 项）；
- `python3 scripts/verify-android-v11-parity.py`：通过；
- `python3 scripts/verify-android-boundary.py`：通过；
- `python3 scripts/verify-android-scope.py`：通过；
- `python3 scripts/verify-core-scope.py`：通过；
- `python3 scripts/verify-abi.py`（无 Kotlin）：除「未提供 --kotlin」外通过；需带参数才是完整口径。

**未验证**：

- 真实 Android 设备上的 SAF 目录选择、复制、导出/导入（需按 `docs/android/device-runbook.md` 真机验收）；
- 真实百度网盘同步空间/坚果云客户端搬运后的冲突副本行为；
- Windows/macOS 桌面构建（本次只构建 Linux）；
- WebDAV 的真实服务/真机验收（代码已实现，见 `webdav-sync-implementation.md`）。

## 9. 后续必须补的

阶段 1 已实现 WebDAV（`state-<device_id>.json` + 共享 `books/`）、桌面/Android 配置入口、书架同步方向菜单与进度策略。
仍未闭环：

1. **真实服务兼容**：坚果云/InfiniCLOUD/Nextcloud 的 PROPFIND/PUT/HEAD 实测。
2. **凭证**：桌面改用设置表单/系统密钥环；Android 迁到 Keystore/EncryptedSharedPreferences。
3. **冲突 UX**：进度冲突弹窗；书签/分类墓碑与保留期。
4. **退避/配额**：429/503 指数退避、请求节流（坚果云 600/30min）、断点续传。
5. **线程模型**：桌面网络移出 UI 线程；Android 网络期间不长时间持有阅读串行门。
6. **真机验收**：至少两台 Android + 一台桌面，覆盖 WebDAV 双向同步与目录包手动同步。
