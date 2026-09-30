# Flutter 原型任务

## 状态
- Overall: 未开始

- [ ] FL-T1 — 建立 Flutter Linux 工程、锁文件和分页失败测试
  - Requirements: FL-R1, FL-R4
  - Affected areas: `experiments/ui-selection/flutter-probe/pubspec.yaml`, `lib/`, `test/`, `linux/`
  - Depends on: shared-foundation 已实现，hybrid-native-probe 的优先尖峰源码与桌面证据已交付
  - Definition of done: SDK/包版本锁定，复杂 Unicode 页末测试先失败。
- [ ] FL-T2 — 实现 PageLayouter 与字节映射
  - Requirements: FL-R1
  - Affected areas: `flutter-probe/lib/page_layouter.dart`, `flutter-probe/test/`
  - Depends on: FL-T1
  - Definition of done: 1,000 页、反向、重排和确定性检查完成判定。
- [ ] FL-T3 — 实现 `dart:ffi` 桥、回调和取消
  - Requirements: FL-R2
  - Affected areas: `flutter-probe/lib/ffi/`, `flutter-probe/test/ffi_test.dart`
  - Depends on: FL-T1
  - Definition of done: 生命周期、有限窗口、UI isolate 切换、取消和复制统计测试通过。
- [ ] FL-T4 — 条件实现统一三页面及探针
  - Requirements: FL-R3
  - Affected areas: `flutter-probe/lib/pages/`, `flutter-probe/lib/probe/`
  - Depends on: FL-T2, FL-T3
  - Definition of done: 分页通过时 UI 契约通过；否则合法淘汰记录完成。
- [ ] FL-T5 — Release 构建、X11 启动与证据归档
  - Requirements: FL-R4
  - Depends on: FL-T4
  - Definition of done: 全部必需检查通过，能力状态单独记录。

## 依赖摘要
- shared-foundation → FL-T1 → FL-T2/FL-T3 → FL-T4 → FL-T5。
