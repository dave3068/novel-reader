#!/usr/bin/env python3
"""novel-reader-txt-v1 看板卡片的权威清单、创建器与审计器。

用法：
  python3 kanban-cards.py --emit            # 生成 kanban-cards.json（权威清单）
  python3 kanban-cards.py --create          # 建板 + 建卡 + 封锁根卡片
  python3 kanban-cards.py --audit           # 程序化审计卡片图与看板状态

设计约束（来自 hermes-kanban-operations）：
  - 全部卡片串行成链，除两个根卡片外每张卡都有父卡片，保证 ready=0；
  - 两个根卡片（M1 / ENV1）以 blocked + needs_input 创建，阻止网关自动认领整条链；
  - 延期环境验证卡只挂在 ENV1 上，ENV1 不门禁任何实现卡与 now-lane 验证卡；
  - 单一写入者：共用 dir 工作区，靠串行链与 max_in_progress=1 保证不并发。
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

REPO = "${NR_PROJECT_DIR}"
SPEC_ROOT = f"{REPO}/.hermes/specs/novel-reader-txt-v1"
BOARD = "novel-reader-txt-v1"
BOARD_NAME = "Novel Reader TXT V1"
BOARD_DESC = "docs/novel-reader-v1-development-plan.md 与 .hermes/specs/novel-reader-txt-v1 的 Spec 驱动实施看板（仅 TXT，桌面优先）"
HERMES = os.environ.get("HERMES_BIN") or os.path.expanduser("~/.hermes/hermes-agent/venv/bin/hermes")
SCRIPT_JSON = Path(__file__).with_name("kanban-cards.json")

COMMON = f"""## 仓库与规格（绝对路径）
- 仓库：`{REPO}`
- 主规格：`.hermes/specs/novel-reader-txt-v1/spec.md`
- 子规格：`.hermes/specs/novel-reader-txt-v1/<子规格>/spec.md` + `tasks.md` + `checklist.md`
- 开发方案：`docs/novel-reader-v1-development-plan.md`
- 共享设计：`.hermes/specs/novel-reader-txt-v1/design.md`

## 通用规则
- 只在本卡「允许改动范围」内改动文件，不得扩散到其它模块。
- 不得改写 `docs/novel-reader-solution.md`、`docs/novel-reader-v1-development-plan.md`、`docs/ui-technology-selection-plan.md`；不得改写 `.hermes/specs/` 下既有规格（除非本卡明确要求）。
- 不得把 `experiments/ui-selection/` 的模拟书库、合成文本源、空 External 数据源引入正式产品路径。
- 完成必须以真实命令、退出码、关键输出为依据；禁止只写结论或自报成功。
- 无法满足完成定义时：developer 用 `kanban_block`（kind=needs_input）写明缺失前提；verifier 用 `request_changes` 给出复现步骤与期望。
"""

# code, title, assignee, runtime, skills, parents, 交付物, 允许改动范围, 禁止, DoD, [(命令, 期望)], 证据文件
CARDS: list[dict] = [
    # ================= M1：启动门禁（根卡片，blocked） =================
    dict(
        code="M1",
        title="M1 启动门禁：规格冻结、基线确认与开工条件裁定",
        assignee="manager",
        runtime="30m",
        skills=["spec"],
        parents=[],
        deliverable="""冻结 V1 契约并确认开工条件，产出 `results/gates/gate-0-baseline.md`：
1. 五个规格文件的 sha256（`spec.md`、三个子规格 `spec.md`、`design.md`）；
2. 冻结契约清单：ABI 版本号、错误码枚举、单次窗口默认 64 KiB / 上限 256 KiB、`ContentPosition` 字段、布局签名字段、SQLite 表清单、JNI 入口清单；
3. 工具链与基线命令真实输出：`rustc --version`、`cargo --version`、Slint 环境加载、`java -version`、Gradle、NDK 路径、`fc-match sans-serif:lang=zh-cn`；
4. 开工条件裁定：本机可完成哪些卡、哪些卡属于延期环境（Android 真机 / Windows / Wayland），并写明延期卡不动摇 now-lane 结。""",
        scope="`results/gates/`、`.hermes/specs/novel-reader-txt-v1/`（只读）、`scripts/`（仅新建可空的基线目录结构）",
        forbidden="""- 不得修改任何产品代码（`crates/`、`desktop/`、`android/`）；
- 不得把任何卡片从 blocked/todo 改成 ready；
- 不得降低或改写规格里的验收标准。""",
        dod="`results/gates/gate-0-baseline.md` 存在，四项内容齐全且命令输出为真实粘贴；延期项与 now-lane 边界有明确结论。",
        commands=[
            ("cd " + REPO + " && sha256sum .hermes/specs/novel-reader-txt-v1/spec.md .hermes/specs/novel-reader-txt-v1/design.md .hermes/specs/novel-reader-txt-v1/*/spec.md", "exit 0，输出 5 个哈希并写入 gate-0"),
            ("rustc --version && cargo --version && fc-match sans-serif:lang=zh-cn && ls /opt/dev-tools/android-sdk/ndk/", "exit 0，版本与中文字体匹配结果写入 gate-0"),
            ("source /opt/dev-tools/slint-libs/slint-env.sh && pkg-config --modversion fontconfig freetype2", "exit 0，Slint 编译依赖可用"),
        ],
        evidence="results/gates/gate-0-baseline.md",
    ),
    # ================= reading-core：开发 =================
    dict(
        code="rc1",
        title="RC1 Rust workspace、crate 边界与错误/ABI 版本骨架",
        assignee="developer",
        runtime="45m",
        skills=[],
        parents=["M1"],
        deliverable="""建立正式 Rust workspace 与五个核心 crate 的可编译骨架：
- `crates/reading-core`、`reading-source`、`reading-index`、`reading-storage`、`reading-ffi`；
- 统一错误类型与错误码（含 `InvalidArgument/OutOfRange/Utf8Boundary/BookNotFound/NoOpenBook/Io/Cancelled/Internal`）；
- `ABI_VERSION` 常量与 `CORE_VERSION`；
- `rust-toolchain.toml` 固定 Rust 版本；
- 依赖边界检查脚本 `scripts/verify-core-scope.py`：核心不得依赖 UI/Android/网络，且不得出现 Synthetic/空 External 生产数据源。""",
        scope="`Cargo.toml`、`rust-toolchain.toml`、`crates/reading-*/Cargo.toml`、`crates/reading-core/src/{lib.rs,error.rs}`、`crates/reading-ffi/src/lib.rs`、`scripts/verify-core-scope.py`",
        forbidden="""- 核心 crate 不得依赖 Slint、Compose、Android SDK、网络库或窗口系统；
- 不得在本卡实现数据源、编码、章节、分页或存储业务（后续卡片负责）；
- 不得引入浮动依赖，必须锁定版本。""",
        dod="`cargo check` 对五个 crate 全部通过；错误码与 ABI 版本有单测覆盖；`cargo tree -p reading-core` 无 UI/Android 依赖。",
        commands=[
            ("cd " + REPO + " && cargo check -p reading-core -p reading-source -p reading-index -p reading-storage -p reading-ffi", "exit 0"),
            ("cd " + REPO + " && cargo test -p reading-core --release -- error_codes abi_version", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && cargo tree -p reading-core --edges normal", "不含 slint / android / tokio-tungstenite 等 UI 或网络依赖"),
            ("cd " + REPO + " && python3 scripts/verify-core-scope.py", "exit 0，核心无 UI/Android/网络依赖，无 Synthetic/空 External 生产数据源"),
        ],
        evidence="results/rc1-workspace.md",
    ),
    dict(
        code="rc2",
        title="RC2 TXT 有限窗口数据源、字节边界与文件指纹",
        assignee="developer",
        runtime="60m",
        skills=[],
        parents=["rc1"],
        deliverable="""实现 `TextSource` 抽象与文件型实现：
- `read_at(offset, len)` 定长读取，默认窗口 64 KiB、硬上限 256 KiB，超出即拒绝；
- 窗口两端自动修正到合法 UTF-8 字符边界，并回报 `dropped_leading` / `trimmed_tail` / `eof`；
- 文件指纹：大小 + 头/中/尾抽样哈希 + 可得的修改时间；
- 夹具生成器 `scripts/generate-fixtures.py`（`smoke` 与 `pressure` 两档，确定性 seed，输出 manifest 含哈希）。""",
        scope="`crates/reading-source/src/{lib.rs,source.rs,fingerprint.rs}`、`scripts/generate-fixtures.py`、`tests/fixtures/`（仅小型夹具）",
        forbidden="""- 不得一次性读取整个文件或把全文放入任何结构；
- 不得把 100MB/1GB 压力夹具提交到版本库（只提交生成脚本与 manifest 规则）；
- 不得在窗口层做编码转换（编码是 RC3 的职责）。""",
        dod="窗口上限、UTF-8 边界修正、EOF 语义、指纹稳定性均有单测；压力档夹具可在本机生成且单次物化字节 ≤ 256 KiB。",
        commands=[
            ("cd " + REPO + " && python3 scripts/generate-fixtures.py --profile smoke --seed 20260912 --output /tmp/nr-smoke", "exit 0，生成 manifest.json"),
            ("cd " + REPO + " && cargo test -p reading-source --release -- bounded_window fingerprint utf8_boundary", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && python3 scripts/generate-fixtures.py --profile pressure --seed 20260912 --output /tmp/nr-pressure && cargo test -p reading-source --release -- max_window_cap --test-threads=1", "exit 0，单次窗口 ≤ 262144 字节有断言"),
        ],
        evidence="results/rc2-source.md",
    ),
    dict(
        code="rc3",
        title="RC3 编码识别（BOM/UTF-8/UTF-16/GB18030/GBK/Big5）与手动覆盖",
        assignee="developer",
        runtime="60m",
        skills=[],
        parents=["rc2"],
        deliverable="""实现编码识别与覆盖：
- BOM 优先（UTF-8/UTF-16LE/UTF-16BE）；
- 无 BOM 时先验证 UTF-8，不可信时对 GB18030/GBK/Big5 做统计评分；
- 返回 `encoding + confidence + detector_version`；
- 支持用户手动覆盖，覆盖后使内容/章节/分页索引失效但**不修改原文件**；
- 检测只读有限窗口（头部 + 采样），不得整本扫描。""",
        scope="`crates/reading-source/src/encoding.rs`、`crates/reading-source/tests/`、`tests/fixtures/`（编码夹具）",
        forbidden="""- 不得写回或改写用户 TXT；
- 不得对整本文件做编码统计；
- 不得把低可信度结果伪装成高可信度。""",
        dod="七类编码夹具各有断言；低可信度场景返回可覆盖状态；原文件 sha256 在切换编码前后一致。",
        commands=[
            ("cd " + REPO + " && cargo test -p reading-source --release -- encoding", "exit 0 且覆盖 BOM/UTF-8/UTF-16LE/UTF-16BE/GB18030/GBK/Big5"),
            ("cd " + REPO + " && cargo test -p reading-source --release -- encoding_override_keeps_file_untouched", "exit 0，切换编码后原文件哈希不变"),
        ],
        evidence="results/rc3-encoding.md",
    ),
    dict(
        code="rc4",
        title="RC4 章节索引、ContentPosition 与锚点恢复",
        assignee="developer",
        runtime="60m",
        skills=[],
        parents=["rc3"],
        deliverable="""实现位置与章节语义：
- `ContentPosition`：`book_id + content_unit_id + chapter_index + original_byte_offset + nearby_fingerprint`，V1 单 TXT 固定一个 content unit；
- 章节规则（第一章/第1章/第001章/第一卷/卷一/楔子/序章/前言/后记/尾声/Chapter N/独立数字标题）；
- 无章节命中时生成单一虚拟全文章节，保证文件仍可读；
- 锚点生成与有界半径就近匹配恢复（复用有限窗口，不整本读）。""",
        scope="`crates/reading-core/src/{position.rs,chapter.rs,content_unit.rs}`、`crates/reading-index/src/`、对应 tests",
        forbidden="""- 不得把动态页码作为位置身份；
- 不得把正文存入数据库或长期内存；
- 章节扫描不得阻塞调用方（需可取消/可增量）。""",
        dod="章节规则、虚拟章节回退、位置往返一致、锚点在同文本重复出现的段落中能就近命中，均有断言。",
        commands=[
            ("cd " + REPO + " && cargo test -p reading-core -p reading-index --release -- chapter position anchor", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && cargo test -p reading-core --release -- no_chapter_falls_back_to_virtual_chapter", "exit 0"),
        ],
        evidence="results/rc4-index.md",
    ),
    dict(
        code="rc5",
        title="RC5 分页边界、布局签名与缓存失效",
        assignee="developer",
        runtime="60m",
        skills=[],
        parents=["rc4"],
        deliverable="""实现分页协调语义：
- 布局签名字段：视口宽高、方向、字体族/字重、字号、行距、段距、首行缩进、边距、排版算法版本、平台字体指纹；
- `commit_page_boundary` / 查询 / `invalidate(signature)`；
- 边界校验：`end >= start`、end 位于合法字符边界、必须严格前进、同一签名同一起点结果一致；
- 过期结果拒绝：按 `request_id` / `layout_id` 丢弃旧排版结果。""",
        scope="`crates/reading-core/src/{pagination.rs,layout.rs}`、`crates/reading-index/src/boundary.rs`、对应 tests",
        forbidden="""- 不得在本卡做字形光栅或依赖任何 UI 框架；
- 不得允许 end ≤ start 造成死循环；
- 不得用百分比冒充准确页码。""",
        dod="正反向分页连续无丢字/重字、非法边界被拒、签名变更全部失效、过期 layout_id 结果被丢弃，均有断言。",
        commands=[
            ("cd " + REPO + " && cargo test -p reading-core --release -- pagination boundary layout", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && cargo test -p reading-core --release -- stale_layout_result_rejected invalidate_by_signature", "exit 0"),
        ],
        evidence="results/rc5-pagination.md",
    ),
    dict(
        code="rc6",
        title="RC6 SQLite schema、迁移与仓储（进度/书签/索引安全）",
        assignee="developer",
        runtime="60m",
        skills=[],
        parents=["rc5"],
        deliverable="""实现本地存储：
- `migrations/0001_init.sql`：`books`、`book_sources`、`chapters`、`reading_progress`、`bookmarks`、`reader_settings`、`layout_records`、`app_meta`；
- 事务化仓储：导入 upsert、重复导入、进度幂等 upsert、书签 CRUD；
- 迁移前备份数据库；迁移失败可恢复；
- `清理索引/分页缓存` 只删可重建数据，**不得**删除书籍、进度、书签。""",
        scope="`crates/reading-storage/`、`migrations/`、对应 tests",
        forbidden="""- 数据库不得保存完整正文，只允许元数据、索引、位置和短摘要；
- 不得使用破坏性迁移覆盖用户数据；
- 不得删除用户原始文件。""",
        dod="初始迁移可重复执行；模块化安全测试证明清理索引后书籍/进度/书签仍在；迁移与异常中断恢复有断言。",
        commands=[
            ("cd " + REPO + " && cargo test -p reading-storage --release", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && cargo test -p reading-storage --release -- index_clear_keeps_user_metadata migration_recovery", "exit 0"),
        ],
        evidence="results/rc6-storage.md",
    ),
    dict(
        code="rc7",
        title="RC7 正式 C ABI / JNI 导出层与头文件契约",
        assignee="developer",
        runtime="60m",
        skills=[],
        parents=["rc6"],
        deliverable="""冻结跨语言契约：
- `include/novel_reader.h`：不透明句柄、64 位 id/偏移、UTF-8+长度字符串、显式 `nr_release_buffer`、错误码、`nr_cancel_request`、ABI 版本宏；
- `crates/reading-ffi` 导出实现，覆盖打开 TXT、读窗口、提交页边界、保存/读取位置、章节批次、书签、失效布局、取消；
- Android 外部内容协议：Kotlin 提供受限窗口字节，Rust 裁定坐标、边界与位置，禁止传整本正文；
- 校验脚本 `scripts/verify-abi.py`：比对头文件与 Rust 导出符号；Kotlin 目录存在时一并比对 Kotlin 声明，缺失时必须显式打印跳过项而不是静默通过。""",
        scope="`crates/reading-ffi/`、`include/novel_reader.h`、`scripts/verify-abi.py`、对应 tests",
        forbidden="""- 不得跨边界传递 Kotlin/Slint 对象或 Rust 引用；
- 不得一次返回整本正文；
- 不得让缓冲区所有权含糊（谁申请、谁释放必须写明并有测试）。""",
        dod="头文件与 Rust 导出符号逐项一致（本阶段 Android Kotlin 尚未建立，Kotlin 声明一致性由 AN2 负责）；`verify-abi.py --self-test` 用临时 fixture 证明缺失项会被报出而非静默通过；句柄生命周期、错误码、缓冲释放、取消均有测试；ABI 版本有宏与断言。",
        commands=[
            ("cd " + REPO + " && cargo build -p reading-ffi --release", "exit 0"),
            ("cd " + REPO + " && python3 scripts/verify-abi.py --header include/novel_reader.h --allow-missing-kotlin --self-test", "exit 0，头文件与导出符号一致；自测证明缺失项可被报出；Kotlin 缺失时明确打印跳过项"),
            ("cd " + REPO + " && cargo test -p reading-ffi --release", "exit 0 且测试数 > 0"),
        ],
        evidence="results/rc7-abi.md",
    ),
    # ================= reading-core：验证 =================
    dict(
        code="vrc1",
        title="V-RC1 独立验证：核心窗口/编码/位置/分页（AC1–AC4）",
        assignee="verifier",
        runtime="40m",
        skills=[],
        parents=["rc7"],
        deliverable="""独立复跑并给出 AC1–AC4 判定，证据写入 `results/verify/core-functional-verify.md`：
- 逐条列出执行的命令、退出码、关键输出（测试数、断言名）；
- 明确 AC1（有限窗口/100MB-1GB）、AC2（编码/原文件不改）、AC3（章节/位置/锚点）、AC4（分页/失效）各自的 PASS/FAIL 与理由；
- FAIL 时给出最小复现命令与期望值。""",
        scope="`results/verify/`（只写证据文件）",
        forbidden="""- 不得修改产品代码来让检查通过（发现问题用 `request_changes` 退回 developer）；
- 不得把「代码存在」「自报通过」当作证据；
- 不得降低断言。""",
        dod="核心全量测试与压力档命令均真实执行；四条 AC 判定齐全；任何 FAIL 都带复现命令。",
        commands=[
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && cargo test --workspace --all-targets --release", "exit 0；记录测试数与失败数（桌面卡片合入后 workspace 含 Slint 依赖，必须保留 slint-env 前缀）"),
            ("cd " + REPO + " && python3 scripts/generate-fixtures.py --profile pressure --seed 20260912 --output /tmp/nr-verify-pressure && cargo test -p reading-source --release -- bounded_window --test-threads=1", "exit 0，单窗口上限断言通过"),
            ("cd " + REPO + " && python3 scripts/verify-core-scope.py", "exit 0，核心无 Slint/Compose/Android/网络依赖，且无 Synthetic 生产数据源"),
        ],
        evidence="results/verify/core-functional-verify.md",
    ),
    dict(
        code="vrc2",
        title="V-RC2 独立验证：存储安全与 ABI 契约（AC5–AC6）",
        assignee="verifier",
        runtime="40m",
        skills=[],
        parents=["vrc1"],
        deliverable="""独立复跑并给出 AC5–AC6 判定，证据写入 `results/verify/core-storage-abi-verify.md`：
- SQLite 迁移、重复导入、进度/书签幂等、清理缓存不丢用户元数据；
- 原始 TXT 在全部操作后 sha256 不变；
- ABI 头文件/导出符号/错误码/句柄释放/取消一致性；
- 文档未被反向改写。""",
        scope="`results/verify/`（只写证据文件）",
        forbidden="""- 不得用自造数据库替代真实迁移脚本执行；
- 不得修改产品代码；
- 不得跳过原文件哈希对比。""",
        dod="两块证据齐全并给出 PASS/FAIL；原文件哈希前后一致已实测；文档 diff 检查为空。",
        commands=[
            ("cd " + REPO + " && cargo test -p reading-storage --release", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && python3 scripts/verify-abi.py --header include/novel_reader.h --kotlin android/app/src/main/java --allow-missing-kotlin", "exit 0"),
            ("cd " + REPO + " && git diff --exit-code -- docs/novel-reader-solution.md docs/novel-reader-v1-development-plan.md docs/ui-technology-selection-plan.md", "exit 0，方案文档未被改写"),
        ],
        evidence="results/verify/core-storage-abi-verify.md",
    ),
    # ================= M2：核心阶段门禁 =================
    dict(
        code="M2",
        title="M2 阶段门禁 1：reading-core 验收汇总与放行裁定",
        assignee="manager",
        runtime="30m",
        skills=["spec"],
        parents=["vrc2"],
        deliverable="""汇总核心阶段验收，写入 `results/gates/gate-1-core.md`：
- 读取 V-RC1/V-RC2 的真实证据文件，逐条核对 AC1–AC6，不得只引用卡片状态；
- 判定 PASS / FAIL / 有条件通过；FAIL 时创建或指定 remediation 卡并写清依赖重排；
- 明确 reading-core 是否可冻结供桌面端与 Android 端使用，以及冻结的 ABI 版本号。""",
        scope="`results/gates/`",
        forbidden="""- 不得在证据缺失时判定 PASS；
- 不得改写子规格 checklist 的已完成状态来迁就现状；
- 不得直接修改产品代码（发现问题走 remediation 卡）。""",
        dod="gate-1-core.md 含逐条 AC 判定、引用到的证据文件路径与真实输出摘要；放行结论明确。",
        commands=[
            ("cd " + REPO + " && ls -l results/verify/core-functional-verify.md results/verify/core-storage-abi-verify.md", "两个证据文件均存在且非空"),
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && cargo test --workspace --all-targets --release 2>&1 | tail -20", "exit 0；把测试汇总行粘贴进 gate-1"),
        ],
        evidence="results/gates/gate-1-core.md",
    ),
    # ================= desktop：开发 =================
    dict(
        code="dt1",
        title="DT1 Slint 正式桌面工程、页面状态投影与布局常量集中管理",
        assignee="developer",
        runtime="45m",
        skills=["novel-reader-ui-env"],
        parents=["M2"],
        deliverable="""建立桌面产品骨架：
- `crates/desktop-app`（Rust 应用层）+ `desktop/ui/`（Slint 文件）；
- 书架 / 阅读 / 设置 三页面骨架可启动，状态由 Rust 投影到 Slint，UI 不含阅读业务；
- 版式常量（顶栏/底部栏/正文内边距/菜单高度）集中为单一来源，供 Rust 排版与 Slint 渲染共用；
- 后台结果切回 Slint UI 线程的机制；
- 范围检查脚本 `scripts/verify-desktop-scope.py`：生产桌面不得出现 Synthetic 书籍、空 External 数据源、整本正文复制、截图 OCR。""",
        scope="`crates/desktop-app/`、`desktop/ui/`、`desktop/Cargo.toml`、`scripts/verify-desktop-scope.py`",
        forbidden="""- UI 层不得实现编码识别、章节识别、分页边界、位置校验、书签规则；
- 不得沿用探针的模拟书库作为产品数据源；
- 不得使用私有 Slint 文本 API。""",
        dod="Release 可编译并可启动出窗口；三页面可切换；版式常量只有一个定义处（grep 可验证）。",
        commands=[
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && cargo check -p desktop-app", "exit 0"),
            ("cd " + REPO + " && cargo test -p desktop-app --release -- layout_constants_single_source", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && python3 scripts/verify-desktop-scope.py", "exit 0，生产桌面无 Synthetic 书籍/空 External/整本复制/截图 OCR"),
        ],
        evidence="results/dt1-desktop-skeleton.md",
    ),
    dict(
        code="dt2",
        title="DT2 桌面文件选择、TXT 导入、来源指纹与失效重新关联",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["dt1"],
        deliverable="""实现导入链路：
- 桌面文件选择器接入，仅接受 `.txt`；
- 校验扩展名 / 可读性 / 元数据，失败给出可读错误且**不留半成品记录**；
- 写入 `books` + `book_sources`（含指纹）；
- 原文件不存在或指纹变化时进入「需要重新关联」，保留进度与书签，提供重新选择入口；
- 任何操作都不修改、不删除用户原始文件。""",
        scope="`crates/desktop-app/src/{import.rs,source_status.rs}`、`desktop/ui/`、`tests/e2e/`",
        forbidden="""- 不得删除或重命名用户文件；
- 不得在导入失败时留下可打开的残缺书籍记录；
- 不得自动用新文件覆盖旧进度。""",
        dod="导入成功/非 TXT/空文件/不可读/失效重连五类行为均有测试；导入后原文件哈希不变。",
        commands=[
            ("cd " + REPO + " && cargo test -p desktop-app --release -- import", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && cargo test -p desktop-app --release -- import_rejects_non_txt missing_source_relink", "exit 0"),
        ],
        evidence="results/dt2-import.md",
    ),
    dict(
        code="dt3",
        title="DT3 分页测量、正文区域对齐与页面缓存",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["dt2"],
        deliverable="""实现桌面分页闭环：
- 窗口映射完成后才取正文区域尺寸（禁止用事件循环启动前的 0×0 尺寸）；
- 测量与 Slint 渲染同口径：字号、行高、字距、段距、首行缩进、边距一致；
- 当前页 + 前一页 + 后三页预读，缓存有上限；
- 字号/窗口/DPI 变化后以当前位置或锚点重新分页（合并与串行化由 DT3B 负责）；
- 正文区不出现滚动条、半字裁切或旧页面覆盖；EOF 有明确状态不产生空白死循环页。""",
        scope="`crates/desktop-app/src/{pagination.rs,page_cache.rs,reflow.rs}`、`desktop/ui/`",
        forbidden="""- 不得在 UI 层复制分页算法（必须走 reading-core）；
- 不得用 verticalScroll 或百分比伪装整页分页；
- 不得并发重排互相覆盖、不得让旧结果覆盖新页面。""",
        dod="正反向翻页、重排、UTF-8 边界、EOF、正文区域一致性均有测试；实际启动后正文区无滚动且铺满。",
        commands=[
            ("cd " + REPO + " && cargo test -p desktop-app --release -- pagination utf8_boundary page_cache", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && cargo test -p desktop-app --release -- pagination_200_pages backward_100_pages", "exit 0，无丢字/重字/非法边界"),
            ("cd " + REPO + " && cargo test -p desktop-app --release -- viewport_after_map_only paragraph_and_indent_zero", "exit 0"),
        ],
        evidence="results/dt3-pagination.md",
    ),
    dict(
        code="dt3b",
        title="DT3B 桌面重排合并、串行化与过期结果丢弃",
        assignee="developer",
        runtime="45m",
        skills=["novel-reader-ui-env"],
        parents=["dt3"],
        deliverable="""实现重排的并发与状态正确性（与 DT3 分页本身分离）：
- 字号/窗口/DPI/边距等连续变更触发**合并式**重排（拖拽只保留最终值）；
- 所有重排/翻页/保存位置入口经**同一把锁串行化**，避免并发重填缓存；
- 过期 `request_id`/`layout_id` 结果必须丢弃，不得覆盖当前页面；
- 重排期间 UI 保持可响应并显示状态。""",
        scope="`crates/desktop-app/src/{reflow.rs,app_state.rs}`、`desktop/ui/`",
        forbidden="""- 不得在 UI 层复制分页算法；
- 不得用「最后写入者获胜」掩盖并发覆盖；
- 不得让重排阻塞 UI 线程。""",
        dod="合并式重排、串行化、过期结果丢弃均有断言；连续 30 次字号变更只产生一次最终重排。",
        commands=[
            ("cd " + REPO + " && cargo test -p desktop-app --release -- merged_reflow_requests stale_layout_result_rejected", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && cargo test -p desktop-app --release -- serialized_state_mutations reflow_non_blocking", "exit 0"),
        ],
        evidence="results/dt3b-reflow.md",
    ),
    dict(
        code="dt4",
        title="DT4 桌面章节、进度、书签、全局设置与重启恢复",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["dt3b"],
        deliverable="""补齐阅读产品功能：
- 章节目录与章节跳转（走 reading-core 章节与位置语义）；
- 进度与书签（新增/删除/重命名/跳转），位置以 `ContentPosition` 为准；
- 全局排版设置：字体、字号、行距、段距、首行缩进、边距、明亮/暗黑、手势或按钮模式；
- 应用重启后恢复最近成功保存位置；
- 端到端脚本 `tests/e2e/run_txt_lifecycle.py`（导入→阅读→翻页→设置→书签→保存→重启→恢复）。""",
        scope="`crates/desktop-app/src/{chapters.rs,progress.rs,bookmarks.rs,settings.rs}`、`desktop/ui/`、`tests/e2e/run_txt_lifecycle.py`",
        forbidden="""- 不得用页码作为进度身份；
- 不得把正文写入数据库；
- 不得为每本书另建一套独立设置体系（V1 只有全局设置）。""",
        dod="端到端脚本真实执行通过；重启恢复位置正确；书签增删改跳有效。",
        commands=[
            ("cd " + REPO + " && python3 tests/e2e/run_txt_lifecycle.py --platform desktop --fixture tests/fixtures/smoke.txt", "exit 0，输出含导入/翻页/书签/重启恢复各阶段结果"),
            ("cd " + REPO + " && cargo test -p desktop-app --release -- progress bookmarks chapters settings", "exit 0 且测试数 > 0"),
        ],
        evidence="results/dt4-reading.md",
    ),
    dict(
        code="dt5",
        title="DT5 桌面 Linux X11 E2E 与 Release 冒烟脚本",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["dt4"],
        deliverable="""建立可重复的桌面门禁脚本与证据目录：
- `scripts/smoke-desktop.sh`：Release 构建 + X11 启动 + 结构化就绪输出 + 退出清理（无残留进程）；
- `tests/e2e/run_txt_safety.py`：移除书架记录与清理缓存后原文件/进度/书签安全；
- `results/` 目录结构与结果 JSON 字段约定；
- 运行说明写入 `docs/desktop/`（构建、启动、夹具、清理）。""",
        scope="`scripts/smoke-desktop.sh`、`tests/e2e/run_txt_safety.py`、`results/`、`docs/desktop/`",
        forbidden="""- 不得以截图或肉眼观察替代程序化退出码证据；
- 不得在脚本中使用 `| head -1` 这类会触发 SIGPIPE 的写法（set -o pipefail 下会静默 141）；
- 不得留下僵尸进程或后台服务。""",
        dod="两个脚本在 Linux X11 真实执行通过；`results/` 输出被后续 verifier 卡直接复用。",
        commands=[
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt", "exit 0，就绪字段与退出清理均通过，无残留进程"),
            ("cd " + REPO + " && python3 tests/e2e/run_txt_safety.py", "exit 0，原文件与用户元数据安全"),
        ],
        evidence="results/dt5-desktop-gates.md",
    ),
    dict(
        code="dt6",
        title="DT6 Windows 与 Linux Wayland 构建/打包路径（目标状态保持待验证）",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["dt5"],
        deliverable="""补齐非当前环境的构建与打包路径：
- `desktop/packaging/` 下的 Windows 与 Linux 打包脚本/说明（完整发布目录口径，不使用单可执行文件口径）；
- `scripts/verify-desktop-targets.py`：同时支持 `--dry-run`（只校验打包脚本、依赖清单、状态字段齐全）与默认模式（输出各目标状态，未实测目标输出 `待验证`）；
- 依赖与系统要求文档（含 glibc/字体/运行库）。""",
        scope="`desktop/packaging/`、`scripts/verify-desktop-targets.py`、`docs/desktop/`",
        forbidden="""- 不得伪造 Windows/Wayland 的实机结果；
- 不得在未实测时把状态写成「通过」；
- 不得为此卡引入新的第三方运行时依赖。""",
        dod="校验脚本可执行且对未实测目标明确输出 `待验证`；打包路径与依赖文档齐全并经过一次本机 dry-run。",
        commands=[
            ("cd " + REPO + " && python3 scripts/verify-desktop-targets.py --targets windows-x86_64 linux-wayland --dry-run", "exit 0，未实测目标标记 `待验证`"),
        ],
        evidence="results/dt6-desktop-targets.md",
    ),
    # ================= desktop：验证 =================
    dict(
        code="vdt1",
        title="V-DT1 独立验证：桌面 Linux X11 阅读闭环（AC-D1–AC-D5）",
        assignee="verifier",
        runtime="50m",
        skills=["novel-reader-ui-env"],
        parents=["dt6"],
        deliverable="""独立复跑桌面闭环，证据写入 `results/verify/desktop-verify.md`：
- Release 构建 + X11 冒烟 + 端到端 lifecycle + 200 页正向 / 100 页反向 + 正文区域与重排；
- 逐条给出 AC-D1…AC-D5 的 PASS/FAIL；
- 用真实 TXT 夹具，不得使用模拟书库或合成文本。""",
        scope="`results/verify/`",
        forbidden="""- 不得修改产品代码让检查通过；
- 不得以「窗口打开了」当作分页正确；
- 不得用探针工程替代正式桌面应用。""",
        dod="证据文件含全部命令、退出码、关键输出与五条 AC 判定；发现的问题已用 request_changes 退回并记录复现命令。",
        commands=[
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && cargo build --release -p desktop-app", "exit 0"),
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && bash scripts/smoke-desktop.sh --backend x11 --fixture tests/fixtures/smoke.txt", "exit 0，无残留进程"),
            ("cd " + REPO + " && python3 tests/e2e/run_txt_lifecycle.py --platform desktop --fixture tests/fixtures/smoke.txt", "exit 0"),
            ("cd " + REPO + " && python3 scripts/verify-desktop-scope.py", "exit 0，生产桌面无 Synthetic 书籍/空 External/整本复制/截图 OCR"),
        ],
        evidence="results/verify/desktop-verify.md",
    ),
    dict(
        code="vdt2",
        title="V-DT2 安全回归：书架移除与缓存清理不伤用户数据",
        assignee="verifier",
        runtime="40m",
        skills=[],
        parents=["vdt1"],
        deliverable="""独立验证数据安全，证据写入 `results/verify/desktop-safety-verify.md`：
- 移除书架记录后原 TXT 仍存在且哈希不变；
- 清理分页/章节/解压类缓存后书籍、进度、书签仍在；
- 重新关联流程不会静默丢弃旧进度；
- 列出所有涉及删除语义的操作及其范围确认。""",
        scope="`results/verify/`",
        forbidden="""- 不得修改产品代码；
- 不得在测试中删除用户的真实文件（只使用 /tmp 下的夹具副本）。""",
        dod="每一项删除语义都有前后对比证据；结论明确列出「应用永不删除原始文件」是否被代码证据支持。",
        commands=[
            ("cd " + REPO + " && python3 tests/e2e/run_txt_safety.py --json", "exit 0，输出原文件哈希前后一致"),
            ("cd " + REPO + " && cargo test -p desktop-app --release -- safety cache_clear", "exit 0"),
        ],
        evidence="results/verify/desktop-safety-verify.md",
    ),
    # ================= M3：桌面阶段门禁 =================
    dict(
        code="M3",
        title="M3 阶段门禁 2：桌面端验收汇总与延期项登记",
        assignee="manager",
        runtime="30m",
        skills=["spec"],
        parents=["vdt2"],
        deliverable="""汇总桌面阶段验收，写入 `results/gates/gate-2-desktop.md`：
- 核对 AC-D1…AC-D6，引用 V-DT1/V-DT2 真实证据；
- 明确 Windows/Wayland 状态（当前应为 `待验证`）并登记为延期项，不得写成通过；
- 判定桌面端是否达到「V1 桌面可用」并放行 Android 阶段；
- 如有 FAIL，指定 remediation 卡与依赖重排。""",
        scope="`results/gates/`",
        forbidden="""- 不得把未实测平台写成通过；
- 不得绕过证据直接采信 worker 自报。""",
        dod="gate-2-desktop.md 含逐条判定、证据路径、延期项清单与放行结论。",
        commands=[
            ("cd " + REPO + " && ls -l results/verify/desktop-verify.md results/verify/desktop-safety-verify.md", "证据文件均存在且非空"),
            ("cd " + REPO + " && python3 scripts/verify-desktop-targets.py --targets windows-x86_64 linux-wayland", "exit 0，未实测目标标记 `待验证`"),
        ],
        evidence="results/gates/gate-2-desktop.md",
    ),
    # ================= android：开发 =================
    dict(
        code="an1",
        title="AN1 Android Gradle 工程、版本锁与 ABI 构建脚本",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["M3"],
        deliverable="""建立正式 Android 工程与原生库构建链：
- `android/`（JDK 17、compileSdk/targetSdk 36、minSdk 31、`ndkVersion` 与已装 NDK 一致、`abiFilters` 首发 `arm64-v8a`）；
- `buildConfigField BUILD_MARKER` 供真机核对构建版本；
- `scripts/build-android-libs.sh`：交叉编译 Rust 静态库并链接为**自包含** `libnr_jni.so`，输出到 `jniLibs/<abi>/`；
- 不使用 CMake/externalNativeBuild（本机 SDK 无 CMake）；
- Debug/Release 两种构建目标与 `local.properties` 生成；
- **必须生成 Gradle Wrapper**（`gradle wrapper --gradle-version 8.11.1`，系统 Gradle 位于 /opt/dev-tools/gradle/gradle-8.11.1），使后续卡片统一使用 `./gradlew`；仓库中提交 wrapper 脚本与配置。""",
        scope="`android/`、`scripts/build-android-libs.sh`",
        forbidden="""- 不得引入 CMake 或 externalNativeBuild；
- 不得把 `.so` 手工提交为唯一来源（必须可由脚本重建）；
- 不得省略 `ndkVersion`（否则符号不剥离、包体异常）。""",
        dod="Gradle 构建可跑通；Rust arm64 交叉编译与 JNI 链接脚本 exit 0，`.so` 符号已剥离。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:assembleDebug", "exit 0，生成 Debug APK"),
            ("cd " + REPO + " && bash scripts/build-android-libs.sh --abi arm64-v8a", "exit 0，生成 libnr_jni.so 并完成符号剥离"),
        ],
        evidence="results/an1-gradle.md",
    ),
    dict(
        code="an2",
        title="AN2 JNI 薄封装、错误码映射与句柄生命周期",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["an1"],
        deliverable="""实现 Kotlin 侧薄封装：
- `NrCore`（external 声明）与 `NrEngine`（生命周期、关闭、取消、缓冲释放）与 `include/novel_reader.h` 严格一致；
- 错误码 → 业务错误映射（不解析错误文本）；
- 64 位 id/偏移保持精确（必要时以字符串传输，避免 JSON 数字截断）；
- JVM 单元测试覆盖边界映射、错误映射与句柄释放；
- `scripts/verify-android-boundary.py`：校验 JNI 声明/导出、错误码、句柄、缓冲释放、取消与 Kotlin 业务边界；
- `scripts/verify-android-scope.py`：扫描 Kotlin 不得复制章节/位置/分页/缓存失效/书签业务，不得整本正文传递，支持 `--check-no-whole-file-read` 标志。""",
        scope="`android/app/src/main/java/**/core/`、`android/app/src/test/java/**`",
        forbidden="""- 不得在 Kotlin 实现章节/位置/分页/缓存失效/书签业务；
- 不得把 Activity 或 Compose 状态传入核心；
- 不得用 `System.loadLibrary` 之外的方式绕过自包含 `.so`。""",
        dod="Kotlin 声明与头文件逐项一致；错误映射与释放有 JVM 单测；`verify-abi.py` 不再需要 `--allow-missing-kotlin`。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && python3 scripts/verify-abi.py --header include/novel_reader.h --kotlin android/app/src/main/java", "exit 0，三方声明一致"),
        ],
        evidence="results/an2-jni.md",
    ),
    dict(
        code="an3",
        title="AN3 Compose 三页面与真实状态接线",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["an2"],
        deliverable="""实现 Android 三页面与状态编排：
- 书架 / 阅读 / 设置 Composable（含 `testTag` 与可访问语义）；
- ViewModel：`StateFlow` 发布状态；每个控件接真实状态（空回调清零）；
- 排版/翻页/设置控件与状态双向一致（受控滑杆必须同步写回）；
- edge-to-edge 与状态栏内边距处理（顶栏不遮挡）；
- 正文区不使用 `verticalScroll`。""",
        scope="`android/app/src/main/java/**/ui/`、`android/app/src/main/java/**/reader/`、`android/app/src/test/java/**`",
        forbidden="""- 不得复制 Rust 阅读业务到 Kotlin；
- 不得让多个挂起入口并发重填缓存；
- 不得用空回调占位（每个控件必须接真实状态）。""",
        dod="三页面可编译；并发与重排合并有 JVM 单测；空回调清零（grep/静态检查可验证）。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && python3 scripts/verify-android-scope.py", "exit 0，无业务复制、无整本正文传递、无空回调占位"),
        ],
        evidence="results/an3-compose.md",
    ),
    dict(
        code="an3b",
        title="AN3B Android 状态串行化、重排合并与过期结果丢弃",
        assignee="developer",
        runtime="45m",
        skills=["novel-reader-ui-env"],
        parents=["an3"],
        deliverable="""实现 Android 侧并发与状态正确性（与页面接线分离）：
- 打开书籍/切章/翻页/重排/保存位置经**同一 Mutex 串行化**，避免预读缓存被并发重填；
- 字号等设置**同步**写回状态；重排请求**合并**（拖拽只保留最终值）；
- 过期 `request_id`/`layout_id` 结果丢弃，不覆盖当前页面；
- 旋转/后台切换时取消在途任务但不破坏应用级句柄。""",
        scope="`android/app/src/main/java/**/reader/`、`android/app/src/test/java/**`",
        forbidden="""- 不得复制 Rust 阅读业务到 Kotlin；
- 不得让多个挂起入口并发修改阅读状态；
- 不得用「最后写入者获胜」掩盖覆盖问题。""",
        dod="串行化、重排合并、过期丢弃、取消语义均有 JVM 单测；并发重填缓存的回归用例通过。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest --tests '*Concurrency*'", "exit 0 且测试数 > 0"),
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest --tests '*Reflow*'", "exit 0"),
        ],
        evidence="results/an3b-concurrency.md",
    ),
    dict(
        code="an4",
        title="AN4 SAF 数据源、持久 URI 权限与私有缓存回退",
        assignee="developer",
        runtime="90m",
        skills=["novel-reader-ui-env"],
        parents=["an3b"],
        deliverable="""实现 Android 真实数据源：
- `OpenDocument` + `takePersistableUriPermission`，保存 URI、显示名、大小与权限状态；
- 支持可靠随机访问时按窗口读取；
- 不支持时**可取消地**复制到应用私有缓存再以文件方式分页（复制有进度与取消）；
- 权限失效进入「需要重新关联」，不删除书籍记录；
- 旋转/后台切换不持有失效 Activity 引用。""",
        scope="`android/app/src/main/java/**/source/`、`AndroidManifest.xml`、`android/app/src/test/java/**`",
        forbidden="""- 不得保留探针的空 `ExternalTextSource` 行为作为生产路径；
- 不得一次读取整本文件到 ByteArray；
- 不得在无权限时静默失败。""",
        dod="SAF 元数据/权限/窗口读取/回退复制均有 JVM 或静态证据；无整本加载路径。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest --tests '*Saf*'", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && python3 scripts/verify-android-scope.py --check-no-whole-file-read", "exit 0"),
        ],
        evidence="results/an4-saf.md",
    ),
    dict(
        code="an5",
        title="AN5 Android 排版口径与页边界提交（像素/字距/行高）",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["an4"],
        deliverable="""固化 Android 排版契约：
- 字号统一用像素参与测量（`sp × density`），测量得到的实际行高回传 Compose；
- Compose 显式 `letterSpacing = 0.sp` 且与测量完全一致；关键排版属性不再依赖 Material 默认值；
- 测量预留整行安全余量（宁可少排一行，不裁切）；
- 向核心提交合法 UTF-8 页末字节，过期结果不覆盖当前页面。""",
        scope="`android/app/src/main/java/**/reader/`、`android/app/src/test/java/**`",
        forbidden="""- 不得混用像素与 sp 口径；
- 不得依赖 `bodyLarge` 等默认字距/行高；
- 不得用滚动容器掩盖分页错误。""",
        dod="口径一致性、UTF-8 页末、安全余量与过期丢弃均有单测；小字号末行不做裁切。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest --tests '*Pagination*'", "exit 0 且测试数 > 0"),
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest --tests '*Layout*'", "exit 0"),
        ],
        evidence="results/an5-layout.md",
    ),
    dict(
        code="an6",
        title="AN6 APK 构建、内容/签名校验脚本与真机调试包",
        assignee="developer",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["an5"],
        deliverable="""完成 Android 构建交付物：
- `scripts/verify-android-apk.py`：校验包名、minSdk/targetSdk、`lib/arm64-v8a/libnr_jni.so` 存在、DEX 含构建标记、签名状态；
- Debug 与 Release APK 均可产出（Release 未签名属预期，需在结论中写明）；
- 打包前清理旧 APK（避免增量残留尾部数据）；
- 产出分发目录 `dist/` 与真机安装说明 `docs/android/`（含 BUILD_MARKER 核对方式、MIUI 安装/调试开关限制说明）；
- `scripts/verify-android-device-readiness.py`：校验 `docs/android/` 的真机步骤、期望结果与 `待验证` 状态字段，以及 `results/` 的延期证据路径约定。""",
        scope="`scripts/verify-android-apk.py`、`scripts/build-android-apk.sh`、`dist/`、`docs/android/`、`android/`",
        forbidden="""- 不得把未签名 Release 当作签名失败；
- 不得用单个 `classes.dex` 判断新代码是否入包（多 dex 场景必须扫全部）；
- 不得声称已真机验证。""",
        dod="Debug/Release APK 均生成；校验脚本 exit 0 并输出包体、ABI 库、DEX 标记与签名状态；`docs/android/` 说明可供真机使用。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:assembleDebug :app:assembleRelease", "exit 0，两个 APK 均生成"),
            ("cd " + REPO + " && python3 scripts/verify-android-apk.py android/app/build/outputs/apk/debug/app-debug.apk --abi arm64-v8a", "exit 0，输出包名/版本/JNI 库/DEX 标记/签名状态"),
        ],
        evidence="results/an6-apk.md",
    ),
    # ================= android：验证 =================
    dict(
        code="van1",
        title="V-AN1 独立验证：Android 单测、边界扫描与 APK 内容（AC-A1–AC-A5）",
        assignee="verifier",
        runtime="50m",
        skills=["novel-reader-ui-env"],
        parents=["an6"],
        deliverable="""独立复跑 Android now-lane 验证，证据写入 `results/verify/android-verify.md`：
- JVM 单测全量、Rust arm64 交叉编译、JNI `.so`、Debug/Release APK、APK 内容与签名；
- Kotlin 业务边界扫描（无章节/位置/分页/缓存失效/书签复制，无整本正文传递）；
- 逐条给出 AC-A1…AC-A5 判定，并**单列** AC-A6 的真机延期项清单（状态 `待验证`）。""",
        scope="`results/verify/`",
        forbidden="""- 不得把 APK 构建成功等同于 Android 全面通过；
- 不得修改产品代码；
- 不得跳过边界扫描。""",
        dod="证据文件含全部命令与判定；延期项清单明确且未被计入通过项。",
        commands=[
            ("cd " + REPO + "/android && ./gradlew :app:testDebugUnitTest", "exit 0 且测试数 > 0"),
            ("cd " + REPO + " && python3 scripts/verify-android-boundary.py --header include/novel_reader.h --kotlin android/app/src/main/java", "exit 0"),
            ("cd " + REPO + " && python3 scripts/verify-android-apk.py android/app/build/outputs/apk/debug/app-debug.apk --abi arm64-v8a", "exit 0"),
        ],
        evidence="results/verify/android-verify.md",
    ),
    # ================= M4 / M5：收口门禁 =================
    dict(
        code="rel1",
        title="REL1 发布物清单、版本标识与发布校验脚本",
        assignee="developer",
        runtime="45m",
        skills=[],
        parents=["van1"],
        deliverable="""建立发布校验脚本与发布物清单：
- `scripts/verify-v1-structure.py`：正式工程目录、三个子规格要求的目录、迁移、脚本与 ABI 头文件齐全；
- `scripts/verify-v1-release.py --desktop --android`：输出发布物清单、版本、构建标识、依赖许可、SHA-256 与**未验证项**；
- `docs/release/` 发布说明模板（含已知限制与延期验收项占位）。""",
        scope="`scripts/verify-v1-structure.py`、`scripts/verify-v1-release.py`、`docs/release/`",
        forbidden="""- 不得伪造未实测平台的结果；
- 不得把未签名 Release APK 当作构建失败；
- 不得把延期项写成通过。""",
        dod="两个脚本可执行且真实运行；发布物清单、版本标识、许可清单与未验证项清单均被输出。",
        commands=[
            ("cd " + REPO + " && python3 scripts/verify-v1-structure.py", "exit 0"),
            ("cd " + REPO + " && python3 scripts/verify-v1-release.py --desktop --android", "exit 0，输出发布物与未验证项清单"),
        ],
        evidence="results/rel1-release-scripts.md",
    ),
    dict(
        code="qa1",
        title="QA1 全仓质量门禁：fmt / clippy / Gradle lint 与有界修复",
        assignee="developer",
        runtime="60m",
        skills=[],
        parents=["rel1"],
        deliverable="""执行全仓质量门禁并做**有界**修复：
- `cargo fmt --all -- --check`；
- `source /opt/dev-tools/slint-libs/slint-env.sh && cargo clippy --workspace --all-targets -- -D warnings`；
- `cd android && ./gradlew lintDebug`；
- 记录基线与最终结果，并把三条命令的退出码写入证据文件。""",
        scope="各 crate/模块内部的格式与 lint 修复、`scripts/`",
        forbidden="""- 不得通过放宽 allow/`#[allow]` 范围、关闭 lint 规则、修改 CI 阈值或改动本卡范围外的模块来「变绿」；
- 若某个问题的修复需要改动超过 5 个文件、或涉及其他卡片负责的业务语义，**必须 block** 并说明，不得静默修改；
- 不得顺手重构无关代码。""",
        dod="三条命令 exit 0；证据文件含基线与最终退出的真实输出；如有 block 则写明具体问题与归属卡片。",
        commands=[
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && cargo fmt --all -- --check", "exit 0"),
            ("source /opt/dev-tools/slint-libs/slint-env.sh && cd " + REPO + " && cargo clippy --workspace --all-targets -- -D warnings", "exit 0"),
            ("cd " + REPO + "/android && ./gradlew lintDebug", "exit 0；仅允许已在证据文件中登记的非阻断告警"),
        ],
        evidence="results/qa1-quality-gate.md",
    ),
    dict(
        code="M4",
        title="M4 阶段门禁 3：Android 构建交付验收与延期门禁登记",
        assignee="manager",
        runtime="30m",
        skills=["spec"],
        parents=["qa1"],
        deliverable="""汇总 Android 阶段验收，写入 `results/gates/gate-3-android.md`：
- 核对 AC-A1…AC-A6，引用 V-AN1 真实证据；
- 明确「可构建/可测试」与「真机未验证」的分界，登记延期验收项与前置条件（设备、开发者选项开关）；
- 判定是否放行最终门禁。""",
        scope="`results/gates/`",
        forbidden="""- 不得把真机项写成通过；
- 不得绕过证据文件。""",
        dod="gate-3-android.md 含逐条判定、延期项与前置条件、放行结论。",
        commands=[
            ("cd " + REPO + " && ls -l results/verify/android-verify.md", "证据文件存在且非空"),
            ("cd " + REPO + " && python3 scripts/verify-android-device-readiness.py --docs docs/android --results results", "exit 0，真机步骤与 `待验证` 状态完整"),
        ],
        evidence="results/gates/gate-3-android.md",
    ),
    dict(
        code="M5",
        title="M5 V1 发布前总门禁、结构校验与延期项清单",
        assignee="manager",
        runtime="45m",
        skills=["spec"],
        parents=["M4"],
        deliverable="""执行 V1 总门禁并产出发布裁定：
- `scripts/verify-v1-structure.py`：正式工程目录、三个子规格要求的目录、迁移、脚本与 ABI 头文件齐全；
- `scripts/verify-v1-release.py --desktop --android`：发布物、版本、构建标识、依赖许可与已知未验证项；
- `results/gates/gate-final.md`：发布物清单、SHA-256、许可证清单、已知限制、**延期验收项**（Windows/Wayland/Android 真机）及其不阻塞当前判定的说明；
- 明确 V1 结论只能是「TXT 桌面可用 + Android 可构建」；两个延期 verifier 卡未完成时不得宣称全平台发布。""",
        scope="`scripts/verify-v1-structure.py`、`scripts/verify-v1-release.py`、`results/gates/`、`docs/release/`",
        forbidden="""- 不得把延期项计为通过；
- 不得在 required 检查失败时宣布完成；
- 不得删除用户数据或原始文件。""",
        dod="两个校验脚本可在本机执行；gate-final.md 含发布物清单、许可证、延期项与明确裁定。",
        commands=[
            ("cd " + REPO + " && python3 scripts/verify-v1-structure.py", "exit 0"),
            ("cd " + REPO + " && python3 scripts/verify-v1-release.py --desktop --android", "exit 0，输出发布物与未验证项清单"),
            ("cd " + REPO + " && git diff --exit-code -- docs/novel-reader-solution.md docs/novel-reader-v1-development-plan.md", "exit 0，方案文档未被改写"),
        ],
        evidence="results/gates/gate-final.md",
    ),
    # ================= ENV1：延期环境门禁（根卡片，blocked） =================
    dict(
        code="ENV1",
        title="ENV1 延期环境门禁：Android 真机 / Windows / Linux Wayland",
        assignee="manager",
        runtime="20m",
        skills=[],
        parents=[],
        deliverable="""作为延期验收的前置门禁，登记所需环境与前置条件：
- Android 真机（Android 12+，含 MIUI/澎湃的「USB 安装」「USB 调试（安全设置）」开关）与 adb 可用性；
- Windows 10/11 x86_64 干净环境；
- Linux Wayland 会话（本机仅有 X11）；
- 每项的前置条件、验收范围、对应延期验证卡编号与期望证据；
- 结论写入 `results/gates/gate-env-deferred.md`。
本卡只门禁延期验证卡（V-DT3 / V-AN2），不门禁任何实现卡与 now-lane 验证卡。""",
        scope="`results/gates/`",
        forbidden="""- 不得在本卡内执行或伪造设备测试结果；
- 不得把本卡解除封锁作为实现卡的前置条件；
- 不得宣称延期项已通过。""",
        dod="延期门禁文档存在；三项环境的前置条件与对应验证卡编号齐全；本卡保持 blocked 直到用户提供环境。",
        commands=[
            ("cd " + REPO + " && python3 -c \"import json,pathlib;p=pathlib.Path('results/gates/gate-env-deferred.md');print('exists' if p.exists() else 'missing')\"", "输出 exists 且文档含三项环境与对应卡片编号"),
        ],
        evidence="results/gates/gate-env-deferred.md",
    ),
    # ================= 延期验证卡（挂 ENV1） =================
    dict(
        code="vdt3",
        title="V-DT3 延期验证：Windows 10/11 与 Linux Wayland 发布物实机运行",
        assignee="verifier",
        runtime="60m",
        skills=["novel-reader-ui-env"],
        parents=["dt6", "ENV1"],
        deliverable="""在具备环境后执行延期验收，证据写入 `results/verify/desktop-deferred-verify.md`：
- Windows 10/11 与 Linux Wayland 干净环境下运行完整发布目录（非单可执行文件）；
- 中文渲染、文件选择、键盘/鼠标翻页、重排、退出；
- 逐项给出 AC-D6 判定；环境不可得时保持 `待验证` 并写明缺失前提。""",
        scope="`results/verify/`",
        forbidden="""- 不得在未实测环境伪造结果；
- 不得用模拟器或兼容层结果替代真机结论（需注明）；
- 不得修改产品代码。""",
        dod="在环境可得时产出真实结果；否则保持 `待验证` 并记录前置条件，不计入通过项。",
        commands=[
            ("cd " + REPO + " && python3 scripts/verify-desktop-targets.py --targets windows-x86_64 linux-wayland", "exit 0；实测后才可把对应目标状态从 `待验证` 改为实测结论"),
        ],
        evidence="results/verify/desktop-deferred-verify.md",
    ),
    dict(
        code="van2",
        title="V-AN2 延期验证：Android 真机 SAF/旋转/后台/返回/JNI 取消",
        assignee="verifier",
        runtime="90m",
        skills=["novel-reader-ui-env"],
        parents=["an6", "ENV1"],
        deliverable="""在具备真机后执行延期验收，证据写入 `results/verify/android-device-verify.md`：
- 安装 APK 并按 BUILD_MARKER 核对装机版本；
- 打开 UTF-8/GB18030/UTF-16 TXT，确认无乱码、无滚动正文、无半字裁切；
- 横竖屏切换、Home/后台恢复、进程重建、系统返回、权限失效重关联、批次读取中途取消；
- 记录机型、系统版本、未覆盖项；环境不可得时保持 `待验证`。""",
        scope="`results/verify/`",
        forbidden="""- 不得把 APK 构建成功当作真机通过；
- 不得用模拟器结论替代物理设备结论；
- 不得修改产品代码（问题走 remediation 卡）。""",
        dod="环境可得时产出真实机型证据与 AC-A6 判定；否则保持 `待验证` 并记录前置条件。",
        commands=[
            ("adb devices && adb install -r " + REPO + "/dist/app-debug.apk", "环境可得时 exit 0；不可得时记录 `待验证` 与缺失前提"),
        ],
        evidence="results/verify/android-device-verify.md",
    ),
]


def build_body(card: dict) -> str:
    lines: list[str] = []
    lines.append(f"## 角色：`{card['assignee']}`\n")
    lines.append(COMMON)
    lines.append("## 本卡交付物（有界范围）\n" + card["deliverable"] + "\n")
    lines.append("## 允许改动范围\n" + card["scope"] + "\n")
    lines.append("## 禁止事项\n" + card["forbidden"] + "\n")
    lines.append("## 完成定义（DoD）\n" + card["dod"] + "\n")
    lines.append("## 必须真实执行的验证命令")
    for cmd, expect in card["commands"]:
        lines.append(f"```bash\n{cmd}\n```\n期望：{expect}")
    lines.append("")
    lines.append(
        "## 证据与交接\n"
        f"- 把真实命令、退出码与关键输出写入 `{card['evidence']}`，并在 Kanban 完成交接中引用该文件路径。\n"
        "- 自报结论不算证据；缺证据的项必须标为未完成。\n"
        "- 失败或前提不足时不得静默通过：developer 用 `kanban_block`，verifier 用 `request_changes`。"
    )
    return "\n".join(lines)


def card_records() -> list[dict]:
    out = []
    for c in CARDS:
        out.append(
            {
                "code": c["code"],
                "title": c["title"],
                "assignee": c["assignee"],
                "parents": c["parents"],
                "max_runtime": c["runtime"],
                "skills": c["skills"],
                "workspace": f"dir:{REPO}",
                "evidence": c["evidence"],
                "body": build_body(c),
            }
        )
    return out


def hermes(args: list[str], timeout: int = 180) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    env.setdefault("HERMES_BIN", HERMES)
    env.setdefault("HERMES_HOME", os.path.expanduser("~/.hermes"))
    return subprocess.run(
        [HERMES, "kanban", "--board", BOARD, *args],
        capture_output=True, text=True, timeout=timeout, env=env,
    )


def parse_task_id(stdout: str) -> str:
    """`--json` 输出为多行美化 JSON，且 stdout 可能带前置告警行，需首个 '{' 起解析。"""
    idx = stdout.find("{")
    if idx < 0:
        raise ValueError(f"未能从输出中解析任务 JSON：{stdout[:400]!r}")
    obj, _ = json.JSONDecoder().raw_decode(stdout[idx:])
    tid = obj.get("id") or obj.get("task", {}).get("id")
    if not tid:
        raise ValueError(f"输出中缺少任务 id：{obj!r}")
    return tid


def card_status(tid: str) -> str:
    proc = hermes(["list", "--json"], timeout=180)
    if proc.returncode != 0:
        return "unknown"
    for t in json.loads(proc.stdout):
        if t.get("id") == tid:
            return t.get("status", "unknown")
    return "unknown"


def block_root(tid: str, reason: str) -> None:
    """根卡片必须在创建后立即封锁：`--initial-status blocked` 会被网关调度器提升为 ready 并认领。"""
    status = card_status(tid)
    if status == "blocked":
        print(f"  root {tid} already blocked")
        return
    proc = hermes(["block", "--kind", "needs_input", tid, reason])
    print(f"  block root {tid}: rc={proc.returncode} status_before={status}")


def emit() -> int:
    records = card_records()
    SCRIPT_JSON.write_text(
        json.dumps({"board": BOARD, "repo": REPO, "spec_root": SPEC_ROOT, "cards": records},
                   ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    print(f"wrote {SCRIPT_JSON} with {len(records)} cards")
    return 0


def ensure_board() -> None:
    env = dict(os.environ)
    env.setdefault("HERMES_HOME", os.path.expanduser("~/.hermes"))
    subprocess.run(
        [HERMES, "kanban", "boards", "create", BOARD, "--name", BOARD_NAME,
         "--description", BOARD_DESC, "--default-workdir", REPO],
        capture_output=True, text=True, timeout=180, env=env,
    )


def create() -> int:
    ensure_board()
    records = card_records()
    ids: dict[str, str] = {}
    root_reasons = {
        "M1": "等待用户明确授权开工：create-only 阶段，禁止网关自动认领整条开发链。",
        "ENV1": "等待用户提供延期验收环境（Android 真机 / Windows / Wayland）；本卡只门禁 V-DT3 与 V-AN2。",
    }
    # 两个根卡片先创建并**立即封锁**，避免网关在创建其余卡片期间认领整条链
    for rec in records:
        if rec["parents"]:
            continue
        args = ["create", rec["title"], "--body", rec["body"], "--assignee", rec["assignee"],
                "--workspace", rec["workspace"], "--max-runtime", rec["max_runtime"],
                "--initial-status", "blocked", "--json", "--created-by", "spec-driven-planner",
                "--idempotency-key", f"nrtxtv2-{rec['code']}"]
        for sk in rec["skills"]:
            args += ["--skill", sk]
        proc = hermes(args)
        if proc.returncode != 0:
            print(f"FAILED create {rec['code']}: {proc.stderr or proc.stdout}", file=sys.stderr)
            return 1
        ids[rec["code"]] = parse_task_id(proc.stdout)
        print(f"created {rec['code']} -> {ids[rec['code']]} (root)")
        block_root(ids[rec["code"]], root_reasons[rec["code"]])

    for rec in records:
        if not rec["parents"]:
            continue
        args = ["create", rec["title"], "--body", rec["body"], "--assignee", rec["assignee"],
                "--workspace", rec["workspace"], "--max-runtime", rec["max_runtime"],
                "--json", "--created-by", "spec-driven-planner",
                "--idempotency-key", f"nrtxtv2-{rec['code']}"]
        for sk in rec["skills"]:
            args += ["--skill", sk]
        for parent in rec["parents"]:
            args += ["--parent", ids[parent]]
        proc = hermes(args)
        if proc.returncode != 0:
            print(f"FAILED create {rec['code']}: {proc.stderr or proc.stdout}", file=sys.stderr)
            return 1
        ids[rec["code"]] = parse_task_id(proc.stdout)
        print(f"created {rec['code']} -> {ids[rec['code']]} parents={rec['parents']}")

    # 兜底：再次确认两个根卡片处于 blocked（重复同 kind 封锁不解除时不会进 triage）
    for code in ("M1", "ENV1"):
        block_root(ids[code], root_reasons[code])

    Path(SCRIPT_JSON.with_name("kanban-cards.ids.json")).write_text(
        json.dumps(ids, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print("ids written")
    return 0


def fetch_board() -> list[dict]:
    proc = hermes(["list", "--json"], timeout=180)
    if proc.returncode != 0:
        raise SystemExit(f"list failed: {proc.stderr}")
    return json.loads(proc.stdout)


def audit() -> int:
    records = {r["code"]: r for r in card_records()}
    ids_file = SCRIPT_JSON.with_name("kanban-cards.ids.json")
    ids = json.loads(ids_file.read_text(encoding="utf-8")) if ids_file.exists() else {}
    problems: list[str] = []

    # --- 静态图审计 ---
    codes = list(records)
    if len(codes) != len(set(codes)):
        problems.append("重复卡片 code")
    roots = [c for c in codes if not records[c]["parents"]]
    if sorted(roots) != ["ENV1", "M1"]:
        problems.append(f"根卡片不是 M1/ENV1，而是 {roots}")
    for c in codes:
        for p in records[c]["parents"]:
            if p not in records:
                problems.append(f"{c} 的父卡片 {p} 不存在")
    # 拓扑排序（环检测）
    indeg = {c: len([p for p in records[c]["parents"] if p in records]) for c in codes}
    children: dict[str, list[str]] = {c: [] for c in codes}
    for c in codes:
        for p in records[c]["parents"]:
            if p in records:
                children[p].append(c)
    queue = [c for c in codes if indeg[c] == 0]
    seen = 0
    while queue:
        cur = queue.pop()
        seen += 1
        for ch in children[cur]:
            indeg[ch] -= 1
            if indeg[ch] == 0:
                queue.append(ch)
    if seen != len(codes):
        problems.append(f"卡片图存在环：visited={seen}/{len(codes)}")
    # 每张卡的正文要素
    for c in codes:
        body = records[c]["body"]
        for marker in ("仓库与规格", "允许改动范围", "禁止事项", "完成定义（DoD）", "必须真实执行的验证命令", "证据与交接"):
            if marker not in body:
                problems.append(f"{c} 正文缺少「{marker}」")
        if "```bash" not in body:
            problems.append(f"{c} 正文没有可执行命令块")
        if records[c]["assignee"] not in ("manager", "developer", "verifier"):
            problems.append(f"{c} 角色非法：{records[c]['assignee']}")
        # 运行时长 ≤ 90m
        rt = records[c]["max_runtime"]
        if not rt.endswith("m") or int(rt[:-1]) > 90:
            problems.append(f"{c} max_runtime 超出预算：{rt}")
    # 延期门的隔离性：ENV1 只能作为延期验证卡的父卡片
    env_children = sorted(children["ENV1"])
    if set(env_children) != {"vdt3", "van2"}:
        problems.append(f"ENV1 的子卡片应为 vdt3/van2，实际 {env_children}")
    # vdt3/van2 必须也挂在各自 scope 卡上
    if "dt6" not in records["vdt3"]["parents"]:
        problems.append("vdt3 未挂在 dt6 上")
    if "an6" not in records["van2"]["parents"]:
        problems.append("van2 未挂在 an6 上")
    # 串行链：除根卡片与两处刻意的双父延期卡外，每张卡恰好一个父，保证 ready ≤ 1
    deferred = {"vdt3", "van2"}
    for c in codes:
        if c in ("M1", "ENV1") or c in deferred:
            continue
        if len(records[c]["parents"]) != 1:
            problems.append(f"{c} 父卡片数量不为 1（会破坏串行链）：{records[c]['parents']}")
    # 兄弟就绪检查：排除延期卡后，任何节点的可调度子卡片不得超过 1 个，否则会同时出现 ready > 1
    for c in codes:
        active_children = [ch for ch in children[c] if ch not in deferred]
        if len(active_children) > 1:
            problems.append(f"{c} 有多个可调度子卡片（会同时 ready）：{active_children}")

    # --- 看板实况审计 ---
    live = {}
    if ids:
        tasks = {t["id"]: t for t in fetch_board()}
        for code, tid in ids.items():
            t = tasks.get(tid)
            if not t:
                problems.append(f"{code} 在看板上不存在：{tid}")
                continue
            live[code] = t
        status_counts: dict[str, int] = {}
        for t in live.values():
            status_counts[t.get("status", "?")] = status_counts.get(t.get("status", "?"), 0) + 1
        print("看板状态分布：", json.dumps(status_counts, ensure_ascii=False))
        for code in ("M1", "ENV1"):
            st = live.get(code, {}).get("status")
            if st != "blocked":
                problems.append(f"根卡片 {code} 状态应为 blocked，实际 {st}")
        for code, t in live.items():
            st = t.get("status")
            if st in ("ready", "running"):
                problems.append(f"{code} 处于 {st}，创建阶段不应有可运行卡片")
        # 边级校验：读取看板 SQLite 的 task_links，剔除已归档任务后与权威图逐边比对
        db = Path.home() / ".hermes" / "kanban" / "boards" / BOARD / "kanban.db"
        if db.exists():
            import sqlite3

            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            archived = {r[0] for r in con.execute("select id from tasks where status='archived'")}
            rows = con.execute("select parent_id, child_id from task_links").fetchall()
            rev = {v: k for k, v in ids.items()}
            live_edges = {
                (rev.get(p, p), rev.get(c, c))
                for p, c in rows
                if p not in archived and c not in archived
            }
            expected_edges = {(p, c) for c in codes for p in records[c]["parents"]}
            print(f"边级校验：active={len(live_edges)} expected={len(expected_edges)} archived={len(archived)}")
            if live_edges != expected_edges:
                problems.append(
                    "边级不一致："
                    f"missing={sorted(expected_edges - live_edges)} "
                    f"unexpected={sorted(live_edges - expected_edges)}"
                )
    else:
        print("提示：未找到 ids 文件，仅执行静态审计")

    print(f"卡片总数：{len(codes)}；根卡片：{roots}")
    if problems:
        print("审计发现问题：")
        for p in problems:
            print("  - " + p)
        return 1
    print("审计通过：无问题")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--emit", action="store_true")
    ap.add_argument("--create", action="store_true")
    ap.add_argument("--audit", action="store_true")
    args = ap.parse_args()
    if args.emit:
        return emit()
    if args.create:
        return create()
    if args.audit:
        return audit()
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
