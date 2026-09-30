# 真机（Android）安装与延期验收说明 — AN6 分发包

适用卡片：AN6（`t_66d593c6`，交付构建产物与分发包）；延期验收卡片：V-AN2（`t_45b1cd70`）。
范围：把 `dist/app-debug.apk` 装到**物理 Android 设备**并按步骤核对。

> **诚实口径**：本文件只描述「怎么装、怎么核对、期望看到什么」，**不包含任何真机结论**。
> AN6 的完成定义只覆盖「可构建 + 可校验」；所有真机观察项在下表一律标记 `待验证`，
> 由 V-AN2 卡在具备设备后执行并写入 `results/verify/android-device-verify.md`。

## 1. 分发包与构建标记（BUILD_MARKER）核对方式

| 文件 | 用途 | 签名状态 | 安装方式 |
|---|---|---|---|
| `dist/app-debug.apk` | 真机调试包（本文件全部步骤都用它） | 已签名（Android Debug 调试签名） | `adb install -r` 直接安装 |
| `dist/app-release-unsigned.apk` | Release 产物 | **未签名**（工程未配置 signingConfig，属预期） | 需自行签名后才能安装 |

APK 大小、sha256 与本次构建标记以 `dist/BUILD-INFO.md` 为准（由 `scripts/build-android-apk.sh` 自动写入）。

BUILD_MARKER 三步核对（任一步不一致即说明**装的是别的包或过期包**）：

1. 应用启动后，标题栏第二行显示形如 `BUILD_MARKER / an6-apk-…-kotlin-2.0.21` 的字符串；
2. 与 `dist/BUILD-INFO.md` 中记录的 `BUILD_MARKER` 值逐字符比对；
3. 与 `android/app/build.gradle.kts` 里 `buildConfigField("String", "BUILD_MARKER", …)` 的值比对。

不一致时的处置：重新执行 `bash scripts/build-android-apk.sh` 生成新分发包，再 `adb install -r` 覆盖安装
（`-r` 保留应用数据；包名与 versionCode 未变时无需卸载）。

构建期自检命令（**不依赖设备**，可随时复跑）：

```bash
cd ${NR_PROJECT_DIR}
python3 scripts/verify-android-apk.py android/app/build/outputs/apk/debug/app-debug.apk --abi arm64-v8a
```

期望：exit 0，输出包名 `com.novelreader.android`、minSdk 31 / targetSdk 36、
`lib/arm64-v8a/libnr_jni.so`（含 ELF 头 ABI 判定）、DEX 构建标记命中的 dex 清单、签名状态。

## 2. 设备前提

| 项 | 要求 | 期望结果 |
|---|---|---|
| 系统版本 | Android 12 及以上（minSdk 31） | `adb shell getprop ro.build.version.release` 输出 ≥ 12 |
| ABI | arm64-v8a（首发唯一 ABI） | `adb shell getprop ro.product.cpu.abi` 输出 `arm64-v8a` |
| USB 调试 | 已授权本机 | `adb devices` 列出该设备且状态为 `device`（非 `unauthorized`） |
| MIUI/澎湃附加开关 | 「USB 安装」与「USB 调试（安全设置）」 | 见 `docs/android/miui-notes.md` |
| 测试文本 | 真实中文 TXT（UTF-8 / GB18030 / UTF-16 各一份） | 文件可被系统文件选择器看到 |

设备或开关不可得时：本文件所有步骤保持 `待验证`，并在延期证据中记录缺失前提（不得记为通过）。

## 3. 安装

```bash
adb devices
adb install -r ${NR_PROJECT_DIR}/dist/app-debug.apk
```

期望：`adb devices` 输出一行 `device`；`adb install` 输出 `Success`。

若 MIUI 未开启「USB 安装」，`adb install` 会报
`INSTALL_FAILED_USER_RESTRICTED: Install canceled by user`。两种处置：
开启该开关后重试，或把 `dist/app-debug.apk` 拷入手机后用文件管理器点击安装（绕过 USB 安装限制）。

## 4. 真机步骤与期望结果（AC-A6 延期项）

| # | 步骤 | 操作 / 命令 | 期望结果 | 状态 |
|---|---|---|---|---|
| S1 | 装包与版本核对 | `adb install -r dist/app-debug.apk`，打开应用 | 输出 `Success`；标题栏 BUILD_MARKER 与 `dist/BUILD-INFO.md` 一致 | 待验证 |
| S2 | UTF-8 中文 TXT | 书架 → 导入 UTF-8 TXT → 打开 | 正文中文正常、无乱码；能翻页 | 待验证 |
| S3 | GB18030 TXT | 导入 GB18030 编码 TXT | 正文无乱码（编码自动识别生效） | 待验证 |
| S4 | UTF-16 TXT | 导入 UTF-16（含 BOM）TXT | 正文无乱码、无空行错位 | 待验证 |
| S5 | 正文不滚动、无半字裁切 | 字号从最小 12sp 逐档放大到最大 | 正文区无滚动条；每页末行完整不出屏；行首行尾无半字 | 待验证 |
| S6 | 横竖屏切换 | 旋转设备 90° 后转回 | 重排后仍停在原阅读位置附近；无滚动、无裁切 | 待验证 |
| S7 | Home / 后台恢复 | 按 Home 后回前台（不杀进程） | 回到原页与原进度，无重排跳动 | 待验证 |
| S8 | 进程重建 | `adb shell am force-stop com.novelreader.android` 后重新打开 | 恢复到上次阅读位置（进度已持久化） | 待验证 |
| S9 | 系统返回 | 阅读页按系统返回键 | 回到书架，不崩溃、不丢进度 | 待验证 |
| S10 | 权限失效重关联 | 撤销该 TXT 的 URI 权限后再次打开该书 | 提示重新选择文件；不崩溃；库记录不丢 | 待验证 |
| S11 | 批次读取中途取消 | 导入大文件后在其读取过程中返回/取消 | 无残留半成品缓存；可重新导入；不崩溃 | 待验证 |
| S12 | 崩溃日志排查 | `adb logcat -d > results/verify/android-device-logs/logcat.txt` | 无本包名 PID 的 `FATAL EXCEPTION`（shell 进程崩溃不算） | 待验证 |

## 5. 证据与路径约定

| 项 | 约定路径 | 说明 |
|---|---|---|
| 主证据 | `results/verify/android-device-verify.md` | V-AN2 写入：逐条 S1–S12 判定、机型、系统版本、未覆盖项 |
| 附件日志 | `results/verify/android-device-logs/` | logcat、`adb shell uiautomator dump` 层级、安装输出等原始输出 |
| 构建期证据 | `results/an6-apk.md`、`results/an6-logs/` | AN6 产出：构建命令、退出码、校验输出（本文件不替代它） |

记录要求：写明**机型号 + 系统版本 + 构建标记**；环境不可得时把对应行保持 `待验证` 并注明前置条件
（例如「需开启开发者选项 → USB 安装（安全设置）」）。

## 6. 判读要点（避免误判）

- **未签名 Release 不是失败**：`dist/app-release-unsigned.apk` 未配置 signingConfig，`apksigner verify`
  报 `Missing META-INF/MANIFEST.MF` 属预期；真机安装请用 `dist/app-debug.apk`。
- **只看 `classes.dex` 会误判**：本项目的构建标记只出现在靠后的 dex（Debug 在 `classes5.dex`、
  Release 在 `classes2.dex`），判定「新代码是否入包」必须扫全部 `classes*.dex`（校验脚本已如此实现）。
- **shell 崩溃 ≠ 应用崩溃**：`logcat` 中 `com.android.commands.svc`、`uiautomator` 等 shell 进程的
  `FATAL EXCEPTION` 与本应用无关，判定前先确认 PID/进程名。
- **模拟器结论不能替代物理设备**：本文件所有步骤均以物理设备为准。
