# 看板暂停状态与恢复指引

> 暂停时间：2026-09-13 10:2x　暂停原因：**用户指示 —— token 消耗接近上限**

## 0. 当前全局状态（已验证）

```text
novel-reader-txt-v1   done=11  blocked=2  todo=23  ready=0  running=0
data-ingestion        done=44  blocked=4  todo=7   ready=0  running=0
worker 进程（全部看板）= 0
```

两个看板均等待一个完整派发周期（70 秒）后复核，`ready`/`running` 均为 0，确认派发器无法再拉起任务。

## 1. novel-reader-txt-v1

- **done = 11**：M1, rc1…rc7, vrc1, vrc2, M2（core 阶段已交付并通过独立验证）
- **blocked = 2**：`ENV1`（延期环境门禁，设计内保持封锁）、`t_cbd977c2`（DT1，用户暂停）
- **todo = 23**：因链根被封而不会自动提升
- 真实证据落盘于 `results/`：`gates/gate-0-baseline.md`、`gates/gate-1-core.md`、
  `rc1-workspace.md` … `rc7-abi.md`、`rc5-logs/`、`rc6-logs/`、`rc7-logs/`、`verify/`
- 工程骨架：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`crates/reading-*`、
  `include/`、`migrations/`、`scripts/`、`tests/`

**中断点**：DT1 被中断时正在编译（`crates/desktop-app/target/debug`，时间戳 10:11），
属**未完成、未验证**的中间状态。本仓库**不是 git 仓库**，无提交级回滚点。
恢复时不得把 DT1 的中间产物当作完成证据。

## 2. data-ingestion

- **done = 44**、**blocked = 4**（其中 2 张为本次暂停：`t_3af51cce`、`t_e4d34d76`）、**todo = 7**
- 被中断的两张卡：`O4 Kafka 排除与既有编排类型兼容回归`、`接入运行内核装配：IngestionTaskRunLauncher …`
- 同样按「回收在途认领 → `block --kind needs_input`」处置，工作区内可能留有未提交中间产物，
  仅供人工查看，不得计为完成证据。

## 3. Session store 问题（已修复）

**症状**：worker 日志出现
`Session store unavailable — this conversation will NOT be saved to disk … vtable constructor failed: messages_fts_trigram`

**根因**：不是默认 `~/.hermes/state.db`（该库一直健康，官方自检全绿），而是
**三个 worker profile 各自的 `state.db`**（`manager` / `developer` / `verifier`）中
`messages_fts_trigram` 虚拟表损坏且 trigram 触发器缺失。命名 profile 使用自己的
`HERMES_HOME`，因此「默认库健康」与「worker 报错」可以并存。

**修复过程**：

1. 先备份三个 profile 的 `state.db`（含 `-wal`/`-shm`）到
   `~/.hermes/backups/profile-state-db-20260913_102255/`；
2. **只做删除**：丢弃 3 个 trigram 触发器 + 从 `sqlite_master` 删除虚拟表条目 + 丢弃影子表；
3. 交由 Hermes 自身重建（打开 SessionDB 即触发 `_ensure_fts_schema`），
   使用**当前代码版本**的定义（外部内容表 + `messages_fts_trigram_src` 视图 + 带 `WHEN` 的触发器）；
4. 端到端校验：插入→命中、更新→旧串失配/新串命中、删除→失配，全部在回滚事务内完成。

**结果**：

```text
[manager]   诊断 OK  轮询测试 PASS (insert=1 old=0 new=1 del=0)
[developer] 诊断 OK  轮询测试 PASS
[verifier]  诊断 OK  轮询测试 PASS
官方自检：✓ All checks passed — Hermes SQLite is healthy
```

修复脚本：`~/.hermes/scripts/repair_profile_state_db.py`（缺省仅诊断，`--apply` 才备份并修复；
幂等，健康库会被跳过）。

**教训**：不要照抄旧版触发器 SQL。旧版定义与当前代码不一致，手写会让真实写入失败，
把「读侧告警」升级为「消息静默丢失」。正确做法是删干净后让 Hermes 重建。

## 3.1 本轮收尾闸门：预先阻塞法（取代看门狗）

用户要求「本轮仅处理 2 张卡（DT1、DT2）后停止」。最终采用**隔离父卡片（quarantine parent）**实现：

- **为何不用直接 `block`**：依赖等待中的 `todo` 卡会被拒绝封锁（实测 `cannot block t_bfdfffc9`，
  `block_kind` 未写入）。直接封锁只对 `ready`/`blocked` 等状态有效。
- **为何不用看门狗**：运行期看门狗在第 2 张完成后才动作，而网关每个派发周期都可能先行把
  第 3 张提升并**启动**，存在最长约 2 分钟的竞态窗口，期间白烧额度。预先阻塞无此窗口。
- **做法**：新建人工闸门卡 `t_dfd31639`（标题以 `QUARANTINE` 开头，`workspace=scratch`，
  状态 `blocked`），再 `link t_dfd31639 t_bfdfffc9`，把它追加为 **DT3 的额外父卡片**。
- **效果**：DT3 的父卡片 = `DT2` + 闸门卡。DT2 完成（`done`）后，DT3 仍因闸门卡未完成而
  不可提升；DT3 是串行链的唯一入口，故 DT3B/DT4/DT5/DT6/VDT1/VDT2/M3/AN…/M5 **全部随之停住**。
  确定性、零模型开销、无竞态、无竞态残留。
- **看门狗已停用**：cron 任务 `d7a4dd6820ad`（`pause-novel-reader-after-2-cards`）已 `pause`
  （保留以便恢复，避免两套机制并存互相干扰）。脚本仍在
  `~/.hermes/scripts/pause-novel-reader-after-n.py`。

解除闸门（额度与优先级确认后，链路自 DT3 起恢复）：

```bash
export HERMES_BIN=$HOME/.hermes/hermes-agent/venv/bin/hermes
export HERMES_HOME=$HOME/.hermes
hermes kanban --board novel-reader-txt-v1 unblock t_dfd31639 --reason "额度与优先级已确认，恢复链路"
hermes kanban --board novel-reader-txt-v1 dispatch --max 1 --json
```

## 3.2 硬时间门限：2026-09-16 23:30 自动挂起（桌面优先）

用户指示：「优先跑桌面端；设定今晚 23:30 的门限，到时间就暂停（挂起、阻塞）」。

### 链路闸门重新布置

| 项 | 状态 |
|---|---|
| 旧闸门 `t_dfd31639`（曾挂 dt3，用于「处理 2 张卡后停」） | 已 `unlink` 并 **archived**（使命完成） |
| 新闸门 `t_c6bbb471`（`GATE-ANDROID`） | 挂到 **`an1`**：`an1 parents = [M3, t_c6bbb471]` |

效果：**桌面链**（dt3 → dt3b → dt4 → dt5 → dt6 → vdt1 → vdt2 → M3）不受阻、可自由推进；
**Android 链**（an1 起，含 an2…an6/an3b/van1/rel1/qa1/M4/M5）在闸门保持 blocked 时不可提升。

### 时间门限机制

- 脚本：`~/.hermes/scripts/pause-novel-reader-at-deadline.py`（门限常量 `DEADLINE = 2026-09-16 23:30:00`）
- cron 任务：**`pause-novel-reader-at-2300`（job id `9e5b795b2fe1`）**，调度 `30 23 16 9 *`，
  **Next run = 2026-09-16T23:30:00+08:00**，`--no-agent`、`deliver local`
- 到点动作：回收在途认领 → 阻塞 `running`/`ready` 卡片 → 写标记 `~/.hermes/kanban/deadline-paused-novel-reader-txt-v1.marker`
  → 输出一行摘要。仅动作一次（幂等）。
- **卡间空隙兜底**：若到点瞬间恰好无 `running`/`ready` 卡，脚本会等一个派发周期（65 秒）重试，
  最多 3 轮，避免漏网。
- 未到点或已动作时 stdout 为空，满足 cron 静默契约（实测 0 字节）。

### 验证记录

| 项 | 结果 |
|---|---|
| 未到点默认运行 | `stdout` 0 字节（静默）✔ |
| 模拟已过点（临时副本 + `--dry-run`） | 正确识别 `ready=[t_bfdfffc9]` 并计划「回收+阻塞」✔ |
| cron 建立 | `Next run 2026-09-16T23:30:00+08:00`（初建为 23:00，后按用户指示顺延至 23:30）✔ |
| 桌面链放行 | DT3 `running`（run#25，pid 1679207，预算 60 分钟，心跳新鲜）✔ |
| Android 链阻断 | `an1 parents=[M3, t_c6bbb471]`，闸门 `blocked` ✔ |

### 固有代价（须知）

到 23:30 时若某卡正在运行，会被**回收终止**：其未提交的中间产物**不计为完成证据**，
恢复时该卡需从头重跑。这是「到点即停」的必然取舍。

### 恢复步骤

```bash
export HERMES_BIN=$HOME/.hermes/hermes-agent/venv/bin/hermes
export HERMES_HOME=$HOME/.hermes
# 1) 解除被门限阻塞的那张卡（list --status blocked 查看）
hermes kanban --board novel-reader-txt-v1 list --status blocked
hermes kanban --board novel-reader-txt-v1 unblock <被阻塞卡id> --reason "额度已恢复，继续桌面链"
# 2) 若要接着跑 Android 段，再解除 Android 闸门
hermes kanban --board novel-reader-txt-v1 unblock t_c6bbb471 --reason "开始 Android 段"
# 3) 派发
hermes kanban --board novel-reader-txt-v1 dispatch --max 1 --json
# 4) 停用本门限任务（如不再需要）
hermes cron remove pause-novel-reader-at-2300
```

## 3.3 第二轮时间门限：2026-09-18 23:00

用户于 2026-09-18 20:40 指示：「继续该 kanban 任务，并设置 23:00 门禁」。

### 上一轮门限的实际结果（已验证）

- 2026-09-16 23:30:45 **如期触发**（`cron runs` 记录 `ff994dddd4c5407aa008c988e7eee634 completed`），
  冻结的卡为 **DT4**（`t_1f76a9a6`）；此后看板 `running=0`、worker 进程 0，**两日零消费**。
- 上一轮本轮（DT3 起）产出：DT3 完成于 21:22:14、DT3B 完成于 21:46:29。

### 本轮设定（复用同一脚本与 cron 任务，只改两处）

| 项 | 值 |
|---|---|
| cron 任务 | `pause-novel-reader-at-2300`（job id `9e5b795b2fe1`）不变 |
| 调度 | `0 23 18 9 *` → **Next run = 2026-09-18T23:00:00+08:00** |
| 脚本门限 | `DEADLINE = dt.datetime(2026, 9, 18, 23, 0, 0)` |
| 静默契约 | 未到点 stdout 0 字节（实测）；过点动作分支用临时副本 `--dry-run` 复测通过 |

### ⚠️ 复用该门限的三个必备步骤（缺一即失效）

1. **改脚本门限常量**（`DEADLINE` 与 `REASON` 文案）——若只改调度不改脚本，
   到点那次触发会因「未到点」静默，而调度已过、门限**永久失效**；
2. **改 cron 调度**——若只改脚本不改调度，旧调度会在旧时刻按**新门限**判断而静默，同样失效；
   反之若新门限早于旧调度，则会在旧时刻**提前挂起**；
3. **归档旧门限标记**——脚本以标记文件短路（幂等，只动作一次）。旧标记不归档，
   新一轮**永远不会动作**。本轮已将 `deadline-paused-novel-reader-txt-v1.marker`
   改名为 `.fired-20260916` 保留审计痕迹（**勿删**）。

### 本轮恢复动作

`unblock t_1f76a9a6`（DT4）→ `dispatch` → **run#31，pid 2847790，预算 60 分钟，心跳 42 秒（新鲜）**；
worker 日志显示其主动核对「源码自 09-16 22:50 后未变动」并重新执行卡片要求的命令取证
（即不采信上一轮中断期的中间产物）。

## 3.4 第三轮时间门限：2026-09-19 15:00

用户于 2026-09-19 10:37 指示：「继续该 kanban 任务，并设置 15:00 门禁」。

### 第二轮门限的结果与后续演进（已验证）

- 2026-09-18 23:00:32 **如期触发**（cron 记录 `1e46d07b0d8447b1b94b6ba55ec1ec8f completed`），
  冻结卡为 **`t_b26927a9` V-DT1B 独立复验（桌面 AC-D4 输入入口）**。
- 09-18 当晚（门限触发前）完成：`dt4`、`dt5`、`dt6`、`vdt1`(22:08)、`vdt2`(22:20)、`M3`(22:31)，
  并新建+完成补救卡 `t_825f122b` **DT8 桌面 AC-D4 输入入口补齐**（22:57）。
- **manager 自我修复链路正常运转**：V-DT1 发现 AC-D4（输入入口）缺口 → manager 建 DT8 补救卡
  → 完成后建 **V-DT1B 独立复验**卡 → 并预设 **M3 复审**卡 `t_afa6a78b`，
  同时把 V-DT1B 追加为 `an1` 的父卡片（Android 链也需等该复验关闭）。

### 本轮设定

| 项 | 值 |
|---|---|
| cron 任务 | `pause-novel-reader-at-2300`（job id `9e5b795b2fe1`）沿用 |
| 调度 | `0 15 19 9 *` → **Next run = 2026-09-19T15:00:00+08:00** |
| 脚本门限 | `DEADLINE = dt.datetime(2026, 9, 19, 15, 0, 0)` |
| 旧标记归档 | `deadline-paused-...marker` → `.marker.fired-20260918`（加上 09-16 那份，共两份审计痕迹） |
| 静默契约 | 未到点 stdout 0 字节；过点分支以临时副本 `--dry-run` 复测通过 |

（复用门限的三必备步骤见 §3.3：改脚本门限 → 改调度 → 归档旧标记。）

### 本轮恢复动作与存活证据

`unblock t_b26927a9` → `dispatch` → **run#40，pid 112546**（verifier）。
本次改用**严格判据**验证存活：要求「心跳事件的 created_at ≥ 本 run 的 started_at」。
实测前 30 秒心跳仍是上一轮的旧值（约 11.6 小时前，`新鲜=False`），
第 32 秒起变为 **6 秒且 `新鲜=True`** 才判定存活 —— 仅凭「有心跳字段」会误报。
日志显示其已读取 `spec.md`/`checklist.md` 与 `vdt1-input-audit.py`，进入独立复验。

### 桌面段剩余范围

`V-DT1B`（复验）→ `t_afa6a78b`（M3 复审）。这两张完成后**桌面段即收尾**；
Android 段仍被 `t_c6bbb471` 闸门阻断，`vdt3`/`van2` 仍被 `ENV1` 阻断。

## 3.5 Android 闸门放行（仍排在桌面段之后）— 2026-09-19 10:42

用户指示：「解除 android 端，但排在 桌面端 之后」。

### 发现并处理的一处依赖缺陷

只解除闸门**不足以**满足「排在桌面段之后」：`V-DT1B` 的子卡片同时是
`an1`（Android 入口）与 `t_afa6a78b`（M3 复审，桌面段最后一张），
两者原本**互不依赖**。V-DT1B 一完成，二者会**同时变为可运行**，
Android 段可能抢先或在桌面复审之前启动。

处理：先把 `t_afa6a78b`（M3 复审）**追加为 `an1` 的父卡片**，再开闸。

### 放行方式

闸门卡 `t_c6bbb471` 不是工作卡，若用 `unblock` 会使其变为 `ready` 并被派发执行（错误）。
故改用 **`complete`** 表达「已放行」：作为父卡片即视为满足，且不会被执行；
并追加评论记录放行原因与依赖调整。

### 变更后的依赖快照（已验证）

```text
V-DT1B (running)
  └── M3复审 t_afa6a78b (todo)
        └── an1 t_d1c13c73 (todo)
              └── an2 → an3 → an3b → an4 → an5 → an6 → van1 → rel1 → qa1 → M4 → M5

an1 父卡片 = [M3(done), Android闸门 t_c6bbb471(done), V-DT1B(running), M3复审(todo)]
```

⇒ `an1` 必须等 **V-DT1B 与 M3 复审全部完成** 才可提升，即「Android 严格排在桌面段之后」；
校验时「当前可提升的卡片集合」为空，符合预期。

### 门限状态

15:00 硬门限仍处于 armed 状态（`Next run 2026-09-19T15:00:00+08:00`，`已动作=False`）。
预期：桌面段（V-DT1B → M3 复审）约 11:30 前收尾 → Android 链自动接续运行 →
**15:00 门限到点挂起**（按 Android 每张 30–40 分钟估算，届时大致推进到 `an3`/`an4` 一带）。

## 4. 恢复步骤

```bash
export HERMES_BIN=$HOME/.hermes/hermes-agent/venv/bin/hermes
export HERMES_HOME=$HOME/.hermes

# 额度恢复后，按需分别解除封锁
hermes kanban --board novel-reader-txt-v1 unblock t_cbd977c2 --reason "额度已恢复，继续 DT1"
hermes kanban --board data-ingestion unblock t_3af51cce --reason "额度已恢复"
hermes kanban --board data-ingestion unblock t_e4d34d76 --reason "额度已恢复"

# 逐个派发（跨看板并行会成倍消耗额度，建议一次只放一个看板）
hermes kanban --board novel-reader-txt-v1 dispatch --max 1 --json

# 核验存活：status=running + 单一 worker PID + 新鲜心跳 + 日志出现真实文件/命令动作
hermes kanban --board novel-reader-txt-v1 show t_cbd977c2
hermes kanban --board novel-reader-txt-v1 log  t_cbd977c2 --tail 4000
```

`ENV1` 应继续保持封锁，直到具备 Android 真机 / Windows / Wayland 环境。

## 5. 恢复前需知的残留风险

1. **worker profile 技能解析**：`novel-reader-ui-env` 已通过符号链接接入
   `manager`/`developer`/`verifier` 三个 profile。校验：
   `hermes -p developer skills list | grep -c novel-reader-ui-env`（期望 `1`）。
   若 profile 被重建或技能目录被覆盖，需重新链接，否则携带该 `--skill` 的 18 张卡会以
   `Error: Unknown skill(s)` 崩溃并熔断。
2. **网关进程**：`hermes-gateway.service` 自 2026-09-12 13:09 起持续运行。本次修复为
   DB 级修复，新的 worker 进程会正常；若日后仍有 `Session store unavailable`，再考虑重启网关
   （须在无 worker 运行时执行）。
3. **跨看板并行会成倍消耗额度**：`max_in_progress` 按看板生效，两个看板同时运行 = 2 个 worker 并行。
