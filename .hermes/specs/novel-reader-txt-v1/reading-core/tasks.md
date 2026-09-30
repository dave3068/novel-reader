# reading-core 任务

## 状态

- Overall: 未开始

## Phase 1 — 正式核心骨架

- [ ] C-T1 — 建立 Rust workspace、crate 边界、版本和错误模型
  - Requirements: CR1, CR6
  - Affected areas: `Cargo.toml`, `crates/reading-*`, `crates/reading-ffi/`
  - Depends on: 无
  - Definition of done: 五个核心 crate 可编译；核心无 UI/Android 依赖；错误码和 ABI 版本有单测。

- [ ] C-T2 — 迁移并重构 TXT 数据源、编码和文件指纹
  - Requirements: CR1, CR2
  - Affected areas: `crates/reading-source/`, `tests/fixtures/`, `scripts/generate-fixtures.py`
  - Depends on: C-T1
  - Definition of done: 规定编码、BOM、换行、超长行和边界夹具可重复生成；有限窗口和解码测试通过。

## Phase 2 — 位置、索引与存储

- [ ] C-T3 — 实现章节、ContentPosition、锚点和分页边界
  - Requirements: CR3, CR4
  - Affected areas: `crates/reading-core/`, `crates/reading-index/`
  - Depends on: C-T2
  - Definition of done: 位置严格按原始字节；章节缺失有虚拟章节；正反向分页、锚点和布局失效测试通过。

- [ ] C-T4 — 实现 SQLite schema、迁移和仓储
  - Requirements: CR5
  - Affected areas: `crates/reading-storage/`, `migrations/`
  - Depends on: C-T1, C-T3
  - Definition of done: 书籍、来源、章节、进度、书签和布局记录可事务写入；索引清理不影响用户元数据。

## Phase 3 — 跨端接口与门禁

- [ ] C-T5 — 实现正式 C ABI/JNI 适配和外部数据源边界
  - Requirements: CR1, CR3, CR4, CR6
  - Affected areas: `crates/reading-ffi/`, `android/`, `include/`
  - Depends on: C-T2, C-T3
  - Definition of done: 句柄、64 位偏移、缓冲区释放、错误、取消、request/layout id 和 Android 外部窗口契约冻结。

- [ ] C-T6 — 完成核心 TDD、压力和范围验证
  - Requirements: CR1–CR6
  - Affected areas: `tests/`, `scripts/`, `crates/`
  - Depends on: C-T4, C-T5
  - Definition of done: 所有 reading-core checklist required checks 通过，真实日志和结果写入 `results/`。

## 依赖摘要

- C-T1 → C-T2 → C-T3。
- C-T1/C-T3 → C-T4。
- C-T2/C-T3 → C-T5。
- C-T4/C-T5 → C-T6。
