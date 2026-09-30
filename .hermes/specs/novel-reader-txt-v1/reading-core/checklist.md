# reading-core 验证清单

## Environment

- Rust 1.98.0；测试可在离线环境执行。
- 夹具输出目录：`tests/generated/` 或临时目录；100MB/1GB 文件不得提交版本库。
- 当前阶段不要求 Android 真机。

## Programmatic Required Checks

- [ ] C1 (C-T1, CR6) — `cmd:` `cargo check -p reading-core -p reading-source -p reading-index -p reading-storage -p reading-ffi`; expect exit 0，核心 crate 编译且依赖边界正确。
- [ ] C2 (C-T2, CR1, CR2) — `test:` `cargo test -p reading-source --release`; expect exit 0，BOM、UTF-8/UTF-16/GB18030/GBK/Big5、换行、超长行和窗口边界通过。
- [ ] C3 (C-T2, CR1) — `cmd:` `python3 scripts/generate-fixtures.py --profile pressure --seed 20260912 --output /tmp/nr-core-fixtures && cargo test -p reading-source --release -- bounded_window --test-threads=1`; expect exit 0，100MB/1GB 读取不超过 256 KiB 单窗口，物化字节有界。
- [ ] C4 (C-T3, CR3, CR4) — `test:` `cargo test -p reading-core --release -- position pagination anchor`; expect exit 0，章节、无章节回退、正反向分页、按**内容编码**判定的字符边界（UTF-8 沿用旧规则；GBK/GB18030 合法字符边界必须被接受、字符内部偏移必须被拒绝）、锚点和布局失效通过。
- [ ] C5 (C-T4, CR5) — `test:` `cargo test -p reading-storage --release`; expect exit 0，迁移、重复导入、进度/书签 upsert、索引清理和原始文件安全通过。
- [ ] C6 (C-T5, CR6) — `cmd:` `python3 scripts/verify-abi.py --header include/novel_reader.h --kotlin android/app/src/main/java`; expect exit 0，ABI 版本、句柄、错误码、64 位字段、缓冲释放和取消声明一致。
- [ ] C7 (C-T6, CR1–CR6) — `test:` `cargo test --workspace --all-targets`; expect exit 0，核心全量测试通过。
- [ ] C8 (C-T6, CR1–CR6) — `cmd:` `python3 scripts/verify-core-scope.py`; expect exit 0，生产核心不依赖 Slint、Compose、Android SDK、网络或 Synthetic/空 External 生产数据源。

## Regression Checks

- [ ] C9 — `cmd:` `source /opt/dev-tools/slint-libs/slint-env.sh && cargo clippy --workspace --all-targets -- -D warnings`; expect exit 0，无新增警告（桌面 crate 已在 workspace 内，必须保留 `slint-env` 前缀）。
- [ ] C10 — `file:` `git diff --exit-code -- docs/novel-reader-solution.md docs/novel-reader-v1-development-plan.md`; expect exit 0，方案文档保持不变。

## Advisory Manual Checks

- [ ] M1 (manual-advisory, AC2) — action: 用桌面和 Android UI 各打开 GB18030/UTF-16 文件观察视觉乱码情况；expected: 与 C2 的解码结果一致；programmatic fallback: C2；final reminder: 后续端到端人工验收时复核。

- [ ] C12 (C-T12, CR2, CR4, CR5) — `test:` `cargo test -p reading-ffi --test abi_contract gbk_`；expect exit 0，
  针对 **GBK 夹具**断言：① 扫描出真实章节（≥2）且标题为可读 UTF-8；② GBK 合法字符边界处的页边界提交被接受（当前 legacy 编码不做字符边界单项校验，见 spec CR4 可判定性说明）；
  ③ 落在 GBK 字符**内部**的偏移仍被拒绝（负向对照，确保修复不退化为「一律放行」）。
- [ ] C13 (C-T13, CR2, CR4) — `test:` `cargo test -p reading-source is_boundary_is_encoding_aware`；expect exit 0，
  `is_boundary` 在 UTF-8 / UTF-16（码元对齐）/ GB18030（含四字节形式）/ GBK / Big5 下均按编码规则判定。

- [ ] C14 (C-T14, CR2) — `test:` `cargo test -p reading-ffi --test abi_contract set_encoding_`；expect exit 0，
  针对**第 19 项导出 `nr_set_encoding`**：① 强制 UTF-8 解释 GBK 夹具 ⇒ 重扫、索引版本递增、章节回退为单章；
  ② 传 0 清除覆盖 ⇒ 回到自动检测、真实章节重新出现；③ 非法编码码被拒；
  ④ **原文件逐字节不变**（「永不写回原始文件」硬约束）。
