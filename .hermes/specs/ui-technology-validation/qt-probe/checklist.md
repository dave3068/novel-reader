# Qt Quick/QML 验证清单

## 必需程序化检查
- [ ] QT-C1 (QT-T1, QT-T2, QT-R1) — test: `bash experiments/ui-selection/scripts/test-pagination.sh qt --pages 1000`; expect exit 0 并生成合法能力判定。
- [ ] QT-C2 (QT-T3, QT-R2) — test: `ctest --test-dir experiments/ui-selection/qt-probe/build/release -R ffi --output-on-failure`; expect exit 0，桥接生命周期、线程、取消和复制断言通过。
- [ ] QT-C3 (QT-T5, QT-R4) — cmd: `bash experiments/ui-selection/scripts/build-linux.sh --release qt`; expect exit 0，Release 构建物存在。
- [ ] QT-C4 (QT-T5, QT-R4) — cmd: `bash experiments/ui-selection/scripts/smoke-linux.sh --backend x11 --timeout 30 qt`; expect exit 0、就绪 JSON 合法且无残留进程。
- [ ] QT-C5 (QT-T4, QT-R3) — cmd: `python3 experiments/ui-selection/scripts/verify-ui-contract.py --candidate qt`; expect exit 0，三页面契约或淘汰门禁记录合法。
- [ ] QT-C6 (QT-T4, QT-R5) — file: `python3 experiments/ui-selection/scripts/verify-license-inventory.py --candidate qt`; expect exit 0，Qt 模块、动态库、QML 插件和许可来源字段完整。

## 人工建议检查
- [ ] QT-M1 (manual-advisory, QT-AC4) — action: 操作 QML 三页面、中文翻页和文件选择入口；expected: 状态一致且无渲染缺陷；programmatic fallback: QT-C1、QT-C4、QT-C5。
