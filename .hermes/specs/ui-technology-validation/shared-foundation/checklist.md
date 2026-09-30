# 共享基础验证清单

## 环境
- 当前 Deepin 20.9 x86_64；测试不得要求 Flutter/Compose/Qt/Slint 或 Android SDK 已安装。

## 必需程序化检查
- [ ] S-C0 (S-T1, S-R6) — cmd: `bash experiments/ui-selection/scripts/bootstrap-linux.sh --check`; expect exit 0，识别 Rust/Cargo 1.98.0、系统 glibc 2.28、用户 glibc 2.41、`glibc-wrap`、JDK/Android SDK 与其余工具状态；兼容 smoke 清除 `LD_PRELOAD` 后通过。
- [ ] S-C1 (S-T3, S-R1) — test: `cargo test --release --manifest-path experiments/ui-selection/shared/rust-reading-core/Cargo.toml abi_`; expect exit 0，11 个入口、生命周期、错误码和跨边界释放通过。
- [ ] S-C2 (S-T3, S-R2) — test: `cargo test --release --manifest-path experiments/ui-selection/shared/rust-reading-core/Cargo.toml bounded_window_`; expect exit 0，100MB/1GB 有限窗口与内存上限断言通过。
- [ ] S-C3 (S-T4, S-R3, S-R5) — test: `cargo test --release --manifest-path experiments/ui-selection/shared/rust-reading-core/Cargo.toml 'anchor_|invalidate_|async_'`; expect exit 0，锚点、边界、失效、事件、取消和指标断言通过。
- [ ] S-C4 (S-T4, S-R7) — test: `cargo test --release --manifest-path experiments/ui-selection/shared/rust-reading-core/Cargo.toml hybrid_semantics_`; expect exit 0，不同 UI 页边界保持位置语义。
- [ ] S-C5 (S-T5, S-R4) — cmd: `python3 experiments/ui-selection/scripts/generate-fixtures.py --profile smoke --seed 20260905 --output /tmp/nr-fixtures-a && python3 experiments/ui-selection/scripts/generate-fixtures.py --profile smoke --seed 20260905 --output /tmp/nr-fixtures-b && python3 experiments/ui-selection/scripts/compare-manifests.py /tmp/nr-fixtures-a/manifest.json /tmp/nr-fixtures-b/manifest.json`; expect exit 0 且哈希、规模相同。
- [ ] S-C6 (S-T2, S-T6, S-R6, S-R7) — cmd: `python3 experiments/ui-selection/scripts/verify-workspace.py --shared`; expect exit 0，11 ABI、五候选目录、业务职责清单、锁文件、CLI、schema 正反例完整。
- [ ] S-C7 (S-T7, S-R1–S-R7) — cmd: `bash experiments/ui-selection/scripts/test-shared.sh --release`; expect exit 0，直接 Rust/C ABI 外部消费者 smoke 与日志通过。

## 回归检查
- [ ] S-C8 — grep: `python3 experiments/ui-selection/scripts/verify-scope.py --path experiments/ui-selection/shared`; expect exit 0，不含数据库、网络、遥测或正式产品依赖。
