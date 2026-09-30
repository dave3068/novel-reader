# 优先候选：桌面 Slint + Android 原生 UI + Rust 共享业务库

## 目标
验证桌面端直接调用 Rust + Slint 与 Android Kotlin 原生 UI（默认 Jetpack Compose，必要时公开 View/TextLayout）能否共享同一 Rust 阅读业务，而不是强制共享 UI 源码。

## 范围
### 范围内
- `hybrid-native-probe/desktop-slint/`：桌面 Rust/Slint 分页尖峰、直接 Rust 调用、当前 Linux Release 启动和条件完整原型。
- `hybrid-native-probe/android-native/`：Kotlin/Compose 分页、薄 JNI/C ABI、Android 状态/错误/取消/SAF/生命周期实现入口、静态边界检查和操作指南。
- 跨端内容位置、分页锚点、布局失效和共享业务复用证据。
### 范围外
- 在本机实际 Android 构建、安装、UI/SAF/生命周期验收；正式 Android 应用、数据库、发布和性能结论。

## 需求
### H-R1 — 桌面直接 Rust + Slint
桌面端 SHALL 通过直接 Rust API 完成公共 API 分页和共享业务调用，支持当前 Linux Release 就绪探针。
### H-R2 — Android 原生排版
Android 端 SHALL 以 Kotlin + Jetpack Compose 实现 UI；若 Compose 的公开文字 API不足，局部可用公开 View/TextLayout。必须输出合法 UTF-8 页末位置并映射到共享库位置。
### H-R3 — 薄 JNI/C ABI
Android 端 SHALL 仅以 JNI/C ABI 调用 Rust 共享库，显式管理缓冲、线程切换、取消、异常和生命周期，不传整本正文。
### H-R4 — 双 UI 共享业务语义
两端 SHALL 使用同一 Rust 位置、锚点、缓存失效、模拟领域状态和错误模型；分页边界可不同，但两端各自连续且可恢复原文附近位置。
### H-R5 — 复用与维护成本证据
实现 SHALL 记录 Rust 业务覆盖率、平台专属代码量、重复 UI 代码量和 Android 业务复制扫描结果。
### H-R6 — 条件完整原型
通过各自分页门槛的端 SHALL 实现共同 UI 语义；Android 未实测不允许把整体候选标为通过。
### H-R7 — 当前 Linux 与 Android 指南
桌面端 SHALL 当前 Linux Release 构建/启动；Android SHALL 提供逐步安装、构建、ADB、SAF、返回、旋转、后台恢复操作方式并标未验证。

## 验收标准
- H-AC1：桌面分页、Rust 业务与 Linux X11 启动检查通过。
- H-AC2：Android 项目、JNI 头/实现和 Compose/View 适配存在，静态检查证明未复制共享业务且边界完整。
- H-AC3：共享契约测试验证不同页边界不破坏内容/锚点语义。
- H-AC4：复用/平台专属/重复 UI 指标与 Android `未验证`状态写入结果。

## 风险
| 风险 | 缓解 |
|---|---|
| Kotlin 重复实现 Rust 业务 | 契约和静态边界检查；限制 Kotlin 包职责 |
| Compose 无法提供所需末尾位置 | 允许公开原生 TextLayout 局部实现；记录选择依据 |
| Android 未实测被误判为通过 | `未验证`为有效状态且在父级结果强制检查 |
| 双 UI 功能漂移 | 共同 UI 语义、共享业务测试和操作矩阵 |

## 开放与延期
- Android 设备实机验证和最终 Compose/View 选择延期；两者均不得影响当前 Linux 完成判定。
