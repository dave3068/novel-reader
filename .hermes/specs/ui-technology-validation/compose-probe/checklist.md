# Compose Multiplatform 验证清单

## 必需程序化检查
- [ ] CM-C1 (CM-T1, CM-T2, CM-R1) — test: `bash experiments/ui-selection/scripts/test-pagination.sh compose --pages 1000`; expect exit 0 并生成合法能力判定。
- [ ] CM-C2 (CM-T3, CM-R2) — test: `cd experiments/ui-selection/compose-probe && ./gradlew desktopTest --tests '*Ffi*'`; expect exit 0，桥接生命周期、线程、取消和复制断言通过。
- [ ] CM-C3 (CM-T5, CM-R4) — cmd: `bash experiments/ui-selection/scripts/build-linux.sh --release compose`; expect exit 0，Release 构建物及运行依赖清单存在。
- [ ] CM-C4 (CM-T5, CM-R4) — cmd: `bash experiments/ui-selection/scripts/smoke-linux.sh --backend x11 --timeout 30 compose`; expect exit 0、就绪 JSON 合法且无残留进程。
- [ ] CM-C5 (CM-T4, CM-R3) — cmd: `python3 experiments/ui-selection/scripts/verify-ui-contract.py --candidate compose`; expect exit 0，三页面契约或淘汰门禁记录合法。

## 人工建议检查
- [ ] CM-M1 (manual-advisory, CM-AC4) — action: 验证键盘、鼠标、页面切换和重排；expected: 输入与页面状态一致；programmatic fallback: CM-C1、CM-C5。
