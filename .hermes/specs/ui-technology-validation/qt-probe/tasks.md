# Qt Quick/QML 原型任务

## 状态
- Overall: 未开始

- [ ] QT-T1 — 建立锁定 Qt 的 CMake/QML 工程和失败测试
  - Requirements: QT-R1, QT-R4, QT-R5
  - Affected areas: `experiments/ui-selection/qt-probe/CMakeLists.txt`, `src/`, `qml/`, `tests/`
  - Depends on: shared-foundation 已实现，hybrid-native-probe 的优先尖峰源码与桌面证据已交付
  - Definition of done: Qt 模块最小化、版本锁定、QTextLayout 边界测试先失败。
- [ ] QT-T2 — 实现 PageLayouter
  - Requirements: QT-R1
  - Affected areas: `qt-probe/src/page_layouter.*`, `qt-probe/tests/`
  - Depends on: QT-T1
  - Definition of done: 1,000 页、反向、重排和 Unicode 判定完成。
- [ ] QT-T3 — 实现 C++/Rust C ABI 薄桥
  - Requirements: QT-R2
  - Affected areas: `qt-probe/src/ffi_bridge.*`, `qt-probe/tests/ffi_bridge_test.cpp`
  - Depends on: QT-T1
  - Definition of done: 生命周期、错误、回调线程、取消和复制统计通过。
- [ ] QT-T4 — 条件实现统一三页面和许可清单生成
  - Requirements: QT-R3, QT-R5
  - Affected areas: `qt-probe/qml/`, `qt-probe/src/probe_state.*`, `qt-probe/cmake/`
  - Depends on: QT-T2, QT-T3
  - Definition of done: UI 契约或淘汰门禁记录合法；动态库/插件/许可清单存在。
- [ ] QT-T5 — Release 构建、X11 启动和归档
  - Requirements: QT-R4
  - Depends on: QT-T4
  - Definition of done: 全部必需检查通过并记录候选能力状态。

## 依赖摘要
- shared-foundation → QT-T1 → QT-T2/QT-T3 → QT-T4 → QT-T5。
