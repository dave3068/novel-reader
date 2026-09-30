# Flutter 验证清单

## 必需程序化检查
- [ ] FL-C1 (FL-T1, FL-T2, FL-R1) — test: `bash experiments/ui-selection/scripts/test-pagination.sh flutter --pages 1000`; expect exit 0 并生成合法分页能力判定；若能力不满足，须是有证据的“不通过”而非测试工具失败。
- [ ] FL-C2 (FL-T3, FL-R2) — test: `cd experiments/ui-selection/flutter-probe && flutter test --release test/ffi_test.dart`; expect exit 0，窗口、释放、回调、取消、复制量断言通过。
- [ ] FL-C3 (FL-T5, FL-R4) — cmd: `bash experiments/ui-selection/scripts/build-linux.sh --release flutter`; expect exit 0 且完整 Linux Release 目录存在。
- [ ] FL-C4 (FL-T5, FL-R4) — cmd: `bash experiments/ui-selection/scripts/smoke-linux.sh --backend x11 --timeout 30 flutter`; expect exit 0、就绪 JSON 合法且无残留进程。
- [ ] FL-C5 (FL-T4, FL-R3) — cmd: `python3 experiments/ui-selection/scripts/verify-ui-contract.py --candidate flutter`; expect exit 0，三页面契约或淘汰门禁记录合法。

## 人工建议检查
- [ ] FL-M1 (manual-advisory, FL-AC4) — action: 操作翻页手势、菜单和配置重排；expected: 页面内容和位置同步；programmatic fallback: FL-C1、FL-C5。
