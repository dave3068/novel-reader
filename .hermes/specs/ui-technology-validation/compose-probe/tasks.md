# Compose Multiplatform 原型任务

## 状态
- Overall: 未开始

- [ ] CM-T1 — 建立 Gradle Wrapper、版本目录和桥接选择尖峰
  - Requirements: CM-R2, CM-R4
  - Affected areas: `experiments/ui-selection/compose-probe/settings.gradle.kts`, `build.gradle.kts`, `gradle/`, `src/`
  - Depends on: shared-foundation 已实现，hybrid-native-probe 的优先尖峰源码与桌面证据已交付
  - Definition of done: 版本锁定；桥接方案通过最小生命周期测试并有选择记录。
- [ ] CM-T2 — 先测后实现 PageLayouter
  - Requirements: CM-R1
  - Affected areas: `compose-probe/src/desktopTest/`, `compose-probe/src/desktopMain/.../PageLayouter.kt`
  - Depends on: CM-T1
  - Definition of done: 1,000 页、反向、重排和 Unicode 判定完成。
- [ ] CM-T3 — 完成 FFI 回调、取消、线程和复制测量
  - Requirements: CM-R2
  - Affected areas: `compose-probe/src/desktopMain/.../ffi/`, `desktopTest/`
  - Depends on: CM-T1
  - Definition of done: UI dispatcher、取消竞态、释放和窗口大小断言通过。
- [ ] CM-T4 — 条件实现统一三页面及探针
  - Requirements: CM-R3
  - Affected areas: `compose-probe/src/desktopMain/.../ui/`
  - Depends on: CM-T2, CM-T3
  - Definition of done: UI 契约或淘汰门禁记录通过。
- [ ] CM-T5 — Release 构建、X11 启动和归档
  - Requirements: CM-R4
  - Depends on: CM-T4
  - Definition of done: 全部必需检查通过并记录候选能力状态。

## 依赖摘要
- shared-foundation → CM-T1 → CM-T2/CM-T3 → CM-T4 → CM-T5。
