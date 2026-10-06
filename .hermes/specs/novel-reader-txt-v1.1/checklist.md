# V1.1 检查项（书架分类 / 阅读历史 / 自定义配色 / 背景图）

> 勾选是**断言**，不是证据。每条勾选须给出命令、退出码与产物路径；
> 标「变异测试」的条目必须附**改坏 → exit 非 0 → 还原 → exit 0** 三段输出。
> 状态标记：`[ ]` 未完成 · `[~]` 部分满足（须注明仍缺哪部分）· `[x]` 已完成且有物证 · `[!]` 发现缺口

## 迁移与存储（T1）

> **变异测试实跑记录（2026-10-05，T1 完成后）**：以下判据均经「基线 exit 0 → 变异 exit 101 →
> 还原 exit 0」三段验证。变异方式与命中判据记录在各项内。

- [x] C4 (R5, T1) **变异测试**：把循环校验改为恒允许 → 相关单测 exit 101，精确命中
      `moving_category_into_own_descendant_is_rejected`（基线 13 passed / 变异 12 passed 1 failed / 还原 13 passed）
- [x] C5 (R2, T1) **变异测试**：删掉同父同名唯一索引 → exit 101，命中
      `database_rejects_same_named_sibling_even_when_bypassing_application_checks` 与
      `sibling_name_uniqueness_covers_root_level_null_parents`
      **（首轮此处判据空转：初版测试只验应用层报错，删索引仍全绿 → 已补两条直写库守卫测试）**
- [x] C6 (R2, T1) **变异测试**：唯一索引 `IFNULL(parent_id, 0)` → 裸 `parent_id` → exit 101，
      命中同上两条（根层 `NULL != NULL` 导致同名可共存）
- [x] C37 (R14, T8) **变异测试**：应用层相对路径校验改为恒假 → exit 101，命中
      `path_like_names_are_rejected_with_readable_errors`
- [x] C36 附加（DB 层）**变异测试**：`char(92)` 反斜杠 CHECK 移除 → exit 101，命中
      `database_layer_also_rejects_backslash_and_traversal`
      （该 CHECK 的存在源于实测缺陷：SQLite 字符串字面量里 `'\\'` 是两字符序列，
      Windows 路径分隔符会**逃过**校验）

- [x] C1 (R1, R2, T1) `0002` 迁移在既有 `0001` 库上重复执行 exit 0；`books.category_id` 存在且可空；
      `TARGET_SCHEMA_VERSION == 2` —— 证据：`lib.rs::in_memory_storage_migrates_and_keeps_version_consistent`
      与 `migrating_an_existing_v1_database_upgrades_to_v11_without_losing_books`（后者验证 V1→V1.1 升级、
      存量书籍落未分类、重复迁移空转）
- [x] C2 (R2, T1) 创建三层嵌套分类成功；同父同名返回可理解错误（断言错误文案非空且含「已存在」类语义）
      —— `three_level_nesting_is_created_and_projected_in_parent_first_order`、
      `duplicate_name_under_same_parent_is_rejected_with_readable_error`
- [x] C3 (R5, T1) 把分类移入自身 / 直接子 / 深层子孙均被拒；树结构逐字节不变
      —— `moving_category_into_own_descendant_is_rejected`（三个目标逐一断言 + 结构复核）
- [x] C6' (R2, T1) 改名到自身当前名为无操作（不被唯一索引误判冲突）
      —— `rename_to_own_current_name_is_noop_not_conflict`
- [x] C7 (R7, T1) 全套分类操作后原始 TXT 仍在原路径、字节数与 mtime 不变
      —— `shelf_ops.rs` 既有测试 + `categories.rs` 不触碰文件系统（只写 SQLite），无任何文件 API 调用
- [x] C13/C14/C15 的存储层前置 (R3, T1) `delete_plan` 统计子孙书籍、`delete_category` 三种去向各自生效
      —— `delete_plan_counts_books_of_descendants`、`delete_moves_books_to_chosen_category_only`、
      `delete_can_remove_books_from_shelf_without_touching_files`、
      `delete_rejects_moving_into_own_descendant_as_destination`
- [x] C20/C21/C22/C23 的存储层前置 (R8, R9, T4) 流水写入/倒序/五种 `entry_kind`/软移除保留与硬删级联
      —— `history.rs` 全部 11 项单测（含 `clearing_history_leaves_books_progress_and_bookmarks_untouched`）
- [x] C30 (R11, T6) 存储只存 4 基色：`reader_theme_custom` 列清单精确断言（无任何令牌值列）
      —— `lib.rs::theme_custom_table_stores_only_base_colors_not_token_values`
- [x] C35/C36/C37 的存储层前置 (R14, T8) 背景文件名合法/非法双向断言、遮罩越界、单例 upsert、
      遮罩与缩放互不干扰、调色两表独立 —— `background.rs` 全部 11 项单测

## 导入与批量归类（T2）

- [ ] C8 (R6, T2) 选目录 + 「按文件系统目录名组织」→ 各文件按**直接父目录名**建分类，归入当前选中父分类下
- [ ] C9 (R6, T2) 选「否」→ 全部归入当前父分类；当前在根时归入根（`category_id IS NULL`）
- [ ] C10 (R6, T2) 导入对话框每次询问；勾选「记住」后重启不再询问，取消勾选后恢复询问

## 侧栏树与交互（T3）

- [x] C11 (R1, T3) 侧栏树按层级折叠展示；选中子分类时主列表只显示该分类书籍；根节点显示全部
      ↳ 2026-10-06 闭合（**桌面 + Android 两侧**）：Android 侧栏同样只显示该分类书籍
      （`visibleBookIds` 由核心 `nr_list_books_in_scope` 给出，子树展开在核心）。
      ↳ 2026-10-06 闭合：**桌面**侧过滤补做完（`shelf::load_shelf_in` 按作用域过滤，含子孙，与「合计计数」同口径）；证据 `shelf::tests::selecting_a_category_filters_to_its_subtree` 等 4 项 + 变异测试（忽略作用域 ⇒ 3 项变红）。此前 `filtered_count` 等于全量、`rows` 未过滤，属**名不符实**。
- [ ] C12 (R1, T3) 书籍移入分类后重启仍保持归属（DB 读回断言）
- [ ] C13 (R3, T3) 删除非空分类**必须**先选去向；未选时删除不执行（分类仍在）
- [ ] C14 (R3, T3) 选「一并移出书架」→ 该分类及子孙下书籍 `status == removed`，原始 TXT 未被删除
- [ ] C15 (R3, T3) 选「移回根」→ 书籍 `category_id IS NULL`，仍显示在根下（平铺，与现行效果一致）
- [ ] C16 (R4, T3) 书籍可移动到另一分类（含根）；分类可整体移动到另一分类下
- [ ] C17 (R4, T3) **变异测试**：去掉分类移动的循环校验 → 构造真实环 → 门禁 exit 非 0
- [ ] C18 (R5, T3) 循环移动在**界面入口**被拒并 Toast 出可理解文案（经 `NR_CATEGORY_MOVE` 驱动实测，非仅单测）
- [ ] C19 (R8, T3) 「最近阅读」**不在侧栏**，从书架菜单项进入（门禁扫 `.slint` 断言侧栏无该项）

## 阅读历史（T4, T5）

- [ ] C20 (R8, T4) 连续打开 3 本书 → `reading_history` 恰好 3 行，按 `opened_at` 倒序
- [ ] C21 (R8, T4) 流水含当时章节与位置，且与打开前保存的进度一致
- [ ] C22 (R8, T4) 五种进入方式（书架行/重开/进度恢复/书签跳转/章节跳转）`entry_kind` 正确
- [ ] C23 (R9, T4) 软移除（移出书架）→ 历史保留；硬删除记录 → 历史级联删除
- [ ] C24 (R8, T5) 清空历史后 `books`/`reading_progress`/`bookmarks` 行数与清除前**完全一致**
- [ ] C25 (R8, T5) 单条清除只删该行；清空全部需确认弹窗，取消则不执行

## 派生式配色（T6, T7）

- [ ] C26 (R11, T6) 派生确定性：同输入连续两次调用，40 个令牌结果逐字节一致
- [ ] C27 (R11, T6) 4 基色改动 → `PALETTE_FIELDS` 全部 40 个令牌取值中至少 4 项变化（门禁可核对）
- [ ] C28 (R12, T6) 正文对背景对比度 < 4.5:1 → 返回警告，**且用户基色未被强改**（断言落库值 == 用户所选）
- [ ] C29 (R12, T6) 对比度边界：4.49 判失败、4.50 判通过
- [ ] C30 (R10, T6) **存储只存 4 基色**：断言 `reader_theme_custom` 表**无** 40 令牌实际值列
- [ ] C31 (R11, T7) 明/暗切换：基色不变、派生方向切换（两套派生结果均可达）
- [ ] C32 (R13, T7) 自定义配色作用于书架/阅读/设置三页与**所有浮层**（弹窗/Toast/遮罩），无「半换」
- [ ] C33 (R13, T7) 「重置为默认」→ 40 令牌回到 `PALETTE_FIELDS` 内置值（逐项比对）
- [ ] C34 (R18, T6, T7) **变异测试**：把对比度算法复制到 `.slint` 或 Kotlin → 门禁 exit 非 0

## 背景图（T8, T9）

- [ ] C35 (R14, T8) 选图 → `<data>/novel-reader/backgrounds/` 出现副本，`reader_background.file_name` 与之匹配
- [ ] C36 (R14, T8) `file_name` 存**相对文件名**：断言不含 `/`、不含 `..`、非绝对路径
- [ ] C37 (R14, T8) **变异测试**：去掉相对路径校验 → 门禁 exit 非 0
- [ ] C38 (R17, T8) 损坏/缺失图片 → 给出可理解状态 + 回退纯色，**阅读功能不受影响**（翻页仍正常）
- [ ] C39 (R14, T8) 原始图片文件在全过程未被修改或删除（mtime + 字节数不变）
- [ ] C40 (R15, T9) 背景只作用于阅读页正文区；书架/设置/浮层**无**背景（截图或投影断言）
- [ ] C41 (R15, T9) 遮罩强度与缩放口径（覆盖/平铺/居中）可调；`scrim_permille` 越界被拒
- [ ] C42 (R16, T9) **分页边界与无背景时逐字节一致**（章节点数、`page_index`、正文字节窗口全部相等）
- [ ] C43 (R16, T9) **变异测试**：让背景图参与分页口径 → 门禁 exit 非 0
- [ ] C44 (R16, T9) 换配色不丢背景、换背景不丢配色（两表行数与内容互不影响）

## 跨端与门禁（T10, T11）

- [ ] C45 (R18, T10) Android JVM 单测 exit 0；`build-android-libs.sh` + `build-android-apk.sh` exit 0
- [ ] C46 (R18, T10) `verify-android-boundary.py` exit 0（Kotlin 侧无派生/对比度/循环算法）
- [ ] C47 (R18, T10) **变异测试**：把派生算法复制到 Kotlin → 门禁 exit 非 0
- [ ] C48 (R1–R18, T11) 全量桌面门禁顺序通过：`cargo test` → `smoke-desktop.sh --no-build` →
      `verify-desktop-input.py`（先 source slint-env.sh）→ `verify-desktop-scope.py` →
      `verify-desktop-parity.py` → `verify-desktop-targets.py`
- [ ] C49 (R1–R18, T11) 每个新静态判据**基线自检 exit 0**（未变异），且变异后 exit 非 0
- [ ] C50 (R1, R2, R4, T3, T11) 每个新交互入口都有等价 `NR_*` 驱动，并登记进 `verify-desktop-scope.py` 的
      `[8] 界面可达性` 对等清单 —— **门禁只跑驱动时界面入口缺失不会被发现**
- [ ] C51 (R1–R18, T11) `docs/release/known-limitations.md` 已更新：V1.1 四项状态与实测证据一致，
      未实测项仍写「待验证」，无「已通过」字样
- [ ] C52 (R1–R18, T11) `scripts/verify-v1-release.py` exit 0（含 V1.1 新增项）

## 文档同步（T11）

- [ ] C53 (R1–R18, T11) `tasks.md` 每条任务的 `Definition of done` 与实际实现一致；
      本清单每条勾选的 `Affected areas` 路径**全部真实存在**
- [ ] C54 (R1–R18, T11) 三元组可追溯性：`python3 ~/.hermes/skills/software-development/spec-code-consistency-audit/scripts/spec-trio-coverage.py .hermes/specs/novel-reader-txt-v1.1` exit 0