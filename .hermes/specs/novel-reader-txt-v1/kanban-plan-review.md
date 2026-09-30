# Kanban 拆分评估与修正记录

> 看板：`novel-reader-txt-v1`（workdir `${NR_PROJECT_DIR}`，全部卡片 `dir:` 共用工作区）
> 权威清单：`kanban-cards.json` / 生成与审计器：`kanban-cards.py` / 卡片 id 映射：`kanban-cards.ids.json`
> 记录时间：2026-09-12

## 1. 结论

- 任务来自 `spec.md` 与三个子规格的 `tasks.md`，**不是**按技术层切分，而是按「可独立构建与验证的能力切片」切分。
- 最终 **36 张卡片**：manager 6 / developer 23 / verifier 7。
- 全部卡片串行成链，除两个根卡片外每张卡只有一个父卡片 ⇒ 任一时刻 `ready ≤ 1`。
- 两个根卡片为 `blocked`：`M1`（等待用户授权开工）、`ENV1`（等待延期验收环境）。
- 现况：`blocked=2, todo=34, ready=0, running=0, triage=0, done=0`，无 worker 进程。

## 2. 角色分配

| 角色 | 卡片 | 说明 |
|---|---|---|
| manager | M1, M2, M3, M4, M5, ENV1 | M1 契约冻结与基线；M2/M3/M4 三个阶段门禁；M5 发布前总门禁；ENV1 延期环境门禁 |
| developer | rc1–rc7, dt1–dt6+dt3b, an1–an6+an3b, rel1, qa1 | 每个能力切片单卡，禁止跨模块扩散 |
| verifier | vrc1, vrc2, vdt1, vdt2, vdt3, van1, van2 | 独立复跑并给出 AC 判定；延期卡 vdt3/van2 挂 ENV1 |

## 3. 依赖链

```text
M1 → rc1 → rc2 → rc3 → rc4 → rc5 → rc6 → rc7 → vrc1 → vrc2 → M2
   → dt1 → dt2 → dt3 → dt3b → dt4 → dt5 → dt6 → vdt1 → vdt2 → M3
   → an1 → an2 → an3 → an3b → an4 → an5 → an6 → van1 → rel1 → qa1 → M4 → M5

延期支线（只被 ENV1 门禁，不影响 now-lane）：
dt6 + ENV1 → vdt3        an6 + ENV1 → van2
```

## 4. 评估中发现并已修正的问题

| # | 问题类型 | 具体 | 修正 |
|---|---|---|---|
| 1 | 无人负责的前向契约 | `verify-core-scope.py` 被 V-RC1 引用但无卡片创建 | 归入 `rc1`（含命令与范围） |
| 2 | 无人负责的前向契约 | `verify-desktop-scope.py` 被 V-DT1 引用 | 归入 `dt1` |
| 3 | 无人负责的前向契约 | `verify-android-boundary.py`、`verify-android-scope.py` 被 V-AN1/AN3/AN4 引用 | 归入 `an2`（含 `--check-no-whole-file-read`） |
| 4 | 无人负责的前向契约 | `verify-android-device-readiness.py` 被 M4 引用 | 归入 `an6` |
| 5 | 当前不可能达成的 DoD | `rc7` 要求「头文件/Rust 导出/Kotlin 声明三方一致」，但此时 Android 尚无 Kotlin 核心 | 改为「头文件 ↔ Rust 导出符号一致 + `--self-test` 证明缺失项可报出」；Kotlin 一致性移交 `an2` |
| 6 | 假前提（门禁必然失败） | `cargo test --workspace` 在桌面 crate 合入后缺 Slint/fontconfig 环境 | V-RC1 与 M2 命令加 `slint-env` 前缀；同步修正父规格 C2 与 reading-core C9 |
| 7 | 过大的多关注点卡片 | `dt3`（90m）同时含分页正确性与重排并发串行化 | 拆为 `dt3`（分页/正文区域/缓存，60m）+ `dt3b`（重排合并/串行化/过期丢弃，45m） |
| 8 | 过大的多关注点卡片 | `an3`（90m）同时含三页面接线与 ViewModel 并发编排 | 拆为 `an3`（页面与状态接线，60m）+ `an3b`（串行化/重排合并/过期丢弃，45m） |
| 9 | 缺少构建前提 | `an1` 之后的卡片统一用 `./gradlew`，但工程内无 wrapper | `an1` 必须生成 Gradle Wrapper（8.11.1） |
| 10 | 双模式未说明 | `dt6` 的脚本在 DT6 用 `--dry-run`、在 M3 用默认模式 | 明确脚本须同时支持两种模式 |
| 11 | 门禁脚本作者与执行者同一 | M5 自行创建并执行发布校验脚本 | 新增 `rel1`（developer 编写发布校验脚本与清单），M5 只运行 |
| 12 | 质量门禁缺位 | 三份子清单的 `fmt`/`clippy`/`lintDebug` 无卡片覆盖 | 新增 `qa1`（含「不得靠放宽 allow/关闭规则变绿」「超范围即 block」的有界修复规则） |
| 13 | 兄弟就绪缺陷 | 修正后 `rel1` 同时是 `qa1` 与 `M4` 的父卡片 ⇒ 两张卡同时 `ready` | `M4` 的父改为 `qa1`；并把「兄弟就绪检查」加入审计器 |

## 5. 事故与处置（网关自动认领）

`create --initial-status blocked` **不足以**阻止网关调度器把根卡片提升并认领：

- run#1：`M1`（旧卡 `t_e95c3ee8`）被自动认领为 `running`，worker pid 3415420，运行约 42s，**未产生任何产出物**；
- 处置：`reclaim`（记为 `manual_reclaim`）→ `block --kind needs_input` → 确认无残留 worker 进程；
- 新建卡时改为「创建后立即 `block`」，两次重建都实测捕获到 `status_before=ready`，证明该竞态真实存在；
- 为保持权威清单与看板一致，旧 32 张卡已整批**归档**（保留 comments/events/runs 供追溯，未做 `--rm` 永久删除）：

```text
旧卡 id（已归档，仅作证据）：
M1 t_e95c3ee8  ENV1 t_35439ff4  rc1 t_3ef3eb8e  rc2 t_e5309a19  rc3 t_dd944e82
rc4 t_3b2e28b8  rc5 t_9d241421  rc6 t_6a470cdb  rc7 t_84aac9b3  vrc1 t_fe3d2842
vrc2 t_377a8011  M2 t_c5f077be  dt1 t_5ac4c029  dt2 t_df54728d  dt3 t_720b6188
dt4 t_dd30c6df  dt5 t_0915cd02  dt6 t_c39af6b6  vdt1 t_211c319e  vdt2 t_9830dd4e
M3 t_711ecc07  an1 t_9f8386aa  an2 t_8e4728ae  an3 t_e063587a  an4 t_35c095cb
an5 t_0f72f990  an6 t_4b300c01  van1 t_1d090a55  M4 t_091dd199  M5 t_4c8fcdc5
vdt3 t_3d892ca7  van2 t_42404acc
```

另有一处操作陷阱已记录：探测/终止命令的**命令行自身**若包含 `work kanban task` 字样，按该模式匹配进程会终止自己的 shell（实测被 SIGTERM）。核对 worker 时须用不含该字面量的匹配方式。

## 6. 审计证据（可复现）

```bash
python3 .hermes/specs/novel-reader-txt-v1/kanban-cards.py --emit    # 生成权威清单
python3 .hermes/specs/novel-reader-txt-v1/kanban-cards.py --audit   # 静态图 + 看板实况 + 边级校验
```

真实输出（2026-09-12）：

```text
看板状态分布： {"blocked": 2, "todo": 34}
边级校验：active=36 expected=36 archived=32
卡片总数：36；根卡片：['M1', 'ENV1']
审计通过：无问题
```

审计器覆盖的规则：

1. 两个根卡片必须是 `M1` / `ENV1`，且状态为 `blocked`；
2. 拓扑排序无环；除根卡片与延期卡外每张卡父卡片数量为 1；
3. 排除延期卡后任何节点的可调度子卡片不超过 1（防止同时 `ready`）；
4. `ENV1` 的子卡片恰为 `vdt3` / `van2`，且两者同时挂在 `dt6` / `an6` 上（延期门不得门禁 now-lane）；
5. 每张卡正文含：仓库与规格、允许改动范围、禁止事项、完成定义、可执行命令块、证据与交接；
6. 角色只能是 manager/developer/verifier，`max_runtime ≤ 90m`；
7. 看板上不存在 `ready` / `running` 卡片；
8. 剔除已归档任务后，`task_links` 与权威图逐边一致。

## 7. 评估保留项（已判断、暂不修正）

| 项 | 判断 |
|---|---|
| `rc2` 含数据源实现 + 夹具生成器 | 夹具是本切片测试的必要前提，同卡更安全；DoD 有界且可验证，保留 |
| `rc7` 含 ABI 冻结 + Android 外部窗口协议 | 同一份契约两侧，拆开会造成两处定义；保留 |
| `an4` 90m（SAF + 私有缓存回退） | 同属「Android 数据源适配」单一关注点；若首次运行接近预算，按自动拆分策略续卡 |
| `van2` 90m | 延期设备验证卡，当前不可运行，预算不影响 now-lane |
| `--max-retries` 用默认（2） | 与网关 `failure_limit` 一致，未逐卡覆盖 |

## 8. 开工前置

1. 用户明确授权后，才解除 `M1` 封锁并派发第一张卡；
2. 派发前确认 worker profile 可无头执行（manager/developer/verifier 的 `single_query_mode` 已为 `approve`）；
3. 桌面端优先：M1 → reading-core → 桌面链先跑，Android 链在同一条串行链上顺次执行；
4. `ENV1` 保持 blocked，直到具备 Android 真机 / Windows / Wayland 环境；解除后 `vdt3`、`van2` 方可进入 `todo → ready`。
