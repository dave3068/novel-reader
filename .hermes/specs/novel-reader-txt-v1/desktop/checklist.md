# desktop 验证清单

## Environment

- 当前优先环境：Deepin 20.9、x86_64、X11、`DISPLAY=:0`。
- Slint 编译前：`source /opt/dev-tools/slint-libs/slint-env.sh`。
- 测试夹具：`tests/fixtures/smoke.txt`；压力夹具由核心脚本生成。
- Windows/Wayland 尚未作为当前阻塞环境，完成前保持待验证。

## Programmatic Required Checks

- [ ] C1 (D-T1, DR2, DR4) — `cmd:` `source /opt/dev-tools/slint-libs/slint-env.sh && cargo check -p desktop-app`; expect exit 0，正式桌面工程可编译且没有 Android/UI 无关核心依赖倒灌。
- [ ] C2 (D-T2, DR1) — `test:` `cargo test -p desktop-app --release -- import`; expect exit 0，TXT 扩展名、空文件、不可读文件和非 TXT 拒绝行为通过。
- [ ] C3 (D-T3, DR3) — `test:` `cargo test -p desktop-app --release -- pagination reflow utf8_boundary`; expect exit 0，正反向分页、重排、正文区域、合法边界和 EOF 通过。
- [ ] C4 (D-T4, DR2, DR5) — `cmd:` `python3 tests/e2e/run_txt_lifecycle.py --platform desktop --fixture tests/fixtures/smoke.txt`; expect exit 0，导入→章节→翻页→设置→书签→保存→重启恢复流程通过。
- [ ] C5 (D-T5, DR1–DR4) — `cmd:` `source /opt/dev-tools/slint-libs/slint-env.sh && bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt`; expect exit 0，窗口就绪、首屏、结构化状态和退出清理通过。
- [ ] C6 (D-T5, DR3) — `test:` `cargo test -p desktop-app --release -- pagination_200_pages backward_100_pages`; expect exit 0，页面无丢字、重字、半字、非法边界和死循环。
- [ ] C7 (D-T6, DR6) — `cmd:` `python3 scripts/verify-desktop-targets.py --targets windows-x86_64 linux-wayland`; expect exit 0，目标构建脚本、依赖和状态字段完整；未实际运行的目标标记 `待验证`。
- [ ] C8 (D-T7, DR1–DR6) — `cmd:` `python3 scripts/verify-desktop-scope.py`; expect exit 0，生产桌面不使用 Synthetic 书籍、空 External 数据源、截图 OCR 或整本正文复制。
- [ ] C11 (DT8, DR4) — `cmd:` `source /opt/dev-tools/slint-libs/slint-env.sh && python3 scripts/verify-desktop-input.py`; expect exit 0，键盘接收点（`FocusScope.key-pressed`）与正文区滚轮接收点（`TouchArea.scroll-event`）真实存在且只转发原始事件，键盘/滚轮都接到用户翻页入口（`NR-INPUT`，与「上一页/下一页」按钮同一函数），目录行/书签行/输入框/按钮均有可访问名称，且真实 `KeyPressed`/`PointerScrolled` 窗口事件驱动整页翻页、输入框聚焦时不翻页。

## Regression Checks

- [ ] C9 — `cmd:` `source /opt/dev-tools/slint-libs/slint-env.sh && cargo fmt --all -- --check && cargo clippy -p desktop-app --all-targets -- -D warnings`; expect exit 0。
- [ ] C10 — `cmd:` `source /opt/dev-tools/slint-libs/slint-env.sh && cargo build --release -p desktop-app`; expect exit 0，Release 构建成功。

## Advisory Manual Checks

- [ ] M1 (manual-advisory, AC-D4) — action: 在 Linux X11 实际操作文件选择、键盘/鼠标翻页、目录、书签、字号、暗黑模式和窗口缩放；expected: 视觉与程序化状态一致；programmatic fallback: C4、C5、C6；final reminder: 交付后请您复核桌面操作体验。
- [ ] M2 (manual-advisory, AC-D6) — action: 在 Windows 10/11 和 Linux Wayland 实际启动发布物；expected: 补充设备结果；programmatic fallback: C7；final reminder: 当前不把未实测平台视为已发布。
