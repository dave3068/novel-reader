/*
 * novel_reader.h — Novel Reader V1 正式跨语言 ABI（C ABI / JNI 共享契约）
 *
 * 版本：NR_ABI_VERSION = 1
 * 冻结依据：results/gates/gate-0-baseline.md §2.1（版本）/§2.2（12 项错误码）/
 *           §2.3（64 KiB 默认窗口、256 KiB 硬上限）/§2.7（18 项入口）
 * 实现：crates/reading-ffi（Rust）—— 头文件与导出符号由 scripts/verify-abi.py 逐项校验，
 *       头文件与 Rust 结构体字段由 crates/reading-ffi/tests/header_parity.rs 逐项校验。
 *
 * ============================ 通用约定 ============================
 *
 * 1) 句柄：nr_engine 是不透明 64 位整数，0 永远无效；永不复用；
 *    销毁后再次使用返回 NR_ERR_ENGINE_DESTROYED(9)，
 *    从未分配过的句柄返回 NR_ERR_INVALID_ARGUMENT(1)。
 * 2) 错误码：所有函数返回 int32_t 的错误码（0 = 成功），数值与 NR_ERR_* 一致；
 *    唯一例外是 nr_abi_version()，它直接返回 uint32_t 版本号。
 *    错误文本只用于展示，禁止用文本判断业务分支。
 * 3) 字符串：一律 UTF-8 指针 + 长度（不依赖 NUL 结尾）；指针为 NULL 时长度必须为 0。
 * 4) 64 位：书籍 ID、文件长度、字节偏移全部是 64 位；跨语言不得退化为浮点/32 位。
 * 5) 缓冲区所有权：
 *    - 由 Rust 分配的缓冲区（nr_read_window / nr_get_chapters / nr_get_position /
 *      nr_get_last_error 的输出）调用方必须调用一次 nr_release_buffer 归还；
 *    - 归还时 Rust 释放内存并把结构体三个字段全部置零；
 *    - 对已归零的缓冲区再次调用 nr_release_buffer 是安全 no-op（返回 0），因此重复归还不会崩溃；
 *    - 调用方不得修改 ptr/len/capacity；伪造字段（len > capacity 等）会被拒绝为
 *      NR_ERR_INVALID_ARGUMENT(1)；
 *    - nr_release_buffer 不需要引擎句柄，引擎销毁后仍可归还此前的缓冲区。
 * 6) 并发与串行化：同一引擎的调用在同一把互斥锁下串行执行；
 *    平台窗口回调（见 §平台窗口内容协议）在调用线程同步执行，且同一时刻只有一次回调在飞；
 *    回调内禁止回调任何 nr_* 入口（会造成同一引擎重入）。
 * 7) 取消：nr_cancel_request 标记 (引擎, request_id) 为已取消，标志与引擎锁分离存放，
 *    因此可以在另一个线程上于长任务（nr_rebuild_index）运行期间生效；
 *    长任务在每个窗口检查该标志，命中即以 NR_ERR_CANCELLED(7) 返回部分结果
 *    （nr_rebuild_result.partial = 1），且不把不完整索引写入本地存储。
 * 8) 版本：每次不兼容 ABI 变更必须同时更新本头文件、Rust 常量、契约测试并递增
 *    NR_ABI_VERSION（reading-core NFR-C4）。
 *
 * ============================ 平台窗口内容协议 ============================
 *
 * Android 等平台的数据源（SAF URI）没有可 seek 的本地路径，字节由平台提供：
 *
 *   - 引擎创建时传入 nr_source_provider 回调（nr_engine_config.provider）；
 *   - 打开时使用 kind = NR_SOURCE_KIND_PLATFORM，并提供 identity（URI）与 total_bytes；
 *   - Rust 决定读取区间（offset 与 max_len）后调用平台回调；max_len 恒 ≤ 引擎窗口上限
 *     （默认 NR_DEFAULT_WINDOW_BYTES，硬上限 NR_MAX_WINDOW_BYTES）；
 *   - 平台把该区间字节写入 dst，并回报 out_len 与 out_eof；
 *   - Rust 校验 out_len ≤ max_len：回报超长（试图一次交付整本正文）一律判为
 *     NR_ERR_INTERNAL(10) 且不采信该长度；
 *   - 随后由 Rust 在字节层面修正 UTF-8 边界，并回报 dropped_leading / trimmed_tail / eof。
 *
 * 结论：正文字节永远以"受限窗口"形式跨越边界，单次交付量由 Rust 裁定，
 * 平台无法通过自述长度扩大交付量。
 */

#ifndef NOVEL_READER_H
#define NOVEL_READER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ============================ 版本与常量 ============================ */

/* ABI 版本。编译期断言示例：
 *   _Static_assert(NR_ABI_VERSION == 1u, "ABI 版本不匹配");
 * 运行期对照：nr_abi_version()。
 */
#define NR_ABI_VERSION 1u

/* 单次读取的默认窗口（64 KiB）与硬上限（256 KiB）。 */
#define NR_DEFAULT_WINDOW_BYTES 65536u
#define NR_MAX_WINDOW_BYTES 262144u

/* 单次章节批次返回的最大条目数（禁止一次取走整库章节）。 */
#define NR_MAX_CHAPTER_BATCH 256u

/* V1 全局唯一的内容单元标识（一个普通 TXT = 一个内容单元）。 */
#define NR_CONTENT_UNIT_SINGLE 1u

/* 排版算法版本（属于排版签名；变化会失效全部分页缓存）。 */
#define NR_LAYOUT_ALGORITHM_VERSION 1u

/* 数据源种类。 */
#define NR_SOURCE_KIND_FILE 1u      /* 本地文件（桌面路径 / 私有缓存副本路径） */
#define NR_SOURCE_KIND_PLATFORM 2u  /* 平台窗口（Android SAF 等，需要 nr_source_provider） */

/* 页边界提交阶段。 */
#define NR_SUBMIT_BEGIN_LAYOUT 1u /* 登记排版签名，返回 layout_id 与 request_id */
#define NR_SUBMIT_COMMIT 2u       /* 提交平台测量出的页边界 */

/* ============================ 错误码（冻结 12 项） ============================ */

#define NR_ERR_OK 0                        /* 成功 */
#define NR_ERR_INVALID_ARGUMENT 1          /* 空指针、长度为 0、非法句柄等 */
#define NR_ERR_OUT_OF_RANGE 2              /* 偏移/索引越界 */
#define NR_ERR_UTF8_BOUNDARY 3             /* 偏移未落在合法 UTF-8 边界 */
#define NR_ERR_BOOK_NOT_FOUND 4            /* 书籍不存在 */
#define NR_ERR_NO_OPEN_BOOK 5              /* 尚未打开书籍 */
#define NR_ERR_IO 6                        /* 数据源 I/O 失败 */
#define NR_ERR_CANCELLED 7                 /* 请求被取消（部分成功） */
#define NR_ERR_LAYOUT_NOT_FOUND 8          /* 布局标识无登记边界 */
#define NR_ERR_ENGINE_DESTROYED 9          /* 引擎已销毁 */
#define NR_ERR_INTERNAL 10                 /* 内部错误（含 panic 拦截） */
#define NR_ERR_CHAPTER_OUT_OF_RANGE 11     /* 章节越界 / 偏移与章节不一致 */

/* ============================ 编码稳定数值 ============================ */

#define NR_ENCODING_UTF8 1u
#define NR_ENCODING_UTF8_BOM 2u
#define NR_ENCODING_UTF16LE 3u
#define NR_ENCODING_UTF16BE 4u
#define NR_ENCODING_GB18030 5u
#define NR_ENCODING_GBK 6u
#define NR_ENCODING_BIG5 7u

/* ============================ 页边界状态与拒绝原因 ============================ */

/* nr_boundary_result.status 取值：
 * 0..=5 与 reading-core 的 BoundaryCommitStatus 同数值，100 为 BEGIN_LAYOUT 成功，
 * 6 表示该世代下未登记该起点。 */
#define NR_BOUNDARY_COMMITTED 0u
#define NR_BOUNDARY_DUPLICATE_IDENTICAL 1u
#define NR_BOUNDARY_REJECTED 2u
#define NR_BOUNDARY_STALE_LAYOUT_ID 3u
#define NR_BOUNDARY_STALE_REQUEST_ID 4u
#define NR_BOUNDARY_STALE_CONTENT_INDEX_VERSION 5u
#define NR_BOUNDARY_NOT_FOUND 6u
#define NR_BOUNDARY_LAYOUT_READY 100u

/* nr_boundary_result.rejection 取值（仅 NR_BOUNDARY_REJECTED 时非 0）。 */
#define NR_REJECT_REVERSED_INTERVAL 1u
#define NR_REJECT_EMPTY_INTERVAL 2u
#define NR_REJECT_OUT_OF_RANGE 3u
#define NR_REJECT_ILLEGAL_CHAR_BOUNDARY 4u
#define NR_REJECT_GAP_BEFORE_PREVIOUS 5u
#define NR_REJECT_OVERLAP_WITH_PREVIOUS 6u
#define NR_REJECT_NOT_ADVANCING 7u
#define NR_REJECT_CONFLICT_WITH_CACHED 8u
#define NR_REJECT_TOTAL_LENGTH_MISMATCH 9u

/* ============================ 句柄与缓冲区 ============================ */

/* 不透明引擎句柄；0 永远无效。 */
typedef uint64_t nr_engine;

/* 由 Rust 分配、调用方用 nr_release_buffer 归还的字节缓冲区。 */
typedef struct nr_buffer {
    uint8_t *ptr;
    size_t len;
    size_t capacity;
} nr_buffer;

/* ============================ 数据源提供者 ============================ */

/* 平台窗口回调：把 [offset, offset + out_len) 的字节写入 dst（容量 max_len），
 * 并回报实际写入长度 out_len 与是否到达内容末尾 out_eof（0/1）。
 * 返回 0 表示成功，其余为 NR_ERR_* 错误码；返回 NR_ERR_CANCELLED(7) 按 I/O 中断处理。
 * 必须在调用线程同步返回；禁止调用任何 nr_* 入口。 */
typedef struct nr_source_provider {
    void *user;
    int32_t (*read_window)(void *, uint64_t, uint64_t, uint32_t, uint8_t *, uint32_t *, int32_t *);
} nr_source_provider;

/* ============================ 引擎配置 ============================ */

typedef struct nr_engine_config {
    uint32_t struct_size;     /* 必须等于 sizeof(nr_engine_config) */
    uint32_t abi_version;     /* 必须等于 NR_ABI_VERSION */
    uint64_t max_window_bytes; /* 单次读取窗口上限；0 表示 NR_MAX_WINDOW_BYTES */
    const nr_source_provider *provider; /* 平台数据源必需；文件数据源可为 NULL */
    const uint8_t *db_path_utf8; /* 数据库路径（UTF-8）；NULL 或长度 0 表示内存数据库 */
    size_t db_path_len;
} nr_engine_config;

/* ============================ 打开请求与结果 ============================ */

typedef struct nr_open_request {
    uint32_t struct_size;     /* 必须等于 sizeof(nr_open_request) */
    uint32_t kind;            /* NR_SOURCE_KIND_FILE / NR_SOURCE_KIND_PLATFORM */
    uint64_t total_bytes;     /* 平台数据源必填；文件数据源忽略（以文件实际长度为准） */
    const uint8_t *identity_utf8; /* 文件路径 / SAF URI */
    size_t identity_len;
    const uint8_t *display_name_utf8; /* 可为 NULL：由引擎从来源标识推导 */
    size_t display_name_len;
} nr_open_request;

typedef struct nr_open_result {
    uint64_t book_id;                    /* 本地存储中的书籍标识 */
    uint64_t total_bytes;                /* 坐标空间总字节数（文件为真实长度） */
    uint64_t content_index_version;      /* 分页缓存键的一半 */
    uint32_t content_unit_id;            /* V1 恒为 NR_CONTENT_UNIT_SINGLE */
    uint32_t encoding;                   /* NR_ENCODING_* */
    uint32_t encoding_confidence;        /* 0..=1000 千分比 */
    uint32_t encoding_score;             /* 0..=1000 综合评分 */
    uint32_t needs_encoding_confirmation; /* 1 = 低可信度，需要用户确认 */
    uint32_t chapters;                   /* 章节数（至少 1） */
    uint32_t index_from_cache;           /* 1 = 索引来自本地存储，未重新扫描 */
    uint32_t reserved0;                  /* 必须为 0 */
} nr_open_result;

/* ============================ 读取窗口 ============================ */

typedef struct nr_window {
    uint64_t offset;         /* 窗口起始偏移（等于请求的 offset） */
    uint64_t end_byte;       /* 窗口结束偏移（不含）；可能小于 offset + len */
    uint64_t request_id;     /* 引擎侧读取序号（单调递增） */
    uint64_t total_bytes;    /* 坐标空间总字节数 */
    uint32_t len;            /* 实际返回字节数（等于 nr_buffer.len） */
    uint32_t dropped_leading; /* 因字符边界裁掉的头部字节数 */
    uint32_t trimmed_tail;   /* 因字符边界裁掉的尾部字节数 */
    uint32_t eof;            /* 1 = 已到达内容末尾 */
} nr_window;

/* ============================ 排版签名与页边界 ============================ */

/* 排版签名：13 项冻结字段（字体族以 UTF-8 指针 + 长度表达）+ 保留字段。
 * 所有尺寸都是"千分之一像素"的整数：跨平台/跨语言不出现浮点不一致。 */
typedef struct nr_layout_signature {
    uint32_t viewport_width;
    uint32_t viewport_height;
    uint32_t direction;              /* 0 横 / 1 纵 */
    uint32_t font_weight;
    uint32_t font_size_milli;
    uint32_t line_spacing_milli;
    uint32_t paragraph_spacing_milli;
    uint32_t first_line_indent_milli;
    uint32_t margin_horizontal_milli;
    uint32_t margin_vertical_milli;
    uint32_t algorithm_version;      /* 必须等于 NR_LAYOUT_ALGORITHM_VERSION */
    uint32_t reserved0;              /* 必须为 0 */
    uint64_t platform_font_fingerprint; /* 0 = 尚未取得度量指纹 */
    const uint8_t *font_family_utf8; /* 非空 UTF-8 字体族 */
    size_t font_family_len;
} nr_layout_signature;

typedef struct nr_page_boundary_submission {
    uint32_t struct_size;    /* 必须等于 sizeof(nr_page_boundary_submission) */
    uint32_t phase;          /* NR_SUBMIT_BEGIN_LAYOUT / NR_SUBMIT_COMMIT */
    uint64_t content_index_version; /* 必须等于 nr_open_result.content_index_version */
    uint64_t layout_id;      /* BEGIN_LAYOUT 返回；提交阶段必填 */
    uint64_t request_id;     /* BEGIN_LAYOUT 返回；提交阶段必填 */
    uint64_t total_bytes;    /* 必须等于书籍长度 */
    uint64_t start_byte;     /* 页面起始字节 */
    uint64_t end_byte;       /* 页面结束字节（必须位于合法 UTF-8 边界） */
    const nr_layout_signature *signature; /* BEGIN_LAYOUT 必填；提交阶段可为 NULL */
} nr_page_boundary_submission;

typedef struct nr_boundary_result {
    uint32_t status;         /* NR_BOUNDARY_* */
    uint32_t rejection;      /* NR_REJECT_*（未拒绝为 0） */
    int32_t error_code;      /* NR_ERR_*（成功为 0） */
    uint32_t reserved0;      /* 必须为 0 */
    uint64_t layout_id;      /* 当前排版世代 */
    uint64_t request_id;     /* BEGIN_LAYOUT 返回的请求标识 */
    uint64_t stale_layout_id;  /* 被丢弃的世代（过期类状态才有值，否则 0） */
    uint64_t stale_request_id; /* 被丢弃的请求（NR_BOUNDARY_STALE_REQUEST_ID 才有值） */
    uint64_t start_byte;     /* 已登记边界起点（未命中为 0） */
    uint64_t end_byte;       /* 已登记边界终点（未命中为 0） */
    uint64_t total_bytes;    /* 坐标空间总字节数（回显） */
    uint64_t page_count;     /* 当前世代已登记的边界数量 */
} nr_boundary_result;

/* ============================ 内容位置 ============================ */

typedef struct nr_position {
    uint32_t struct_size;    /* 必须等于 sizeof(nr_position) */
    uint32_t chapter_index;  /* 必须与偏移所在章节一致，否则 NR_ERR_CHAPTER_OUT_OF_RANGE */
    uint64_t book_id;        /* 必须等于当前打开的书籍 */
    uint64_t content_unit_id; /* V1 必须为 NR_CONTENT_UNIT_SINGLE */
    uint64_t original_byte_offset; /* 原始文件字节偏移 */
    const uint8_t *nearby_fingerprint_utf8; /* 可为 NULL：由引擎在保存时自动采样 */
    size_t nearby_fingerprint_len;
} nr_position;

typedef struct nr_position_result {
    uint32_t struct_size;    /* 必须等于 sizeof(nr_position_result) */
    uint32_t found;          /* 0 = 该书尚无进度 */
    uint32_t chapter_index;
    uint32_t restored;       /* 1 = 已用锚点在有界半径内恢复 */
    uint32_t fallback_to_chapter_start; /* 1 = 回退到章节开头 */
    uint32_t reserved0;      /* 必须为 0 */
    uint64_t book_id;
    uint64_t content_unit_id;
    uint64_t original_byte_offset;
    uint64_t updated_at;     /* Unix 秒 */
    nr_buffer nearby_fingerprint; /* UTF-8；Rust 分配，调用方用 nr_release_buffer 归还 */
} nr_position_result;

/* ============================ 章节批次 ============================ */

/* 章节条目：定长、无指针、无正文字段。标题字节位于批次缓冲区起始处的
 * title_offset（相对 nr_chapter_batch.bytes.ptr），长度为 title_len。 */
typedef struct nr_chapter {
    uint32_t sequence;       /* 章节序号（从 0 起） */
    uint32_t rule_id;        /* 命中规则数值 */
    uint64_t start_byte;
    uint64_t end_byte;       /* 不含 */
    uint32_t title_offset;   /* 相对 bytes.ptr */
    uint32_t title_len;
    uint32_t index_version;
    uint32_t reserved0;      /* 必须为 0 */
} nr_chapter;

/* 章节批次缓冲区布局：[nr_chapter; count] 后紧接全部标题的 UTF-8 字节。
 * 一次分配、单一所有者：整块缓冲区由 Rust 分配、调用方归还。 */
typedef struct nr_chapter_batch {
    uint32_t count;          /* 本次返回条目数（≤ NR_MAX_CHAPTER_BATCH） */
    uint32_t index_version;
    uint32_t total_chapters; /* 该书章节总数 */
    uint32_t reserved0;      /* 必须为 0 */
    nr_buffer bytes;         /* Rust 分配，调用方用 nr_release_buffer 归还 */
} nr_chapter_batch;

/* ============================ 书签 ============================ */

typedef struct nr_bookmark_input {
    uint32_t struct_size;    /* 必须等于 sizeof(nr_bookmark_input) */
    uint32_t chapter_index;  /* 必须与偏移所在章节一致 */
    uint64_t book_id;        /* 必须等于当前打开的书籍 */
    uint64_t content_unit_id; /* V1 必须为 NR_CONTENT_UNIT_SINGLE */
    uint64_t byte_offset;
    const uint8_t *title_utf8; /* 可为 NULL / 长度为 0：使用章节标题 */
    size_t title_len;
} nr_bookmark_input;

/* ============================ 索引重建 ============================ */

typedef struct nr_rebuild_result {
    uint32_t struct_size;    /* 必须等于 sizeof(nr_rebuild_result) */
    uint32_t partial;        /* 1 = 被取消，结果不完整（不落库） */
    uint32_t chapters;
    uint32_t index_from_cache;
    uint64_t scanned_bytes;
    uint64_t total_bytes;
    uint64_t content_index_version; /* 重建后的内容索引版本 */
} nr_rebuild_result;

/* 手动指定内容编码（nr_set_encoding）的出参。
 * 语义：只改变「如何解释文件字节」，**永不写回原始文件**；覆盖后按新编码立即重扫章节索引，
 * 并递增内容索引版本（旧章节缓存与分页边界全部失效）。 */
typedef struct nr_set_encoding_result {
    uint32_t struct_size;           /* 必须等于 sizeof(nr_set_encoding_result) */
    uint32_t encoding;              /* 生效编码（NR_ENCODING_*） */
    uint64_t content_index_version; /* 重建后的内容索引版本 */
    uint32_t chapters;              /* 按新编码重扫后的章节数量 */
    uint32_t index_rebuilt;         /* 1 = 本次就地重扫了新索引 */
} nr_set_encoding_result;

/* ============================ 最近错误 ============================ */

typedef struct nr_error_info {
    int32_t code;            /* NR_ERR_*；无错误为 0 */
    uint32_t has_message;    /* 1 = message 有效 */
    nr_buffer message;       /* UTF-8；Rust 分配，调用方用 nr_release_buffer 归还 */
} nr_error_info;

/* ==================== 入口（冻结 18 项 + nr_set_encoding，共 19 项） ==================== */

/* 1. 查询 ABI 版本。无需句柄；可与编译期宏 NR_ABI_VERSION 对照。 */
uint32_t nr_abi_version(void);

/* 2. 创建引擎。config 必须指向当前 ABI 版本的 nr_engine_config；
 *    成功时把句柄写入 out_engine（0 表示失败）。 */
int32_t nr_engine_create(const nr_engine_config *config, uint64_t *out_engine);

/* 3. 销毁引擎。首次销毁返回 0；重复销毁返回 NR_ERR_ENGINE_DESTROYED。 */
int32_t nr_engine_destroy(uint64_t engine);

/* 4. 打开 TXT：导入 upsert、编码识别（有限采样）、章节索引（优先本地缓存）。
 *    正文不入库、不整本读取。成功后当前书籍被替换，旧排版缓存作废。 */
int32_t nr_open_txt(uint64_t engine, const nr_open_request *request, nr_open_result *out_result);

/* 5. 关闭当前书籍（book_id 必须与打开的书籍一致）。 */
int32_t nr_close_book(uint64_t engine, uint64_t book_id);

/* 6. 读取有限窗口。length == 0 或 length > 引擎窗口上限时返回
 *    NR_ERR_INVALID_ARGUMENT 且不读取任何字节；offset 必须在 chapter_index 的区间内。
 *    out_bytes 由 Rust 分配，调用方必须用 nr_release_buffer 归还。 */
int32_t nr_read_window(uint64_t engine, uint32_t chapter_index, uint64_t offset, uint64_t length, nr_window *out_window, nr_buffer *out_bytes);

/* 7. 提交页边界。BEGIN_LAYOUT：登记签名并返回 layout_id/request_id；
 *    COMMIT：提交平台测量结果，过期结果按 status 处丢弃而不覆盖当前页。 */
int32_t nr_submit_page_boundary(uint64_t engine, const nr_page_boundary_submission *submission, nr_boundary_result *out_result);

/* 8. 查询页边界：layout_id 必须是当前世代，且 start_byte 属于 chapter_index。
 *    未命中返回 NR_ERR_LAYOUT_NOT_FOUND（status = NR_BOUNDARY_NOT_FOUND）。 */
int32_t nr_get_page_boundary(uint64_t engine, uint64_t layout_id, uint32_t chapter_index, uint64_t start_byte, nr_boundary_result *out_result);

/* 9. 保存阅读位置（幂等 upsert）。坐标与章节由引擎裁定：偏移越界返回
 *    NR_ERR_OUT_OF_RANGE，偏移与 chapter_index 不一致返回 NR_ERR_CHAPTER_OUT_OF_RANGE。
 *    nearby_fingerprint 为 NULL 时由引擎在保存时自动采样。out_changed 可为 NULL。 */
int32_t nr_save_position(uint64_t engine, const nr_position *position, int32_t *out_changed);

/* 10. 读取阅读位置。打开同一本书时用附近文本指纹在有界半径内恢复。
 *     out_result.nearby_fingerprint 由 Rust 分配，调用方必须归还。 */
int32_t nr_get_position(uint64_t engine, uint64_t book_id, nr_position_result *out_result);

/* 11. 章节批次（1 ≤ max_count ≤ NR_MAX_CHAPTER_BATCH）。out_batch.bytes 由 Rust 分配。 */
int32_t nr_get_chapters(uint64_t engine, uint64_t book_id, uint32_t start_sequence, uint32_t max_count, nr_chapter_batch *out_batch);

/* 12. 新增书签，返回书签 id；名称为空时使用章节标题（名称是短摘要，不是正文）。 */
int32_t nr_create_bookmark(uint64_t engine, const nr_bookmark_input *input, uint64_t *out_bookmark_id);

/* 13. 删除书签；out_deleted 回报是否确有删除（可为 NULL）。 */
int32_t nr_delete_bookmark(uint64_t engine, uint64_t bookmark_id, int32_t *out_deleted);

/* 14. 失效当前排版世代：清空该世代全部分页边界并递增世代，
 *     同时清理本地存储中该签名的分页记录。layout_id 必须是当前世代，否则
 *     NR_ERR_LAYOUT_NOT_FOUND。out_removed 可为 NULL。 */
int32_t nr_invalidate_layout(uint64_t engine, uint64_t layout_id, uint64_t *out_removed);

/* 15. 重建章节索引（可取消）。成功时更新内容索引版本并失效旧分页缓存；
 *     被取消时返回 NR_ERR_CANCELLED，out_result.partial = 1，且不落库。 */
int32_t nr_rebuild_index(uint64_t engine, uint64_t book_id, uint64_t request_id, nr_rebuild_result *out_result);

/* 19. 手动指定书籍内容编码（规格 CR2「用户手动覆盖」；向后兼容新增）。
 *     encoding_code 取 NR_ENCODING_*（1..=7）；**0 = 清除覆盖**，回到自动检测结果。
 *     书籍必须是当前打开的那一本（与 nr_rebuild_index 同一约定）。
 *     覆盖后立即按新编码重扫章节索引并递增内容索引版本；out_result 可为 NULL。 */
int32_t nr_set_encoding(uint64_t engine, uint64_t book_id, uint32_t encoding_code, nr_set_encoding_result *out_result);

/* 16. 取消请求（协作式）。request_id 必须非 0；对长任务可从另一线程调用。 */
int32_t nr_cancel_request(uint64_t engine, uint64_t request_id);

/* 17. 最近一次错误（错误码 + 可读消息）。message 由 Rust 分配，调用方必须归还。 */
int32_t nr_get_last_error(uint64_t engine, nr_error_info *out_info);

/* 18. 归还 Rust 分配的缓冲区；归还后字段全部置零，重复归还是安全 no-op。
 *     本函数不需要引擎句柄。 */
int32_t nr_release_buffer(nr_buffer *buf);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NOVEL_READER_H */
