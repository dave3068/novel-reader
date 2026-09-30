# 父级任务

## 状态
- Overall: 未开始

## 子规格门禁
- [ ] P1 — [shared-foundation](./shared-foundation/) 已实现并通过全部必需检查
  - Requirements: PR1, PR7
  - Depends on: 无
  - Definition of done: 子规格状态为已实现，父级读取直接 Rust/C ABI 与业务语义证据。
- [ ] P2 — [hybrid-native-probe](./hybrid-native-probe/) 已实现优先候选源码与当前 Linux 桌面证据
  - Requirements: PR2, PR3, PR4, PR5
  - Depends on: P1
  - Definition of done: 桌面尖峰通过 Linux 门禁；Android 源码/JNI/操作指南/静态边界证据完整，实机状态明确为未验证。
- [ ] P3 — [slint-probe](./slint-probe/) 已实现并产生候选 B 判定
  - Requirements: PR3, PR4, PR5
  - Depends on: P1, P2
  - Definition of done: 子规格必需检查通过；能力结论独立于实施状态。
- [ ] P4 — [flutter-probe](./flutter-probe/) 已实现并产生候选 C 判定
  - Requirements: PR3, PR4, PR5
  - Depends on: P1, P2
  - Definition of done: 同 P3。
- [ ] P5 — [compose-probe](./compose-probe/) 已实现并产生候选 D 判定
  - Requirements: PR3, PR4, PR5
  - Depends on: P1, P2
  - Definition of done: 同 P3。
- [ ] P6 — [qt-probe](./qt-probe/) 已实现并产生候选 E 判定
  - Requirements: PR3, PR4, PR5
  - Depends on: P1, P2
  - Definition of done: 同 P3。
- [ ] P7 — [platform-guides](./platform-guides/) 已实现
  - Requirements: PR6
  - Depends on: P2, P3, P4, P5, P6
  - Definition of done: 五候选命令与最终工程路径一致且文档检查通过。

## 父级集成
- [ ] P8 — 运行五个桌面 Linux Release 构建与 X11 启动验收
  - Requirements: PR5
  - Affected areas: `experiments/ui-selection/scripts/`, `experiments/ui-selection/results/raw/`
  - Depends on: P2, P3, P4, P5, P6
  - Definition of done: C2、C3 通过，五个桌面尖峰均有独立构建物和就绪证据。
- [ ] P9 — 执行追溯、范围、结果与文档总门禁
  - Requirements: PR1–PR7
  - Affected areas: `experiments/ui-selection/`, `docs/ui-selection/`
  - Depends on: P7, P8
  - Definition of done: C1、C4、C5、C6 通过，所有子状态同步后 Overall 改为已实现。

## 依赖摘要
- P1 → P2 → P3/P4/P5/P6（四个对照候选可在隔离目录中并行）。
- P2/P3/P4/P5/P6 → P7/P8 → P9。
