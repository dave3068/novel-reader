# novel-reader V1.1 规格：书架分类、阅读历史、自定义配色、背景图

> 状态：**待主人审阅**（草案）。本文只写规格，不含实现。
> 依据：`docs/novel-reader-solution.md`（原方案承诺）、`docs/novel-reader-v1-development-plan.md`（V1 延期决定）、
> `.hermes/specs/novel-reader-txt-v1/spec.md:26`（范围外声明）、主人口头裁定（2026-10-05）。
> 基线版本：`novel-reader` v0.3.0（`git 42203f9`）。

---

## 1. 背景与主人裁定

V1 已把以下四项明确排除：

| 项 | 排除出处 | 原文口径 |
|---|---|---|
| 书架分类树 | `spec.md:26`、`desktop/spec.md:22` | 「V1 的背景图片、每本书独立阅读设置和**多级分类树**」 |
| 背景图片 | 开发计划:114 | 「V1 不要求实现背景图片……移至 V1.1 或后续版本」 |
| 阅读历史 | `solution.md:23` 承诺、V1 未落 | 原方案列为独立能力，V1 只做了 `last_opened_at` |
| 派生自定义配色 | `solution.md:25` 承诺、V1 降级 | 原方案「支持派生自定义配色」，V1 只做明/暗二态 |

主人裁定（2026-10-05）：

1. **四项工作列入 V1.1 实施**（本期范围）。
2. **背景图存储**：用户图片复制到应用数据目录，DB 只存相对路径；原图永不修改/删除。
3. **自定义配色**：**派生式**（用户调少数基色，Rust 派生其余令牌，保证对比度）。
4. **分类树**：**多层树**；批量导入时**只按文件直接父目录名建一个分类**（不递归建整条路径）；
   导入落到**当前书架所处父分类**下；用户可在书架中移动书籍或分类到另一个分类下。
5. **阅读历史**：每次打开记一条流水（书名 / 时间 / 当时章节与位置），可查看、可清除，**不统计时长**。

---

## 2. 范围

### 范围内

- `migrations/0002_*.sql` 新增分类与历史表（**只新增，不改写 `0001`**）。
- 分类树：创建 / 重命名 / 删除 / 移动（书籍与分类）、循环移动防护、侧栏树投影。
- 批量导入：多选文件 / 选目录导入，导入前询问是否按文件系统目录名组织归类。
- 阅读历史流水：写入、列表（可按书聚合）、清除（单条 / 全部）。
- 派生式自定义配色：基色选取 → Rust 派生 40 个语义令牌 → 对比度校验。
- 背景图：选取本地图片 → 复制到应用数据目录 → 阅读区衬底 + 弱化遮罩；字号/窗口变化时不影响分页口径。
- 桌面端与 Android 端**语义一致**（同一 Rust 核心 API），两端 UI 各自平台化。

### 范围外（本期不做）

- ZIP / EPUB / MOBI / PDF（仍只 TXT）。
- 自动关联文件系统目录树（分类与真实文件夹**无任何绑定**）。
- 每本书独立阅读设置（仍全局；分类可作为将来的覆盖作用域，但本期不实现）。
- 精确阅读时长统计（主人已裁定不做）。
- 正文全文搜索、连续滚动。
- 背景图的每本书独立配置（本期背景图为**全局**一张）。
- 分类的合并（两个分类合成一个）与批量重分类。

---

## 3. 数据模型（`migrations/0002_*.sql`）

沿用 `0001_init.sql` 的硬约束：**只存元数据，不存正文**；ID/偏移用 64 位；DDL 幂等；不放 PRAGMA。

### 3.1 分类表 `book_categories`

```sql
-- 分类（多级树；parent_id = NULL 表示根下分类）。
-- 单书归属：books.category_id（可空 = 根）。
CREATE TABLE IF NOT EXISTS book_categories (
    id           INTEGER PRIMARY KEY,
    parent_id    INTEGER REFERENCES book_categories (id) ON DELETE RESTRICT,
    name         TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- 同父下同名即冲突（唯一索引在下方单独创建，便于幂等重跑）。
    sort_index   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_categories_sibling_name
    ON book_categories (IFNULL(parent_id, 0), name);

-- 层级自防护：1=根。防止写入把分类挂到自己的子孙下（配合应用层校验）。
ALTER TABLE books ADD COLUMN category_id INTEGER
    REFERENCES book_categories (id) ON DELETE SET NULL;
```

> **设计取舍（须主人确认，见 §9 开放问题 Q1）**：`parent_id` 用邻接表而非「路径物化字段」。
> 邻接表的循环防护必须由应用层做（SQL 触发器跨平台不可移植），已在 AC-V11-05 强制要求。

### 3.2 阅读历史表 `reading_history`

```sql
-- 每次打开书籍记一条流水（主人裁定：不做时长统计）。
-- book_id 不设级联删除以外的约束：硬删除书籍时历史一并删除（见 §4.2）。
CREATE TABLE IF NOT EXISTS reading_history (
    id             INTEGER PRIMARY KEY,
    book_id        INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    -- 冗余书名快照：书籍被改名/删除后历史仍可读（只读展示，不参与业务）。
    book_title     TEXT    NOT NULL,
    opened_at      INTEGER NOT NULL,
    chapter_index  INTEGER NOT NULL DEFAULT 0 CHECK (chapter_index >= 0),
    byte_offset    INTEGER NOT NULL DEFAULT 0 CHECK (byte_offset >= 0),
    -- 来源：0=阅读页打开 1=进度恢复 2=书签跳转 3=章节跳转
    entry_kind     INTEGER NOT NULL DEFAULT 0 CHECK (entry_kind BETWEEN 0 AND 3),
    source_kind    TEXT    NOT NULL DEFAULT 'desktop_path'
                    CHECK (source_kind IN ('desktop_path', 'android_uri', 'private_copy')),
    source_locator TEXT    NOT NULL,
    content_index_version INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_history_opened_at ON reading_history (opened_at DESC);
CREATE INDEX IF NOT EXISTS idx_history_book      ON reading_history (book_id, opened_at DESC);
```

### 3.3 背景图与配色

```sql
-- 背景图引用（全局一张；只存应用数据目录内的相对路径）。
CREATE TABLE IF NOT EXISTS reader_background (
    id             INTEGER PRIMARY KEY CHECK (id = 1),  -- 单例
    file_name      TEXT    NOT NULL,     -- 相对 <data>/backgrounds/ 的文件名
    file_size      INTEGER NOT NULL CHECK (file_size >= 0),
    -- 弱化遮罩不透明度千分比（0..1000）；衬底下的正文可读性由 Rust 校验。
    scrim_permille INTEGER NOT NULL DEFAULT 850 CHECK (scrim_permille BETWEEN 0 AND 1000),
    -- 图片自身的缩放口径：0=覆盖铺满 1=按原始尺寸平铺 2=按原始尺寸居中
    fit_mode       INTEGER NOT NULL DEFAULT 0 CHECK (fit_mode IN (0, 1, 2)),
    updated_at     INTEGER NOT NULL
);

-- 派生式自定义配色（全局一套）。
CREATE TABLE IF NOT EXISTS reader_theme_custom (
    id             INTEGER PRIMARY KEY CHECK (id = 1),  -- 单例
    enabled        INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    -- 基色（HEX，#RRGGBB，Rust 侧校验/规范化后入库；用户未改动时为 NULL）。
    base_foreground TEXT,
    base_background TEXT,
    base_primary    TEXT,
    base_accent     TEXT,
    -- 派生参数（Rust 唯一口径；持久化便于「重置为默认」与门禁可核对）。
    derived_dark    INTEGER NOT NULL DEFAULT 0 CHECK (derived_dark IN (0, 1)),
    -- 与 settings 表的 key 对齐（见 §4.4）。
    updated_at      INTEGER NOT NULL
);
```

> **要点**：配色**不存 40 个令牌的实际值**，只存 4 个基色 + 派生开关；实际值每次由 Rust
> 从基色确定性派生。好处：主人日后调亮/暗切换时，明暗两套都由同一基色派生，**不会出现
> 「亮色改了暗色没改」的半改状态**。本版本**不要求用户分别设定明色与暗色**（Q3 已裁定）。

### 3.4 侧栏虚拟节点与最近阅读入口（Q5 已裁定）

- **根 / 未分类**：侧栏根节点即「全部书籍」，其下第一项为「未分类」（`books.category_id IS NULL` 的书）。
  未归类书籍**平铺显示在根下**（与 V1 现行效果一致），**不单独设「未分类」虚拟分类**——
  它只是根节点下的一个过滤入口，删除它等同于回到根。
- **最近阅读**：**不放在侧栏**，而是从书架的**另一个菜单项**进入（与「设置」「添加 TXT」同级）。
  这样侧栏只承载真实分类，最近阅读不与分类树混在同一层级。

---

## 4. 需求

### R1 — 分类树展示

系统 SHALL 在书架侧栏展示分类树（根下分类 → 子分类，可任意嵌套），支持折叠/展开与选中过滤。

#### Scenario: 侧栏树与过滤
- **GIVEN** 存在分类「科幻」，其下有子分类「中国科幻」
- **WHEN** 用户在侧栏选中「中国科幻」
- **THEN** 主列表只显示该分类下的书籍；根节点（未选分类）显示全部书籍

### R2 — 分类维护

用户 SHALL 能创建分类（指定父分类，默认当前选中分类）、重命名分类、删除分类；分类名在**同一父下唯一**，同父同名时给出可理解错误，不静默合并、不静默重命名。

### R3 — 删除非空分类时选择书籍去向

删除分类时系统 SHALL 要求用户显式选择其下书籍（含子孙分类书籍）的去向：**移入另一分类 / 一并移出书架 / 移回根（未分类）**；未选择时 SHALL NOT 执行删除。系统 SHALL NOT 隐式把书籍丢弃给用户。

### R4 — 移动书籍与分类

用户 SHALL 能把书籍移动到另一个分类（含根/未分类），也能把**分类整体移动**到另一个分类之下。

### R5 — 分类循环移动防护

移动分类 SHALL 拒绝把分类移入其自身子孙（否则产生环 → 树投影死循环）；错误须可理解，树结构保持不变。

### R6 — 批量导入按目录归类

批量导入时系统 SHALL 在导入前询问「是否按文件系统目录名组织归类」；选「是」时**只取文件的直接父目录名**建一个分类，并落入**当前书架所处父分类**下；选「否」时归入当前父分类（或根）。

#### Scenario: 批量导入按目录归类
- **GIVEN** 书架当前处于分类「科幻」下，用户选目录 `/home/u/小说`，其中含 `/科幻/三体.txt`、`/历史/明朝那些事.txt`
- **WHEN** 用户在导入对话框选择「按文件系统目录名组织」并确认
- **THEN** 在「科幻」下按各文件直接父目录名建分类「科幻」与「历史」，`三体.txt` 归入「科幻」、`明朝那些事.txt` 归入「历史」，两条书目均出现在侧栏对应节点下

#### Scenario: 分类循环移动被拒
- **GIVEN** 分类 A 下有子分类 B，B 下有子分类 C
- **WHEN** 用户尝试把 A 移动到 B（或 C）之下
- **THEN** 操作被拒绝，提示「不能把分类移动到它自己的下级分类中」，分类树结构不变

### R7 — 分类操作的数据安全

分类操作 SHALL 不修改、不移动、不删除用户原始 TXT（延续 V1 数据安全口径）。

### R8 — 阅读历史流水

每次打开书籍 SHALL 记一条流水：时间、当时章节与位置、进入方式；用户 SHALL 能查看（时间倒序，可按书聚合）与清除（单条 / 全部），且清除 SHALL NOT 删除书籍、进度、书签。

### R9 — 阅读历史与书籍生命周期

书籍被硬删除时其历史 SHALL 一并删除；书籍被软移除（移出书架）时历史 SHALL 保留。

### R10 — 派生式自定义配色：基色

用户 SHALL 能选取 4 个基色：正文色 / 背景色 / 主色 / 强调色；系统 SHALL NOT 要求用户分别设定明色与暗色。

### R11 — 派生式自定义配色：令牌派生

Rust 共享核心 SHALL 由基色**确定性派生**全部 40 个语义令牌（明/暗两套）；用户 SHALL 不需也无法逐个设置。切换明/暗时**基色不变**，仅派生方向随主题变化。

### R12 — 自定义配色的对比度校验

系统 SHALL 在派生后校验正文对背景的对比度，**≥ 4.5:1**；低于阈值时 SHALL 给出提示并**保留用户所选基色**（不强改用户选择）。

### R13 — 自定义配色的作用范围与重置

自定义配色 SHALL 同时作用于书架、阅读、设置三页与所有浮层（弹窗 / Toast / 遮罩），不出现「只换了部分区域」；用户 SHALL 能「重置为默认」回到内置明/暗。

### R14 — 背景图的选取与存储

用户 SHALL 能从本地选取一张图片作为阅读区背景；应用 SHALL 把图片**复制**到应用数据目录 `backgrounds/`，数据库只存**相对文件名**；应用 SHALL **永不**修改或删除用户原图。本版本背景图为**全局一张**（不按分类或每书配置）。

### R15 — 背景图的呈现与调节

背景 SHALL 只作用于**阅读页正文区**（非书架 / 设置 / 浮层），并叠加弱化遮罩（默认 85%）保证正文可读；用户 SHALL 能调节遮罩强度与缩放口径（覆盖 / 平铺 / 居中），并能移除背景。

### R16 — 背景图不得影响分页口径

背景图 SHALL **不影响**分页口径：页面边界仍只由内容编码、字号、行距、边距决定；字号 / 窗口变化照常重排。背景图与自定义配色 SHALL 互相独立（换配色不丢背景，换背景不丢配色）。

### R17 — 背景图加载失败的回退

图片加载失败 SHALL 给出可理解状态（如「背景图无法读取，已回退到纯色」）并 SHALL NOT 影响阅读功能。

### R18 — 跨端一致性

分类、历史、配色、背景图的**存储与语义**（表结构、循环防护、派生口径、遮罩口径）SHALL 全部在 Rust 共享核心，桌面与 Android 调用同一 API；两端 UI 各自平台化且 SHALL NOT 复制派生算法、对比度算法。

**边界（重要）**：背景图字节由 Slint 运行时加载。Slint 的 `@image-url` 仅支持编译期内嵌，
运行时需 `slint::Image::from_rgba8` 或 `SharedPixelBuffer`。门禁 `verify-desktop-scope.py` 的
`UNBOUNDED_READ_TOKENS` 含 `include_bytes!`/`fs::read(`，故运行时加载路径必须带
`scope-allow:` 豁免注释并写明「读取的是用户图片文件，不是书籍正文」。

**Android 端（XC2）**：Android SHALL 提供同语义入口（分类侧栏、历史列表、取色器、图片选取 SAF），
本期只要求源码 + JVM 单测 + 交叉编译 + APK 构建通过，真机交互仍按 `known-limitations.md` D1 口径记为待验证。

---

## 5. 影响面

| 区域 | 改动 |
|---|---|
| `migrations/` | 新增 `0002_*.sql`；`migrate.rs` `MIGRATIONS` 加一条、`TARGET_SCHEMA_VERSION` 1→2 |
| `crates/reading-storage/src/` | 新增 `categories.rs`、`history.rs`、`theme.rs`（配色派生 + 对比度）、`background.rs` |
| `crates/desktop-app/src/` | `shelf.rs`（树投影 + 分组行）、`shelf_ops.rs`（分类/历史动作）、`settings.rs`（配色/背景设置项）、`layout.rs`（派生令牌计算）、`ui.rs`（投影 + 回调）、`import.rs`（`FilePicker` 扩多选/选目录）、`app_state.rs`（新投影结构） |
| `desktop/ui/app.slint` | 新增 `CategoryTree` 组件、`ShelfRow` 增分组头、设置页增 2 组、浮层增归类确认框；`Skin` 增背景相关属性；`UiMetrics` 可能增侧栏宽等常量 |
| `android/.../ui/` | `ShelfScreen.kt` 增分类侧栏与历史入口；新增 `CategoryTree.kt`、`HistoryScreen.kt`、`ThemePicker.kt` |
| `scripts/` | `verify-desktop-scope.py` 增 `.slint` 语义扫描项、豁免白名单；`verify-desktop-parity.py` 增运行时项；`verify-v1-release.py` 增 V1.1 项 |
| `docs/` | `known-limitations.md` 撤 L5 相关条目 / 增 V1.1 闭环记录 |

---

## 6. 分期与验收

主人未指定顺序；在下建议按**代价 / 依赖**排序，每期独立可验收：

| 期 | 内容 | 依赖 | 关键验收 |
|---|---|---|---|
| **V1.1-P1** | 分类树（CR1–CR8） | — | 创建/重命名/删除（含去向选择）/移动书籍/移动分类/循环拒绝；桌面 `NR_*` 驱动实测 + 门禁静态项 |
| **V1.1-P2** | 阅读历史（HR1–HR4） | P1（复用侧栏入口） | 连续打开 3 本书产生 3 条流水；软移除保留、硬删除级联；清空后书籍与进度不变 |
| **V1.1-P3** | 派生式配色（TP1–TP5） | — | 4 基色改动 → 40 令牌变化（门禁可核对）；对比度 <4.5:1 时提示且不强改；重置回默认 |
| **V1.1-P4** | 背景图（BG1–BG6） | P3（配色与背景独立，须回归） | 选图→复制到数据目录；遮罩/缩放可调；分页边界与无背景时逐字节一致；加载失败回退 |

**分期理由**：P1 唯一需要新迁移与循环防护，是最重的一项，先做可让后续三期都落在稳定的
分类投影之上。P2/P3/P4 互不依赖（背景图与配色的独立性正是 BG5b 要验收的项）。

---

## 7. 验收标准

### 分类（AC-V11-C）

- **AC-V11-01**：`0002` 迁移在既有 `0001` 库上重复执行不报错，`books.category_id` 存在且可空。
- **AC-V11-02**：创建三层嵌套分类（科幻 → 中国 → 科幻文学）成功，侧栏树按层级折叠展示。
- **AC-V11-03**：书籍移入分类后，书架列表按当前分类过滤，重启后归属保持。
- **AC-V11-04**：删除非空分类必须先选去向；选「移出书架」后该分类下书籍变为 `status=removed`，原始 TXT 未被删除。
- **AC-V11-05**：把分类移入自身子孙被拒绝，错误文案可理解，树结构不变（**变异测试**：构造真实环 → 必须 exit 非 0）。
- **AC-V11-05b**：**变异测试**：把应用层循环校验改为恒假（恒允许）→ 门禁须 exit 非 0。
- **AC-V11-06**：批量导入选目录 + 「按文件系统目录名组织」→ 各文件按直接父目录名建分类并归入当前父分类下。
- **AC-V11-07**：同父同名分类创建被拒并给出错误；同父不同名成功。
- **AC-V11-07b**：**变异测试**：去掉唯一索引 → 同父同名可插入 → 门禁须 exit 非 0。
- **AC-V11-07c**：**变异测试**：把 `IFNULL(parent_id,0)` 改成 `parent_id`（根分类之间不再互斥）→ 门禁须 exit 非 0。
- **AC-V11-08**：分类动作全程原始 TXT 未被修改/移动/删除（与 V1 `shelf_ops` 同一口径）。

### 阅读历史（AC-V11-H）

- **AC-V11-H1**：连续打开 3 本书 → `reading_history` 恰好 3 行，时间倒序。
- **AC-V11-H2**：流水含当时章节与位置，且与打开前保存的进度一致。
- **AC-V11-H3**：移出书架（软移除）→ 历史保留；彻底删除记录（硬删除）→ 历史级联删除。
- **AC-V11-H4**：清空历史后 `books`/`reading_progress`/`bookmarks` 行数与清除前完全一致。

### 配色（AC-V11-T）

- **AC-V11-T1**：4 基色改动 → `PALETTE_FIELDS` 至少 4 个令牌取值变化，门禁可核对。
- **AC-V11-T2**：正文对背景对比度 < 4.5:1 → 出现提示，**且基色未被强改**（用户所选被保留）。
- **AC-V11-T3**：「重置为默认」→ 全部 40 令牌回到 `PALETTE_FIELDS` 内置值。
- **AC-V11-T4**：明/暗切换 → 基色不变、派生方向切换；自定义配色作用于书架/阅读/设置/浮层全区域。

### 背景图（AC-V11-B）

- **AC-V11-B1**：选图 → 应用数据目录 `backgrounds/` 出现副本，`reader_background` 单例行 `file_name` 与之匹配。
- **AC-V11-B2**：DB 存的 `file_name` 存的是**相对文件名**（断言不含 `/` 且不含 `..`）。
- **AC-V11-B2b**：**变异测试**：把相对路径校验去掉 → 门禁须 exit 非 0。
- **AC-V11-B3**：分页边界（章节点数、`page_index`、正文字节窗口）在有/无背景图两种状态下**逐字节一致**。
- **AC-V11-B4**：遮罩与缩放可调，遮罩千分比在 0..1000 范围内校验。

### 跨端（AC-V11-X）

- **AC-V11-X1**：分类/历史/配色/背景的存储与派生语义只存在于 Rust 核心；`.slint` 与 Kotlin 不含派生算法或对比度算法。
- **AC-V11-X2**：**变异测试**：把对比度算法复制到 `.slint` / Kotlin → 门禁须 exit 非 0。

## 8. 开放问题（主人已裁定，2026-10-05）

| # | 问题 | 裁定 |
|---|---|---|
| Q1 | 邻接表 `parent_id` vs「路径物化字段」 | **邻接表**（多层树）。循环防护在应用层，AC-V11-05b 做变异测试证明门禁有牙 |
| Q2 | 删除非空分类时其下书籍去向 | **用户显式选择**：移入另一分类 / 一并移出书架 / 移回根（未分类）。未选则不执行删除 |
| Q3 | 配色是否需分别设定明色与暗色 | **不需要**。4 基色 + 派生方向（明/暗）即足 |
| Q4 | 背景图全局一张还是按书/分类配置 | **全局一张**。不与「每书独立阅读设置」耦合 |
| Q5 | 「最近阅读」入口位置 | **不放侧栏**，从书架的另一个菜单项进入；侧栏只承载真实分类，「未分类」为根下过滤入口，书籍平铺在根下 |
| Q6 | 批量导入对话框 | 每次询问，可勾选「记住」持久化（实现细节见 T2） |

## 9. 数据安全口径（延续 V1）

- 分类、历史、配色、背景图操作**永不**修改、移动或删除用户原始 TXT。
- 背景图副本存于应用数据目录，删除背景设置只删副本，不动原图。
- 清空阅读历史只删 `reading_history`，不触碰 `books`/`reading_progress`/`bookmarks`。
- 数据库迁移只新增版本（`0002`），不覆盖用户数据；迁移失败保留备份（延续 `reading-storage` 现有机制）。

## 10. 参考

- `docs/novel-reader-solution.md:23,25`（原方案承诺：分类树、阅读历史、派生配色）
- `docs/novel-reader-v1-development-plan.md:114`（背景图片移至 V1.1）、`:603`（分类树延期）
- `.hermes/specs/novel-reader-txt-v1/spec.md:26`、`desktop/spec.md:22`（范围外声明）
- `crates/desktop-app/src/layout.rs:659`（`PALETTE_FIELDS` 40 令牌）、`ui.rs:416`（`apply_palette` 唯一注入口）
- `crates/reading-storage/src/migrate.rs:44`（`MIGRATIONS` 数组）、`migrations/0001_init.sql`（硬约束）