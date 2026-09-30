# reading-core：TXT 阅读共享核心

## 背景与目标

将技术验证中的 `rust-reading-core` 从模拟数据原型收敛为正式产品核心，为桌面 Rust + Slint 和 Android JNI 提供同一套 TXT 阅读语义。核心必须支持真实 TXT 的有限窗口读取、编码处理、章节/位置/分页索引和本地持久化，但不依赖任何 UI 框架。

## 范围

### 范围内

- Rust workspace 中的 `reading-core`、`reading-source`、`reading-index`、`reading-storage` 和 `reading-ffi`。
- 普通 TXT 数据源、编码识别/覆盖、文件指纹、章节索引和内容位置。
- 页面边界、锚点、布局签名、缓存失效和取消语义。
- SQLite 元数据、索引、进度、书签和迁移。
- 稳定 C ABI/JNI 所需的句柄、错误、缓冲区和请求契约。
- 小型固定夹具以及 100MB/1GB 生成式压力夹具。

### 范围外

- Slint 或 Compose 页面及最终字形渲染。
- ZIP、EPUB、全文搜索、网络、同步和背景图片。
- 平台文件选择器和 Android Activity 生命周期。
- 将完整正文或完整解码字符串写入数据库/内存。

## Requirements

### CR1 — 有限窗口 TXT 数据源

系统 SHALL 通过 `TextSource` 按原始字节偏移读取有限窗口，单次读取默认不超过 64 KiB、硬上限不超过 256 KiB，并在**内容实际编码**（CR2 的 7 种编码）下做边界安全处理（窗口两端对齐到该编码的字符边界）。

#### Scenario: 大文件读取
- **GIVEN** 一个 100MB 或 1GB TXT 文件
- **WHEN** 请求任意窗口
- **THEN** 只物化受上限约束的字节，返回窗口位置和 EOF 状态，不载入全文。

### CR2 — 编码识别与覆盖

系统 SHALL 支持 UTF-8、UTF-8 BOM、UTF-16 LE/BE、GB18030、GBK/CP936、Big5 的识别结果和用户手动覆盖，并记录检测器版本与可信度。

#### Scenario: 低可信度编码
- **GIVEN** 无 BOM 且多个编码评分接近的 TXT
- **WHEN** 完成检测
- **THEN** 返回低可信度状态，允许用户指定编码；不修改原文件。

### CR3 — 内容位置与章节索引

系统 SHALL 以内容单元、章节序号和原始字节偏移表示位置，以附近文本指纹辅助恢复；没有识别到章节时生成单一虚拟全文章节。

### CR4 — 分页边界与重排失效

系统 SHALL 校验页面起止边界、**按内容实际编码（CR2 的 7 种编码，UTF-8 为默认）判定的合法字符边界**（**可判定性说明**：GB18030/GBK/Big5 等传统双字节编码的字符边界判定需要一个**已知对齐的基点**；核心的窗口对齐当前仍为 UTF-8 专属，从其小窗口起点步进会**误拒合法边界**（真机表现为「打开即 ER_ERR_3 且无法翻页」），故在窗口对齐改为按内容编码之前，这三种编码**暂不执行**字符边界单项校验，其余规则（区间非空、不越界、与上一页严格前进、缓存一致、总长一致）一律照旧 —— 见 `results/encoding-core-boundary.md` §窗口对齐）和严格前进关系，并用包含视口、字体、字号、行距、段距、缩进、边距及算法版本的布局签名管理分页缓存。

#### Scenario: 排版设置改变
- **GIVEN** 当前页面存在已保存的布局边界
- **WHEN** 字号、视口或字体发生变化
- **THEN** 旧布局边界失效，并可用当前位置/锚点附近位置重新建立页面。

### CR5 — 本地存储与安全

系统 SHALL 持久化书籍、来源、编码、章节、进度和书签，但不得保存完整正文；删除书架记录或清理索引不得删除原始 TXT。

### CR6 — 跨语言契约

系统 SHALL 提供稳定错误码、64 位 ID/偏移、UTF-8 长度字符串、明确缓冲区释放、请求取消和句柄生命周期接口，供 Android JNI 使用。

## Non-Functional Requirements

- NFR-C1：核心库不得依赖 Slint、Compose、Android SDK 或桌面窗口。
- NFR-C2：核心测试可离线执行；压力夹具通过脚本生成，不提交 GB 级正文。
- NFR-C3：数据库迁移可重复、可回滚备份；索引损坏不应破坏书籍/进度/书签。
- NFR-C4：所有对外 ABI 变更必须更新头文件、Kotlin 声明、契约测试和 ABI 版本。

## Impact Analysis

- Affected code：`crates/reading-core/`、`crates/reading-source/`、`crates/reading-index/`、`crates/reading-storage/`、`crates/reading-ffi/`、`migrations/`、`tests/`、`scripts/`。
- Migration：从 `experiments/ui-selection/shared/rust-reading-core/` 迁移语义和测试，移除模拟书库作为生产来源。
- API/schema：新增正式 Rust API、C ABI、SQLite 初始 schema；无网络接口。
- Unaffected：`desktop/` 和 `android/` 的 UI 外观由各自子规格负责；原始方案文档不改写。

## Acceptance Criteria

- AC1 (CR1)：有限窗口测试覆盖 100MB/1GB，单次窗口不超过 256 KiB，Rust Release 测试通过。
- AC2 (CR2)：所有规定编码的代表夹具识别/覆盖测试通过，原文件哈希保持不变。
- AC3 (CR3)：章节、无章节虚拟章节、位置和锚点恢复测试通过。
- AC4 (CR4)：正向/反向分页、合法边界、重排失效和过期布局结果测试通过。
- AC5 (CR5)：SQLite 迁移、进度/书签 upsert、索引清理和原始文件安全测试通过。
- AC6 (CR6)：C ABI/JNI 句柄、错误、缓冲区释放、取消和 ABI 版本契约检查通过。

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| GB18030/UTF-16 边界实现错误 | Medium | 乱码或位置错乱 | 固定多编码夹具；按原始字节保存位置；属性测试边界 |
| 分页算法与平台文本测量不一致 | High | UI 裁切或重复 | 核心只裁定边界语义；由 desktop/android 子规格验证测量口径 |
| SQLite 索引损坏 | Medium | 书籍无法打开 | schema 版本、事务、备份和可重建索引分离 |
| FFI 缓冲区生命周期错误 | Medium | Android 崩溃/泄漏 | 明确 owner/release；C ABI 与 JVM 调用契约测试 |

## Open Questions and Deferred Decisions

- Blocking：无。
- Deferred：最终 SQLite 加密需求、最终发行许可证、是否加入 32 位 Android ABI；不影响 V1 核心 API。

## References

- `docs/novel-reader-v1-development-plan.md`
- `experiments/ui-selection/shared/rust-reading-core/`
- [父级设计](../design.md)
