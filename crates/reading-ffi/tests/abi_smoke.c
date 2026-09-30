/*
 * abi_smoke.c — C 侧 ABI 冒烟与布局断言（RC7 契约证据）
 *
 * 编译并运行（在仓库根目录）：
 *   cc -std=c11 -Wall -Wextra -I include crates/reading-ffi/tests/abi_smoke.c \
 *      -L target/release -lreading_ffi -lpthread -ldl -lm -o /tmp/nr_abi_smoke
 *   LD_LIBRARY_PATH=target/release /tmp/nr_abi_smoke
 *
 * 断言内容：
 *   1. 编译期 ABI 版本断言（_Static_assert）——头文件宏与调用方约定一致；
 *   2. 结构体字节大小与 Rust 侧 tests/abi_layout.rs 的 EXPECTED_SIZES **同一组数字**；
 *   3. 运行期完整链路：创建引擎 → 打开 TXT → 读窗口 → 归还缓冲区 →
 *      提交/查询页边界 → 保存/读取位置 → 章节批次 → 书签 → 销毁引擎；
 *   4. 错误码契约：非法参数、销毁后句柄、越界窗口都必须返回冻结的错误码；
 *   5. 缓冲区所有权：归还后字段置零、重复归还安全。
 *
 * 退出码：0 = 全部通过；非 0 = 断言失败（打印失败项）。
 */

#include "novel_reader.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h> /* getpid()：唯一 POSIX 依赖（临时文件路径去重） */

/* ---- 1. 编译期冻结断言 ---- */
_Static_assert(NR_ABI_VERSION == 1u, "ABI 版本必须是 1");
_Static_assert(NR_DEFAULT_WINDOW_BYTES == 65536u, "默认窗口必须是 64 KiB");
_Static_assert(NR_MAX_WINDOW_BYTES == 262144u, "窗口硬上限必须是 256 KiB");
_Static_assert(NR_ERR_CHAPTER_OUT_OF_RANGE == 11, "错误码 11 必须是章节越界");
_Static_assert(sizeof(nr_buffer) == 24, "nr_buffer 必须是 24 字节");
_Static_assert(sizeof(nr_source_provider) == 16, "nr_source_provider 必须是 16 字节");
_Static_assert(sizeof(nr_engine_config) == 40, "nr_engine_config 必须是 40 字节");
_Static_assert(sizeof(nr_open_request) == 48, "nr_open_request 必须是 48 字节");
_Static_assert(sizeof(nr_open_result) == 56, "nr_open_result 必须是 56 字节");
_Static_assert(sizeof(nr_window) == 48, "nr_window 必须是 48 字节");
_Static_assert(sizeof(nr_layout_signature) == 72, "nr_layout_signature 必须是 72 字节");
_Static_assert(sizeof(nr_page_boundary_submission) == 64, "提交结构体必须是 64 字节");
_Static_assert(sizeof(nr_boundary_result) == 80, "页边界结果必须是 80 字节");
_Static_assert(sizeof(nr_position) == 48, "nr_position 必须是 48 字节");
_Static_assert(sizeof(nr_position_result) == 80, "位置结果必须是 80 字节");
_Static_assert(sizeof(nr_chapter) == 40, "nr_chapter 必须是 40 字节");
_Static_assert(sizeof(nr_chapter_batch) == 40, "章节批次必须是 40 字节");
_Static_assert(sizeof(nr_bookmark_input) == 48, "书签输入必须是 48 字节");
_Static_assert(sizeof(nr_rebuild_result) == 40, "重建结果必须是 40 字节");
_Static_assert(sizeof(nr_error_info) == 32, "错误信息必须是 32 字节");

static int failures = 0;

static void check(int condition, const char *label) {
    if (condition) {
        printf("  PASS  %s\n", label);
    } else {
        printf("  FAIL  %s\n", label);
        failures += 1;
    }
}

static const char *BODY =
    "第一章 起点\n"
    "这是第一章的正文，用于验证 C 侧 ABI 的窗口读取与章节批次。\n"
    "第二章 转折\n"
    "这是第二章的正文，窗口必须落在合法 UTF-8 边界上。\n"
    "第三章 收束\n"
    "这是第三章的正文，读到文件末尾时必须回报 EOF。\n";

static char *write_temp_book(void) {
    char *path = (char *)malloc(256);
    if (path == NULL) {
        return NULL;
    }
    snprintf(path, 256, "/tmp/nr_abi_smoke_%d.txt", (int)getpid());
    FILE *file = fopen(path, "wb");
    if (file == NULL) {
        free(path);
        return NULL;
    }
    fwrite(BODY, 1, strlen(BODY), file);
    fclose(file);
    return path;
}

int main(void) {
    printf("nr_abi_smoke: 头文件 ABI 版本 = %u，运行期版本 = %u\n", NR_ABI_VERSION, nr_abi_version());
    check(nr_abi_version() == NR_ABI_VERSION, "运行期 ABI 版本与编译期宏一致");

    char *path = write_temp_book();
    if (path == NULL) {
        printf("FAIL  无法创建临时书籍文件\n");
        return 2;
    }

    nr_engine_config config;
    memset(&config, 0, sizeof(config));
    config.struct_size = (uint32_t)sizeof(config);
    config.abi_version = NR_ABI_VERSION;
    config.max_window_bytes = 0; /* 使用硬上限 */

    uint64_t engine = 0;
    check(nr_engine_create(&config, &engine) == NR_ERR_OK, "创建引擎成功");
    check(engine != 0, "句柄为非 0");

    nr_open_request request;
    memset(&request, 0, sizeof(request));
    request.struct_size = (uint32_t)sizeof(request);
    request.kind = NR_SOURCE_KIND_FILE;
    request.identity_utf8 = (const uint8_t *)path;
    request.identity_len = strlen(path);

    nr_open_result opened;
    memset(&opened, 0, sizeof(opened));
    check(nr_open_txt(engine, &request, &opened) == NR_ERR_OK, "打开 TXT 成功");
    check(opened.total_bytes == strlen(BODY), "总字节数与文件一致");
    check(opened.content_unit_id == NR_CONTENT_UNIT_SINGLE, "内容单元标识为 1");
    check(opened.encoding == NR_ENCODING_UTF8, "识别为 UTF-8");
    check(opened.chapters >= 3, "章节数 ≥ 3");

    /* 读取窗口：长度受硬上限约束，返回字节必须是合法 UTF-8。 */
    nr_window window;
    memset(&window, 0, sizeof(window));
    nr_buffer bytes;
    memset(&bytes, 0, sizeof(bytes));
    check(nr_read_window(engine, 0, 0, NR_DEFAULT_WINDOW_BYTES, &window, &bytes) == NR_ERR_OK,
          "读取窗口成功");
    check(window.len == strlen(BODY), "小文件一次读完");
    check(window.eof == 1, "读到末尾回报 EOF");
    check(bytes.len == window.len && bytes.ptr != NULL, "缓冲区长度与窗口一致");
    check(memcmp(bytes.ptr, BODY, window.len) == 0, "窗口字节与文件内容一致");
    check(nr_release_buffer(&bytes) == NR_ERR_OK, "归还缓冲区成功");
    check(bytes.ptr == NULL && bytes.len == 0 && bytes.capacity == 0, "归还后缓冲区字段置零");
    check(nr_release_buffer(&bytes) == NR_ERR_OK, "重复归还为安全 no-op");

    /* 超过硬上限的请求必须被拒绝。 */
    memset(&window, 0, sizeof(window));
    memset(&bytes, 0, sizeof(bytes));
    check(nr_read_window(engine, 0, 0, NR_MAX_WINDOW_BYTES + 1u, &window, &bytes) ==
              NR_ERR_INVALID_ARGUMENT,
          "超出硬上限的窗口请求被拒绝");
    check(bytes.ptr == NULL, "被拒绝的请求不得分配缓冲区");
    check(nr_read_window(engine, 99, 0, 16, &window, &bytes) == NR_ERR_CHAPTER_OUT_OF_RANGE,
          "越界章节返回 CHAPTER_OUT_OF_RANGE");

    /* 错误详情。 */
    nr_error_info info;
    memset(&info, 0, sizeof(info));
    check(nr_get_last_error(engine, &info) == NR_ERR_OK, "读取最近错误成功");
    check(info.code == NR_ERR_CHAPTER_OUT_OF_RANGE && info.has_message == 1,
          "最近错误码与消息可用");
    check(nr_release_buffer(&info.message) == NR_ERR_OK, "归还错误消息缓冲区成功");

    /* 页边界：登记排版 → 提交 → 查询 → 失效。 */
    const char *family = "Noto Sans CJK SC";
    nr_layout_signature signature;
    memset(&signature, 0, sizeof(signature));
    signature.viewport_width = 1080;
    signature.viewport_height = 1920;
    signature.direction = 0;
    signature.font_weight = 400;
    signature.font_size_milli = 20000;
    signature.line_spacing_milli = 8000;
    signature.paragraph_spacing_milli = 12000;
    signature.first_line_indent_milli = 40000;
    signature.margin_horizontal_milli = 24000;
    signature.margin_vertical_milli = 24000;
    signature.algorithm_version = NR_LAYOUT_ALGORITHM_VERSION;
    signature.platform_font_fingerprint = 0;
    signature.font_family_utf8 = (const uint8_t *)family;
    signature.font_family_len = strlen(family);

    nr_page_boundary_submission submission;
    memset(&submission, 0, sizeof(submission));
    submission.struct_size = (uint32_t)sizeof(submission);
    submission.phase = NR_SUBMIT_BEGIN_LAYOUT;
    submission.content_index_version = opened.content_index_version;
    submission.signature = &signature;

    nr_boundary_result boundary;
    memset(&boundary, 0, sizeof(boundary));
    check(nr_submit_page_boundary(engine, &submission, &boundary) == NR_ERR_OK,
          "BEGIN_LAYOUT 成功");
    check(boundary.status == NR_BOUNDARY_LAYOUT_READY && boundary.layout_id > 0 &&
              boundary.request_id > 0,
          "返回 layout_id 与 request_id");

    uint64_t first_line = 1;
    while (first_line < strlen(BODY) && BODY[first_line - 1] != '\n') {
        first_line += 1;
    }
    submission.phase = NR_SUBMIT_COMMIT;
    submission.layout_id = boundary.layout_id;
    submission.request_id = boundary.request_id;
    submission.total_bytes = opened.total_bytes;
    submission.start_byte = 0;
    submission.end_byte = first_line;
    submission.signature = NULL;
    check(nr_submit_page_boundary(engine, &submission, &boundary) == NR_ERR_OK,
          "COMMIT 成功");
    check(boundary.status == NR_BOUNDARY_COMMITTED && boundary.page_count == 1,
          "边界已登记且页数为 1");

    uint64_t layout_id = boundary.layout_id;
    check(nr_get_page_boundary(engine, layout_id, 0, 0, &boundary) == NR_ERR_OK &&
              boundary.end_byte == first_line,
          "查询命中的页边界");
    check(nr_get_page_boundary(engine, layout_id, 0, 1, &boundary) == NR_ERR_LAYOUT_NOT_FOUND,
          "查询未命中的起点返回 LAYOUT_NOT_FOUND");

    uint64_t removed = 0;
    check(nr_invalidate_layout(engine, layout_id, &removed) == NR_ERR_OK && removed == 1,
          "失效当前世代并删除 1 条边界");
    check(nr_invalidate_layout(engine, 424242, &removed) == NR_ERR_LAYOUT_NOT_FOUND,
          "失效未登记的世代返回 LAYOUT_NOT_FOUND");

    /* 位置：保存 → 读取。 */
    nr_position position;
    memset(&position, 0, sizeof(position));
    position.struct_size = (uint32_t)sizeof(position);
    position.chapter_index = 0;
    position.book_id = opened.book_id;
    position.content_unit_id = NR_CONTENT_UNIT_SINGLE;
    position.original_byte_offset = 16;
    int32_t changed = 0;
    check(nr_save_position(engine, &position, &changed) == NR_ERR_OK && changed == 1,
          "首次保存位置写入成功");
    check(nr_save_position(engine, &position, &changed) == NR_ERR_OK && changed == 0,
          "重复保存位置幂等");

    nr_position_result stored;
    memset(&stored, 0, sizeof(stored));
    check(nr_get_position(engine, opened.book_id, &stored) == NR_ERR_OK && stored.found == 1,
          "读取位置成功");
    check(stored.original_byte_offset == 16, "读取到的偏移与保存一致");
    check(stored.restored == 1, "打开同一本书时执行了锚点恢复");
    check(nr_release_buffer(&stored.nearby_fingerprint) == NR_ERR_OK, "归还指纹缓冲区成功");

    /* 章节批次：解析缓冲区里的定长数组与标题偏移。 */
    nr_chapter_batch batch;
    memset(&batch, 0, sizeof(batch));
    check(nr_get_chapters(engine, opened.book_id, 0, 2, &batch) == NR_ERR_OK, "读取章节批次成功");
    check(batch.count == 2, "批次按 max_count 截断");
    if (batch.count == 2) {
        const nr_chapter *chapters = (const nr_chapter *)batch.bytes.ptr;
        check(chapters[0].sequence == 0 && chapters[1].sequence == 1, "章节序号连续");
        check(chapters[0].end_byte > chapters[0].start_byte, "章节区间非空");
        const char *title = (const char *)batch.bytes.ptr + chapters[0].title_offset;
        check(chapters[0].title_offset + chapters[0].title_len <= batch.bytes.len,
              "标题偏移落在缓冲区范围内");
        check(chapters[0].title_len > 0 && strncmp(title, "第一章", strlen("第一章")) == 0,
              "标题字节可读且内容正确");
    }
    check(nr_release_buffer(&batch.bytes) == NR_ERR_OK, "归还章节批次缓冲区成功");
    check(nr_get_chapters(engine, opened.book_id, 0, NR_MAX_CHAPTER_BATCH + 1u, &batch) ==
              NR_ERR_INVALID_ARGUMENT,
          "超过批次上限被拒绝");

    /* 书签。 */
    nr_bookmark_input bookmark;
    memset(&bookmark, 0, sizeof(bookmark));
    bookmark.struct_size = (uint32_t)sizeof(bookmark);
    bookmark.chapter_index = 0;
    bookmark.book_id = opened.book_id;
    bookmark.content_unit_id = NR_CONTENT_UNIT_SINGLE;
    bookmark.byte_offset = 0;
    bookmark.title_utf8 = NULL;
    bookmark.title_len = 0;
    uint64_t bookmark_id = 0;
    check(nr_create_bookmark(engine, &bookmark, &bookmark_id) == NR_ERR_OK && bookmark_id > 0,
          "新增书签成功（空标题使用章节标题）");
    int32_t deleted = 0;
    check(nr_delete_bookmark(engine, bookmark_id, &deleted) == NR_ERR_OK && deleted == 1,
          "删除书签成功");

    /* 取消：request_id 0 必须被拒绝。 */
    check(nr_cancel_request(engine, 0) == NR_ERR_INVALID_ARGUMENT, "取消 request_id 0 被拒绝");

    /* 关闭与销毁。 */
    check(nr_close_book(engine, opened.book_id) == NR_ERR_OK, "关闭书籍成功");
    check(nr_read_window(engine, 0, 0, 16, &window, &bytes) == NR_ERR_NO_OPEN_BOOK,
          "关闭后读取返回 NO_OPEN_BOOK");
    check(nr_engine_destroy(engine) == NR_ERR_OK, "销毁引擎成功");
    check(nr_engine_destroy(engine) == NR_ERR_ENGINE_DESTROYED, "重复销毁返回 ENGINE_DESTROYED");
    check(nr_engine_destroy(0) == NR_ERR_INVALID_ARGUMENT, "句柄 0 返回 INVALID_ARGUMENT");
    check(nr_release_buffer(NULL) == NR_ERR_INVALID_ARGUMENT, "NULL 缓冲区返回 INVALID_ARGUMENT");

    remove(path);
    free(path);

    printf("nr_abi_smoke: %s（%d 项失败）\n", failures == 0 ? "PASS" : "FAIL", failures);
    return failures == 0 ? 0 : 1;
}
