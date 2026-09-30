package com.novelreader.android.source

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

/**
 * 私有缓存回退复制（NFR-A2；checklist C4 的「私有缓存回退」部分）。
 *
 * 断言重点：
 * 1. **定长块**流式复制：每次请求都不超过 [SafPolicy.COPY_CHUNK_BYTES]（无整本加载路径）；
 * 2. 取消 / 失败都**不留半成品**（半成品永远不会被当成一本书）；
 * 3. 进度单调且最终等于实际字节数；
 * 4. 真正落到文件系统（`TemporaryFolder`）而不只是假 sink。
 */
class SafFallbackCopyTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val uri = SafTestData.uri()

    private fun store(): SafCacheStore = SafCacheStore(File(folder.root, "saf-cache"))

    @Test
    fun copyIsChunkedAndNeverRequestsTheWholeFile() {
        val content = SafTestData.bytes(200 * 1024)
        val stream = RecordingStream(content)
        val sink = RecordingSink()
        // 明确传入源长度：若实现改成「按源长度一次性缓冲」，本用例会立刻变红。
        val outcome = SafCacheCopyTask(stream, sink, expectedBytes = content.size.toLong()).run()

        assertTrue(outcome is SafCopyOutcome.Completed)
        val completed = outcome as SafCopyOutcome.Completed
        assertEquals(200L * 1024L, completed.bytes)
        assertTrue(completed.sizeMatched)
        assertEquals("/cache/an4.txt", completed.cachePath)
        // 每次 read 的 maxLen 都等于块大小（64 KiB），而不是「源长度」；
        // 第 5 次调用返回 0，用来确认已到末尾（而不是假设长度可信）。
        assertEquals(5, stream.requestedLengths.size)
        assertTrue(stream.requestedLengths.all { it == SafPolicy.COPY_CHUNK_BYTES })
        assertEquals(listOf(65536, 65536, 65536, 8192), sink.chunkSizes)
        assertEquals(200L * 1024L, sink.totalBytes)
        assertTrue(sink.committed)
        assertFalse(sink.aborted)
        assertTrue(stream.closed)
    }

    @Test
    fun progressIsMonotonicAndEndsAtActualByteCount() {
        val content = SafTestData.bytes(150 * 1024)
        val progress = mutableListOf<SafCopyProgress>()
        val outcome = SafCacheCopyTask(
            stream = RecordingStream(content),
            sink = RecordingSink(),
            expectedBytes = 150L * 1024L,
            onProgress = { progress += it },
        ).run()

        assertTrue(outcome is SafCopyOutcome.Completed)
        assertEquals(3, progress.size)
        assertEquals(listOf(65536L, 131072L, 153600L), progress.map { it.bytesCopied })
        assertEquals(65536.0 / 153600.0, progress[0].fraction, 1e-9)
        assertEquals(131072.0 / 153600.0, progress[1].fraction, 1e-9)
        assertEquals(1.0, progress[2].fraction, 0.0)
        assertEquals(153600L, progress.first().totalBytes)
    }

    @Test
    fun unreportedTotalLengthYieldsUncertainProgress() {
        val progress = mutableListOf<SafCopyProgress>()
        SafCacheCopyTask(
            stream = RecordingStream(SafTestData.bytes(10)),
            sink = RecordingSink(),
            expectedBytes = SafPolicy.UNKNOWN_SIZE,
            onProgress = { progress += it },
        ).run()
        assertEquals(1, progress.size)
        assertEquals(-1.0, progress.first().fraction, 0.0)
    }

    @Test
    fun realFileSinkCommitsAtomicallyAndLeavesNoPartFile() {
        val content = SafTestData.bytes(200 * 1024)
        val cache = store()
        val outcome = SafCacheCopyTask(
            stream = RecordingStream(content),
            sink = cache.sinkFor(uri),
            expectedBytes = content.size.toLong(),
        ).run()

        val completed = outcome as SafCopyOutcome.Completed
        val target = File(completed.cachePath)
        assertTrue(target.exists())
        assertEquals(content.size.toLong(), target.length())
        assertArrayEquals(content, target.readBytes())
        assertFalse(cache.partFileFor(uri).exists())
        assertEquals(cache.cacheFileFor(uri).absolutePath, completed.cachePath)
    }

    @Test
    fun cancelBeforeStartLeavesNothingBehind() {
        val cache = store()
        val token = SafCancelToken()
        token.cancel()
        val sink = RecordingSink()
        val outcome = SafCacheCopyTask(
            stream = RecordingStream(SafTestData.bytes(4096)),
            sink = sink,
            token = token,
        ).run()

        assertEquals(SafCopyOutcome.Cancelled(0L), outcome)
        assertTrue(sink.aborted)
        assertFalse(sink.committed)
        assertFalse(cache.cacheFileFor(uri).exists())
        assertFalse(cache.partFileFor(uri).exists())
    }

    @Test
    fun cancelDuringCopyRemovesTheHalfWrittenCopy() {
        val content = SafTestData.bytes(200 * 1024)
        val cache = store()
        val token = SafCancelToken()
        val outcome = SafCacheCopyTask(
            stream = RecordingStream(content),
            sink = cache.sinkFor(uri),
            token = token,
            onProgress = { token.cancel() },
        ).run()

        assertEquals(SafCopyOutcome.Cancelled(65536L), outcome)
        // 半成品必须不可见：既没有转正文件，也没有 .part 残留。
        assertFalse(cache.cacheFileFor(uri).exists())
        assertFalse(cache.partFileFor(uri).exists())
    }

    @Test
    fun readFailureLeavesNoPartialCopy() {
        val cache = store()
        val stream = RecordingStream(SafTestData.bytes(200 * 1024), failAfterBytes = 65536L)
        val outcome = SafCacheCopyTask(stream = stream, sink = cache.sinkFor(uri)).run()

        assertTrue(outcome is SafCopyOutcome.Failed)
        assertEquals(65536L, (outcome as SafCopyOutcome.Failed).bytesCopied)
        assertFalse(cache.cacheFileFor(uri).exists())
        assertFalse(cache.partFileFor(uri).exists())
        assertTrue(stream.closed)
    }

    @Test
    fun writeFailureReportsFailureAndAborts() {
        val sink = RecordingSink().apply { failOnWrite = true }
        val outcome = SafCacheCopyTask(stream = RecordingStream(SafTestData.bytes(4096)), sink = sink).run()

        assertTrue(outcome is SafCopyOutcome.Failed)
        assertTrue((outcome as SafCopyOutcome.Failed).detail.contains("模拟磁盘写入失败"))
        assertTrue(sink.aborted)
        assertFalse(sink.committed)
    }

    @Test
    fun oversizedChunkFromPlatformIsRejected() {
        val sink = RecordingSink()
        val stream = RecordingStream(SafTestData.bytes(4096), overrideReturn = SafPolicy.COPY_CHUNK_BYTES + 1)
        val outcome = SafCacheCopyTask(stream = stream, sink = sink).run()

        assertTrue(outcome is SafCopyOutcome.Failed)
        assertTrue((outcome as SafCopyOutcome.Failed).detail.contains("超长块"))
        assertTrue(sink.aborted)
    }

    @Test
    fun illegalChunkSizeFailsFastWithoutTouchingStream() {
        val stream = RecordingStream(SafTestData.bytes(4096))
        val outcome = SafCacheCopyTask(stream, RecordingSink(), chunkBytes = 0).run()
        assertTrue(outcome is SafCopyOutcome.Failed)
        assertTrue(stream.requestedLengths.isEmpty())
        assertTrue(stream.closed)
    }

    @Test
    fun cacheKeysAreStableHexAndUriSpecific() {
        val cache = store()
        val key = cache.cacheKeyOf(uri)
        assertEquals(64, key.length)
        assertTrue(key.all { it in "0123456789abcdef" })
        assertEquals(key, cache.cacheKeyOf(uri))
        assertNotEquals(key, cache.cacheKeyOf(SafTestData.uri("other")))
        assertTrue(cache.cacheKeyOf("../逃逸").all { it in "0123456789abcdef" })
        assertEquals(key + SafPolicy.CACHE_FILE_EXTENSION, cache.cacheFileFor(uri).name)
        assertEquals(key + SafPolicy.CACHE_FILE_EXTENSION + ".part", cache.partFileFor(uri).name)
    }

    @Test
    fun purgeRemovesBothCopyAndPart() {
        val cache = store()
        cache.ensureRoot()
        cache.cacheFileFor(uri).writeBytes(byteArrayOf(1, 2, 3))
        cache.partFileFor(uri).writeBytes(byteArrayOf(4))
        assertTrue(cache.remove(uri))
        assertFalse(cache.cacheFileFor(uri).exists())
        assertFalse(cache.partFileFor(uri).exists())
        assertFalse(cache.remove(uri))
    }
}
