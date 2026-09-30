# desktop：桌面 TXT 阅读器

## 背景与目标

在 `reading-core` 正式核心之上，优先交付 Linux 桌面端 Rust + Slint 的可用 TXT 阅读器，再完成 Windows 10/11 和 Linux Wayland 发布验证。桌面端是 V1 的首要用户可见交付，不继续使用技术验证探针的模拟书库作为产品数据。

## 范围

### 范围内

- Rust 桌面应用层与 Slint 书架、阅读、设置页面。
- 桌面文件选择器、TXT 导入、来源指纹和失效文件重新关联。
- 真实 TXT 首屏、整页翻页、章节、书签、进度恢复。
- 中文文本测量/渲染、窗口调整、高 DPI、鼠标和键盘。
- Linux X11 优先；Windows 10/11 和 Linux Wayland 构建/启动验证。
- 桌面 Release 构建、启动冒烟和端到端测试。

### 范围外

- Android UI、JNI、SAF 和移动生命周期。
- ZIP/EPUB/其他格式、连续滚动、网络和同步。
- 正文全文搜索、背景图片和多级分类树。
- 通过截图 OCR 或私有 Slint 文本 API实现分页。

## Requirements

### DR1 — 桌面 TXT 导入

桌面 SHALL 通过系统文件选择器选择 `.txt`，创建书籍记录并打开首屏；非 TXT 文件应被拒绝并显示可理解的错误。

#### Scenario: 文件导入
- **GIVEN** 用户选择可读取的 TXT
- **WHEN** 导入完成
- **THEN** 书籍进入书架，显示书名和状态，并可从首屏开始阅读。

### DR2 — Linux 优先阅读闭环

桌面 SHALL 在 Linux X11 上完成书架、阅读、设置、进度、书签和错误状态的真实闭环，UI 不复制 Rust 核心阅读语义。

### DR3 — 中文整页分页

桌面 SHALL 使用实际 Slint 阅读区域尺寸和实际渲染参数完成整页分页，窗口变化或排版设置改变后以内容位置重新排版，不出现丢字、重字、半字、滚动正文或旧页面覆盖。

### DR4 — 桌面交互与可访问性

桌面 SHALL 支持鼠标点击/滚轮、键盘翻页和窗口关闭；按钮、目录、书签、输入框有明确的可见状态和可访问名称。

### DR5 — 进度、书签与失效文件

桌面 SHALL 使用共享核心保存进度和书签；原文件不可用时保留书架记录并提供重新关联，不删除原始文件。

### DR6 — Windows/Wayland 发布准备

桌面 SHALL 提供 Windows 10/11 和 Linux Wayland 的构建/启动路径；在这些环境尚未提供实测证据前，状态必须保持待验证。

## Non-Functional Requirements

- NFR-D1：静止阅读不保持无意义动画、定时器或持续出帧。
- NFR-D2：Release 构建启用符号剥离/合理优化，完整发布目录而非单可执行文件作为包体口径。
- NFR-D3：Slint 文本渲染与分页测量的字号、行高、字距、边距和正文区域必须同口径。
- NFR-D4：Linux 首先通过当前 Deepin 20.9/X11 基线；不能用锁屏黑屏截图代替窗口存在性证据。

## Impact Analysis

- Affected code：`crates/desktop-app/`、`desktop/ui/`、`desktop/packaging/`、`tests/e2e/`、`scripts/`。
- Depends on：`reading-core` 的 Rust API、SQLite 仓储和页面边界契约。
- Migration：从 `experiments/ui-selection/hybrid-native-probe/desktop-slint/` 迁移 UI/编排思路，移除合成书库和验证专用状态。
- Unaffected：Android 源码及真机门禁；既有方案和验证证据文档。

## Acceptance Criteria

- AC-D1 (DR1)：真实 TXT 导入后书架记录、首屏和错误状态可通过程序化 E2E 检查。
- AC-D2 (DR2)：Linux X11 Release 启动和 TXT 阅读闭环通过。
- AC-D3 (DR3)：200 页正向、100 页反向和字号/窗口重排测试通过，页面边界合法且无正文滚动。
- AC-D4 (DR4)：键盘/鼠标入口存在、状态投影稳定、Slint 组件具备可访问名称。
- AC-D5 (DR5)：重启恢复进度、书签跳转、失效文件重新关联和安全清理测试通过。
- AC-D6 (DR6)：Windows/Wayland 构建路径和未验证状态有证据；完成实测前不标发布通过。

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Slint 真实正文区与排版尺寸脱节 | High | 页面留白/裁切 | 窗口映射后取得尺寸；集中管理布局参数；自动检查正文区域 |
| Linux 可运行但 Windows 缺少依赖 | Medium | 无法发布 | 完整发布目录、干净环境和独立构建门禁 |
| UI 层复制分页/位置语义 | Medium | 双端行为分叉 | desktop-app 只调用核心；边界扫描和契约测试 |
| 当前探针模拟内容混入产品 | Medium | 用户看到假数据 | 生产 scope 检查禁止 Synthetic 生产入口 |

## Open Questions and Deferred Decisions

- Blocking：无。Linux 桌面先行路径已经确定。
- Deferred：最终 Windows 打包格式、Linux Wayland 发行格式、最终主题视觉细节；不影响 Linux 先行开发。

## References

- `docs/novel-reader-v1-development-plan.md`
- `docs/ui-selection/desktop.md`
- `experiments/ui-selection/hybrid-native-probe/desktop-slint/`
- [父级设计](../design.md)
