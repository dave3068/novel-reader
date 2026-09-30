# Slint 验证清单

## 必需程序化检查
- [ ] SL-C1 (SL-T1, SL-T2, SL-R1) — test: `bash experiments/ui-selection/scripts/test-pagination.sh slint --pages 1000`; expect exit 0，所有分页正确性断言通过或以“公共 API 不满足”生成合法不通过证据且测试工具自身成功完成判定。
- [ ] SL-C2 (SL-T2, SL-R2) — test: `cargo test --release --manifest-path experiments/ui-selection/slint-probe/Cargo.toml ffi_`; expect exit 0，C ABI、回调线程和复制统计通过。
- [ ] SL-C3 (SL-T3, SL-R3) — cmd: `bash experiments/ui-selection/scripts/build-linux.sh --release slint --variant femtovg && bash experiments/ui-selection/scripts/build-linux.sh --release slint --variant skia`; expect exit 0 且构建物/日志独立。
- [ ] SL-C4 (SL-T3, SL-R3) — cmd: `bash experiments/ui-selection/scripts/run-diagnostic.sh slint --variant software --check chinese-text`; expect exit 0 并输出合法能力状态、原因和证据，不要求结果为“通过”。
- [ ] SL-C5 (SL-T3, SL-R5) — cmd: `bash experiments/ui-selection/scripts/smoke-linux.sh --backend x11 --timeout 30 slint --variants femtovg,skia`; expect exit 0、两种后端就绪且无残留进程。
- [ ] SL-C6 (SL-T4, SL-T5, SL-R4) — cmd: `python3 experiments/ui-selection/scripts/verify-ui-contract.py --candidate slint`; expect exit 0，分页通过时三页面状态契约完整；不通过时淘汰门禁记录完整。

## 人工建议检查
- [ ] SL-M1 (manual-advisory, SL-AC4) — action: 使用 FemtoVG、Skia 各操作一次中文翻页和配置重排；expected: 无缺字/裁切且行为一致；programmatic fallback: SL-C1、SL-C5、SL-C6。
