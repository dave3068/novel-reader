# 共享 Rust 阅读业务库、夹具与编排基础

## 目标
为优先双 UI 候选和四个对照候选提供唯一的 UI 无关 Rust 阅读业务库、直接 Rust API、C ABI、确定性数据、位置/分页锚点语义与自动化入口，避免平台 UI 自行复制不可比较的业务逻辑。

## 范围
### 范围内
- `experiments/ui-selection/shared/rust-reading-core/`：验证范围的有限窗口、模拟领域状态、位置、锚点、页面边界、布局失效、缓存、取消、事件与指标。
- 直接 Rust API、11 个 C ABI 基线入口、JNI 消费者所需头文件与契约 schema。
- 10MB/100MB/1GB/Unicode/边界文本、背景图和 1,000 类/10,000 书/2,000 历史的确定性生成器。
- 五候选构建、分页、结果、范围和指南校验的统一脚本。
### 范围外
- 正式数据库、ZIP、网络、完整索引、最终字形布局及正式阅读引擎。
- Android/Windows/Wayland 实机编译、性能或功耗结论。

## 需求
### S-R1 — 共享业务与双入口
核心 SHALL 提供 UI 无关的直接 Rust API 及等价 C ABI：生命周期、窗口、模拟批次、位置、页面边界、布局失效、事件和取消。
### S-R2 — 有限窗口读取
核心 SHALL 按原始字节偏移读取有限窗口并修正 UTF-8 边界，100MB/1GB 文件不得整体进入内存。
### S-R3 — 位置、锚点与布局失效
核心 SHALL 保存内容位置、登记/查询页面边界、在排版配置变化后失效缓存，并允许不同 UI 页边界恢复同一原文附近位置。
### S-R4 — 确定性数据
脚本 SHALL 用固定种子生成方案规定的文本、书架和图片数据，并输出可校验 manifest；大文件不提交 Git。
### S-R5 — 事件、取消与测量
核心 SHALL 支持异步事件、请求取消、错误模型、调用耗时和复制字节数。
### S-R6 — 统一自动化与结果契约
脚本 SHALL 编排五候选；结果支持`通过`、`有条件通过`、`不通过`、`未验证`并保留版本、命令、环境、日志、平台和退出码。
### S-R7 — 共享业务不被 UI 复制
核心 SHALL 提供可由 hybrid-native 静态检查消费的业务职责清单和契约测试，覆盖文件窗口、模拟领域数据、位置、锚点、缓存、取消与错误模型。

## 影响
- 新增：`shared/rust-reading-core/`、`shared/ffi/`、`shared/contracts/`、`scripts/`、`toolchains.lock`。
- 无正式 API、数据库或既有产品文件变更。

## 验收标准
- S-AC1 (S-R1)：Rust API、11 个 C ABI 入口和跨边界释放 smoke 通过。
- S-AC2 (S-R2)：100MB/1GB 有限窗口读取正确，内存不随文件大小线性增长。
- S-AC3 (S-R3)：不同 UI 页边界与重排失效后的位置/锚点恢复测试通过。
- S-AC4 (S-R4)：同种子 manifest、哈希和规模一致。
- S-AC5 (S-R5)：事件、取消、错误和指标测试通过。
- S-AC6 (S-R6, S-R7)：五候选 workspace/schema 与 hybrid 业务边界检查通过。

## 风险
| 风险 | 缓解 |
|---|---|
| C ABI/JNI 布局不一致 | 固定宽度 `repr(C)`、头文件、Rust ABI 与外部消费者测试 |
| 共享库变成正式产品范围 | 只保留验证所需内存模型；范围扫描阻断数据库/网络/正式索引 |
| Android Kotlin 复制业务 | 定义职责清单、包边界和静态扫描；共享语义测试 |
| 1GB 夹具耗时/占盘 | 只提交生成器与 manifest；测试用临时/稀疏文件 |

## 开放与延期
- 阻塞：无。Rust 1.98.0 可直接及经用户 glibc 2.41 包装运行。
- 已验证约束：不得仅设置 `LD_LIBRARY_PATH=~/.local/glibc/lib`；兼容启动使用 `env -u LD_PRELOAD GLIBC_HOME="$HOME/.local/glibc" ~/.local/bin/glibc-wrap <binary>`。
- 延期：正式领域模型、持久化与 Android 实机动态行为。
