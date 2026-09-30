# desktop 任务

## 状态

- Overall: 未开始
- 优先级：高；核心 API 冻结后优先于 Android 真机调试。

## Phase 1 — Linux 桌面骨架

- [ ] D-T1 — 建立 Rust + Slint 正式桌面工程和状态编排
  - Requirements: DR2, DR3, DR4
  - Affected areas: `crates/desktop-app/`, `desktop/ui/`
  - Depends on: reading-core C-T6
  - Definition of done: 书架/阅读/设置页面骨架可启动；UI 只做状态投影；窗口线程和后台线程边界明确。

- [ ] D-T2 — 接入桌面文件选择、TXT 导入和来源状态
  - Requirements: DR1, DR5
  - Affected areas: `crates/desktop-app/`, `desktop/`, `tests/e2e/`
  - Depends on: D-T1, reading-core C-T2, C-T4
  - Definition of done: 真实 TXT 可导入；非 TXT、空文件、权限和失效文件有稳定错误；原文件不被修改。

## Phase 2 — 阅读闭环

- [ ] D-T3 — 实现中文分页、窗口重排和页面缓存
  - Requirements: DR3
  - Affected areas: `crates/desktop-app/`, `desktop/ui/`, `tests/contract/`
  - Depends on: D-T2, reading-core C-T3, C-T5
  - Definition of done: 实际正文区域与测量参数一致；当前/前后页缓存、合法边界、重排和 EOF 通过测试。

- [ ] D-T4 — 实现章节、进度、书签、设置和恢复
  - Requirements: DR2, DR5
  - Affected areas: `crates/desktop-app/`, `desktop/ui/`, `migrations/`
  - Depends on: D-T3
  - Definition of done: 章节跳转、书签、全局排版设置、重启恢复、失效文件重新关联可用。

## Phase 3 — 桌面优先门禁

- [ ] D-T5 — 完成 Linux X11 程序化 E2E 和 Release 冒烟
  - Requirements: DR1–DR5
  - Affected areas: `tests/e2e/`, `scripts/`, `results/`
  - Depends on: D-T4
  - Definition of done: Linux 真实 TXT 导入→阅读→保存→重启恢复→清理安全流程通过；Release 无残留进程。

- [ ] D-T6 — 补齐 Windows/Wayland 构建和发布验证路径
  - Requirements: DR6
  - Affected areas: `desktop/packaging/`, `.github/` 或 CI 配置、`scripts/`
  - Depends on: D-T5
  - Definition of done: 命令和依赖文档可复现；有实测环境前状态保持待验证，不伪造通过。

- [ ] D-T7 — 桌面回归和质量检查
  - Requirements: DR1–DR6
  - Affected areas: `crates/desktop-app/`, `desktop/`, `tests/`
  - Depends on: D-T5, D-T6
  - Definition of done: clippy/格式化/Release 构建/范围检查/关键 E2E 全部通过，证据已落盘。

## 依赖摘要

- reading-core → D-T1 → D-T2 → D-T3 → D-T4 → D-T5。
- D-T5 → D-T6 → D-T7。
- 只有 D-T3 完成后才允许扩展大量 UI 设置，避免在分页契约不稳定时堆叠页面。
