-- migrations/0002_categories_history_theme.sql
-- novel-reader V1.1 迁移：书架分类树、阅读历史流水、派生式自定义配色、阅读背景图。
--
-- 冻结依据（结果不改写，只对齐）：
--   - .hermes/specs/novel-reader-txt-v1.1/spec.md §3.1–§3.3（R1–R17 的数据模型）
--   - docs/design/novel-reader-v1.1-desktop-design.md（侧栏/历史/配色/背景的交互与数据安全口径）
--   - migrations/0001_init.sql 的硬约束（只存元数据、不存正文、DDL 幂等、不放 PRAGMA）
--
-- 硬约束（继承 0001）：
--   1. 只新增表/列/索引，**不修改 0001 的任何既有列语义**；不改写、不删除用户数据。
--   2. 全部 ID / 字节偏移使用 64 位整数（INTEGER）。
--   3. 本文件可重复执行（IF NOT EXISTS），由 reading-storage 的迁移器在事务内完成。
--   4. 本文件只包含 DDL 与幂等索引创建；不含 PRAGMA。

-- ---------------------------------------------------------------------------
-- 1. 分类表（多级树；parent_id = NULL 表示根下分类）
-- ---------------------------------------------------------------------------
-- 层级口径（规格 R5 / 设计稿 C6）：循环防护**在应用层**沿父链上溯完成
-- （SQL 触发器跨平台不可移植），此处只提供结构与「同父同名唯一」的数据库级兜底。
CREATE TABLE IF NOT EXISTS book_categories (
    id           INTEGER PRIMARY KEY,
    parent_id    INTEGER REFERENCES book_categories (id) ON DELETE RESTRICT,
    name         TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- 同父下同名必须互斥（含根层：根的 parent_id 为 NULL，用 IFNULL 归一到 0 才能
    -- 让「根下同名」也被唯一索引覆盖；写成 parent_id 时 NULL 之间不相等 → 漏检）。
    sort_index   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_categories_sibling_name
    ON book_categories (IFNULL(parent_id, 0), name);

CREATE INDEX IF NOT EXISTS ix_categories_parent ON book_categories (parent_id, sort_index);

-- ---------------------------------------------------------------------------
-- 2. 书籍归属分类（根 = NULL，即「未分类」，与 V1 现行平铺效果一致）
-- ---------------------------------------------------------------------------
-- 删分类时：分类行被删，书籍回落为 NULL（未分类），不随分类消失。
ALTER TABLE books ADD COLUMN category_id INTEGER
    REFERENCES book_categories (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS ix_books_category ON books (category_id);

-- ---------------------------------------------------------------------------
-- 3. 阅读历史流水（规格 R8 / R9；主人裁定：不统计阅读时长）
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS reading_history (
    id                     INTEGER PRIMARY KEY,
    book_id                INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    -- 冗余书名快照：书籍被改名后历史仍可读（只读展示，不参与业务判定）。
    book_title             TEXT    NOT NULL,
    opened_at              INTEGER NOT NULL,
    chapter_index          INTEGER NOT NULL DEFAULT 0 CHECK (chapter_index >= 0),
    byte_offset            INTEGER NOT NULL DEFAULT 0 CHECK (byte_offset >= 0),
    -- 进入方式：0=书架打开 1=阅读页重开 2=进度恢复 3=书签跳转 4=章节跳转
    entry_kind             INTEGER NOT NULL DEFAULT 0 CHECK (entry_kind BETWEEN 0 AND 4),
    source_kind            TEXT    NOT NULL DEFAULT 'desktop_path'
                            CHECK (source_kind IN ('desktop_path', 'android_uri', 'private_copy')),
    source_locator         TEXT    NOT NULL,
    content_index_version  INTEGER NOT NULL DEFAULT 0 CHECK (content_index_version >= 0)
);

CREATE INDEX IF NOT EXISTS ix_history_opened_at ON reading_history (opened_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS ix_history_book ON reading_history (book_id, opened_at DESC, id DESC);

-- ---------------------------------------------------------------------------
-- 4. 阅读背景图（规格 R14；主人裁定：全局一张，存应用数据目录内的相对文件名）
-- ---------------------------------------------------------------------------
-- file_name 必须是 backgrounds/ 下的**相对文件名**：应用层拒绝 `/`、`..` 与绝对路径，
-- 数据库层用 CHECK 兜底（防止任何写入路径绕过应用层）。
CREATE TABLE IF NOT EXISTS reader_background (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    file_name       TEXT    NOT NULL
                            -- 用 char(92) 表示反斜杠：SQLite 字符串字面量里 '\' 不作转义，
                            -- 写 '\\' 会被当成「反斜杠 + 反斜杠」两字符序列，
                            -- 导致 Windows 路径分隔符**逃过**校验（实测踩中，见 checklist C36）。
                            CHECK (length(file_name) > 0
                                   AND instr(file_name, '/') = 0
                                   AND instr(file_name, char(92)) = 0
                                   AND instr(file_name, '..') = 0),
    file_size       INTEGER NOT NULL DEFAULT 0 CHECK (file_size >= 0),
    -- 弱化遮罩千分比（0..1000）；默认 850 = 85%。
    scrim_permille  INTEGER NOT NULL DEFAULT 850 CHECK (scrim_permille BETWEEN 0 AND 1000),
    -- 缩放口径：0=覆盖铺满 1=原始尺寸平铺 2=原始尺寸居中。
    fit_mode        INTEGER NOT NULL DEFAULT 0 CHECK (fit_mode IN (0, 1, 2)),
    updated_at      INTEGER NOT NULL
);

-- ---------------------------------------------------------------------------
-- 5. 派生式自定义配色（规格 R10–R13；主人裁定：不要求分别设定明色与暗色）
-- ---------------------------------------------------------------------------
-- 只存 4 个基色与派生开关，**不存 40 个令牌的实际值**：实际值每次由 Rust 核心
-- 从基色确定性派生（避免「改了亮色漏了暗色」的半改状态）。
CREATE TABLE IF NOT EXISTS reader_theme_custom (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    enabled           INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    -- 基色（#RRGGBB，大小写不限；NULL = 未改动该项，沿用内置值）。
    base_foreground   TEXT    CHECK (base_foreground IS NULL
                                     OR base_foreground GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
    base_background   TEXT    CHECK (base_background IS NULL
                                     OR base_background GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
    base_primary      TEXT    CHECK (base_primary IS NULL
                                     OR base_primary GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
    base_accent       TEXT    CHECK (base_accent IS NULL
                                     OR base_accent GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
    updated_at        INTEGER NOT NULL
);