package com.novelreader.android.core

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * 出参结构体的字节布局解码（小端）。
 *
 * 布局是 C 层 `_Static_assert` 冻结的 `#[repr(C)]` 结构体（`android/jni/nr_jni.c` 顶部），
 * 字段偏移与尺寸见 `android/jni/README.md` §4。本文件只做**装箱/拆箱**：
 * - `u32` 一律解成 `Long`（`and 0xFFFFFFFFL`），不出现 32 位截断或浮点退化；
 * - `i32`（仅 `nr_error_info.code`）解成 `Int`；
 * - 结构体与调用方预分配数组尺寸不符属于内部协议违例，按 `NR_ERR_INTERNAL` 拒绝。
 */
internal object NrStructs {

    /** 校验尺寸并返回小端视图。 */
    fun buffer(bytes: ByteArray, expectedSize: Int, what: String): ByteBuffer {
        if (bytes.size != expectedSize) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "$what 结构体应为 $expectedSize 字节，实际 ${bytes.size}",
            )
        }
        return ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
    }
}

/** `u32` 字段按无符号解成 `Long`：不做 32 位截断。 */
internal fun ByteBuffer.u32(offset: Int): Long = getInt(offset).toLong() and 0xFFFFFFFFL

/** `nr_open_result`：打开 TXT 的结果（56 字节）。 */
data class NrOpenResult(
    val bookId: Long,
    val totalBytes: Long,
    val contentIndexVersion: Long,
    val contentUnitId: Long,
    val encoding: Long,
    val encodingConfidence: Long,
    val encodingScore: Long,
    val needsEncodingConfirmation: Long,
    val chapters: Long,
    val indexFromCache: Long,
) {
    companion object {
        /** 与 `sizeof(nr_open_result)` 一致（C 层编译期冻结）。 */
        const val STRUCT_SIZE = 56

        internal fun decode(bytes: ByteArray): NrOpenResult {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_open_result")
            return NrOpenResult(
                bookId = buf.getLong(0),
                totalBytes = buf.getLong(8),
                contentIndexVersion = buf.getLong(16),
                contentUnitId = buf.u32(24),
                encoding = buf.u32(28),
                encodingConfidence = buf.u32(32),
                encodingScore = buf.u32(36),
                needsEncodingConfirmation = buf.u32(40),
                chapters = buf.u32(44),
                indexFromCache = buf.u32(48),
            )
            // 偏移 52 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }
}

/** `nr_window`：有限窗口读取的元数据（48 字节）。`len` 与归还凭据的 `len` 必须一致。 */
data class NrWindow(
    val offset: Long,
    val endByte: Long,
    val requestId: Long,
    val totalBytes: Long,
    val len: Long,
    val droppedLeading: Long,
    val trimmedTail: Long,
    val eof: Long,
) {
    companion object {
        const val STRUCT_SIZE = 48

        internal fun decode(bytes: ByteArray): NrWindow {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_window")
            return NrWindow(
                offset = buf.getLong(0),
                endByte = buf.getLong(8),
                requestId = buf.getLong(16),
                totalBytes = buf.getLong(24),
                len = buf.u32(32),
                droppedLeading = buf.u32(36),
                trimmedTail = buf.u32(40),
                eof = buf.u32(44),
            )
        }
    }
}

/** `nr_boundary_result`：页边界提交 / 查询结果（80 字节）。 */
data class NrBoundaryResult(
    val status: Long,
    val rejection: Long,
    val errorCode: Int,
    val layoutId: Long,
    val requestId: Long,
    val staleLayoutId: Long,
    val staleRequestId: Long,
    val startByte: Long,
    val endByte: Long,
    val totalBytes: Long,
    val pageCount: Long,
) {
    companion object {
        const val STRUCT_SIZE = 80

        internal fun decode(bytes: ByteArray): NrBoundaryResult {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_boundary_result")
            return NrBoundaryResult(
                status = buf.u32(0),
                rejection = buf.u32(4),
                errorCode = buf.getInt(8),
                layoutId = buf.getLong(16),
                requestId = buf.getLong(24),
                staleLayoutId = buf.getLong(32),
                staleRequestId = buf.getLong(40),
                startByte = buf.getLong(48),
                endByte = buf.getLong(56),
                totalBytes = buf.getLong(64),
                pageCount = buf.getLong(72),
            )
            // 偏移 12 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }
}

/**
 * `nr_position_result`：阅读位置（80 字节）。
 *
 * [nearbyFingerprintPtr] / [len] / [capacity] 是结构体内嵌的归还槽位，与 `outBuffer`
 * 是**同一份值**：只用于与凭据交叉断言，**不得**据此单独归还（会二次释放）。
 */
data class NrPositionResult(
    val structSize: Long,
    val found: Long,
    val chapterIndex: Long,
    val restored: Long,
    val fallbackToChapterStart: Long,
    val bookId: Long,
    val contentUnitId: Long,
    val originalByteOffset: Long,
    val updatedAt: Long,
    val nearbyFingerprintPtr: Long,
    val nearbyFingerprintLen: Long,
    val nearbyFingerprintCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 80

        internal fun decode(bytes: ByteArray): NrPositionResult {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_position_result")
            return NrPositionResult(
                structSize = buf.u32(0),
                found = buf.u32(4),
                chapterIndex = buf.u32(8),
                restored = buf.u32(12),
                fallbackToChapterStart = buf.u32(16),
                bookId = buf.getLong(24),
                contentUnitId = buf.getLong(32),
                originalByteOffset = buf.getLong(40),
                updatedAt = buf.getLong(48),
                nearbyFingerprintPtr = buf.getLong(56),
                nearbyFingerprintLen = buf.getLong(64),
                nearbyFingerprintCapacity = buf.getLong(72),
            )
            // 偏移 20 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }
}

/**
 * `nr_chapter_batch`：章节批次元数据（40 字节）。
 *
 * [bytesPtr] / [bytesLen] / [bytesCapacity] 是结构体内嵌的归还槽位（与 `outBuffer` 同一份值）。
 */
data class NrChapterBatch(
    val count: Long,
    val indexVersion: Long,
    val totalChapters: Long,
    val bytesPtr: Long,
    val bytesLen: Long,
    val bytesCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 40

        internal fun decode(bytes: ByteArray): NrChapterBatch {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_chapter_batch")
            return NrChapterBatch(
                count = buf.u32(0),
                indexVersion = buf.u32(4),
                totalChapters = buf.u32(8),
                bytesPtr = buf.getLong(16),
                bytesLen = buf.getLong(24),
                bytesCapacity = buf.getLong(32),
            )
            // 偏移 12 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }
}

/** `nr_rebuild_result`：索引重建结果（40 字节）。`partial = 1` 表示被取消、未落库。 */
data class NrRebuildResult(
    val structSize: Long,
    val partial: Long,
    val chapters: Long,
    val indexFromCache: Long,
    val scannedBytes: Long,
    val totalBytes: Long,
    val contentIndexVersion: Long,
) {
    companion object {
        const val STRUCT_SIZE = 40

        internal fun decode(bytes: ByteArray): NrRebuildResult {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_rebuild_result")
            return NrRebuildResult(
                structSize = buf.u32(0),
                partial = buf.u32(4),
                chapters = buf.u32(8),
                indexFromCache = buf.u32(12),
                scannedBytes = buf.getLong(16),
                totalBytes = buf.getLong(24),
                contentIndexVersion = buf.getLong(32),
            )
        }
    }
}

/**
 * `nr_error_info`：最近一次错误（32 字节）。
 *
 * [messagePtr] / [messageLen] / [messageCapacity] 是结构体内嵌的归还槽位（与 `outBuffer` 同一份值）。
 * [code] 只作为数值与展示使用 —— 判断业务分支一律走 [NrErrorKind.of]。
 */
/**
 * `nr_set_encoding_result`（手动指定内容编码）。
 *
 * 布局（C `#pragma pack` 默认对齐）：struct_size u32 @0、encoding u32 @4、
 * content_index_version u64 @8、chapters u32 @16、index_rebuilt u32 @20 ⇒ 共 24 字节。
 */
data class NrSetEncodingResult(
    val structSize: Int,
    val encoding: Int,
    val contentIndexVersion: Long,
    val chapters: Int,
    val indexRebuilt: Boolean,
) {
    companion object {
        const val STRUCT_SIZE = 24

        internal fun decode(bytes: ByteArray): NrSetEncodingResult {
            val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
            return NrSetEncodingResult(
                structSize = buf.getInt(0),
                encoding = buf.getInt(4),
                contentIndexVersion = buf.getLong(8),
                chapters = buf.getInt(16),
                indexRebuilt = buf.getInt(20) != 0,
            )
        }
    }
}

data class NrErrorInfo(
    val code: Int,
    val hasMessage: Long,
    val messagePtr: Long,
    val messageLen: Long,
    val messageCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 32

        internal fun decode(bytes: ByteArray): NrErrorInfo {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_error_info")
            return NrErrorInfo(
                code = buf.getInt(0),
                hasMessage = buf.u32(4),
                messagePtr = buf.getLong(8),
                messageLen = buf.getLong(16),
                messageCapacity = buf.getLong(24),
            )
        }
    }
}

/** 章节条目（批次字节里的定长部分，40 字节；无指针、无正文字段）。 */
data class NrChapterEntry(
    val sequence: Long,
    val ruleId: Long,
    val startByte: Long,
    val endByte: Long,
    val titleOffset: Long,
    val titleLen: Long,
    val indexVersion: Long,
) {
    companion object {
        const val STRUCT_SIZE = 40
    }
}

/**
 * 章节批次字节的解码（README §4）：
 * `[nr_chapter; count]` 之后紧接全部标题的 UTF-8 字节，`titleOffset` 相对批次字节起点。
 *
 * 这里只有布局解码与边界校验，不判断章节语义（章节识别规则属于 Rust 核心）。
 */
object NrChapterCodec {

    /** 解码 `count` 个章节条目；越界按 `NR_ERR_INTERNAL` 拒绝（跨边界布局已错位）。 */
    fun entries(bytes: ByteArray, count: Long): List<NrChapterEntry> {
        if (count < 0 || count > Int.MAX_VALUE) {
            throw NrException(NrErrors.NR_ERR_INTERNAL, message = "章节条目数非法：$count")
        }
        val needed = count * NrChapterEntry.STRUCT_SIZE
        if (needed > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "章节批次字节过短：需要 $needed 字节，实际 ${bytes.size}",
            )
        }
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        return (0 until count.toInt()).map { index ->
            val base = index * NrChapterEntry.STRUCT_SIZE
            NrChapterEntry(
                sequence = buf.u32(base),
                ruleId = buf.u32(base + 4),
                startByte = buf.getLong(base + 8),
                endByte = buf.getLong(base + 16),
                titleOffset = buf.u32(base + 24),
                titleLen = buf.u32(base + 28),
                indexVersion = buf.u32(base + 32),
            )
        }
    }

    /** 取出某个章节的标题字节（UTF-8）；越界按 `NR_ERR_INTERNAL` 拒绝。 */
    fun title(bytes: ByteArray, entry: NrChapterEntry): ByteArray {
        val start = entry.titleOffset
        val end = start + entry.titleLen
        if (start < 0 || end > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "章节标题越界：[$start, $end) 超出批次字节 ${bytes.size}",
            )
        }
        return bytes.copyOfRange(start.toInt(), end.toInt())
    }
}
