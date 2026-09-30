# Novel Reader V1 发布说明（模板）

> 本文件是**模板**：`{{...}}` 占位符必须由发布人用真实产物/脚本输出填写。
> `scripts/verify-v1-release.py` 会校验占位符存在（把模板填成结论会被判为失败），
> 并要求「已知限制」与「延期验收项」两节与 `docs/release/known-limitations.md` 保持一致。
> 填写前先运行门禁命令并把输出作为证据：

```bash
cd ${NR_PROJECT_DIR}
python3 scripts/verify-v1-structure.py
python3 scripts/verify-v1-release.py --desktop --android --json results/rel1-release-manifest.json
```

---

## 1. 版本与构建标识

| 项 | 值 |
|---|---|
| 版本 | `{{VERSION}}` |
| 发布类型 | `{{RELEASE_KIND}}`（内部/候选/正式） |
| Rust workspace / 桌面 crate | `{{RUST_WORKSPACE_VERSION}}` / `{{DESKTOP_CRATE_VERSION}}` |
| Android versionName / versionCode | `{{ANDROID_VERSION_NAME}}` / `{{ANDROID_VERSION_CODE}}` |
| Android BUILD_MARKER | `{{BUILD_MARKER}}` |
| 桌面构建标识（ELF build-id） | `{{DESKTOP_BUILD_ID}}` |
| 工具链 | rust `{{RUST_CHANNEL}}`、Gradle `{{GRADLE_VERSION}}`、AGP `{{AGP_VERSION}}`、Kotlin `{{KOTLIN_VERSION}}`、NDK `{{NDK_VERSION}}` |
| 依赖锁 | `Cargo.lock` 发布时 sha256 `{{CARGO_LOCK_SHA256}}`（package 数 `{{CARGO_LOCK_PACKAGES}}`） |
| 构建主机 / 会话 | `{{BUILD_HOST}}` / `{{BUILD_SESSION}}` |
| 构建时间 | `{{BUILD_TIME}}` |

## 2. 发布物清单与 SHA-256

| 发布物 | 大小(B) | SHA-256 | 签名状态 | 备注 |
|---|---|---|---|---|
| `{{DESKTOP_ARTIFACT}}` | `{{DESKTOP_SIZE}}` | `{{DESKTOP_SHA256}}` | 不适用 | 完整发布目录：`{{DESKTOP_RELEASE_DIR}}` |
| `dist/{{APK_DEBUG_NAME}}` | `{{APK_DEBUG_SIZE}}` | `{{APK_DEBUG_SHA256}}` | `{{APK_DEBUG_SIGNATURE}}` | Debug，仅供开发安装 |
| `dist/{{APK_RELEASE_NAME}}` | `{{APK_RELEASE_SIZE}}` | `{{APK_RELEASE_SHA256}}` | `{{APK_RELEASE_SIGNATURE}}` | Release；未签名时**不得**作为发布包分发 |

- 发布目录自校验：`sha256sum -c MANIFEST.sha256`（输出：`{{MANIFEST_CHECK_RESULT}}`）
- APK 内容校验：`python3 scripts/verify-android-apk.py <apk> --abi arm64-v8a`（结果：`{{APK_AUDIT_RESULT}}`）

## 3. 依赖与许可证

- Rust 依赖许可：覆盖两个 `Cargo.lock` 全量 package，未解析 `{{RUST_UNRESOLVED_COUNT}}` 项；
  需人工复核的许可族：`{{RUST_REVIEW_LICENSES}}`
- Android 依赖许可：覆盖 `build.gradle.kts` 直接声明，未解析 `{{ANDROID_UNRESOLVED_COUNT}}` 项；
  传递依赖缺口见已知限制
- 项目自身发行许可证：`{{PROJECT_LICENSE}}`（未定时必须写入本节，并同步已知限制）

## 4. 安装与启动

- 桌面（Linux X11）：解压 `{{DESKTOP_RELEASE_DIR}}` → 运行 `bin/novel-reader.sh`；系统要求见
  `share/doc/novel-reader/platform-requirements.md`。
- Android：`adb install -r dist/{{APK_RELEASE_NAME}}`（正式签名后）；包名 `{{ANDROID_PACKAGE}}`，
  minSdk `{{MIN_SDK}}`，首发 ABI `{{ABI}}`。

## 5. 验证结果（只写真实执行过的项）

| 检查 | 命令 | 结果 |
|---|---|---|
| 结构门禁 | `python3 scripts/verify-v1-structure.py` | `{{STRUCTURE_GATE_RESULT}}` |
| 发布门禁 | `python3 scripts/verify-v1-release.py --desktop --android` | `{{RELEASE_GATE_RESULT}}` |
| 核心测试 | `cargo test --workspace --all-targets` | `{{CORE_TEST_RESULT}}` |
| 桌面冒烟 | `bash scripts/smoke-desktop.sh --backend x11` | `{{DESKTOP_SMOKE_RESULT}}` |
| Android 单测 / APK | `./gradlew :app:testDebugUnitTest` + `verify-android-apk.py` | `{{ANDROID_TEST_RESULT}}` |
| 真机 / Windows / Wayland | 见已知限制 | 待验证 |

## 6. 已知限制

（从 `docs/release/known-limitations.md` 第 1–2 节择要摘录；不得改写为「通过」）

- `{{KNOWN_LIMITATION_1}}`
- `{{KNOWN_LIMITATION_2}}`

## 7. 延期验收项

（与 `docs/release/known-limitations.md` 第 1 节逐项对应，状态一律为 `待验证`）

| 项 | 状态 | 闭环前置 |
|---|---|---|
| `{{PENDING_ITEM_1}}` | 待验证 | `{{PENDING_ITEM_1_EXIT}}` |
| `{{PENDING_ITEM_2}}` | 待验证 | `{{PENDING_ITEM_2_EXIT}}` |

## 8. 回滚与数据备份

- 备份：退出应用后复制 `library.db`（桌面：`$XDG_DATA_HOME/novel-reader/`；Android：应用私有存储）。
- 回滚：覆盖 `library.db` 后重启；或部署上一发布目录（桌面）/ 覆盖安装上一版本 APK（Android）。
- 兜底：原始 TXT 永不被修改或删除；清缓存/移除书架不影响原始文件。
- 回滚验证结果：`{{ROLLBACK_CHECK_RESULT}}`（未实测时写 待验证）
