# 优先候选任务

## 状态
- Overall: **已实现**（桌面端全部必需检查通过；Android 源码 / 薄 JNI / 操作指南 / 静态边界证据完整，实机状态显式为 `未验证`）

- [x] H-T1 — 建立双端工程、共享契约与指标基线
  - Requirements: H-R4, H-R5
  - Affected areas: `hybrid-native-probe/desktop-slint/`, `hybrid-native-probe/android-native/`, `shared/contracts/`
  - Depends on: shared-foundation 已实现
  - Definition of done: 双端只引用共享业务契约；复用/专属/重复代码指标 schema 与失败测试存在。
  - 证据: `shared/contracts/{abi,result.schema,ready.schema,business-responsibilities}.json`；
    Rust 契约测试 `abi_entry_table_is_complete`（11 入口 / 12 符号 / 头文件与错误码一致）；
    桌面端 UI 只有一份权威定义 `ui/app.slint`（无内联副本 → 重复 UI 代码量 0）；
    H-R5 指标实测复算：`python3 scripts/measure-reuse-metrics.py --write-results` →
    `shared_core_loc=2218`、`platform_specific_loc=2393`、`duplicated_ui_loc=0`、
    `rust_business_coverage=100.0%`（9/9 职责）、`reuse_ratio=48.10%`
    （`results/normalized/metrics-hybrid-native.json`）。

- [x] H-T2 — 先测后实现桌面 Slint 分页及直接 Rust 调用
  - Requirements: H-R1, H-R4
  - Affected areas: `desktop-slint/src/`, `desktop-slint/ui/`, `desktop-slint/tests/`
  - Depends on: H-T1
  - Definition of done: 1,000 页、反向、重排、Unicode、锚点和直接 Rust 调用测试完成。
  - 证据: `cargo test --release --test pagination` → **3/3 ok**；
    `results/raw/hybrid-native-desktop-pagination.json`：`status=通过`、`pages=1000`、
    `backward_checked=50`、`anchor_checked=50`、`corpus_bytes=1380250`。

- [x] H-T3 — 实现 Android Compose/View 分页与薄 JNI 适配
  - Requirements: H-R2, H-R3, H-R4
  - Affected areas: `android-native/app/src/main/`, `android-native/app/src/test/`, `android-native/jni/`
  - Depends on: H-T1
  - Definition of done: Android 排版/JNI/取消/生命周期/SAF 代码与静态契约测试完成；实机状态仍为未验证。
  - 证据: `python3 scripts/verify-hybrid-native-contract.py --check-android-layout --check-jni
    --check-no-business-duplication` → **exit 0，pass=24 fail=0，kotlin_business_duplication=false**；
    NDK 交叉编译证据 `results/raw/android-cross-compile.log`；
    **APK 打包已跑通**：`bash scripts/build-apk.sh --release` → exit 0，
    debug APK 23,705,238 B（调试签名，可安装）/ release 19,372,169 B，
    单元测试 `:app:testDebugUnitTest` tests=4 failures=0 → `results/normalized/build-hybrid-native-android.json`。

- [x] H-T4 — 实现双 UI 业务边界与位置语义检查
  - Requirements: H-R3, H-R4, H-R5
  - Affected areas: `scripts/verify-hybrid-native-contract.py`, `shared/rust-reading-core/tests/`
  - Depends on: H-T2, H-T3
  - Definition of done: 无 Kotlin 业务复制、C ABI/JNI 所有权、不同页边界锚点和指标字段检查通过。
  - 证据: `cargo test --release hybrid_semantics_` → **5/5 ok**；
    `shared/rust-reading-core/tests/hybrid_semantics.rs` 覆盖「两端页边界不同但内容连续 + 位置可跨端恢复」。

- [x] H-T5 — 按端分页门槛条件实现共同三页面
  - Requirements: H-R6
  - Affected areas: `desktop-slint/ui/`, `android-native/app/src/main/`
  - Depends on: H-T2, H-T3
  - Definition of done: 桌面通过时 UI 契约通过；Android 未实测时源码契约完整但结果保持未验证。
  - 证据: `verify-ui-contract.py --candidate hybrid-native --platform desktop` → **exit 0（status=通过）**；
    `--platform android --allow-unverified` → **exit 0（status=未验证）**。

- [x] H-T6 — 执行 Linux Release 启动和 Android 指南门禁
  - Requirements: H-R1, H-R5, H-R7
  - Depends on: H-T4, H-T5
  - Definition of done: 桌面构建/X11 smoke/结果通过；Android 指南和未验证标记通过检查。
  - 证据: `build-linux.sh --release hybrid-native` → exit 0（二进制 21,567,344 字节）；
    `smoke-linux.sh --backend x11 --timeout 30 hybrid-native` → exit 0（`ui_ready=true`、无残留进程）；
    `verify-results.py --phase pagination --candidates hybrid-native` → exit 0；
    Android 指南 `docs/ui-selection/android.md` 存在且被结果记录引用。

## 依赖摘要
- shared-foundation → H-T1 → H-T2/H-T3 → H-T4/H-T5 → H-T6。（已全部完成）
