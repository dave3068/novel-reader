package com.novelreader.android.reader

import com.novelreader.android.core.NrAbi
import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrBoundaryResult
import com.novelreader.android.core.NrChapterBatch
import com.novelreader.android.core.NrChapterEntry
import com.novelreader.android.core.NrEngine
import com.novelreader.android.core.NrErrorInfo
import com.novelreader.android.core.NrErrors
import com.novelreader.android.core.NrSetEncodingResult
import com.novelreader.android.core.NrOpenResult
import com.novelreader.android.core.NrPositionResult
import com.novelreader.android.core.NrReleaser
import com.novelreader.android.core.NrRebuildResult
import com.novelreader.android.core.NrSourceProvider
import com.novelreader.android.core.NrWindow
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** 一页的提交记录（断言页边界链用）。 */
internal data class BoundaryRecord(
    val phase: Int,
    val startByte: Long,
    val endByte: Long,
    val layoutId: Long,
    val requestId: Long,
    val contentIndexVersion: Long,
    val totalBytes: Long,
    val layoutFields: LongArray?,
    val fontFamily: ByteArray?,
)

/** 夹具章节。 */
internal data class FixtureChapter(val sequence: Int, val title: String, val startByte: Long, val endByte: Long)

/**
 * 夹具书籍：真实 UTF-8 字节 + 章节区间（章节区间由夹具自己按段编码算出，
 * 与「核心给出章节区间」的语义一致）。
 */
internal class FixtureBook private constructor(
    val text: String,
    val bytes: ByteArray,
    val chapters: List<FixtureChapter>,
) {
    companion object {
        /**
         * 生成 [chapterCount] 章、每章 [linesPerChapter] 行、每行 [charsPerLine] 个汉字的书。
         *
         * 汉字一律 3 字节，行尾 `\n` 1 字节 ⇒ 每行 3×[charsPerLine]+1 字节，便于手算断言。
         */
        fun build(
            chapterCount: Int = 3,
            linesPerChapter: Int = 6,
            charsPerLine: Int = 10,
        ): FixtureBook {
            val text = StringBuilder()
            val chapters = ArrayList<FixtureChapter>()
            var byteOffset = 0L
            for (index in 0 until chapterCount) {
                val title = "第${index + 1}章 测试标题"
                val head = "$title\n"
                val start = byteOffset
                text.append(head)
                byteOffset += head.toByteArray(Charsets.UTF_8).size.toLong()
                val body = buildString {
                    repeat(linesPerChapter) {
                        append("中".repeat(charsPerLine))
                        append('\n')
                    }
                }
                text.append(body)
                byteOffset += body.toByteArray(Charsets.UTF_8).size.toLong()
                chapters.add(FixtureChapter(index, title, start, byteOffset))
            }
            val encoded = text.toString().toByteArray(Charsets.UTF_8)
            check(encoded.size.toLong() == byteOffset) { "夹具字节长度与累计偏移不一致" }
            return FixtureBook(text.toString(), encoded, chapters)
        }
    }
}

/**
 * 脚本化 ABI：单测里替代 `libnr_jni.so`，行为按 `android/jni/nr_jni.c` 的结构体布局与槽位语义
 * 逐条对齐（空结果 = `(0,0,0)`；窗口 = `[offset, offset+len)`）。
 *
 * 除按夹具返回真实字节外，还提供：
 * - [events]：调用序列（断言串行化与取消时序）；
 * - [maxInFlight]：同时处于 native 调用中的调用数（证明「同一时刻只有一个入口」）；
 * - [commitRecords]：页边界提交记录（断言 UTF-8 页末与页边界链）；
 * - 旋钮：`commitStatusScript`（构造过期/拒绝）、`positionOffset`（构造断点续读）。
 */
internal class ScriptedReaderAbi(
    private val book: FixtureBook,
    private val bookId: Long = 901L,
    private val contentIndexVersion: Long = 5L,
    private val chapterBatchSize: Int = 64,
) : NrAbi {

    val events = mutableListOf<String>()
    val commitRecords = mutableListOf<BoundaryRecord>()
    val boundarySubmissions = mutableListOf<BoundaryRecord>()

    /** 同时进入 native 的最大调用数（>1 即说明阅读状态被并发修改）。 */
    var maxInFlight = 0
        private set
    private var inFlight = 0

    /** 每次读取窗口前的阻塞毫秒数（真实线程测试用：把竞态窗口放大）。 */
    var readWindowDelayMillis = 0L

    /** `nr_open_txt` 返回码（非 0 时不写回执，供构造打开失败）。 */
    var openTxtCode = NrErrors.NR_ERR_OK

    /** `nr_open_result.encoding` 回执（`NR_ENCODING_*`）：用于验证平台按核心编码解码。 */
    var encoding = NrAbiConstants.NR_ENCODING_UTF8

    /** `nr_open_result.encoding_confidence`（0..1000）。 */
    var encodingConfidence = 1000

    /** `nr_open_result.needs_encoding_confirmation`（1 = 低可信度需用户确认）。 */
    var needsEncodingConfirmation = 0

    /** 最近一次 `openTxt` 的数据源种类与坐标空间长度（宿主接线断言用）。 */
    var lastOpenKind = -1
    var lastOpenTotalBytes = -1L
    var invalidateCalls = 0
    var beginLayoutCalls = 0
    var commitCalls = 0
    var readWindowCalls = 0
    var getChaptersCalls = 0
    var cancelRequests = mutableListOf<Long>()

    /** 索引重建前的阻塞毫秒数（真实线程测试用：把「取消必须可达」的窗口放大）。 */
    var rebuildDelayMillis = 0L

    /** BEGIN_LAYOUT 返回的世代推进值。 */
    var layoutIdCounter = 0L
    var requestIdCounter = 0L

    /** COMMIT 状态脚本（依次消费；耗尽后一律 COMMITTED）。 */
    val commitStatusScript = ArrayDeque<Long>()

    /** 章节点数（`nr_boundary_result.page_count` 回显）。 */
    var registeredPageCount = 0L

    /** 进度：`found = 0` 表示尚无进度。 */
    var positionFound = 0L
    var positionChapterIndex = 0L
    var positionOffset = 0L

    /** 索引重建返回值。 */
    var rebuildCode = NrErrors.NR_ERR_OK

    /** 最近一次 `nr_set_encoding` 的请求编码（0 = 清除覆盖）。 */
    var lastRequestedEncoding: Int? = null

    var abiVersionResult = NrAbiConstants.NR_ABI_VERSION

    private var nextPtr = 0x0000_7F00_0000_0000L

    private fun <T> native(call: String, body: () -> T): T {
        events.add("$call:enter")
        inFlight += 1
        maxInFlight = maxOf(maxInFlight, inFlight)
        try {
            return body()
        } finally {
            inFlight -= 1
            events.add("$call:exit")
        }
    }

    private fun writer(size: Int): ByteBuffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    private fun slotsFor(bytes: ByteArray): LongArray {
        if (bytes.isEmpty()) {
            return longArrayOf(0L, 0L, 0L)
        }
        val ptr = nextPtr
        nextPtr += 4096
        return longArrayOf(ptr, bytes.size.toLong(), bytes.size.toLong())
    }

    private fun copyInto(source: ByteBuffer, target: ByteArray) {
        val size = minOf(target.size, source.capacity())
        System.arraycopy(source.array(), 0, target, 0, size)
    }

    // ------------------------------------------------------------------ NrAbi

    override fun abiVersion(): Int = native("abiVersion") { abiVersionResult }

    override fun engineCreate(maxWindowBytes: Long, dbPath: ByteArray?, provider: NrSourceProvider?): Long =
        native("engineCreate") { ENGINE_HANDLE }

    override fun engineDestroy(engine: Long): Int = native("engineDestroy") { NrErrors.NR_ERR_OK }

    /** `nr_open_txt` 前的阻塞毫秒数（真实线程测试用：把「取消落在打开回执之前」的窗口放大）。 */
    var openTxtDelayMillis = 0L

    /**
     * 「已进入 `openTxt`、尚未返回回执」的可控闸门（真实线程测试用）。
     *
     * 取代 [openTxtDelayMillis] 的**墙钟窗口**：测试先 [OpenTxtGate.awaitEntered] 确认进入在途状态，
     * 校验「在途期间」的不变量后再 [OpenTxtGate.release] 放行返回 ——
     * 机器负载高低都不影响判定，从而消除 `hostPauseDuringOpen…` 这类用例的时序抖动。
     */
    var openTxtGate: OpenTxtGate? = null

    /** 见 [openTxtGate]。 */
    class OpenTxtGate {

        private val entered = java.util.concurrent.CountDownLatch(1)
        private val released = java.util.concurrent.CountDownLatch(1)

        /** 等待 native 侧进入 `openTxt`；返回 `false` 表示超时（用例应判失败）。 */
        fun awaitEntered(timeoutMillis: Long = 10_000L): Boolean =
            entered.await(timeoutMillis, java.util.concurrent.TimeUnit.MILLISECONDS)

        /** 放行 `openTxt` 返回回执。 */
        fun release() {
            released.countDown()
        }

        internal fun signalEntered() {
            entered.countDown()
        }

        internal fun awaitRelease(timeoutMillis: Long) {
            released.await(timeoutMillis, java.util.concurrent.TimeUnit.MILLISECONDS)
        }
    }

    override fun openTxt(
        engine: Long,
        kind: Int,
        totalBytes: Long,
        identity: ByteArray,
        displayName: ByteArray?,
        outResult: ByteArray,
    ): Int = native("openTxt") {
        lastOpenKind = kind
        lastOpenTotalBytes = totalBytes
        if (openTxtDelayMillis > 0L) {
            Thread.sleep(openTxtDelayMillis)
        }
        // 可控闸门：确定性地停在「已进入 native、尚未拿到回执」的状态（有界等待，绝不永久挂起）。
        openTxtGate?.let { gate ->
            gate.signalEntered()
            gate.awaitRelease(10_000L)
        }
        if (openTxtCode != NrErrors.NR_ERR_OK) {
            return@native openTxtCode
        }
        val buf = writer(NrOpenResult.STRUCT_SIZE)
        buf.putLong(0, bookId)
        buf.putLong(8, book.bytes.size.toLong())
        buf.putLong(16, contentIndexVersion)
        buf.putInt(24, NrAbiConstants.NR_CONTENT_UNIT_SINGLE.toInt())
        buf.putInt(28, encoding.toInt())
        buf.putInt(32, encodingConfidence)
        buf.putInt(36, 990)
        buf.putInt(40, needsEncodingConfirmation)
        buf.putInt(44, book.chapters.size)
        buf.putInt(48, 1)
        copyInto(buf, outResult)
        NrErrors.NR_ERR_OK
    }

    override fun closeBook(engine: Long, bookId: Long): Int = native("closeBook") { NrErrors.NR_ERR_OK }

    override fun readWindow(
        engine: Long,
        chapterIndex: Int,
        offset: Long,
        length: Long,
        outWindow: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = native("readWindow") {
        readWindowCalls += 1
        if (readWindowDelayMillis > 0L) {
            Thread.sleep(readWindowDelayMillis)
        }
        val total = book.bytes.size.toLong()
        if (offset < 0L || offset > total || length <= 0L) {
            return@native null
        }
        val end = minOf(offset + length, total)
        val slice = book.bytes.copyOfRange(offset.toInt(), end.toInt())
        val slots = slotsFor(slice)
        val buf = writer(NrWindow.STRUCT_SIZE)
        buf.putLong(0, offset)
        buf.putLong(8, end)
        buf.putLong(16, 1L)
        buf.putLong(24, total)
        buf.putInt(32, slice.size)
        buf.putInt(36, 0)
        buf.putInt(40, 0)
        buf.putInt(44, if (end >= total) 1 else 0)
        copyInto(buf, outWindow)
        outBuffer[0] = slots[0]
        outBuffer[1] = slots[1]
        outBuffer[2] = slots[2]
        slice
    }

    override fun submitPageBoundary(
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
    ): Int = native("submitPageBoundary") {
        val record = BoundaryRecord(
            phase = phase,
            startByte = startByte,
            endByte = endByte,
            layoutId = layoutId,
            requestId = requestId,
            contentIndexVersion = contentIndexVersion,
            totalBytes = totalBytes,
            layoutFields = layoutFields,
            fontFamily = fontFamily,
        )
        boundarySubmissions.add(record)
        var status: Long
        val resultLayoutId: Long
        val resultRequestId: Long
        if (phase == NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT) {
            beginLayoutCalls += 1
            layoutIdCounter += 1
            requestIdCounter += 1
            status = NrAbiConstants.NR_BOUNDARY_LAYOUT_READY
            resultLayoutId = layoutIdCounter
            resultRequestId = requestIdCounter
        } else {
            commitCalls += 1
            status = commitStatusScript.removeFirstOrNull() ?: NrAbiConstants.NR_BOUNDARY_COMMITTED
            resultLayoutId = layoutId
            resultRequestId = requestId
            if (status == NrAbiConstants.NR_BOUNDARY_COMMITTED) {
                // 与核心一致：同起点同终点重复提交 → 幂等接受，不新增记录。
                val duplicated = commitRecords.any {
                    it.layoutId == layoutId && it.startByte == startByte && it.endByte == endByte
                }
                if (duplicated) {
                    status = NrAbiConstants.NR_BOUNDARY_DUPLICATE_IDENTICAL
                } else {
                    commitRecords.add(record)
                    registeredPageCount += 1
                }
            }
        }
        val buf = writer(NrBoundaryResult.STRUCT_SIZE)
        buf.putInt(0, status.toInt())
        buf.putInt(4, 0)
        buf.putInt(8, NrErrors.NR_ERR_OK)
        buf.putInt(12, 0)
        buf.putLong(16, resultLayoutId)
        buf.putLong(24, resultRequestId)
        buf.putLong(32, 0L)
        buf.putLong(40, 0L)
        buf.putLong(48, startByte)
        buf.putLong(56, endByte)
        buf.putLong(64, totalBytes)
        buf.putLong(72, registeredPageCount)
        copyInto(buf, outResult)
        NrErrors.NR_ERR_OK
    }

    override fun getPageBoundary(
        engine: Long,
        layoutId: Long,
        chapterIndex: Int,
        startByte: Long,
        outResult: ByteArray,
    ): Int = native("getPageBoundary") { NrErrors.NR_ERR_OK }

    override fun savePosition(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        originalByteOffset: Long,
        fingerprint: ByteArray?,
        outChanged: IntArray?,
    ): Int = native("savePosition") {
        savedPositions.add(chapterIndex.toLong() to originalByteOffset)
        outChanged?.let { it[0] = 1 }
        NrErrors.NR_ERR_OK
    }

    /** 已保存的位置序列（断言翻页/重排都会落进度）。 */
    val savedPositions = mutableListOf<Pair<Long, Long>>()

    override fun getPosition(engine: Long, bookId: Long, outResult: ByteArray, outBuffer: LongArray): ByteArray? =
        native("getPosition") {
            val fingerprint = "锚点".toByteArray(Charsets.UTF_8)
            val slots = slotsFor(fingerprint)
            val buf = writer(NrPositionResult.STRUCT_SIZE)
            buf.putInt(0, NrPositionResult.STRUCT_SIZE)
            buf.putInt(4, positionFound.toInt())
            buf.putInt(8, positionChapterIndex.toInt())
            buf.putInt(12, if (positionFound != 0L) 1 else 0)
            buf.putInt(16, if (positionFound != 0L) 0 else 1)
            buf.putInt(20, 0)
            buf.putLong(24, bookId)
            buf.putLong(32, NrAbiConstants.NR_CONTENT_UNIT_SINGLE)
            buf.putLong(40, positionOffset)
            buf.putLong(48, 1_700_000_000L)
            buf.putLong(56, slots[0])
            buf.putLong(64, slots[1])
            buf.putLong(72, slots[2])
            copyInto(buf, outResult)
            outBuffer[0] = slots[0]
            outBuffer[1] = slots[1]
            outBuffer[2] = slots[2]
            fingerprint
        }

    override fun getChapters(
        engine: Long,
        bookId: Long,
        startSequence: Int,
        maxCount: Int,
        outBatch: ByteArray,
        outBuffer: LongArray,
    ): ByteArray? = native("getChapters") {
        getChaptersCalls += 1
        val selected = book.chapters.filter { it.sequence >= startSequence }.take(maxCount)
        if (selected.isEmpty()) {
            return@native ByteArray(0)
        }
        val titles = selected.map { it.title.toByteArray(Charsets.UTF_8) }
        val headerSize = selected.size * NrChapterEntry.STRUCT_SIZE
        val totalSize = headerSize + titles.sumOf { it.size }
        val payload = ByteArray(totalSize)
        val buf = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN)
        var titleOffset = headerSize
        selected.forEachIndexed { index, chapter ->
            val base = index * NrChapterEntry.STRUCT_SIZE
            buf.putInt(base, chapter.sequence)
            buf.putInt(base + 4, 1)
            buf.putLong(base + 8, chapter.startByte)
            buf.putLong(base + 16, chapter.endByte)
            buf.putInt(base + 24, titleOffset)
            buf.putInt(base + 28, titles[index].size)
            buf.putInt(base + 32, contentIndexVersion.toInt())
            buf.putInt(base + 36, 0)
            System.arraycopy(titles[index], 0, payload, titleOffset, titles[index].size)
            titleOffset += titles[index].size
        }
        val slots = slotsFor(payload)
        val batch = writer(NrChapterBatch.STRUCT_SIZE)
        batch.putInt(0, selected.size)
        batch.putInt(4, contentIndexVersion.toInt())
        batch.putInt(8, book.chapters.size)
        batch.putInt(12, 0)
        batch.putLong(16, slots[0])
        batch.putLong(24, slots[1])
        batch.putLong(32, slots[2])
        copyInto(batch, outBatch)
        outBuffer[0] = slots[0]
        outBuffer[1] = slots[1]
        outBuffer[2] = slots[2]
        payload
    }

    override fun createBookmark(
        engine: Long,
        chapterIndex: Int,
        bookId: Long,
        contentUnitId: Long,
        byteOffset: Long,
        title: ByteArray?,
        outBookmarkId: LongArray,
    ): Int = native("createBookmark") {
        outBookmarkId[0] = 77L
        NrErrors.NR_ERR_OK
    }

    override fun deleteBookmark(engine: Long, bookmarkId: Long, outDeleted: IntArray?): Int =
        native("deleteBookmark") { NrErrors.NR_ERR_OK }

    override fun invalidateLayout(engine: Long, layoutId: Long, outRemoved: LongArray?): Int =
        native("invalidateLayout") {
            invalidateCalls += 1
            outRemoved?.let { it[0] = registeredPageCount }
            registeredPageCount = 0
            NrErrors.NR_ERR_OK
        }

    override fun setEncoding(engine: Long, bookId: Long, encodingCode: Int, outResult: ByteArray): Int =
        native("setEncoding") {
            lastRequestedEncoding = encodingCode
            val buf = writer(NrSetEncodingResult.STRUCT_SIZE)
            buf.putInt(0, NrSetEncodingResult.STRUCT_SIZE)
            // 回显：0 表示「清除覆盖」，替身以夹具默认编码（UTF-8 = 1）回应。
            val effective = if (encodingCode == 0) {
                NrAbiConstants.NR_ENCODING_UTF8.toInt()
            } else {
                encodingCode
            }
            buf.putInt(4, effective)
            buf.putLong(8, contentIndexVersion + 1)
            buf.putInt(16, book.chapters.size)
            buf.putInt(20, 1)
            copyInto(buf, outResult)
            NrErrors.NR_ERR_OK
        }

    override fun rebuildIndex(engine: Long, bookId: Long, requestId: Long, outResult: ByteArray): Int =
        native("rebuildIndex") {
            if (rebuildDelayMillis > 0L) {
                Thread.sleep(rebuildDelayMillis)
            }
            val buf = writer(NrRebuildResult.STRUCT_SIZE)
            buf.putInt(0, NrRebuildResult.STRUCT_SIZE)
            buf.putInt(4, if (rebuildCode == NrErrors.NR_ERR_CANCELLED) 1 else 0)
            buf.putInt(8, book.chapters.size)
            buf.putInt(12, 0)
            buf.putLong(16, book.bytes.size.toLong())
            buf.putLong(24, book.bytes.size.toLong())
            buf.putLong(32, contentIndexVersion)
            copyInto(buf, outResult)
            rebuildCode
        }

    override fun cancelRequest(engine: Long, requestId: Long): Int = native("cancelRequest") {
        cancelRequests.add(requestId)
        rebuildCode = NrErrors.NR_ERR_CANCELLED
        NrErrors.NR_ERR_OK
    }

    override fun getLastError(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray? =
        native("getLastError") {
            val buf = writer(NrErrorInfo.STRUCT_SIZE)
            buf.putInt(0, NrErrors.NR_ERR_OK)
            buf.putInt(4, 0)
            copyInto(buf, outInfo)
            outBuffer[0] = 0L
            outBuffer[1] = 0L
            outBuffer[2] = 0L
            ByteArray(0)
        }

    override fun releaseBuffer(ptr: Long, len: Long, capacity: Long): Int =
        native("releaseBuffer") { NrErrors.NR_ERR_OK }

    companion object {
        const val ENGINE_HANDLE = 5150L
    }
}

/** 用脚本化 ABI 建一个真实 [NrEngine]（走同一套薄封装的装箱/拆箱代码）。 */
internal fun engineOn(abi: NrAbi, releaser: NrReleaser): NrEngine =
    NrEngine.createWith(
        abi = abi,
        maxWindowBytes = NrAbiConstants.NR_DEFAULT_WINDOW_BYTES,
        dbPath = null,
        provider = null,
        releaser = releaser,
    )

/**
 * 确定性测量器（JVM 无 Android 排版栈）：每行最多 [charsPerLine] 个字符，遇 `\n` 提前断行。
 *
 * 行高固定为 [lineHeightPx]，因此「一页几行」可以手算，页边界的字节数也能精确断言。
 */
internal class FakeTextMeasurer(
    private val charsPerLine: Int = 10,
    private val lineHeightPx: Float = 27f,
) : TextMeasurer {

    override fun measure(text: String, spec: TextLayoutSpec): MeasuredText {
        val ends = ArrayList<Int>()
        var index = 0
        while (index < text.length) {
            var end = index
            var consumed = 0
            while (end < text.length && consumed < charsPerLine) {
                val step = if (Character.isHighSurrogate(text[end]) && end + 1 < text.length) 2 else 1
                end += step
                consumed += 1
            }
            // 行尾换行并入当前行：渲染端不会为一个换行单独开一行（否则每行会多出一行）
            if (end < text.length && text[end] == '\n') {
                end += 1
            }
            if (end <= index) {
                end = index + 1
            }
            ends.add(end)
            index = end
        }
        return MeasuredText(ends.size, lineHeightPx, ends)
    }
}
