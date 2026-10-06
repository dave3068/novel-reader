# V1.1 任务分解（书架分类 / 阅读历史 / 自定义配色 / 背景图）

> 依据：同目录 `spec.md`（R1–R18）。每期结束须跑对应门禁并留证据。
> 门禁顺序（延续 V1 口径）：
> `cargo test --manifest-path crates/desktop-app/Cargo.toml` →
> `smoke-desktop.sh --no-build` → `verify-desktop-input.py`（**需先 source slint-env.sh**）→
> `verify-desktop-scope.py` → `verify-desktop-parity.py` → `verify-desktop-targets.py`

## P1 — 分类树（R1–R7）

- [ ] T1 — 迁移与存储层
  - Requirements: R1, R2, R5, R7
  - Affected areas: `migrations/0002_*.sql`, `crates/reading-storage/src/migrate.rs`,
    `crates/reading-storage/src/categories.rs`, `crates/reading-storage/src/books.rs`,
    `crates/reading-storage/src/error.rs`
  - Depends on: 无
  - Definition of done:
    - `migrations/0002_categories_history_theme.sql` 建立 `book_categories`（邻接表 `parent_id`）、
      同父同名唯一索引、`books.category_id`（可空，根=NULL）；
    - `migrate.rs` `MIGRATIONS` 增加该条、`TARGET_SCHEMA_VERSION` 1→2；既有 0001 库重复执行不报错；
    - `categories.rs` 提供 create / rename / delete（含子孙书籍去向）/ move（书籍与分类）/ list_tree；
    - **循环防护在应用层**：`move` 拒绝把分类移入自身子孙（沿父链上溯，含自身）；
    - 同父同名返回可理解错误，不静默合并；
    - 单测：三层嵌套、循环拒绝（同父/深层/自身）、同父同名拒绝、删除带去向、幂等迁移。

- [ ] T2 — 导入与批量归类
  - Requirements: R6
  - Affected areas: `crates/desktop-app/src/import.rs`（`FilePicker` 扩 `pick_txt_files()` /
    `pick_directory()`）、`crates/desktop-app/src/ui.rs`（导入对话框 + 「按目录名组织」询问）、
    `desktop/ui/app.slint`（对话框行）
  - Depends on: T1
  - Definition of done:
    - `FilePicker` trait 扩多选文件与选目录；Linux 走 zenity `--multiple` / `--directory`，
      Windows 走原生多选（`IFileOpenDialog`，注意 `package-windows.ps1` 已验证口径）；
    - 导入前弹窗询问「是否按文件系统目录名组织归类」，可勾选「记住」（持久化到 settings）；
    - 选「是」：按每个文件的**直接父目录名**建分类（同名同父则复用），归入**当前选中父分类**下；
    - 选「否」：全部归入当前选中父分类（或根）。

- [ ] T3 — 桌面侧栏树 UI 与交互
  - Requirements: R1, R2, R3, R4, R5
  - Affected areas: `desktop/ui/app.slint`（新增 `CategoryTree` 组件、`UiMetrics` 增侧栏宽常量、
    归类/删除确认浮层）、`crates/desktop-app/src/layout.rs`（侧栏宽常量）、
    `crates/desktop-app/src/app_state.rs`（`ShelfView` 增树投影）、`crates/desktop-app/src/shelf.rs`（按分类过滤）、
    `crates/desktop-app/src/shelf_ops.rs`（分类动作）、`crates/desktop-app/src/ui.rs`（投影 + 回调）
  - Depends on: T1, T2
  - Definition of done:
    - 侧栏树支持折叠/展开、选中过滤；根节点=全部书籍，其下「未分类」入口显示 `category_id IS NULL` 的书；
    - **「最近阅读」不在侧栏**，改为书架菜单项进入（与「设置」「添加 TXT」同级）；
    - 行级菜单增「移动到分类…」；分类行菜单含新建子分类 / 重命名 / 移动 / 删除；
    - 删除非空分类弹出**去向选择**（移入另一分类 / 一并移出书架 / 移回根），未选不执行；
    - 循环移动被拒并 Toast 出可理解文案；
    - 新增 `NR_*` 程序化驱动（至少：`NR_CATEGORY_ADD` / `NR_CATEGORY_MOVE` / `NR_SHELF_OPEN_CATEGORY`），
      并写入 `verify-desktop-scope.py` 的 `[8] 界面可达性` 对等清单 —— **门禁只跑驱动时，
      界面入口缺失不会被发现**，每个新入口都要配等价驱动。

## P2 — 阅读历史（R8, R9）

- [ ] T4 — 历史存储与写入
  - Requirements: R8, R9
  - Affected areas: `migrations/0002_*.sql`（`reading_history` 表）、
    `crates/reading-storage/src/history.rs`
  - Depends on: T1（同一迁移文件）
  - Definition of done:
    - `history.rs` 提供 record / list（时间倒序，可按书聚合）/ remove_one / clear_all / remove_by_book；
    - 打开书籍的**所有入口**（书架行、阅读页重开、进度恢复、书签跳转、章节跳转）都记流水并标 `entry_kind`；
    - 硬删除书籍级联删历史；软移除保留历史；单测覆盖三条生命周期。

- [ ] T5 — 历史界面
  - Requirements: R8
  - Affected areas: `desktop/ui/app.slint`（历史面板）、`crates/desktop-app/src/app_state.rs`、
    `crates/desktop-app/src/ui.rs`、`crates/desktop-app/src/shelf_ops.rs`
  - Depends on: T4
  - Definition of done:
    - 书架菜单项「最近阅读」进入历史视图：时间倒序、可按书聚合、单条清除、清空全部（**需确认**）；
    - 清空后 `books`/`reading_progress`/`bookmarks` 行数不变（断言）；
    - 新增 `NR_HISTORY_VIEW` / `NR_HISTORY_CLEAR` 驱动并入对等性清单。

## P3 — 派生式自定义配色（R10–R13）

- [ ] T6 — 派生算法与对比度校验
  - Requirements: R10, R11, R12
  - Affected areas: `crates/reading-storage/src/theme.rs`（存储）、
    `crates/desktop-app/src/layout.rs`（派生 + 对比度，纯函数）
  - Depends on: T1（同一迁移文件）
  - Definition of done:
    - `theme.rs` 单例存 4 基色 + `enabled` + `derived_dark`，**不存 40 令牌实际值**；
    - 派生函数由基色**确定性**产出全部 `PALETTE_FIELDS`（40 个，明/暗两套）；
    - 对比度函数算正文对背景比值，< 4.5:1 返回警告但**不改用户基色**；
    - 单测：派生确定性（同输入两次调用结果逐字节一致）、40 令牌全覆盖、对比度边界（4.49 / 4.5）。

- [ ] T7 — 配色设置界面
  - Requirements: R10, R12, R13
  - Affected areas: `crates/desktop-app/src/settings.rs`（新增「自定义配色」组）、
    `desktop/ui/app.slint`（取色器行）、`crates/desktop-app/src/ui.rs`（`apply_palette` 接入自定义层）
  - Depends on: T6
  - Definition of done:
    - 设置页新增 4 个基色选取 + 「重置为默认」+ 启用开关；
    - `apply_palette` 在内置明/暗之上叠加自定义派生（**单一注入口**，不改 `.slint` 令牌声明）；
    - 自定义配色作用于书架/阅读/设置三页与**所有浮层**（弹窗/Toast/遮罩），无「半换」现象；
    - 明/暗切换时基色不变、派生方向切换；
    - 新增 `NR_THEME_SET` 驱动并入对等性清单。

## P4 — 背景图（R14–R17）

- [ ] T8 — 背景存储与图片加载
  - Requirements: R14, R17
  - Affected areas: `crates/reading-storage/src/background.rs`、
    `crates/desktop-app/src/import.rs`（`FilePicker` 扩 `pick_image_file()`）、
    `crates/desktop-app/src/ui.rs`（运行时图片解码）
  - Depends on: T1（同一迁移文件）
  - Definition of done:
    - `background.rs` 单例存 `file_name`（**相对文件名**，拒绝含 `/`、`..`、绝对路径）、`file_size`、
      `scrim_permille`、`fit_mode`；
    - 选图后**复制**到 `<data>/novel-reader/backgrounds/`，原图永不修改；
    - 运行时解码（`image` crate 或 Slint `SharedPixelBuffer`）；**读取路径必须带 `scope-allow:` 注释**
      说明读的是用户图片而非书籍正文（否则撞 `UNBOUNDED_READ_TOKENS`）；
    - 加载失败：给出可理解状态 + 回退纯色，阅读不受影响；单测覆盖损坏图片。

- [ ] T9 — 背景渲染与设置
  - Requirements: R15, R16
  - Affected areas: `desktop/ui/app.slint`（阅读区背景层 + 遮罩）、`crates/desktop-app/src/settings.rs`、
    `crates/desktop-app/src/ui.rs`
  - Depends on: T8
  - Definition of done:
    - 背景只作用于**阅读页正文区**，叠加遮罩（默认 850‰）；
    - 遮罩强度、缩放口径（覆盖/平铺/居中）、移除背景均可调；
    - **分页边界与无背景时逐字节一致**（守门测试 + 变异测试）；
    - 与自定义配色互相独立（换配色不丢背景，反之亦然）；
    - 新增 `NR_BACKGROUND_SET` 驱动并入对等性清单。

## P5 — 跨端与门禁收口（R18）

- [ ] T10 — Android 同语义入口
  - Requirements: R18
  - Affected areas: `android/app/src/main/java/com/novelreader/android/ui/`（`CategoryTree.kt`、
    `HistoryScreen.kt`、`ThemePicker.kt`、`ShelfScreen.kt`）、`android/jni/`、`include/novel_reader.h`
  - Depends on: T1, T4, T6, T8
  - Definition of done:
    - Kotlin 侧调用同一 Rust API；**不含**派生算法 / 对比度算法 / 循环校验（`verify-android-boundary.py` 有牙）；
    - JVM 单测 + `build-android-libs.sh` + `build-android-apk.sh` exit 0；
    - 真机交互仍按 `known-limitations.md` D1 记为待验证，**不得写成已通过**。

- [ ] T11 — 门禁与文档收口
  - Requirements: R1, R2, R3, R4, R5, R6, R7, R8, R9, R10, R11, R12, R13, R14, R15, R16, R17, R18
  - Affected areas: `scripts/verify-desktop-scope.py`、`scripts/verify-desktop-parity.py`、
    `scripts/verify-v1-release.py`、`docs/release/known-limitations.md`
  - Depends on: T3, T5, T7, T9, T10
  - Definition of done:
    - 每个新静态判据都配**变异测试**（改坏源码 → 必须 exit 非 0；还原后 exit 0）；
    - 新判据先跑**基线自检**（未变异时 exit 0）—— 编译失败不得读作「测试有牙」；
    - `known-limitations.md` 的 L5（未含全文搜索等）按实际更新，V1.1 四项从「延期」改为已实现或保留待验证；
    - `.slint` 语义扫描增「分类/配色派生」类禁词（按技能口径：`.slint` 不得出现阅读语义标记，
      语义一律由 Rust 投影）。