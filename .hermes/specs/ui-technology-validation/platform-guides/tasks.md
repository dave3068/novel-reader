# 平台指南任务

## 状态
- Overall: 未开始

- [ ] G-T1 — 写 Linux 安装、构建和启动指南
  - Requirements: G-R1, G-R4
  - Affected areas: `docs/ui-selection/linux.md`
  - Depends on: 五候选最终脚本 CLI 稳定
  - Definition of done: 当前 Deepin/X11 命令附真实证据；hybrid 桌面端与四个对照候选完整；Ubuntu/Wayland 标未实测。
- [ ] G-T2 — 写 Windows 安装、构建和启动指南
  - Requirements: G-R2
  - Affected areas: `docs/ui-selection/windows.md`
  - Depends on: G-T1
  - Definition of done: 五候选均含 PowerShell、构建物、启动、诊断和清理。
- [ ] G-T3 — 写 Android 安装、构建、安装和操作指南
  - Requirements: G-R3
  - Affected areas: `docs/ui-selection/android.md`
  - Depends on: G-T1
  - Definition of done: hybrid 原生 Compose/View/JNI 与四个对照候选均含 SDK/NDK/ARM64/ADB/SAF/生命周期步骤并标未实测。
- [ ] G-T4 — 写统一 UI 操作矩阵
  - Requirements: G-R5
  - Affected areas: `docs/ui-selection/ui-operations.md`
  - Depends on: G-T1, G-T2, G-T3
  - Definition of done: 共同语义、跨端允许差异、页面、输入、重排、文件选择和证据记录可逐项执行。
- [ ] G-T5 — 程序化核对指南与工程
  - Requirements: G-R1–G-R5
  - Affected areas: `experiments/ui-selection/scripts/verify-delivery.py`, `docs/ui-selection/`
  - Depends on: G-T4
  - Definition of done: 本子规格全部必需检查通过。

## 依赖摘要
- 候选 CLI 稳定 → G-T1 → G-T2/G-T3 → G-T4 → G-T5。
