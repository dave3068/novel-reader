# 父级验证清单

## 环境
- 当前强制运行环境：Deepin 20.9、x86_64、X11、`DISPLAY=:0`。
- 前置：Rust 1.98.0；系统 glibc 保持 2.28，只有明确需要时按进程使用用户 glibc 2.41；不得全局覆盖 `LD_LIBRARY_PATH`。
- Android/Windows/Wayland 是操作文档交付范围，不是当前实机验收环境。
- 证据保存到 `experiments/ui-selection/results/raw/`。

## 必需程序化检查
- [ ] C1 (P1, PR1) — cmd: `python3 experiments/ui-selection/scripts/verify-workspace.py --all`; expect exit 0，Rust 共享业务库、11 个 ABI 入口、夹具/schema、优先候选和四个对照候选目录完整。
- [ ] C2 (P8, PAC5, PR5) — cmd: `bash experiments/ui-selection/scripts/build-linux.sh --release all`; expect exit 0，`hybrid-native` 桌面端与四个对照候选的 Release 构建物和日志存在。
- [ ] C3 (P8, PAC5, PR5) — cmd: `bash experiments/ui-selection/scripts/smoke-linux.sh --backend x11 --timeout 30 all`; expect exit 0，五个桌面尖峰分别写出 `ui_ready=true`、事件循环证据，随后无遗留进程。
- [ ] C4 (P2, P3, P4, P5, P6, P9, PR2, PR3, PR4) — file: `python3 experiments/ui-selection/scripts/verify-results.py --phase pagination --candidates hybrid-native slint flutter compose qt`; expect exit 0，五候选均有合法状态与证据；`hybrid-native` 另有复用/重复代码指标和 Android `未验证`标记。
- [ ] C5 (P7, P9, PR6, PR7) — cmd: `python3 experiments/ui-selection/scripts/verify-delivery.py`; expect exit 0，五候选指南完整且范围扫描不含数据库、网络 SDK、遥测或发布配置。

## 回归检查
- [ ] C6 (P9, PR7) — cmd: `git diff --exit-code -- docs/novel-reader-solution.md docs/ui-technology-selection-plan.md`; expect exit 0，实施未改写既有方案文档。

## 人工建议检查
- [ ] M1 (manual-advisory, PAC4) — action: 在当前 X11 按 `docs/ui-selection/ui-operations.md` 操作已通过桌面分页门槛的原型；expected: 三页面、翻页、菜单、重排与文件选择入口可见；programmatic fallback: C3、候选 UI 契约检查。
- [ ] M2 (manual-advisory, PAC6) — action: 在 Android 12+ 真机按指南安装 `hybrid-native/android-native`，完成中文分页、SAF、返回、旋转、后台恢复和 JNI 取消；expected: 生成 Android 真实能力结果；programmatic fallback: C4 的 JNI/业务边界静态检查；final reminder: 当前不声称 Android 已实测。
- [ ] M3 (manual-advisory, PR6) — action: 后续在 Windows、Wayland 实机按指南执行；expected: 可构建启动并完成操作表；programmatic fallback: C5。
