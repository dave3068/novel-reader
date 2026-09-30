# 跨平台安装与 UI 操作指南

## 目标
为当前 Linux 实施者和后续 Android/Windows/Linux 验证人员提供五候选可复制的安装、构建、启动、UI 操作、证据采集和清理说明；优先候选须明确桌面 Slint 与 Android 原生 UI/JNI 的不同步骤。

## 范围
### 范围内
- `linux.md`：Deepin 20.9 X11 实测命令、hybrid 桌面和四个对照候选；Ubuntu/Wayland 后续命令。
- `windows.md`：五候选安装、构建、启动、日志。
- `android.md`：优先 Android 原生 Kotlin/Compose/View/JNI，及三平台 Slint、Flutter、Compose、Qt 的 Android SDK/NDK/ARM64/ADB 步骤。
- `ui-operations.md`：共同 UI 语义、优先候选的允许差异与结果模板。
### 范围外
- 本机代替 Android/Windows/Wayland 实测、系统更新、签名或发布。

## 需求
### G-R1 — 当前 Linux 指南
指南 SHALL 记录 Deepin/X11、Rust 1.98.0、系统 glibc 2.28、用户 glibc 2.41 安全包装、`LD_PRELOAD`清理、hybrid 桌面端和四个对照候选的构建/启动/清理。
### G-R2 — Windows 指南
指南 SHALL 给出五候选 PowerShell 前置检查、SDK/JDK/Rust/框架安装、构建物、启动、诊断和清理。
### G-R3 — Android 指南
指南 SHALL 给出优先候选 Kotlin/Compose（必要时 View/TextLayout）、JNI、SDK/NDK/JDK、ARM64、ADB、SAF、返回、旋转、后台恢复和未实测标记；其余四候选也须有对应步骤。
### G-R4 — Linux X11/Wayland 区分
指南 SHALL 区分当前 X11 实测与未实测 Wayland/Ubuntu，并说明字体、高 DPI、选择器和后端。
### G-R5 — 统一 UI 操作矩阵
操作指南 SHALL 给出共同语义，说明优先候选跨端页边界可不同但内容/锚点语义必须一致。

## 验收标准
- G-AC1：命令引用真实工程路径/脚本及构建物位置。
- G-AC2：当前 Linux 有实测日志路径，其余平台显著标未实测。
- G-AC3：每个平台和五候选都有安装、构建、启动、操作、诊断、清理。
- G-AC4：Android 操作覆盖原生排版、JNI、SAF、返回、旋转、后台恢复；桌面覆盖文件选择、键鼠、高 DPI。

## 风险与延期
| 风险 | 缓解 |
|---|---|
| 未实测 SDK 命令漂移 | 锁版本、校验命令路径/`--help`、标示未实测 |
| 双 UI 操作矩阵漂移 | 共同语义与 hybrid 专项表，脚本校验每端条目 |
| 系统安装副作用 | 区分只读预检、用户目录安装和需 sudo 操作 |

- Android、Windows、Wayland 实测均延期，不得标为已验证。
