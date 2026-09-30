/*
 * novel_reader_probe.h — 共享阅读业务原型 C ABI
 *
 * 技术验证专用（ui-technology-validation），非正式产品接口。
 *
 * 约定
 *  - 字符串一律为 UTF-8 指针 + 长度，不跨边界传语言对象；
 *  - 所有函数返回 NR_ERR_* 错误码（0 为成功）；
 *  - 由 Rust 分配的输出缓冲读取后必须用 nr_release_buffer() 归还；
 *  - 事件回调在触发线程同步执行，data 指针仅在回调期间有效；
 *  - 正文只以有限窗口传输，绝不整体载入。
 *
 * 基线入口：11 个（见 shared/contracts/abi.json），导出 12 个符号；
 * 其余为扩展入口，不属于基线。
 */

#ifndef NOVEL_READER_PROBE_H
#define NOVEL_READER_PROBE_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- 错误码 ---- */
#define NR_ERR_OK                  0
#define NR_ERR_INVALID_ARGUMENT    1
#define NR_ERR_OUT_OF_RANGE        2
#define NR_ERR_UTF8_BOUNDARY       3
#define NR_ERR_BOOK_NOT_FOUND      4
#define NR_ERR_NO_OPEN_BOOK        5
#define NR_ERR_IO                  6
#define NR_ERR_CANCELLED           7
#define NR_ERR_LAYOUT_NOT_FOUND    8
#define NR_ERR_ENGINE_DESTROYED    9
#define NR_ERR_INTERNAL           10
#define NR_ERR_CHAPTER_OUT_OF_RANGE 11

/* ---- 不透明引擎句柄 ---- */
typedef struct nr_engine nr_engine;

/* ---- 由 Rust 分配、调用方归还的缓冲 ---- */
typedef struct nr_buffer {
    uint8_t *ptr;
    size_t   len;
    size_t   capacity;
} nr_buffer;

/* ---- 测量指标 ---- */
typedef struct nr_metrics {
    uint64_t read_calls;
    uint64_t copied_bytes;
    uint64_t materialized_bytes;
    uint64_t last_layout_us;
    uint64_t events_emitted;
    uint64_t cancelled_requests;
    uint64_t page_boundaries;
} nr_metrics;

/* ---- 排版配置 ---- */
typedef struct nr_layout_config {
    float         width;
    float         height;
    float         font_size;
    float         line_spacing;
    float         paragraph_spacing;
    float         first_line_indent;
    float         margin_x;
    float         margin_y;
    const uint8_t *font_family_ptr; /* UTF-8 */
    size_t         font_family_len;
} nr_layout_config;

/* ---- 单页排版结果 ---- */
typedef struct nr_page_layout {
    uint64_t consumed_utf8_bytes;
    uint32_t visible_line_count;
    float    last_line_width;
    float    last_line_height;
    uint64_t layout_time_us;
    uint64_t layout_id;
} nr_page_layout;

/* ---- 页面边界 ---- */
typedef struct nr_page_boundary {
    uint32_t chapter_index;
    uint64_t start_byte;
    uint64_t end_byte;
    uint64_t layout_id;
    uint32_t line_count;
} nr_page_boundary;

/* ---- 阅读位置 ---- */
typedef struct nr_content_position {
    uint64_t book_id;
    uint32_t chapter_index;
    uint64_t byte_offset;
} nr_content_position;

/* ---- 请求进度 ---- */
typedef struct nr_request_progress {
    uint32_t done;
    uint32_t total;
    uint32_t cancelled;
    uint32_t finished;
} nr_request_progress;

/* ---- 事件回调 ---- */
typedef void (*nr_event_callback)(void *user, const uint8_t *json_utf8, size_t json_len);

/* ===================== 基线入口（11 个 / 12 符号） ===================== */

/* 1. 生命周期：创建引擎；max_window_bytes 为 0 时使用默认 256 KiB */
nr_engine *nr_engine_create(uint64_t max_window_bytes);

/* 1. 生命周期：销毁引擎 */
void nr_engine_destroy(nr_engine *e);

/* 2. 打开书籍；path_len 为 0 时使用合成文本源 */
int32_t nr_open_book(nr_engine *e,
                     uint64_t book_id,
                     const uint8_t *path_utf8,
                     size_t path_len,
                     uint64_t total_bytes);

/* 3. 读取有限窗口（章节内偏移），输出合法 UTF-8 字节 */
int32_t nr_read_window(nr_engine *e,
                       uint32_t chapter_index,
                       uint64_t offset,
                       size_t len,
                       nr_buffer *out);

/* 4. 归还缓冲 */
int32_t nr_release_buffer(nr_buffer buf);

/* 5. 书籍批次（UTF-8 JSON） */
int32_t nr_get_book_batch(nr_engine *e, uint64_t start, uint32_t count, nr_buffer *out);

/* 6. 章节批次（UTF-8 JSON） */
int32_t nr_get_chapter_batch(nr_engine *e,
                             uint64_t book_id,
                             uint32_t start,
                             uint32_t count,
                             nr_buffer *out);

/* 7. 保存阅读位置 */
int32_t nr_save_position(nr_engine *e,
                         uint64_t book_id,
                         uint32_t chapter_index,
                         uint64_t byte_offset);

/* 8. 登记页面边界（任一端排版结果均可登记） */
int32_t nr_commit_page_boundary(nr_engine *e,
                                uint32_t chapter_index,
                                uint64_t start_byte,
                                uint64_t end_byte,
                                uint64_t layout_id,
                                uint32_t line_count);

/* 9. 失效排版缓存；out_removed 可为 NULL */
int32_t nr_invalidate_layout(nr_engine *e, uint64_t layout_id, uint64_t *out_removed);

/* 10. 订阅事件，返回订阅 id（负数表示错误码） */
int64_t nr_subscribe_events(nr_engine *e, nr_event_callback cb, void *user);

/* 11. 取消请求（协作式，部分成功语义） */
int32_t nr_cancel_request(nr_engine *e, uint64_t request_id);

/* ===================== 扩展入口（非基线） ===================== */

uint32_t nr_abi_version(void);

/* 共享排版，与桌面端直接 Rust API 语义等价 */
int32_t nr_layout_page(const nr_layout_config *cfg,
                       const uint8_t *text_utf8,
                       size_t text_len,
                       uint64_t start_byte,
                       nr_page_layout *out);

int32_t nr_get_metrics(nr_engine *e, nr_metrics *out);

/* 最近一次错误消息（"CODE|message"） */
int32_t nr_last_error_message(nr_engine *e, nr_buffer *out);

int32_t nr_start_batch_request(nr_engine *e, uint32_t count, uint64_t *out_request_id);

int32_t nr_advance_request(nr_engine *e,
                           uint64_t request_id,
                           uint32_t steps,
                           nr_request_progress *out);

/* 引擎快照（UTF-8 JSON） */
int32_t nr_engine_snapshot_json(nr_engine *e, nr_buffer *out);

/* 打开外部内容：共享库只登记「单章、坐标空间长度 = total_bytes」，
 * 字节由平台侧按有限窗口提供（例如 Android SAF 选中的文件）。
 * 页边界、位置与锚点语义仍由共享库裁定。 */
int32_t nr_open_external_book(nr_engine *e, uint64_t book_id, uint64_t total_bytes);

#ifdef __cplusplus
}
#endif

#endif /* NOVEL_READER_PROBE_H */
