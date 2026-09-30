# 共享基础任务

## 状态
- Overall: 部分实现（代码与契约已完成并通过测试；编排脚本尚未齐全）
- 已完成: S-T2（直接 Rust API / 11 C ABI / 职责清单 / 结果 schema）、S-T3（生命周期、有限窗口、跨边界释放）、
  S-T4（位置/锚点/页面边界/布局失效/模拟数据/事件/取消/指标）。
- 未完成: S-T1（`toolchains.lock` + `bootstrap-linux.sh`）、S-T5（`generate-fixtures.py` 的 Python 夹具与 manifest）、
  S-T6（`verify-workspace.py`、`test-shared.sh`、`verify-scope.py`、`build-linux.sh all` 五候选 adapter）。
- 已验证证据（真实执行）:
  - `cargo build --release --manifest-path shared/rust-reading-core/Cargo.toml` → exit 0
  - `cargo test --release` → **21/21 通过**（`abi_` 5、`anchor_|invalidate_|async_` 6、`bounded_window_` 5、`hybrid_semantics_` 5）
  - `cargo build --release --target {aarch64,x86_64}-linux-android` → exit 0（见 `results/raw/android-cross-compile.log`）

## Phase 1 — 工具链与契约
- [ ] S-T1 — 建立隔离工具链预检和版本锁
  - Requirements: S-R6
  - Affected areas: `experiments/ui-selection/toolchains.lock`, `scripts/bootstrap-linux.sh`
  - Depends on: 无
  - Definition of done: 预检确认 Rust 1.98.0、系统/用户 glibc、包装器、Android/JDK 与其余 SDK 状态。
- [ ] S-T2 — 定义直接 Rust API、11 个 C ABI、业务职责清单与结果 schema
  - Requirements: S-R1, S-R6, S-R7
  - Affected areas: `shared/rust-reading-core/`, `shared/ffi/include/novel_reader_probe.h`, `shared/contracts/`
  - Depends on: S-T1
  - Definition of done: schema 正反例、头文件/Rust 布局和 Kotlin 业务边界规则测试通过。

## Phase 2 — 核心与数据（TDD）
- [ ] S-T3 — 实现生命周期、窗口读取和内存释放
  - Requirements: S-R1, S-R2
  - Affected areas: `shared/rust-reading-core/src/`, `shared/rust-reading-core/tests/`
  - Depends on: S-T2
  - Definition of done: 先红后绿；Release Rust/C ABI/受限内存测试通过。
- [ ] S-T4 — 实现位置、锚点、页面边界、布局失效、模拟数据、事件和取消
  - Requirements: S-R3, S-R4, S-R5, S-R7
  - Affected areas: `shared/rust-reading-core/src/`, `shared/rust-reading-core/tests/`
  - Depends on: S-T3
  - Definition of done: 位置恢复、跨 UI 边界、确定性、回调、取消、错误和指标测试通过。
- [ ] S-T5 — 实现夹具生成器和 manifest
  - Requirements: S-R4
  - Affected areas: `scripts/generate-fixtures.py`, `shared/fixtures/`
  - Depends on: S-T2
  - Definition of done: smoke 数据重复生成并复验；full profile 可生成但不提交大文件。

## Phase 3 — 编排
- [ ] S-T6 — 实现五候选构建、分页、启动、结果和交付校验脚本
  - Requirements: S-R6, S-R7
  - Affected areas: `scripts/`
  - Depends on: S-T3, S-T4, S-T5
  - Definition of done: 包含 `hybrid-native` adapter、`未验证`状态、业务复制扫描及超时/失败测试。
- [ ] S-T7 — 执行共享 Release 门禁
  - Requirements: S-R1–S-R7
  - Affected areas: `shared/`, `scripts/`, `results/raw/`
  - Depends on: S-T6
  - Definition of done: 本子规格全部必需检查通过并记录证据。

## 依赖摘要
- S-T1 → S-T2 → S-T3/S-T5；S-T3 → S-T4；S-T3/S-T4/S-T5 → S-T6 → S-T7。
