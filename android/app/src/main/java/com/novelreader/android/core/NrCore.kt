package com.novelreader.android.core

/**
 * 18 项 C ABI（`include/novel_reader.h`）在 Kotlin 侧的**唯一** JNI 声明处（AN2）。
 *
 * 绑定约定（与 `android/jni/README.md` §1/§2 逐条对应，由
 * `scripts/verify-android-boundary.py` 与 `scripts/verify-abi.py` 双检）：
 *
 * - 包名 / 类名 / 库名已被 JNI 符号名冻结（`Java_com_novelreader_android_core_NrCore_*`）：
 *   移动本文件或改名会让 18 项符号全部无法解析（`UnsatisfiedLinkError`）。
 * - 方法名必须与头文件入口名**逐字相同**；JNI 把方法名里的 `_` 转义为 `_1`
 *   （C 层导出名已按此写死，`scripts/build-android-libs.sh` 校验「转义名 18 + 未转义名 0」）。
 * - 返回 `Int` 的一律是 `NR_ERR_*` 错误码（0 = 成功）。**禁止用错误文本判断业务分支**
 *   （头文件 §2）：错误文本只能经 [nr_get_last_error] 取来展示。
 * - 返回 `ByteArray?` 的四项（本文件第 6/10/11/17 项）：`null` = 调用失败，
 *   **零长度结果是空数组而不是 `null`**，调用失败的具体码经 [nr_get_last_error] 查询。
 * - 字符串一律 `ByteArray`（UTF-8 字节，不依赖 NUL），不使用 `String`。
 * - 出参结构体由调用方预分配 `ByteArray`（尺寸与字段偏移见 README §4）；容量不足返回
 *   `NR_ERR_INVALID_ARGUMENT`，C 层不会越界写。
 * - 64 位：所有 id / 偏移 / 长度都是 `Long`；本文件不得出现承载句柄或偏移的 `Int`，
 *   也不得退化为浮点。
 * - 缓冲区所有权：四个产出缓冲区的入口把 Rust 分配区的 `(ptr, len, capacity)` 写入调用方
 *   `LongArray(3)`；调用方消费后必须**恰好一次**归还（由 [NrBufferHandle] 承担）。
 * - 平台回调（[NrSourceProvider]）由 C 层保存全局引用，回调内**禁止**调用任何 `nr_*` 入口。
 *
 * 本对象只做装箱/拆箱：不含章节、位置、分页、缓存失效或书签业务（子规格 AR2）。
 */
object NrCore {

    init {
        // 自包含 libnr_jni.so（scripts/build-android-libs.sh 产出，非手工提交证据）。
        // 禁止改用 System.load / 绝对路径等方式绕过自包含 .so（子规格「禁止事项」）。
        System.loadLibrary("nr_jni")
    }

    /** 1. ABI 版本号（非错误码）。应与 [NrAbiConstants.NR_ABI_VERSION] 对照。 */
    external fun nr_abi_version(): Int

    /** 2. 创建引擎。**0 = 失败**（与错误码语义不同）。 */
    external fun nr_engine_create(maxWindowBytes: Long, dbPath: ByteArray?, provider: NrSourceProvider?): Long

    /** 3. 销毁引擎。首次 0；重复销毁返回 `NR_ERR_ENGINE_DESTROYED`。 */
    external fun nr_engine_destroy(engine: Long): Int

    /** 4. 打开 TXT；出参结构体 [NrOpenResult.STRUCT_SIZE] 字节。 */
    external fun nr_open_txt(
        engine: Long,
        kind: Int,
        totalBytes: Long,
        identity: ByteArray,
        displayName: ByteArray?,
        outResult: ByteArray,
    ): Int

    /** 5. 关闭当前书籍。 */
    external fun nr_close_book(engine: Long, bookId: Long): Int

    /** 6. 读取有限窗口；返回正文（可能为空数组）；`outBuffer` 承载归还凭据。 */
    external fun nr_read_window(
        engine: Long,
        chapterIndex: Int,
        offset: Long,
        length: Long,
        outWindow: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /** 7. 提交页边界（BEGIN_LAYOUT / COMMIT）；出参 [NrBoundaryResult.STRUCT_SIZE] 字节。 */
    external fun nr_submit_page_boundary(
        engine: Long,
        phase: Int,
        contentIndexVersion: Long,
        layoutId: Long,
        requestId: Long,
        totalBytes: Long,
        startByte: Long,
        endByte: Long,
        layoutFields: LongArray?,
        fontFamily: ByteArray?,
        outResult: ByteArray,
    ): Int

    /** 8. 查询页边界。 */
    external fun nr_get_page_boundary(
        engine: Long,
        layoutId: Long,
        chapterIndex: Int,
        startByte: Long,
        outResult: ByteArray,
    ): Int

    /** 9. 保存阅读位置（幂等 upsert）；`outChanged` 可为 null。 */
    external fun nr_save_position(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        originalByteOffset: Long,
        fingerprint: ByteArray?,
        outChanged: IntArray?,
    ): Int

    /** 10. 读取阅读位置；返回附近文本指纹字节。 */
    external fun nr_get_position(
        engine: Long,
        bookId: Long,
        outResult: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /** 11. 章节批次；返回 `[nr_chapter; count] + 标题 UTF-8` 的批次字节。 */
    external fun nr_get_chapters(
        engine: Long,
        bookId: Long,
        startSequence: Int,
        maxCount: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray?

    /** 12. 新增书签；返回书签 id 经 `outBookmarkId[0]`。 */
    external fun nr_create_bookmark(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        byteOffset: Long,
        title: ByteArray?,
        outBookmarkId: LongArray,
    ): Int

    /** 13. 删除书签；`outDeleted[0]` 回报是否确有删除。 */
    external fun nr_delete_bookmark(engine: Long, bookmarkId: Long, outDeleted: IntArray?): Int

    /** 14. 失效当前排版世代；`outRemoved[0]` 回报清理条数。 */
    external fun nr_invalidate_layout(engine: Long, layoutId: Long, outRemoved: LongArray?): Int

    /** 15. 重建章节索引（可取消）；被取消时仍会写回出参（`partial = 1`）。 */
    external fun nr_rebuild_index(engine: Long, bookId: Long, requestId: Long, outResult: ByteArray): Int
    /** 19. 手动指定内容编码（0 = 清除覆盖、回到自动检测）。 */
    external fun nr_set_encoding(engine: Long, bookId: Long, encodingCode: Int, outResult: ByteArray): Int



    /** 16. 取消请求（协作式）。requestId 必须非 0；可从另一线程调用。 */
    external fun nr_cancel_request(engine: Long, requestId: Long): Int

    /** 17. 最近一次错误（错误码 + 可读消息）；返回消息字节。 */
    external fun nr_get_last_error(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray?

    /** 18. 归还 Rust 分配的缓冲区。`(0,0,0)` 是安全 no-op；重复归还报 `NR_ERR_INVALID_ARGUMENT`。 */
    external fun nr_release_buffer(ptr: Long, len: Long, capacity: Long): Int
}
