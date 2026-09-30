# Slint 原型任务

## 状态
- Overall: 未开始

- [ ] SL-T1 — 建立 Slint 工程、锁文件和失败优先的分页测试
  - Requirements: SL-R1, SL-R3
  - Affected areas: `experiments/ui-selection/slint-probe/Cargo.toml`, `src/`, `tests/`
  - Depends on: shared-foundation 已实现，hybrid-native-probe 的优先尖峰源码与桌面证据已交付
  - Definition of done: 公共排版接口测试先失败，FemtoVG/Skia feature 互斥且版本锁定。
- [ ] SL-T2 — 实现 PageLayouter 与共同 C ABI 接入
  - Requirements: SL-R1, SL-R2
  - Affected areas: `slint-probe/src/page_layouter.rs`, `slint-probe/src/engine.rs`
  - Depends on: SL-T1
  - Definition of done: 1,000 页、反向、重排、Unicode 与 FFI 检查通过。
- [ ] SL-T3 — 实现三种渲染器构建和软件中文诊断
  - Requirements: SL-R3, SL-R5
  - Affected areas: `slint-probe/src/main.rs`, `slint-probe/build.rs`, 候选 adapter
  - Depends on: SL-T2
  - Definition of done: FemtoVG/Skia Release 可构建启动，软件诊断结果合法。
- [ ] SL-T4 — 按分页判定条件实现统一三页面
  - Requirements: SL-R4
  - Affected areas: `slint-probe/ui/`, `slint-probe/src/app_state.rs`
  - Depends on: SL-T2
  - Definition of done: 通过/有条件通过时 UI 契约检查通过；不通过时生成淘汰记录且不存在伪完整原型声明。
- [ ] SL-T5 — 执行候选门禁并记录证据
  - Requirements: SL-R1–SL-R5
  - Depends on: SL-T3, SL-T4
  - Definition of done: 本子规格全部必需检查通过，Overall 改为已实现；能力状态单独记录。

## 依赖摘要
- shared-foundation → SL-T1 → SL-T2 → SL-T3/SL-T4 → SL-T5。
