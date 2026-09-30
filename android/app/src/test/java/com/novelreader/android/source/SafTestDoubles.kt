package com.novelreader.android.source

import java.io.IOException

/**
 * SAF 单测夹具（A-T4 / 检查项 C4）：全部是**纯 JVM** 假实现，不触碰 Android 框架。
 *
 * 假实现刻意保留真实平台的坏脾气，断言才有意义：
 * - [FakeSafPlatform.readAt] 支持**短读**（`maxBytesPerRead`）、**中途失败**（`failingFrom`）；
 * - [RecordingStream] 保留顺序流形态（每次至多 `maxLen`，可选超长返回以验证契约违例）；
 * - [RecordingSink] 记录每次写入长度与 commit/abort，用于证明「半成品不可见」。
 */

internal object SafTestData {
    const val NAME = "斗破苍穹.txt"
    const val MIME_TXT = "text/plain"

    /** 稳定内容：`0,1,2,…` 循环，便于逐字节核对复制结果。 */
    fun bytes(size: Int): ByteArray = ByteArray(size) { index -> (index % 251).toByte() }

    fun uri(suffix: String = "1"): String = "content://com.android.providers.downloads.documents/document/$suffix"
}

internal class MemoryKeyValueStore : SafKeyValueStore {

    val entries = LinkedHashMap<String, String>()

    override fun get(key: String): String? = entries[key]

    override fun put(key: String, value: String) {
        entries[key] = value
    }

    override fun remove(key: String) {
        entries.remove(key)
    }

    override fun keys(): Set<String> = entries.keys.toSet()
}

internal class FakeSafPlatform(private val content: ByteArray = ByteArray(0)) : SafPlatform {

    var metadata: SafDocumentMeta? = SafDocumentMeta(SafTestData.NAME, content.size.toLong(), SafTestData.MIME_TXT)

    var takePersistableSucceeds = true

    var persistedFlags = 1

    var seekable = true

    var descriptorSize = content.size.toLong()

    var descriptorOpenable = true

    var streamOpenable = true

    /** >= 0 时：从该偏移起的 `readAt` 返回 -1（模拟读取中途权限失效 / I/O 故障）。 */
    var failingFrom = -1L

    /** 单次 `readAt` 最多返回的字节数（模拟短读）。 */
    var maxBytesPerRead = Int.MAX_VALUE

    var openDescriptorCalls = 0

    var closedDescriptorCalls = 0

    /** 每次窗口读取请求：(offset, maxLen)。 */
    val readRequests = mutableListOf<Pair<Long, Int>>()

    var lastStream: RecordingStream? = null

    var cacheRootPath = "/tmp/an4-fake-cache"

    override fun persistedReadPermissionFlags(uri: String): Int = persistedFlags

    override fun takePersistableReadPermission(uri: String): Boolean {
        persistedFlags = if (takePersistableSucceeds) 1 else 0
        return takePersistableSucceeds
    }

    override fun queryMetadata(uri: String): SafDocumentMeta? = metadata

    override fun openReadOnlyDescriptor(uri: String): Long {
        if (!descriptorOpenable) return -1L
        openDescriptorCalls += 1
        return DESCRIPTOR_HANDLE
    }

    override fun descriptorSize(handle: Long): Long = descriptorSize

    override fun isSeekable(handle: Long): Boolean = seekable

    override fun readAt(handle: Long, offset: Long, dst: ByteArray, dstOffset: Int, maxLen: Int): Int {
        readRequests += offset to maxLen
        if (failingFrom >= 0L && offset >= failingFrom) return -1
        if (offset >= content.size) return 0
        val available = content.size.toLong() - offset
        val n = minOf(maxLen.toLong(), available, maxBytesPerRead.toLong()).toInt()
        if (n <= 0) return 0
        System.arraycopy(content, offset.toInt(), dst, dstOffset, n)
        return n
    }

    override fun closeDescriptor(handle: Long) {
        closedDescriptorCalls += 1
    }

    override fun openSequentialStream(uri: String): SafSequentialStream? {
        if (!streamOpenable) return null
        val stream = RecordingStream(content, maxBytesPerRead)
        lastStream = stream
        return stream
    }

    override fun privateCacheRoot(): String = cacheRootPath

    companion object {
        const val DESCRIPTOR_HANDLE = 7L
    }
}

internal class RecordingStream(
    private val data: ByteArray,
    private val maxBytesPerRead: Int = Int.MAX_VALUE,
    /** >= 0 时：累计读到该字节数之后返回 -1（模拟中途 I/O 失败）。 */
    var failAfterBytes: Long = -1L,
    /** 故意返回超长块（契约违例）时设为 n > maxLen。 */
    var overrideReturn: Int? = null,
) : SafSequentialStream {

    val requestedLengths = mutableListOf<Int>()

    var bytesServed = 0L
        private set

    var closed = false
        private set

    override fun read(dst: ByteArray, maxLen: Int): Int {
        requestedLengths += maxLen
        if (failAfterBytes >= 0L && bytesServed >= failAfterBytes) return -1
        overrideReturn?.let { return it }
        if (bytesServed >= data.size) return 0
        val n = minOf(maxLen.toLong(), data.size - bytesServed, maxBytesPerRead.toLong()).toInt()
        if (n <= 0) return 0
        System.arraycopy(data, bytesServed.toInt(), dst, 0, n)
        bytesServed += n
        return n
    }

    override fun close() {
        closed = true
    }
}

internal class RecordingSink(private val path: String = "/cache/an4.txt") : SafCacheSink {

    val chunkSizes = mutableListOf<Int>()

    var totalBytes = 0L
        private set

    var committed = false
        private set

    var aborted = false
        private set

    var failOnWrite = false

    override fun write(src: ByteArray, len: Int) {
        if (failOnWrite) throw IOException("模拟磁盘写入失败")
        chunkSizes += len
        totalBytes += len
    }

    override fun commit(): String {
        check(!aborted) { "已 abort 的 sink 不能 commit" }
        committed = true
        return path
    }

    override fun abort() {
        aborted = true
    }
}
