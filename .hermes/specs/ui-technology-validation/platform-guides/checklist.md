# 平台指南验证清单

## 必需程序化检查
- [ ] G-C1 (G-T1, G-R1, G-R4) — file: `python3 experiments/ui-selection/scripts/verify-guides.py --platform linux`; expect exit 0，Deepin/X11、Rust/双 glibc/`LD_PRELOAD`、hybrid 桌面端、四个对照候选、Ubuntu/Wayland 未实测标识和六部分齐全。
- [ ] G-C2 (G-T2, G-R2) — file: `python3 experiments/ui-selection/scripts/verify-guides.py --platform windows`; expect exit 0，五候选 PowerShell 命令、版本、构建物和操作完整且标未实测。
- [ ] G-C3 (G-T3, G-R3) — file: `python3 experiments/ui-selection/scripts/verify-guides.py --platform android`; expect exit 0，hybrid Compose/View/JNI、SDK/NDK/JDK、ARM64、ADB、SAF、返回/旋转/后台及四对照候选完整且标未实测。
- [ ] G-C4 (G-T4, G-R5) — file: `python3 experiments/ui-selection/scripts/verify-guides.py --platform ui-operations`; expect exit 0，共同 UI 语义、跨端页边界差异、页面/输入/重排/选择器/证据矩阵完整。
- [ ] G-C5 (G-T5, G-R1–G-R5) — cmd: `python3 experiments/ui-selection/scripts/verify-delivery.py --check-command-paths --check-help`; expect exit 0，指南命令路径存在且支持 `--help`。

## 人工建议检查
- [ ] G-M1 (manual-advisory, G-AC4) — action: Android 真机先执行 hybrid 原生 UI/JNI 操作矩阵，再执行其余候选；expected: 真实结果回填且不覆盖`未验证`历史；programmatic fallback: G-C3、G-C4。
- [ ] G-M2 (manual-advisory, G-AC2) — action: Windows、Wayland 设备执行者照文档回填；expected: 命令/UI 结果可复现；programmatic fallback: G-C1–G-C5。
