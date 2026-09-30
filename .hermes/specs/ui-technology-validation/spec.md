# 小说阅读器 UI 技术验证程序

## 背景与目标

依据修订后的 `docs/ui-technology-selection-plan.md`，建立可复现的验证工作区，比较五种 UI 技术架构：优先候选“桌面 Rust + Slint、Android 原生 UI、Rust 共享阅读业务库”，以及三平台 Slint、Flutter、Compose Multiplatform、Qt Quick 四个对照候选。本规格只交付验证程序、自动化检查、当前 Linux Release 编译/启动能力和跨系统操作文档，不实现正式小说阅读器，也不在本阶段形成最终选型结论。

本机验收基线为 Deepin 20.9、x86_64、X11（`DISPLAY=:0`），系统 glibc 2.28；用户目录已有经实测可用的 glibc 2.41 兼容层。`rustc`/`cargo` 均为 1.98.0。五个候选均须在当前环境完成其**桌面分页尖峰**的 Release 编译和启动；优先候选的 Android 源码、JNI 适配和操作指南也须交付，但 Android 实机编译、安装和 UI 验收仍属于延期平台验证。

## 范围

### 范围内
- UI 无关的 Rust 共享阅读业务库原型：有限窗口、模拟领域数据、位置、分页锚点、缓存失效、取消、事件和测量。
- 直接 Rust API 与稳定 C ABI/JNI 适配基线，含页面边界提交与布局失效接口。
- 优先候选：`hybrid-native-probe/desktop-slint/` 与 `hybrid-native-probe/android-native/`；Android 默认 Kotlin + Jetpack Compose，局部排版可使用公开原生 View/TextLayout API。
- 三平台 Slint、Flutter、Compose Multiplatform、Qt Quick 四个对照候选的分页尖峰及条件完整原型。
- 当前 Deepin/X11 的五个桌面 Release 构建、启动探测、日志和进程清理。
- Android 12+、Windows 10/11、Linux X11/Wayland 的逐候选安装、构建、启动和 UI 操作指南。
- 版本锁、数据夹具、结果 schema、Rust 业务复用/平台专属代码量记录和机器可读证据。

### 范围外
- 正式产品、SQLite、完整章节扫描、正式 ZIP、持久化历史/书签、网络功能、EPUB 和正式发布流程。
- Android、Windows、Wayland 实机的实际编译、安装、运行、手势或性能验收。
- 安装包签名、商店发布、跨平台性能/功耗测量、加权评分、最终选型报告和 ADR。
- 以私有排版 API、截图 OCR、整本正文复制、在 Kotlin 中复制 Rust 阅读业务或不可维护补丁换取“通过”。
- 修改两份既有方案文档的产品约束。

## 父级需求

### PR1 — 统一 Rust 共享业务基础
系统 SHALL 为五个候选提供同一 Rust 共享阅读业务库、数据、位置/分页锚点语义、契约、脚本入口和结果格式；桌面 Rust 调用与跨语言 C ABI 调用必须语义等价。

### PR2 — 优先候选与双 UI 复用边界
系统 SHALL 首先实现并验证桌面 Slint + Android 原生 UI 的代码闭环：Android 仅保留平台排版/交互/JNI 适配，不得复制文件读取、模拟领域状态、阅读位置、锚点、缓存失效、取消或错误模型；须记录 Rust 复用覆盖率、平台专属代码量和重复 UI 代码量。

### PR3 — 分页硬门槛先行
五个候选 SHALL 先实现中文分页尖峰。每端由稳定公共 API 返回合法 UTF-8 页末位置，并验证正向、反向与重排；未通过者停止完整三页面开发，但保留可复现证据。当前范围内 Android 结果允许为 `未验证`，不得包装为通过。

### PR4 — 功能语义一致而非 UI 强制同构
通过 PR3 的候选 SHALL 以同一数据、内容位置和业务行为实现书架、阅读、配置原型。优先候选允许 Android 和桌面采用各自原生交互与不同分页边界，但两端均须满足内容连续性、位置锚定和共享业务语义。

### PR5 — 当前操作系统可编译启动
五个候选的**桌面分页尖峰** SHALL 在当前 Deepin/X11 完成 Release 编译并由启动探针确认 UI 就绪；候选技术能力“不通过”或 Android“未验证”不等于验证程序实施失败。

### PR6 — 异系统可执行指南
项目 SHALL 提供 Android、Windows、Linux X11/Wayland 的逐候选安装、构建、启动、UI 操作、预期结果与清理方式，并明确当前未实测的步骤。

### PR7 — 严格限制验证阶段
所有代码 SHALL 仅服务于技术验证；不得引入数据库、遥测、网络 SDK、正式业务迁移或发布流程。

## 非功能需求
- PNFR1 — 使用确切版本和锁文件，禁止浮动依赖。
- PNFR2 — Release 是当前 Linux 编译/启动验收口径；Debug 仅用于定位。
- PNFR3 — 正文按有限窗口跨语言边界传输，不得整体载入 100MB/1GB 文件。
- PNFR4 — 自动化命令非交互、失败码明确，原始输出落入 `experiments/ui-selection/results/raw/`。
- PNFR5 — 五个候选使用同一页面语义、数据规模、测试字体约定和结果字段；双 UI 不按源码共享率直接扣分。

## 子规格

| 子规格 | 摘要 | 依赖 | 状态 |
|---|---|---|---|
| [shared-foundation](./shared-foundation/) | Rust 共享阅读业务库、直接 Rust/C ABI、夹具与编排 | 无 | 未开始 |
| [hybrid-native-probe](./hybrid-native-probe/) | 优先候选：桌面 Slint + Android 原生 UI + JNI | shared-foundation | 未开始 |
| [slint-probe](./slint-probe/) | 候选 B：三平台 Slint 多渲染器尖峰及条件原型 | shared-foundation，hybrid-native-probe 的优先尖峰已交付 | 未开始 |
| [flutter-probe](./flutter-probe/) | 候选 C：Flutter + Rust FFI | shared-foundation，hybrid-native-probe 的优先尖峰已交付 | 未开始 |
| [compose-probe](./compose-probe/) | 候选 D：Compose Multiplatform + Rust FFI | shared-foundation，hybrid-native-probe 的优先尖峰已交付 | 未开始 |
| [qt-probe](./qt-probe/) | 候选 E：Qt Quick/QML + Rust C ABI | shared-foundation，hybrid-native-probe 的优先尖峰已交付 | 未开始 |
| [platform-guides](./platform-guides/) | 五候选的当前 Linux 与异系统操作指南 | 前述全部子规格 | 未开始 |

共享架构、双 UI 语义、脚本 CLI 与结果状态见 [design.md](./design.md)。

## 影响分析
- 新增：`experiments/ui-selection/shared/rust-reading-core/`、`hybrid-native-probe/` 和四个对照候选目录。
- 新增：Android Kotlin/Compose/JNI 验证源码及其静态业务边界检查；无正式 Android 产品模块。
- API/数据：新增内部直接 Rust API、C ABI/JNI 和 JSON 结果；无数据库或正式产品 API。
- 兼容性：系统 glibc 2.28 仍可能限制框架；用户级 glibc 2.41 已通过 Rust、C++、NSS、X11 smoke，但不替代各框架验证。
- 不受影响：既有方案文档、未来正式产品代码和跨平台实机验收边界。

## 父级验收标准
- PAC1 (PR1)：共享 Rust 业务测试、直接 Rust/C ABI smoke、夹具和统一 schema 通过。
- PAC2 (PR2)：优先候选有桌面 Slint 与 Android 原生目录、薄 JNI/C ABI 适配和“无 Kotlin 业务复制”静态证据；复用/专属代码指标完整。
- PAC3 (PR3)：五个候选产生结构一致的分页结果；状态只能为 `通过`、`有条件通过`、`不通过`、`未验证`，且 Android 未实测状态显式标记。
- PAC4 (PR4)：通过分页门槛的端完成共同 UI 语义检查；优先候选两端允许布局结果不同但内容/锚点语义一致。
- PAC5 (PR5)：`build-linux.sh --release all` 与 `smoke-linux.sh --backend x11 all` 对五个桌面尖峰逐一获得就绪信号并清理进程。
- PAC6 (PR6)：操作指南覆盖五候选，含 Android 原生 Compose/View、JNI、SAF、返回、旋转和生命周期步骤及未实测标记。
- PAC7 (PR7)：范围扫描不含数据库、网络 SDK、遥测或正式发布配置。

## 风险与缓解

| 风险 | 可能性 | 影响 | 缓解 |
|---|---|---|---|
| 双 UI 逐步复制阅读业务 | 高 | 维护成本失控、行为分叉 | Rust 共享状态机和契约测试；静态禁止 Kotlin 领域逻辑；复用/重复代码量留证 |
| Android 文本布局与桌面页边界不同 | 高 | 错误要求跨端页码相同 | 只要求连续性、合法字节边界和锚点语义相同；分别记录布局结果 |
| Android 当前未实测 | 高 | 优先候选无法得出最终结论 | 结果状态使用`未验证`；提供完整设备指南，后续实机验证另立任务 |
| Deepin glibc 2.28 过旧 | 中 | 桌面框架无法启动 | 默认系统环境；按进程使用用户 glibc 2.41 包装器；禁止全局 `LD_LIBRARY_PATH` |
| Hermes SQLite `LD_PRELOAD` 污染 | 中 | 依赖/资源数据失真 | 兼容启动统一 `env -u LD_PRELOAD` 并记录环境 |
| 五候选投入不均 | 高 | 比较失真 | 优先候选先完成尖峰；其他候选仅达客观对照深度，条件满足再扩展完整原型 |

## 开放问题与延期决定
- 阻塞：无。
- 已采用假设：Android 默认 Jetpack Compose；当 Compose 公开布局能力不足时，局部 `View`/`TextLayout` 可用于排版，但必须记录边界和稳定 API 依据。
- 已采用假设：当前范围内 Android 仅交付源码、静态检查和操作指南，能力状态为`未验证`直至实机执行。
- 延期：开源许可证、最低 Linux/glibc 基线、Android 32 位 ABI、完全便携发布、性能评分、最终推荐与 ADR。

## 参考
- `docs/ui-technology-selection-plan.md`（修订日期 2026-09-05）
- `docs/novel-reader-solution.md`
- [design.md](./design.md)
