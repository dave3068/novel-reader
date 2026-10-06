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


// ===========================================================================
// V1.1 四组功能的出参解码（分类 / 阅读历史 / 配色 / 背景）
//
// 口径与上文一致：**只做装箱拆箱**，不判断任何业务语义 ——
// 排序、同名、循环、派生、对比度、时间格式化全在 Rust 核心（规格 §3.6）。
// 所有批次都是「定长行在头部 + UTF-8 字节池紧随其后」，偏移**相对整块缓冲区**。
// ===========================================================================

/** 分类条目（批次字节里的定长部分，48 字节；无指针、无名称字段）。 */
data class NrCategoryEntry(
    val id: Long,
    val parentId: Long,
    val directBookCount: Long,
    val totalBookCount: Long,
    val depth: Long,
    val nameOffset: Long,
    val nameLen: Long,
) {
    companion object {
        const val STRUCT_SIZE = 48

        /** `parent_id = -1` 表示根下（无父分类）。 */
        const val PARENT_NONE = -1L
    }
}

/** `nr_category_batch`：分类批次元数据（32 字节）。 */
data class NrCategoryBatch(
    val count: Long,
    val bytesPtr: Long,
    val bytesLen: Long,
    val bytesCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 32

        internal fun decode(bytes: ByteArray): NrCategoryBatch {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_category_batch")
            return NrCategoryBatch(
                count = buf.u32(0),
                bytesPtr = buf.getLong(8),
                bytesLen = buf.getLong(16),
                bytesCapacity = buf.getLong(24),
            )
            // 偏移 4 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }
}

/**
 * 分类批次字节的解码：`[nr_category; count]` 之后紧接全部名称的 UTF-8 字节。
 *
 * 树形结构**不在这里拼**：核心回的是扁平列表（含 `parentId` / `depth`），
 * 侧栏由 UI 层按 `parentId` 组装 —— 层级判定仍在核心（`depth` 由核心算）。
 */
object NrCategoryCodec {

    /** 解码 `count` 个分类条目；越界按 `NR_ERR_INTERNAL` 拒绝（布局已错位）。 */
    fun entries(bytes: ByteArray, count: Long): List<NrCategoryEntry> {
        if (count < 0 || count > Int.MAX_VALUE) {
            throw NrException(NrErrors.NR_ERR_INTERNAL, message = "分类条目数非法：$count")
        }
        val needed = count * NrCategoryEntry.STRUCT_SIZE
        if (needed > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "分类批次字节过短：需要 $needed 字节，实际 ${bytes.size}",
            )
        }
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        return (0 until count.toInt()).map { index ->
            val base = index * NrCategoryEntry.STRUCT_SIZE
            NrCategoryEntry(
                id = buf.getLong(base),
                parentId = buf.getLong(base + 8),
                directBookCount = buf.getLong(base + 16),
                totalBookCount = buf.getLong(base + 24),
                depth = buf.u32(base + 32),
                nameOffset = buf.u32(base + 36),
                nameLen = buf.u32(base + 40),
            )
            // 偏移 44 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }

    /** 取出某个分类的名称（UTF-8）；越界按 `NR_ERR_INTERNAL` 拒绝。 */
    fun name(bytes: ByteArray, entry: NrCategoryEntry): String =
        NrBatchText.slice(bytes, entry.nameOffset, entry.nameLen, "分类名称")
}

/** 阅读历史一行（批次字节里的定长部分，72 字节）。 */
data class NrHistoryEntry(
    val id: Long,
    val bookId: Long,
    val openedAt: Long,
    val chapterIndex: Long,
    val byteOffset: Long,
    val entryKind: Long,
    val titleOffset: Long,
    val titleLen: Long,
    val fileOffset: Long,
    val fileLen: Long,
    /** 时间标签（**已由核心格式化**）在字节池中的偏移 / 长度。 */
    val labelOffset: Long,
    val labelLen: Long,
) {
    companion object {
        const val STRUCT_SIZE = 72
    }
}

/** `nr_history_batch`：历史批次元数据（32 字节）。`total` 不受 `limit` 影响。 */
data class NrHistoryBatch(
    val count: Long,
    val total: Long,
    val bytesPtr: Long,
    val bytesLen: Long,
    val bytesCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 32

        internal fun decode(bytes: ByteArray): NrHistoryBatch {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_history_batch")
            return NrHistoryBatch(
                count = buf.u32(0),
                total = buf.u32(4),
                bytesPtr = buf.getLong(8),
                bytesLen = buf.getLong(16),
                bytesCapacity = buf.getLong(24),
            )
        }
    }
}

/**
 * 历史批次字节的解码：`[nr_history; count]` + 每行三段（书名 · 文件名 · 时间标签）。
 *
 * [timeLabel] 直接取核心给的成品文本 —— **界面不得再格式化或截断**（规格 §3.6）。
 */
object NrHistoryCodec {

    fun entries(bytes: ByteArray, count: Long): List<NrHistoryEntry> {
        if (count < 0 || count > Int.MAX_VALUE) {
            throw NrException(NrErrors.NR_ERR_INTERNAL, message = "历史条目数非法：$count")
        }
        val needed = count * NrHistoryEntry.STRUCT_SIZE
        if (needed > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "历史批次字节过短：需要 $needed 字节，实际 ${bytes.size}",
            )
        }
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        return (0 until count.toInt()).map { index ->
            val base = index * NrHistoryEntry.STRUCT_SIZE
            NrHistoryEntry(
                id = buf.getLong(base),
                bookId = buf.getLong(base + 8),
                openedAt = buf.getLong(base + 16),
                chapterIndex = buf.getLong(base + 24),
                byteOffset = buf.getLong(base + 32),
                entryKind = buf.getLong(base + 40),
                titleOffset = buf.u32(base + 48),
                titleLen = buf.u32(base + 52),
                fileOffset = buf.u32(base + 56),
                fileLen = buf.u32(base + 60),
                labelOffset = buf.u32(base + 64),
                labelLen = buf.u32(base + 68),
            )
        }
    }

    /** 书名（UTF-8）。 */
    fun title(bytes: ByteArray, entry: NrHistoryEntry): String =
        NrBatchText.slice(bytes, entry.titleOffset, entry.titleLen, "历史书名")

    /** 文件名（核心已取末段，不含路径分隔符）。 */
    fun fileLabel(bytes: ByteArray, entry: NrHistoryEntry): String =
        NrBatchText.slice(bytes, entry.fileOffset, entry.fileLen, "历史文件名")

    /** 时间标签 —— **核心格式化的成品文本**（今天 `HH:MM` / 本年 `MM-DD HH:MM` / 跨年 `yyyy-MM-dd HH:MM`）。 */
    fun timeLabel(bytes: ByteArray, entry: NrHistoryEntry): String =
        NrBatchText.slice(bytes, entry.labelOffset, entry.labelLen, "历史时间标签")
}

/** `nr_theme_base`：四个基色 + 启用位（56 字节）。**40 个语义令牌不落库**。 */
data class NrThemeBase(
    val structSize: Long,
    val enabled: Boolean,
    val foreground: String,
    val background: String,
    val primary: String,
    val accent: String,
) {
    companion object {
        const val STRUCT_SIZE = 56
        private const val SLOT = 12

        internal fun decode(bytes: ByteArray): NrThemeBase {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_theme_base")
            return NrThemeBase(
                structSize = buf.u32(0),
                enabled = buf.u32(4) != 0L,
                foreground = slot(buf, 8),
                background = slot(buf, 20),
                primary = slot(buf, 32),
                accent = slot(buf, 44),
            )
        }

        /** 12 字节定长槽 → 字符串；**全 0（无 NUL 前的字节）表示该项未设置**（空串）。 */
        private fun slot(buf: ByteBuffer, offset: Int): String {
            val raw = ByteArray(SLOT)
            buf.position(offset)
            buf.get(raw, 0, SLOT)
            val end = raw.indexOf(0.toByte()).let { if (it < 0) SLOT else it }
            return String(raw, 0, end, Charsets.UTF_8)
        }
    }
}

/** `nr_theme_contrast_result`：对比度读数（24 字节）。比值为**整数 × 100**。 */
data class NrThemeContrastResult(
    val structSize: Long,
    val ratioX100: Long,
    val passes: Boolean,
    val worstInDark: Boolean,
) {
    companion object {
        const val STRUCT_SIZE = 24

        internal fun decode(bytes: ByteArray): NrThemeContrastResult {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_theme_contrast_result")
            return NrThemeContrastResult(
                structSize = buf.u32(0),
                ratioX100 = buf.u32(4),
                passes = buf.u32(8) != 0L,
                worstInDark = buf.u32(12) != 0L,
            )
            // 偏移 16 / 20 的 reserved0/1 必须为 0：按头文件约定忽略。
        }
    }
}

/** 背景条目（批次字节里的定长部分，48 字节）。 */
data class NrBackgroundEntry(
    val fileSize: Long,
    val scrimPermille: Long,
    val fitMode: Long,
    val scrimAlpha: Int,
    val fileNameLen: Long,
    val fileNameOffset: Long,
) {
    companion object {
        const val STRUCT_SIZE = 48

        /** 未设置背景时的批次条目数。 */
        const val COUNT_NONE = 0L
    }
}

/** `nr_background_batch`：背景批次元数据（32 字节）。 */
data class NrBackgroundBatch(
    val count: Long,
    val bytesPtr: Long,
    val bytesLen: Long,
    val bytesCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 32

        internal fun decode(bytes: ByteArray): NrBackgroundBatch {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_background_batch")
            return NrBackgroundBatch(
                count = buf.u32(0),
                bytesPtr = buf.getLong(8),
                bytesLen = buf.getLong(16),
                bytesCapacity = buf.getLong(24),
            )
            // 偏移 4 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }
}

/**
 * 背景批次字节的解码：`[nr_background; 1]` 之后紧接文件名的 UTF-8 字节。
 *
 * 文件名是**相对名**（规格 R15）—— 绝对路径由调用方用 app-private 目录拼。
 */
object NrBackgroundCodec {

    /** 取第一条（也是唯一一条）背景条目；未设置返回 `null`。 */
    fun entry(bytes: ByteArray, count: Long): NrBackgroundEntry? {
        if (count == NrBackgroundEntry.COUNT_NONE) {
            return null
        }
        if (count != 1L) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "背景批次条目数应为 0 或 1，实际 $count",
            )
        }
        if (NrBackgroundEntry.STRUCT_SIZE > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "背景批次字节过短：需要 ${NrBackgroundEntry.STRUCT_SIZE} 字节，实际 ${bytes.size}",
            )
        }
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        return NrBackgroundEntry(
            fileSize = buf.getLong(0),
            scrimPermille = buf.getLong(8),
            fitMode = buf.getLong(16),
            scrimAlpha = buf.get(32).toInt() and 0xFF,
            fileNameLen = buf.u32(36),
            fileNameOffset = buf.u32(40),
        )
        // 偏移 24 的 reserved0、33/34 的 reserved1/2、44 的 reserved3 必须为 0：忽略。
    }

    /** 相对文件名（UTF-8）；未设置时返回空串。 */
    fun fileName(bytes: ByteArray, entry: NrBackgroundEntry?): String {
        if (entry == null) {
            return ""
        }
        return NrBatchText.slice(bytes, entry.fileNameOffset, entry.fileNameLen, "背景文件名")
    }
}

/**
 * 批次字节池里的 UTF-8 片段。
 *
 * 偏移是**相对整块缓冲区**的绝对下标（头文件约定）；越界一律按 `NR_ERR_INTERNAL` 拒绝
 * —— 跨语言布局错位属于内部协议违例，不能静默截断成乱码。
 */



/**
 * `nr_category_choice`：移动 / 删除目标的一行（40 字节）。
 *
 * `id = -1` 表示「未分类」；`selectable = false` 表示自身或子孙（选了会成环）——
 * 核心**刻意不隐藏**而只标记，界面据此置灰并解释原因。
 */
data class NrCategoryChoiceEntry(
    val id: Long,
    val depth: Int,
    val selectable: Boolean,
    val totalCount: Long,
    val nameOffset: Long,
    val nameLen: Long,
) {
    companion object {
        const val STRUCT_SIZE = 40

        /** 「未分类」哨兵。 */
        const val UNCATEGORIZED = -1L
    }
}

/** `nr_category_choice_batch`：批次头（32 字节）。 */
data class NrCategoryChoiceBatch(
    val count: Long,
    val bytesPtr: Long,
    val bytesLen: Long,
    val bytesCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 32

        internal fun decode(bytes: ByteArray): NrCategoryChoiceBatch {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_category_choice_batch")
            return NrCategoryChoiceBatch(
                count = buf.u32(0),
                bytesPtr = buf.getLong(8),
                bytesLen = buf.getLong(16),
                bytesCapacity = buf.getLong(24),
            )
        }
    }
}

/** 移动目标批次的解码（定长行 + 名称字节池）。 */
object NrCategoryChoiceCodec {

    fun entries(bytes: ByteArray, count: Long): List<NrCategoryChoiceEntry> {
        if (count < 0 || count > Int.MAX_VALUE) {
            throw NrException(NrErrors.NR_ERR_INTERNAL, message = "目标条目数非法：$count")
        }
        val headerSize = count * NrCategoryChoiceEntry.STRUCT_SIZE
        if (headerSize > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "目标批次字节过短：需要 $headerSize，实际 ${bytes.size}",
            )
        }
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        return (0 until count.toInt()).map { index ->
            val base = index * NrCategoryChoiceEntry.STRUCT_SIZE
            NrCategoryChoiceEntry(
                id = buf.getLong(base),
                depth = buf.getInt(base + 8),
                selectable = buf.getInt(base + 12) == 1,
                totalCount = buf.getLong(base + 16),
                nameOffset = buf.u32(base + 24),
                nameLen = buf.u32(base + 28),
            )
            // 偏移 32/36 的 reserved 必须为 0：按头文件约定忽略。
        }
    }

    /** 一行 + 已解码名称。 */
    fun named(entry: NrCategoryChoiceEntry, bytes: ByteArray): String {
        val start = entry.nameOffset.toInt()
        val end = start + entry.nameLen.toInt()
        if (start < 0 || end > bytes.size || end < start) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "目标名称区间越界：[${entry.nameOffset}, +${entry.nameLen})",
            )
        }
        return String(bytes, start, entry.nameLen.toInt(), Charsets.UTF_8)
    }
}

/**
 * `nr_id_list`：通用 i64 列表（32 字节；字节池是 `count` 个小端 i64）。
 *
 * 目前用于 `nr_list_books_in_scope`（作用域书籍 id）。
 */
data class NrIdList(
    val count: Long,
    val bytesPtr: Long,
    val bytesLen: Long,
    val bytesCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 32

        internal fun decode(bytes: ByteArray): NrIdList {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_id_list")
            return NrIdList(
                count = buf.u32(0),
                bytesPtr = buf.getLong(8),
                bytesLen = buf.getLong(16),
                bytesCapacity = buf.getLong(24),
            )
        }
    }
}

/** `nr_id_list` 载荷的解码。 */
object NrIdListCodec {

    /** 解出 id 列表；长度必须与 `count` 恰好自洽。 */
    fun ids(bytes: ByteArray, count: Long): List<Long> {
        if (count < 0 || count > Int.MAX_VALUE) {
            throw NrException(NrErrors.NR_ERR_INTERNAL, message = "id 列表长度非法：$count")
        }
        val needed = count * Long.SIZE_BYTES
        if (needed != bytes.size.toLong()) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "id 列表字节数不符：count=$count ⇒ 需要 $needed，实际 ${bytes.size}",
            )
        }
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        return (0 until count.toInt()).map { index -> buf.getLong(index * Long.SIZE_BYTES) }
    }
}

/** 作用域种类（与头文件 `nr_list_books_in_scope` 的 `scope_kind` 一致）。 */
object NrScopeKind {
    const val ALL = 0
    const val UNCATEGORIZED = 1
    const val CATEGORY = 2
}

/**
 * `nr_book_category`：书籍 → 分类的一行（16 字节；行内无字符串）。
 *
 * `categoryId = -1` 表示**未分类**（ABI 的 None 哨兵口径）——
 * 界面据此判断，不需要额外约定。
 */
data class NrBookCategoryEntry(
    val bookId: Long,
    val categoryId: Long,
) {
    companion object {
        const val STRUCT_SIZE = 16

        /** 未分类哨兵。 */
        const val UNCATEGORIZED = -1L
    }
}

/** `nr_book_category_batch`：批次头（32 字节）。 */
data class NrBookCategoryBatch(
    val count: Long,
    val bytesPtr: Long,
    val bytesLen: Long,
    val bytesCapacity: Long,
) {
    companion object {
        const val STRUCT_SIZE = 32

        internal fun decode(bytes: ByteArray): NrBookCategoryBatch {
            val buf = NrStructs.buffer(bytes, STRUCT_SIZE, "nr_book_category_batch")
            return NrBookCategoryBatch(
                count = buf.u32(0),
                bytesPtr = buf.getLong(8),
                bytesLen = buf.getLong(16),
                bytesCapacity = buf.getLong(24),
            )
            // 偏移 4 的 reserved0 必须为 0：按头文件约定忽略。
        }
    }
}

/** 书籍分类映射的解码（行内无字符串，字节池就是行本身）。 */
object NrBookCategoryCodec {

    fun entries(bytes: ByteArray, count: Long): List<NrBookCategoryEntry> {
        if (count < 0 || count > Int.MAX_VALUE) {
            throw NrException(NrErrors.NR_ERR_INTERNAL, message = "书籍分类条目数非法：$count")
        }
        val needed = count * NrBookCategoryEntry.STRUCT_SIZE
        if (needed > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "书籍分类批次字节过短：需要 $needed 字节，实际 ${bytes.size}",
            )
        }
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        return (0 until count.toInt()).map { index ->
            val base = index * NrBookCategoryEntry.STRUCT_SIZE
            NrBookCategoryEntry(
                bookId = buf.getLong(base),
                categoryId = buf.getLong(base + 8),
            )
        }
    }

    /**
     * `bookId → categoryId` 映射（界面筛选用）。
     *
     * 用 `associate` 而非 `groupBy`：核心保证一本书只有一条记录；
     * 若真出现重复，后写的胜出（与数据库主键语义一致）。
     */
    fun byBookId(bytes: ByteArray, count: Long): Map<Long, Long> =
        entries(bytes, count).associate { it.bookId to it.categoryId }
}

internal object NrBatchText {

    fun slice(bytes: ByteArray, offset: Long, len: Long, what: String): String {
        if (offset < 0 || len < 0) {
            throw NrException(NrErrors.NR_ERR_INTERNAL, message = "$what 偏移/长度非法：$offset/$len")
        }
        val end = offset + len
        if (end > bytes.size) {
            throw NrException(
                NrErrors.NR_ERR_INTERNAL,
                message = "$what 越界：[$offset, $end) 超出批次字节 ${bytes.size}",
            )
        }
        return String(bytes, offset.toInt(), len.toInt(), Charsets.UTF_8)
    }
}
