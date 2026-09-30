# android 任务

## 状态

- Overall: 未开始
- 当前策略：先完成代码、单元测试、交叉编译和 APK 校验；真机调试延期。

## Phase 1 — Android 工程与 JNI

- [ ] A-T1 — 建立正式 Android Gradle 工程、版本锁和 ABI 构建
  - Requirements: AR5, AR6
  - Affected areas: `android/`, `scripts/build-android-libs.sh`
  - Depends on: reading-core C-T1, C-T5
  - Definition of done: minSdk/targetSdk/NDK/ABI 明确；Gradle Debug/Release 可配置；无 CMake 依赖倒灌。

- [ ] A-T2 — 接入正式 JNI/C ABI 和 Kotlin 薄封装
  - Requirements: AR2, AR5
  - Affected areas: `crates/reading-ffi/`, `android/app/src/main/java/**/core/`
  - Depends on: A-T1, reading-core C-T5
  - Definition of done: 句柄、错误、64 位偏移、缓冲区释放、取消和 request/layout id 与 Rust 一致。

## Phase 2 — Android TXT 阅读编排

- [ ] A-T3 — 实现 Compose 书架、阅读、设置和 ViewModel
  - Requirements: AR3, AR4
  - Affected areas: `android/app/src/main/java/**/ui/`, `android/app/src/main/java/**/reader/`
  - Depends on: A-T2
  - Definition of done: 页面状态可编译；所有阅读状态变更串行；重排请求合并；正文无 verticalScroll 伪分页。

- [ ] A-T4 — 实现 SAF TXT 数据源和非随机访问回退
  - Requirements: AR1, AR2, AR6
  - Affected areas: `android/app/src/main/java/**/source/`, `android/app/src/main/AndroidManifest.xml`
  - Depends on: A-T2, reading-core C-T2
  - Definition of done: URI 权限保存/检查、元数据、窗口读取、私有缓存复制和错误状态均有测试或静态证据。

- [ ] A-T5 — 固化 Android 排版口径和页面边界提交
  - Requirements: AR3, AR4
  - Affected areas: `android/app/src/main/java/**/reader/`, `tests/android/`
  - Depends on: A-T3, A-T4
  - Definition of done: 像素字号、实际行高、0 字距/安全余量、合法 UTF-8 页末和过期结果测试通过。

## Phase 3 — 构建交付与延期门禁

- [ ] A-T6 — 完成 JVM 单测、JNI 库、APK 构建和内容校验
  - Requirements: AR2, AR5
  - Affected areas: `android/`, `scripts/`, `tests/android/`
  - Depends on: A-T5
  - Definition of done: Kotlin/JVM、Rust Android、JNI `.so`、Debug/Release APK、aapt2 和签名校验通过。

- [ ] A-T7 — 生成真机调试包和延期验收清单
  - Requirements: AR1, AR3, AR4, AR6
  - Affected areas: `docs/android/`, `results/android/`
  - Depends on: A-T6
  - Definition of done: APK 带可核对构建标记；真机 SAF/旋转/后台/返回/JNI 取消步骤齐全，状态明确为待验证。

## 依赖摘要

- reading-core → A-T1 → A-T2 → A-T3/A-T4 → A-T5 → A-T6 → A-T7。
- A-T3 与 A-T4 在 JNI 薄封装稳定后可并行。
- A-T7 不要求真机执行；真机实际验证是后续独立门禁。
