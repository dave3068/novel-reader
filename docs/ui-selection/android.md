# Android 原生候选（hybrid-native）构建 · 安装 · 操作指南

> **状态声明**：本机（Deepin 20.9 / x86_64 / 无 KVM）**已完成** Gradle 构建与 APK 打包，
> 并对 APK 做了签名与内容校验；但**尚未在 Android 真机/模拟器上运行**，
> 因此候选能力状态仍为 **`未验证`**。下表逐项标注「已验证 / 未验证」。

## 1. 结论与产物

| 项目 | 状态 | 证据 |
|---|---|---|
| Kotlin/Compose 源码编译 | ✅ 已验证 | `gradle :app:assembleDebug` → `BUILD SUCCESSFUL` |
| JVM 单元测试 | ✅ 已验证（4/4，两条路径） | `gradle :app:testDebugUnitTest`（tests=4 failures=0 errors=0）；`scripts/run-android-unit-tests.sh` → `OK (4 tests)` |
| Rust 共享库交叉编译（arm64 / x86_64） | ✅ 已验证 | `scripts/build-android-libs.sh` |
| JNI `libnr_jni.so` 链接 + 符号剥离 | ✅ 已验证 | 自包含静态链接，12 个 JNI 方法；剥离后 7,158,272 → **1,419,320 B** |
| **APK 打包（arm64-v8a）** | ✅ 已验证 | debug **23,705,238 B（22 MB）**、release 19,372,169 B（18 MB）；`apksigner verify` 退出码 0（debug） |
| 真机安装 / 中文分页 / SAF / 返回 / 旋转 / 后台恢复 | ❌ **未验证** | 需 Android 12+ 设备，见第 5 节 |

成品路径：

```
experiments/ui-selection/dist/android-native-arm64-v8a-debug.apk              # 22 MB，调试签名，可直接安装
experiments/ui-selection/dist/android-native-arm64-v8a-release-unsigned.apk   # 18 MB，未签名（需自行签名）
experiments/ui-selection/hybrid-native-probe/android-native/app/build/outputs/apk/{debug,release}/
```

## 2. 环境准备（本机已装好，此处记录以便复现）

| 组件 | 版本 | 位置 |
|---|---|---|
| JDK | 17.0.12 | `/opt/dev-tools/jdk/jdk-17` |
| Gradle | 8.11.1 | `/opt/dev-tools/gradle/gradle-8.11.1` |
| AGP | 8.10.1（需 Gradle ≥ 8.11.1） | Gradle 依赖缓存 |
| Kotlin / Compose 插件 | 2.0.21 | Gradle 依赖缓存 |
| Android SDK | platforms;android-36、build-tools;36.0.0、platform-tools | `/opt/dev-tools/android-sdk` |
| NDK | 27.3.13750724（仅用于交叉编译与符号剥离） | `/opt/dev-tools/android-sdk/ndk/` |

> 磁盘约束：本机 `/` 仅 15G，**所有工具安装到 `/opt/dev-tools`（位于 393G 的 `/data` 分区）**，
> Gradle 缓存位于 `~/.gradle`（`/home` 分区）。请在其它机器上遵循同一原则。

安装 Gradle（如缺失）：

```bash
GV=8.11.1; mkdir -p /opt/dev-tools/gradle && cd /opt/dev-tools/gradle
curl -fL -o gradle-$GV-bin.zip https://mirrors.cloud.tencent.com/gradle/gradle-$GV-bin.zip
unzip -q gradle-$GV-bin.zip && rm -f gradle-$GV-bin.zip
```

## 3. 构建 APK

```bash
cd ~/my/novel-reader/experiments/ui-selection

# 3.1 交叉编译 Rust 共享库 + 链接 JNI 库 → app/src/main/jniLibs/arm64-v8a/libnr_jni.so
bash scripts/build-android-libs.sh                 # NR_ANDROID_ABIS 可覆盖 ABI 列表

# 3.2 打包 APK（等价于 --with-libs；会自动生成 local.properties）
bash scripts/build-apk.sh                          # debug APK（默认，可直接安装）
bash scripts/build-apk.sh --release                # 额外产出未签名 release APK
```

等价的手工命令：

```bash
export JAVA_HOME=/opt/dev-tools/jdk/jdk-17
export ANDROID_HOME=/opt/dev-tools/android-sdk
export PATH=/opt/dev-tools/gradle/gradle-8.11.1/bin:$PATH
cd hybrid-native-probe/android-native
printf 'sdk.dir=%s\n' "$ANDROID_HOME" > local.properties
gradle :app:assembleDebug          # 或 :app:testDebugUnitTest / :app:assembleRelease
```

**工程约定（重要）**：

- JNI **不**走 `externalNativeBuild`/CMake（本机 SDK 未装 CMake）。共享库由
  `scripts/build-android-libs.sh` 交叉编译为**自包含** `.so`，以预编译形式放在
  `app/src/main/jniLibs/arm64-v8a/`，Gradle 只负责打包。
- 必须设置 `ndkVersion = "27.3.13750724"`，AGP 才能找到 `llvm-strip` 对预编译 `.so`
  做符号剥离（实测 7,158,272 → 1,419,320 B）；否则会出现
  `Unable to strip the following libraries` 且 APK 显著偏大。
- 版本配对：**AGP 8.10.1 需要 Gradle ≥ 8.11.1**（本机用 8.11.1），并支持 `compileSdk = 36`。
  同时固定 `buildToolsVersion = "36.0.0"`，避免 AGP 联网自动下载未安装的版本。
- 本版本 `abiFilters` 仅 **arm64-v8a**；若要加 ABI，执行
  `NR_ANDROID_ABIS=arm64-v8a,x86_64 bash scripts/build-android-libs.sh` 并同步
  `app/build.gradle.kts` 的 `abiFilters`。

**踩坑记录（已修复）**：原生库体积变化后执行增量重打包，AGP 会在 APK 文件尾部
残留旧数据（实测多出 5,738,952 B 陈旧字节，文件 28.1 MB 而实际内容仅 22.6 MB）。
因此 `scripts/build-apk.sh` 在打包前会先删除旧 APK。若手工构建，请自行 `rm -f` 旧 APK。

## 4. 校验 APK

```bash
APK=hybrid-native-probe/android-native/app/build/outputs/apk/debug/app-debug.apk
export PATH=/opt/dev-tools/android-sdk/build-tools/36.0.0:$PATH
aapt2 dump badging "$APK" | head            # package / minSdk / targetSdk / label
unzip -l "$APK" | grep -E 'lib/|classes.*dex'
apksigner verify --print-certs "$APK"        # 退出码 0 = 签名有效
```

实测结果：`package=com.novelreader.probe.debug`、`minSdk=31`、`targetSdk=36`、
`lib/arm64-v8a/libnr_jni.so` 存在、7 个 `classes*.dex`、调试签名有效。

## 5. 在 Android 12+ 真机上验证（**当前未执行**）

> **真机实测记录（2026-09-12，Xiaomi 2211133C / MIUI V816 / Android 16 / arm64-v8a）**
>
> 已证实：**安装成功、可启动、正文可显示**（用户实机确认）。首次实测暴露并已修复 4 个真实缺陷：
>
> | # | 实机症状 | 根因 | 修复 |
> |---|---|---|---|
> | 1 | 顶部三个按钮被系统状态栏遮挡大半 | `targetSdk 36`（Android 15+）**强制 edge-to-edge**，代码未处理任何 `WindowInsets` | `enableEdgeToEdge()` + `Scaffold` + 顶栏 `statusBarsPadding()`；实测标题 y 由 **22 → 143** |
> | 2 | 点击界面/按钮/手势全无反应 | ① `ReaderScreen` 的 `fillMaxSize()` 嵌在外层 `Column` 内导致内容溢出，「上一页/下一页」**被挤出屏幕**；② 多处回调是**空实现**（`SettingsScreen(18f,false,{}, {})`、SAF 回调 `{}`）；③ 无任何可见反馈 | 重写为 `Scaffold`+`NavigationBar`（含选中态）+ 底部固定翻页栏；所有控件接入真实 ViewModel；状态行显示「页码·行数·章内偏移·预读数」 |
> | 3 | 手势无反应 | 正文区无手势处理 | 左 1/3 点击=上一页、右 1/3=下一页 |
> | 4 | 正文呈随机字符拼接 | 合成语料由字符池随机拼装 | 共享库语料改为连贯中文（公有领域诗句/格言） |
> | 5 | 状态行显示「预读 **9** 页」（应为 3） | `setViewport`/`loadBook` 并发重填预读缓存 | `Mutex` 串行化状态变更 |
>
> **第二轮真机复测（用户手动，2026-09-12）**：状态栏、底部导航、书架、翻页按钮、左右手势、系统返回
> **全部确认可用**；新暴露 3 个问题并已修复：
>
> | # | 症状 | 根因 | 修复 |
> |---|---|---|---|
> | 6 | **正文未按页排版，一页装不下、出现竖向滚动**（虽已分页且可翻页） | 测量与渲染**尺寸口径不一致**：`StaticLayout` 用 `18f` **像素**，Compose `Text` 用 `bodyLarge`（≈16**sp**≈51px），量出的「一页」可容约 3 倍字量 | 字号统一用**像素**（UI 传 `sp × density`），测量得出的**行高回传 Compose**（`lineHeight = lineHeightPx.toSp()`），测量预留半行安全余量；正文区去掉 `verticalScroll` |
> | 7 | **设置页字号滑杆无效** | 受控 `Slider` 的 `value` 绑定状态，而 `fontScale` 只在协程结束后写回 → 拖动时滑块弹回、正文不重排；且重排未串行、未合并 | `setFontScale` **同步**写回状态；重排改为**合并式请求**（拖拽只保留最终值）+ `Mutex` 串行 |
> | 8 | **SAF 选中文件后不显示内容** | 共享库的章节映射绑定模拟书籍元数据，外部文件长度与之不匹配 → 无可用坐标空间 | 共享库新增 `nr_open_external_book`：为外部内容登记「单章、长度 = 文件大小」坐标空间，字节由 Android 按窗口提供；页边界/位置/锚点仍由共享库裁定 |
> | 9 | 顶部空白偏多 | 顶栏为两行 + 较大内边距 | 顶栏压缩为**单行**（标题 + 状态，超出省略） |
>
> **第三轮真机复测（用户手动）**：分页已无滚动条、字号切换正常、打开文件正常、上方空白可接受；
> 仅剩「**小字号(12) 时底部最后一行只显示字的一小部分**」一项，已修复：
>
> | # | 症状 | 根因 | 修复 |
> |---|---|---|---|
> | 10 | 字号 **12** 时最后一行被裁切（其他字号正常） | Compose `bodyLarge` 的 **`letterSpacing = 0.5sp`**（已由 androidx 源码 `TypeScaleTokens.BodyLargeTracking = 0.5.sp` 核实）会让每行少放几个字，而 `StaticLayout` 测量不施加字符间距 → 渲染比测量**多出一行**；字号越小，0.5sp 占字宽比例越大（12sp 时 4.2%，18sp 时 2.8%），故仅小字号暴露 | ① Compose 侧显式 `letterSpacing = 0.sp`，与测量口径完全一致；② 测量再预留**整行**安全余量（宁可少排一行，绝不裁切） |
>
> 验证：共享基座 **22 项测试全绿**（含新增 `bounded_window_external_content_registers_single_chapter`）；
> 6 项门禁（H-C1/H-C2/H-C4/H-C5/H-C6 + 指标）全部 exit 0；JNI `.so` 导出 **13** 个 JNI 方法。
>
> 证据：`uiautomator` 层级转储显示「上一页/下一页」位于 y=2009–2064（修复前在屏幕外）、4 个可点击控件存在；`monkey` 注入触控后页面从「设置」切至「阅读」、开关切至「夜间模式」（旧版开关回调为空，绝无可能变化）→ **触控链路已通**。
>
> **注意**：`logcat` 中的 `FATAL EXCEPTION: java.lang.SecurityException ... com.android.commands.svc.Svc.main` 来自 `adb shell svc` 被 MIUI 拒绝，**与本应用无关**；判断崩溃请以 `PID`/堆栈首行是否属于本包名为准。

### 5.1 MIUI/澎湃 机型的 adb 限制（实测）

| 受限能力 | 报错 | 需要的开关 |
|---|---|---|
| 静默安装 APK | `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user` | 开发者选项 →「**USB 安装**」 |
| `adb shell input` 模拟点击 | `SecurityException: requires INJECT_EVENTS permission` | 开发者选项 →「**USB 调试（安全设置）**」 |
| 安装测试 APK（instrumentation） | 同安装限制 | 同上两项 |
| `adb shell settings put` / `svc power` | `SecurityException: WRITE_SETTINGS` | shell 无权限，无法绕过 |

> 未开启上述开关时的可用替代：`adb shell monkey -p <pkg> --throttle 150 -v 30`（monkey 走 instrumentation 路径，**可以**注入触控，用于通路验证）；`adb shell uiautomator dump` 做层级/边界断言（无需特殊权限）。
> 若无法开启开关，可直接把 `dist/*.apk` 拷入手机本地点击安装（绕过 USB 安装限制）。

### 5.2 逐项操作清单

> **构建标记**：`app/build.gradle.kts` 注入 `BuildConfig.BUILD_MARKER`，顶栏会显示
> 「novel-reader 探针 v3-letterSpacing-fix」——真机上据此即可确认装的是哪一次构建。
> （注意：校验 APK 是否含新代码要扫**全部** `classes*.dex`，应用自身类不在 `classes.dex`，
> 那里是体积最大的 Compose 库 dex。）

```bash
adb devices                                   # 确认设备已授权
adb install -r experiments/ui-selection/dist/android-native-arm64-v8a-debug.apk
adb shell am start -n com.novelreader.probe.debug/com.novelreader.probe.MainActivity
adb logcat -s NovelReaderProbe:V              # 如需观察日志
```

逐项操作清单（需填入实测结果）：

1. **中文分页**：打开「阅读」页 → 上/下页翻动 20 次，确认无缺字、无裁切、页码递增、页末不为半个字。
2. **章节切换**：配置页的章节列表点选任意章，确认从该章开头重新分页。
3. **SAF 选书**：书架页触发文件选择（`ActivityResultContracts.OpenDocument`），选一个 UTF-8 文本文件，确认按有限窗口读取而非整本载入。
4. **系统返回**：阅读页按系统返回键 → 应回到书架（`BackHandler`），不直接退出应用。
5. **屏幕旋转**：旋转后当前页/位置不丢失（`rememberSaveable` / `ViewModel`）。
6. **后台恢复**：Home 键切后台再回来，位置保持（`LifecycleEventObserver` `ON_STOP` 时保存位置）。
7. **JNI 取消**：触发一次批次加载并在完成前退出页面，确认走 `cancelRequest` 且结果为部分成功。
8. **生命周期**：反复横竖屏 + 前后台切换 10 次，确认无崩溃、无缓冲泄漏告警。

完成后请把真实结果写回
`experiments/ui-selection/results/normalized/`（`android_verified` 置为 `true`），
并同步更新该文件顶部状态表。

## 6. 清理

```bash
gradle --stop                                          # 停掉 Gradle 守护进程
rm -rf hybrid-native-probe/android-native/app/build    # 清理构建产物
rm -rf hybrid-native-probe/android-native/.gradle
```
