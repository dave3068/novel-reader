# 父级任务

## 状态

- Overall: 未开始

## 子规格门禁

- [ ] P1 — [reading-core](./reading-core/) 已实现并通过全部必需检查
  - Requirements: PR2, PR5, PR6
  - Depends on: 无
  - Definition of done: 子规格状态为 `已实现`；Rust 核心、SQLite、窗口限制、位置语义和 ABI 契约均有真实命令证据。
- [ ] P2 — [desktop](./desktop/) 已实现桌面 TXT 阅读闭环
  - Requirements: PR1, PR2, PR3, PR5
  - Depends on: P1
  - Definition of done: Linux X11 Release 构建、启动、TXT 导入/分页/设置/进度/书签检查通过；Windows 状态明确记录。
- [ ] P3 — [android](./android/) 已完成 Android 代码、单测和编译验证
  - Requirements: PR1, PR2, PR4, PR5
  - Depends on: P1；可与 P2 并行
  - Definition of done: Android 源码、JNI、JVM 单测、Rust Android 交叉编译、Debug/Release APK 校验通过；真机项保持延期状态。

## 父级集成

- [ ] P4 — 桌面优先集成与回归
  - Requirements: PR1–PR3, PR5
  - Affected areas: `crates/`, `desktop/`, `migrations/`, `tests/`
  - Depends on: P1, P2
  - Definition of done: Linux 桌面从真实 TXT 导入到重启恢复位置的端到端程序化检查通过，且旧方案文档未被改写。
- [ ] P5 — Android 构建产物集成
  - Requirements: PR2, PR4, PR6
  - Affected areas: `android/`, `scripts/`, `crates/reading-ffi/`
  - Depends on: P1, P3
  - Definition of done: 预编译 JNI 库、APK、签名/内容检查和未验证项清单完整；不要求真机调试通过。
- [ ] P6 — V1 发布前总门禁
  - Requirements: PR1–PR6
  - Affected areas: 全部正式工程目录
  - Depends on: P4, P5
  - Definition of done: 父级 checklist 的所有 required programmatic checks 通过；Windows/Wayland/Android 真机未完成时，状态只能为 `待验证`，不能标 `已实现`。

## 依赖摘要

- P1 → P2 → P4。
- P1 → P3 → P5。
- P4 与 P5 → P6。
- P2 与 P3 在核心 API 冻结后可并行；实施资源优先投入 P2。
