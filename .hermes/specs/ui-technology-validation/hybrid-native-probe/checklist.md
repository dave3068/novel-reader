# 优先候选验证清单

## 环境
- 实测环境：Deepin 20.9 / x86_64 / X11（`DISPLAY=:0`）/ glibc 2.28；Rust 1.98.0；Slint =1.17.1（Release）。
- Android 侧：NDK 27.3.13750724、API 31；**未在实机/模拟器运行**，能力状态为 `未验证`。

## 必需程序化检查（全部实跑通过）
- [x] H-C1 (H-T2, H-R1, H-R4) — test: `bash experiments/ui-selection/scripts/test-pagination.sh hybrid-native --platform desktop --pages 1000`
  - **exit 0**；`results/raw/hybrid-native-desktop-pagination.json`：`status=通过`、`pages=1000`、
    `backward_checked=50`、`anchor_checked=50`、`read_calls=2121`、`max_window=262144`；
    `results/raw/test-pagination-hybrid-native-desktop.log`、`results/normalized/pagination-hybrid-native-desktop.json`。
- [x] H-C2 (H-T3, H-R2, H-R3) — file: `python3 experiments/ui-selection/scripts/verify-hybrid-native-contract.py --check-android-layout --check-jni --check-no-business-duplication`
  - **exit 0，pass=24 fail=0**，`kotlin_business_duplication=false`；
    Compose/View 公开 API 依据、UTF-8 字节映射、缓冲所有权、取消/异常/生命周期/SAF 入口全部命中；
    `results/raw/verify-hybrid-native-contract.log`。
- [x] H-C3 (H-T4, H-R4, H-R5) — test: `cargo test --release --manifest-path experiments/ui-selection/shared/rust-reading-core/Cargo.toml hybrid_semantics_`
  - **exit 0，5/5 ok**：不同 UI 页边界仍可恢复同一原文附近位置；页链连续、无丢字/重字/非法边界；
    Kotlin 无共享业务复制扫描由 H-C2 的 `--check-no-business-duplication` 覆盖并通过。
- [x] H-C4 (H-T5, H-R6) — cmd: `python3 .../verify-ui-contract.py --candidate hybrid-native --platform desktop && python3 .../verify-ui-contract.py --candidate hybrid-native --platform android --allow-unverified`
  - 两条均 **exit 0**；桌面 `status=通过`（三页面 + 11 项交互入口 + 分页门槛）；Android `status=未验证`
    （三屏 @Composable + SAF/返回/旋转/后台恢复/取消入口完整，且不冒充实测）。
- [x] H-C5 (H-T6, H-R1, H-R7) — cmd: `bash .../build-linux.sh --release hybrid-native && bash .../smoke-linux.sh --backend x11 --timeout 30 hybrid-native`
  - 两条均 **exit 0**；Release 二进制 21,567,344 字节；探针 `ui_ready=true`、`event_loop=running`、
    `backend=x11`、`renderer=femtovg`、`pagination_status=通过`，退出后无残留进程。
- [x] H-C6 (H-T1, H-T6, H-R5, H-R7) — file: `python3 .../verify-results.py --phase pagination --candidates hybrid-native`
  - **exit 0，pass=2 fail=0**；桌面 `status=通过`、Android `status=未验证`；
    metrics 含 `rust_business_coverage=100.0`、`platform_specific_loc=0`、`duplicated_ui_loc=0`、
    `android_verified=false`、`android_guide=docs/ui-selection/android.md`（文件存在）。

## 补充证据
- Android 交叉编译：`cargo build --release --target {aarch64,x86_64}-linux-android` 均 exit 0；
  JNI `.so` 静态链接成功（aarch64 7,158,272 字节 / x86_64 6,767,640 字节），导出 12 个 JNI 方法、
  内联 19 个 `nr_*` 入口，除 1 个 weak libc 符号外无未定义符号 → `results/raw/android-cross-compile.log`。
- **Android 单元测试已真实执行**：`bash scripts/run-android-unit-tests.sh` → **exit 0**，
  `kotlinc`（kotlin-compiler-embeddable 1.9.24）编译 `Utf8OffsetMapper.kt` / `NrCore.kt` 与两个测试类，
  `org.junit.runner.JUnitCore` 输出 **`OK (4 tests)`** → `results/raw/android-jvm-unit-tests.log`、
  `results/normalized/unit-test-hybrid-native-android.json`。
  （说明：本机无 gradle/kotlinc 发行包，故以 kotlinc + JUnit 在 JVM 上执行纯 JVM 测试；
  依赖 Android 框架的部分仍属规格内「未验证」范围。）
- **H-R5 指标已由占位值改为实测复算**：`python3 scripts/measure-reuse-metrics.py --write-results` → exit 0。
  `shared_core_loc=2218`（12 个 .rs / 2799 行）、`shared_ffi_header_loc=102`、`desktop_ui_loc=545`、
  `desktop_rust_glue_loc=1293`、`android_kotlin_loc=438`、`android_jni_c_loc=117`、
  `platform_specific_loc=2393`、`duplicated_ui_loc=0`（按契约同源规则扫描 UI 层实测，非硬编码常量）、
  `rust_business_coverage=100.0%`（9/9 职责在共享库中全部命中）、`reuse_ratio=48.10%`
  → `results/raw/reuse-metrics.log`、`results/normalized/metrics-hybrid-native.json`，
  并已回写进两份分页结果的 `metrics` 字段。
- **门禁稳定性**：三个 bash 门禁（build/smoke/test-pagination）连续 3 轮均为 exit 0；
  期间修复了 `ldd --version | head -1` 在 `set -o pipefail` 下因 `head` 提前关闭管道
  触发 SIGPIPE(141) 而中止脚本的竞态缺陷。
- **Android APK 已成功打包（arm64-v8a）**：`bash scripts/build-apk.sh --release` → exit 0。
  `Gradle 8.11.1 + AGP 8.10.1 + Kotlin 2.0.21`，`compileSdk/targetSdk 36`、`minSdk 31`。
  - debug APK **23,705,238 B（22 MB）**，调试签名，`apksigner verify` 退出码 0，可直接安装
    → `dist/android-native-arm64-v8a-debug.apk`
  - release APK 19,372,169 B（18 MB，未签名）→ `dist/android-native-arm64-v8a-release-unsigned.apk`
  - `lib/arm64-v8a/libnr_jni.so` 经 `llvm-strip` 剥离：7,158,272 → **1,419,320 B**
  - Kotlin 单测再经 Gradle 官方路径执行：`:app:testDebugUnitTest` tests=4 failures=0 errors=0
  - 结果记录：`results/normalized/build-hybrid-native-android.json`（status=通过）
- **踩坑（已修复）**：AGP 增量重打包在原生库体积变化后于 APK 尾部残留 5,738,952 B 陈旧字节
  （文件 28.1 MB 而实际内容 22.6 MB）；`build-apk.sh` 已加入打包前清理旧 APK。

## 人工建议检查
- [ ] H-M1 (manual-advisory, H-AC2) — action: 在 Android 12+ 真机按 `docs/ui-selection/android.md` 构建、安装并依次验证中文分页、SAF、系统返回、旋转、后台恢复与 JNI 取消；expected: 填入 Android 实测结果；programmatic fallback: H-C2、H-C3、H-C4（均已实跑通过）。
  - 状态：**部分完成（人工真机验收通过，自动化项受阻）**。
    - 已由用户在三轮真机复测中人工确认（规格 H-AC2 manual-advisory）：安装与启动、正文中文显示、
      分页（页内无滚动条）、小字号(12)末行无裁切、底部导航三页切换、书架列表与进入书籍、
      上一页/下一页按钮、正文左右点击手势、设置页字号切换与重排、夜间模式、**SAF 打开文件并显示正文**、
      阅读页系统返回、状态栏不遮挡与上方留白。
    - **未覆盖**：屏幕旋转、后台恢复、JNI 取消，以及自动化 instrumentation 用例
      （该 MIUI 机型默认关闭「USB 安装」与「USB 调试（安全设置）」，adb 无法静默安装与注入 `input`）。
    - 证据：`docs/ui-selection/android.md` §5 三轮真机实测记录；`uiautomator` 层级转储；
      `monkey` 触控通路验证；`results/normalized/android-device-verification.json`（人工验收记录）。

> **状态联动机制**：Android 能力状态由 `results/normalized/android-device-verification.json` 驱动——
> 记录存在且 `verified=true` → 门禁写 `通过` 且 `android_verified=true`；记录缺失或未标记 → 一律退回
> `未验证`。已实测两个方向均自洽（缺记录时 H-C6 仍为 0，因为规则要求状态与 `android_verified` 一致）。
> 这样既能把人工验收固化为可复现证据，也杜绝「状态过时」。构建标记 `v3-letterSpacing-fix` 可在真机
> 顶栏核对装机版本。

## 真机实测记录（2026-09-12 · Xiaomi 2211133C / MIUI V816 / Android 16 / arm64-v8a）

- 已由用户实机确认：**安装成功、可正常启动、正文文字可显示**。
- 首次实测暴露并已修复 **5 个真实缺陷**（详见 `docs/ui-selection/android.md` §5）：
  1. 顶部按钮被状态栏遮挡 → `targetSdk 36` 强制 edge-to-edge 而未处理 insets；
     修复后 `uiautomator` 实测标题 y 由 **22 → 143**。
  2. 点击/手势全无反应 → `fillMaxSize()` 溢出使「上一页/下一页」出屏 + 多处回调为空实现
     + 无可见反馈；修复后按钮位于 y=2009–2064（屏内），且 `monkey` 注入触控可切换页面与开关。
  3. 正文为随机字符拼接 → 共享库合成语料改为连贯中文（基座 21 项测试仍全绿）。
  4. 预读缓存并发膨胀（实测 9 页，应为 3）→ `Mutex` 串行化状态变更。
  5. 新增真机 UI 交互回归测试 `app/src/androidTest/.../InteractionTest.kt`（4 用例）。
- **应用自身无崩溃**；`logcat` 中的 `FATAL EXCEPTION` 经核实来自 `adb shell svc`（`Svc.main`），与本应用无关。
- **第二轮真机复测（用户手动）**：状态栏、底部导航、书架、翻页按钮、左右手势、系统返回**均确认可用**；
  新暴露并修复 3 项：正文尺寸口径不一致导致「一页装不下、可滚动」（改为像素口径 + 行高回传）、
  设置页字号滑杆无效（受控滑杆状态未同步 + 重排未合并）、SAF 选中文件不显示内容
  （共享库新增 `nr_open_external_book` 外部内容单章坐标空间）。
- 复测后验证：共享基座 **22 项测试全绿**；H-C1/H-C2/H-C4/H-C5/H-C6 与指标复算全部 exit 0；
  JNI `.so` 导出 13 个 JNI 方法；APK 重新打包成功（23,807,141 B，签名有效）。
- 阻塞项：该 MIUI 机型默认关闭「USB 安装」与「USB 调试（安全设置）」，导致 adb 无法静默安装、
  无法注入 `input` 事件，故**交互修复的真机确认改由用户手动复测**；`android_verified` 维持 `false`。

## 桌面端实测记录（2026-09-12 · Rust + Slint 1.17.1 / X11 / femtovg）

- 由用户人工复测发现 **2 个真实缺陷**，均已修复（详见 `docs/ui-selection/desktop.md` §2）：
  1. **切夜间模式时阅读区不变色**（仍白底黑字）——`reader-bg`/`reader-fg` 只按
     `background-mode` 取色、未考虑 `theme-dark`；已改为同时响应主题与背景模式。
  2. **阅读区看起来「没有分页」**（正文只占上部、下方大片空白）——排版配置固定为
     `desktop_default()` 的 800×560，而真实正文区约 1032×560。修复过程中另发现两个陷阱：
     - `window.size()` 在**事件循环启动前返回 0×0**（日志实锤 `正文区 -48x-160`），
       对齐必须移到**窗口映射后**（就绪探针定时器内），修好后为 `正文区 1032x560 重排=true`；
     - Slint `Text` **不叠加段间距/首行缩进**，而配置带 `paragraph_spacing=8`/`first_line_indent=36`
       → 每页少排 6 行（实测 16 行 vs 应有 22 行）→ 桌面端将两项置 0 后正文铺满。
  - 另修 `reflow_ok` 自检刺激过弱（宽度 −96px 对「一句一行」语料不改变断行）→ 同时改变字号。
- **用户人工核验：夜间模式阅读页已正常** ✔
- 无头截图核验：阅读页显示中文正文、`第1页 · 16行`、底部 `上一页/下一页/保存位置` 齐备、
  配色与排版正常（方法：`xdotool` 置顶 + `ffmpeg x11grab` 仅截窗口 + 图像核验）。
- 相关门禁：`test-pagination.sh --platform desktop`（1000 页硬门槛）、
  `smoke-linux.sh --backend x11`、`verify-ui-contract.py --platform desktop` 均 exit 0。
