# Novel Reader V1（TXT 版）正式开发规格

## 背景与目标

`docs/novel-reader-v1-development-plan.md` 已根据现有技术验证确定产品方向：桌面端采用 Rust + Slint，Android 采用 Kotlin + Jetpack Compose 原生 UI，两端共享 Rust 阅读核心。下一步不再继续进行 UI 候选比较，而是建立可执行、可验收的正式应用开发规格。

本规格的目标是交付一个只支持普通 TXT 文件的本地离线阅读器 V1。开发顺序优先保障桌面端：先完成 Linux 桌面 TXT 阅读闭环，再补齐 Windows 发布验证；Android 端可以并行开发，但当前阶段只要求代码、单元测试、JNI/Gradle 编译和 APK 内容验证，真机调试与生命周期实测作为后续门禁。

## 范围

### 范围内

- 普通 `.txt` 文件导入、书架记录、打开、整页阅读和进度恢复。
- UTF-8、UTF-8 BOM、UTF-16 LE/BE、GB18030、GBK/CP936、Big5 的识别或手动指定。
- 有限窗口读取、中文分页、章节索引、章节跳转、书签和排版重排。
- 桌面 Rust + Slint 正式应用，优先 Linux X11，随后 Windows 10/11 和 Linux Wayland 验证。
- Android Kotlin + Compose 正式应用、SAF 持久 URI、JNI/C ABI、JVM 单元测试和 Debug/Release 构建。
- SQLite 本地元数据、章节索引、阅读进度、书签和分页索引缓存。
- 离线运行、无广告、无遥测、默认不依赖网络。

### 范围外

- ZIP、EPUB、MOBI、PDF 或其他格式。
- 在线书源、账号、云同步和网络服务。
- 正文全文搜索、连续滚动、书签笔记、精确阅读时长。
- V1 的背景图片、每本书独立阅读设置和多级分类树。
- 删除用户原始文件。
- Android 真机自动化调试结果在当前阶段的强制完成；真机使用前不得宣称 Android 全部通过。

## 父级需求

### PR1 — TXT 阅读闭环

系统 SHALL 支持用户导入一个普通 TXT 文件，建立书籍记录，打开正文，进行整页前后翻页，关闭后恢复最近成功保存的内容位置。

### PR2 — 跨平台共享阅读语义

系统 SHALL 将数据源、编码、章节、内容位置、锚点、分页失效、进度、书签和错误模型放在 Rust 共享核心中；桌面直接调用 Rust API，Android 通过稳定 JNI/C ABI 调用，平台 UI 不得复制上述阅读业务。

### PR3 — 桌面端优先可发布路径

系统 SHALL 首先完成 Linux 桌面端的真实 TXT 导入、中文分页、设置重排、进度恢复和本地数据闭环；随后完成 Windows 10/11 发布构建验证。桌面端是 V1 的首要交付和集成路径。

### PR4 — Android 可构建交付

Android SHALL 完成 Kotlin/Compose 页面、SAF 适配、JNI 桥接、ViewModel 状态编排、单元测试和 APK 构建/内容校验。真机旋转、后台恢复、权限失效和交互调试属于后续设备门禁。

### PR5 — 大文件与本地安全

系统 SHALL 使用有限窗口处理 TXT，不能因文件大小线性加载全文；SQLite 和缓存只能保存元数据、索引、位置和必要摘要；书架移除或清理缓存不得删除用户原始 TXT。

### PR6 — 可验证发布

系统 SHALL 提供可复现的构建、测试和发布校验命令；V1 完成必须有 Rust、桌面、Android 编译/测试、端到端契约、Release 包体/签名/依赖和已知限制证据。

## 非功能需求

- NFR1 — 资源：单次正文窗口默认 64 KiB、硬上限 256 KiB；阅读内存主要由有限页面缓存决定。
- NFR2 — 分页：页面结束位置必须是合法 UTF-8 边界；连续正向/反向分页不得丢字、重字、半字或死循环。
- NFR3 — 交互：已缓存翻页 P95 目标不超过 100ms；重排不阻塞 UI 线程，并合并连续设置变化。
- NFR4 — 兼容：Android minSdk 31；首发 arm64-v8a；桌面发布物须在干净环境验证。
- NFR5 — 空闲：静止阅读不得保留无意义的固定周期动画、出帧或后台扫描。
- NFR6 — 数据安全：不修改、不删除原始 TXT；URI 权限、数据库迁移和索引失效必须可恢复。
- NFR7 — 可维护性：ID、字节偏移和错误码跨语言保持精确；任务结果必须带 request/layout 版本，过期结果不得覆盖当前状态。

## 子规格

| 子规格 | 摘要 | 依赖 | 状态 |
|---|---|---|---|
| [reading-core](./reading-core/) | TXT 数据源、编码、位置/分页语义、SQLite 仓储和 JNI 契约 | 无 | 未开始 |
| [desktop](./desktop/) | 桌面 Rust + Slint 的 TXT 产品闭环，Linux 优先 | reading-core | 未开始 |
| [android](./android/) | Android Kotlin/Compose、SAF、JNI、单测和 APK 构建 | reading-core；可与 desktop 并行 | 未开始 |

跨子规格的数据流、边界和决策见 [design.md](./design.md)。

## 影响分析

- 新增正式工程目录：`crates/`、`desktop/`、`android/`、`migrations/`、`tests/`、`scripts/`。
- 迁移参考：`experiments/ui-selection/shared/rust-reading-core/`、`experiments/ui-selection/hybrid-native-probe/desktop-slint/`、`experiments/ui-selection/hybrid-native-probe/android-native/`。
- API：正式 Rust API、Android C ABI/JNI 和 SQLite 内部 schema；无网络 API。
- 数据：首次建立 SQLite 数据库；迁移只新增版本，不覆盖用户数据。
- 不受影响：`docs/novel-reader-solution.md`、`docs/ui-technology-selection-plan.md` 和技术验证证据目录不得被产品实现反向改写。
- 当前未纳入：Windows/Wayland 实机、Android 真机调试结果，不阻塞桌面开发，但阻塞最终跨平台发布声明。

## 父级验收标准

- PAC1 (PR2)：`reading-core` 子规格已实现，Rust 核心测试、窗口上限、位置/分页/存储契约均通过。
- PAC2 (PR1, PR3)：`desktop` 子规格已实现，Linux 真实 TXT 阅读闭环和 Release 构建通过；Windows 构建状态有明确证据。
- PAC3 (PR1, PR4)：`android` 子规格完成源码、JNI、JVM 单测、交叉编译和 APK 构建；未实测项明确标记。
- PAC4 (PR5)：100MB/1GB 夹具测试证明无全文加载；缓存和书架操作安全测试通过。
- PAC5 (PR6)：父级集成命令通过，发布物、校验结果、依赖许可和已知限制完整。
- PAC6 (PR1–PR6)：三端需求、任务和检查项可追溯，无未记录的范围扩张。

## 风险与缓解

| 风险 | 可能性 | 影响 | 缓解 |
|---|---|---|---|
| 双 UI 复制阅读业务 | 高 | 行为分叉、维护成本上升 | 核心子规格冻结位置/分页/存储语义；静态扫描 Kotlin/Slint 只允许编排 |
| SAF 不支持随机访问 | 中 | 大文件分页失败 | 检测 Provider 能力；不支持时复制到私有缓存，复制可取消 |
| 测量和渲染口径不一致 | 高 | 半字、滚动条、页末裁切 | 显式像素、行高、字距和安全余量契约；桌面/Android 分别回归 |
| Android 真机验证延后 | 高 | 无法确认机型兼容性 | 先完成编译和 JVM/静态证据；把真机作为独立后续门禁，不冒充通过 |
| 探针占位实现混入产品 | 中 | 正式版可能返回空正文或模拟数据 | 正式目录与 experiments 分离；产品检查禁止 Synthetic/External 空数据源进入发布包 |
| 大文件索引耗电 | 中 | 前台卡顿、后台耗电 | 低优先级、可取消、增量写入、前台操作让路 |

## 开放问题与延期决定

- 阻塞：无。桌面优先和 Android 编译优先的实施路径已经确定；Android 真机验证不阻塞 `reading-core` 与桌面端开发，但阻塞最终 Android/V1 全平台发布声明。
- 已采用：Android minSdk 31、首发 arm64-v8a、Compose 为默认 UI；必要时使用公开 Android 文本布局 API。
- 已采用：V1 只有 TXT；ZIP 和其他格式不进入当前任务图。
- 延期：Android 真机设备和自动化 instrumentation、Windows/Wayland 实机、最终 Linux 发行格式、许可证最终选择、Android 32 位 ABI。
- 接受的风险：在 Android 真机验证完成前，只能称为“Android 可构建/可测试”，不能称为“Android 全面通过”。

## 参考

- `docs/novel-reader-v1-development-plan.md`
- `docs/novel-reader-solution.md`
- `docs/ui-selection/desktop.md`
- `docs/ui-selection/android.md`
- [design.md](./design.md)
