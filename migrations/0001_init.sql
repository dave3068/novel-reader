-- migrations/0001_init.sql
-- novel-reader V1 初始迁移：本地元数据 / 索引 / 位置 / 短摘要 存储。
--
-- 冻结依据（结果不改写，只对齐）：
--   - results/gates/gate-0-baseline.md §2.6：初始迁移建立 8 张表，正文字段不入库。
--   - docs/novel-reader-v1-development-plan.md §9.1（表结构建议）、§9.2（数据安全规则）、§15.4（升级与数据兼容）。
--   - .hermes/specs/novel-reader-txt-v1/design.md §5：迁移按 0001_*.sql / 0002_*.sql 递增。
--
-- 硬约束：
--   1. 只存元数据、索引、位置与短摘要；**不存完整正文**（无 content/body/全文列）。
--   2. 全部 ID / 字节偏移 / 长度使用 64 位整数（INTEGER）。
--   3. 本文件可重复执行（IF NOT EXISTS），执行由 reading-storage 的迁移器在事务内完成。
--   4. 迁移器要求：本文件只包含 DDL 与幂等索引创建；不包含 PRAGMA（连接级 pragma 由仓储层设置）。

-- 书籍记录（书架条目）。source_* 为主来源的去规范化投影，便于书架列表一次查询渲染；
-- 完整来源（多来源 / 重新关联历史）落在 book_sources。
CREATE TABLE IF NOT EXISTS books (
    id                      INTEGER PRIMARY KEY,
    display_name            TEXT    NOT NULL,
    original_name           TEXT    NOT NULL,
    format                  TEXT    NOT NULL DEFAULT 'txt' CHECK (format = 'txt'),
    source_kind             TEXT    NOT NULL CHECK (source_kind IN ('desktop_path', 'android_uri', 'private_copy')),
    source_locator          TEXT    NOT NULL,
    source_size             INTEGER NOT NULL DEFAULT 0 CHECK (source_size >= 0),
    source_modified_hint    INTEGER,
    source_fingerprint      TEXT,
    encoding                TEXT    NOT NULL DEFAULT 'utf-8',
    encoding_confidence     INTEGER NOT NULL DEFAULT 0 CHECK (encoding_confidence BETWEEN 0 AND 1000),
    encoding_user_override  INTEGER NOT NULL DEFAULT 0 CHECK (encoding_user_override IN (0, 1)),
    status                  TEXT    NOT NULL DEFAULT 'ready'
                                    CHECK (status IN ('ready', 'missing', 'needs_reindex', 'import_failed', 'removed')),
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

-- 来源记录：一个书籍条目可以有多条来源（重新关联、Android 私有副本等）。
CREATE TABLE IF NOT EXISTS book_sources (
    id                      INTEGER PRIMARY KEY,
    book_id                 INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    source_kind             TEXT    NOT NULL CHECK (source_kind IN ('desktop_path', 'android_uri', 'private_copy')),
    source_locator          TEXT    NOT NULL,
    source_size             INTEGER NOT NULL DEFAULT 0 CHECK (source_size >= 0),
    source_modified_hint    INTEGER,
    source_fingerprint      TEXT,
    encoding                TEXT    NOT NULL DEFAULT 'utf-8',
    encoding_confidence     INTEGER NOT NULL DEFAULT 0 CHECK (encoding_confidence BETWEEN 0 AND 1000),
    encoding_user_override  INTEGER NOT NULL DEFAULT 0 CHECK (encoding_user_override IN (0, 1)),
    is_primary              INTEGER NOT NULL DEFAULT 1 CHECK (is_primary IN (0, 1)),
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    UNIQUE (book_id, source_kind, source_locator)
);

-- 章节区间索引（可重建缓存；title 为章节名短摘要，非正文）。
CREATE TABLE IF NOT EXISTS chapters (
    id              INTEGER PRIMARY KEY,
    book_id         INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    sequence        INTEGER NOT NULL CHECK (sequence >= 0),
    title           TEXT    NOT NULL,
    start_byte      INTEGER NOT NULL CHECK (start_byte >= 0),
    end_byte        INTEGER NOT NULL CHECK (end_byte >= start_byte),
    rule_id         TEXT    NOT NULL,
    index_version   INTEGER NOT NULL DEFAULT 0 CHECK (index_version >= 0),
    created_at      INTEGER NOT NULL,
    UNIQUE (book_id, index_version, sequence)
);

-- 阅读进度：以 ContentPosition 为准（gate-0 §2.4），每本书一行，幂等 upsert。
CREATE TABLE IF NOT EXISTS reading_progress (
    book_id             INTEGER PRIMARY KEY REFERENCES books (id) ON DELETE CASCADE,
    content_unit_id     INTEGER NOT NULL DEFAULT 0 CHECK (content_unit_id >= 0),
    chapter_index       INTEGER NOT NULL DEFAULT 0 CHECK (chapter_index >= 0),
    byte_offset         INTEGER NOT NULL DEFAULT 0 CHECK (byte_offset >= 0),
    nearby_fingerprint  TEXT,
    last_page_hint      INTEGER CHECK (last_page_hint IS NULL OR last_page_hint >= 0),
    index_version       INTEGER NOT NULL DEFAULT 0 CHECK (index_version >= 0),
    updated_at          INTEGER NOT NULL,
    last_opened_at      INTEGER NOT NULL
);

-- 书签（以内容位置为准，不绑定动态页码）。
CREATE TABLE IF NOT EXISTS bookmarks (
    id                  INTEGER PRIMARY KEY,
    book_id             INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    content_unit_id     INTEGER NOT NULL DEFAULT 0 CHECK (content_unit_id >= 0),
    chapter_index       INTEGER NOT NULL DEFAULT 0 CHECK (chapter_index >= 0),
    byte_offset         INTEGER NOT NULL CHECK (byte_offset >= 0),
    nearby_fingerprint  TEXT,
    title               TEXT    NOT NULL DEFAULT '',
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

-- 全局排版设置（单行，id 固定为 1）。尺寸/间距均为整数毫单位（与 reading-core LayoutSignature 口径一致）。
CREATE TABLE IF NOT EXISTS reader_settings (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    font_family         TEXT    NOT NULL DEFAULT 'Noto Sans CJK SC',
    font_weight         INTEGER NOT NULL DEFAULT 400 CHECK (font_weight > 0),
    font_size_milli     INTEGER NOT NULL DEFAULT 18000 CHECK (font_size_milli > 0),
    line_spacing_milli  INTEGER NOT NULL DEFAULT 400 CHECK (line_spacing_milli >= 0),
    paragraph_spacing_milli INTEGER NOT NULL DEFAULT 0 CHECK (paragraph_spacing_milli >= 0),
    first_line_indent_milli INTEGER NOT NULL DEFAULT 2000 CHECK (first_line_indent_milli >= 0),
    margin_x_milli      INTEGER NOT NULL DEFAULT 16000 CHECK (margin_x_milli >= 0),
    margin_y_milli      INTEGER NOT NULL DEFAULT 16000 CHECK (margin_y_milli >= 0),
    theme               TEXT    NOT NULL DEFAULT 'system' CHECK (theme IN ('system', 'light', 'dark')),
    operation_mode      TEXT    NOT NULL DEFAULT 'paged' CHECK (operation_mode IN ('paged', 'scroll')),
    default_encoding    TEXT    NOT NULL DEFAULT 'utf-8',
    updated_at          INTEGER NOT NULL
);

-- 分页边界缓存记录（可重建；layout_signature 为 reading-core LayoutSignature 的稳定文本编码，
-- 键 = layout_signature + content_index_version，见 gate-0 §2.5）。
CREATE TABLE IF NOT EXISTS layout_records (
    id                      INTEGER PRIMARY KEY,
    book_id                 INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    layout_signature        TEXT    NOT NULL,
    content_index_version   INTEGER NOT NULL CHECK (content_index_version >= 0),
    generated_until_byte    INTEGER NOT NULL DEFAULT 0 CHECK (generated_until_byte >= 0),
    page_count              INTEGER NOT NULL DEFAULT 0 CHECK (page_count >= 0),
    status                  TEXT    NOT NULL DEFAULT 'partial'
                                    CHECK (status IN ('partial', 'complete', 'stale', 'failed')),
    updated_at              INTEGER NOT NULL,
    UNIQUE (book_id, layout_signature, content_index_version)
);

-- 应用元信息：schema 版本、内容索引版本、迁移历史（key/value）。
CREATE TABLE IF NOT EXISTS app_meta (
    key         TEXT    PRIMARY KEY,
    value       TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- 索引：只加速查询，不含正文；索引可删除重建（不属用户数据）。
CREATE UNIQUE INDEX IF NOT EXISTS ux_books_source ON books (source_kind, source_locator);
CREATE INDEX IF NOT EXISTS ix_books_fingerprint ON books (source_fingerprint);
CREATE INDEX IF NOT EXISTS ix_books_status ON books (status, updated_at);
CREATE INDEX IF NOT EXISTS ix_book_sources_book ON book_sources (book_id);
CREATE INDEX IF NOT EXISTS ix_chapters_book ON chapters (book_id, index_version, sequence);
CREATE INDEX IF NOT EXISTS ix_bookmarks_book ON bookmarks (book_id, byte_offset);
CREATE INDEX IF NOT EXISTS ix_layout_records_book ON layout_records (book_id, content_index_version);
