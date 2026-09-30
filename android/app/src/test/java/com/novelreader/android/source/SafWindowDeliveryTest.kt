package com.novelreader.android.source

import com.novelreader.android.core.NrErrors
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 窗口读取（AR2「有限窗口正文读取」；checklist C4 的「窗口读取」部分）。
 *
 * 断言重点：
 * 1. 单次交付量**恒**不超过核心窗口硬上限（不存在的「一次交付整本」路径）；
 * 2. 短读被补齐、`out_eof` 与坐标空间一致；
 * 3. I/O 失败必须报 `NR_ERR_IO`（绝不静默返回 0 字节假成功）；
 * 4. 取消报 `NR_ERR_CANCELLED`（取消是部分成功，不是 INTERNAL）。
 */
class SafWindowDeliveryTest {

    private val content = SafTestData.bytes(4096)

    private fun readerSetup(
        bytes: ByteArray = content,
        cancels: SafCancelRegistry = SafCancelRegistry(),
        configure: FakeSafPlatform.() -> Unit = {},
    ): Triple<FakeSafPlatform, SafWindowReader, SafCancelRegistry> {
        val platform = FakeSafPlatform(bytes)
        platform.configure()
        val handle = platform.openReadOnlyDescriptor(SafTestData.uri())
        val reader = SafWindowReader(platform, handle, bytes.size.toLong(), cancels)
        return Triple(platform, reader, cancels)
    }

    private fun read(
        reader: SafWindowReader,
        requestId: Long,
        offset: Long,
        maxLength: Int,
        dstSize: Int = 4096,
    ): Triple<Int, Int, Int> {
        val dst = ByteArray(dstSize)
        val info = IntArray(2)
        val code = reader.readWindow(requestId, offset, maxLength, dst, info)
        return Triple(code, info[0], info[1])
    }

    @Test
    fun deliveryIsCappedByEngineWindowEvenWhenAskedForMore() {
        val big = SafTestData.bytes(1024 * 1024)
        val (platform, reader, _) = readerSetup(big)
        val (code, length, eof) = read(reader, 1L, 0L, 1024 * 1024, dstSize = 1024 * 1024)
        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(SafPolicy.MAX_DELIVERY_BYTES, length)
        assertEquals(0, eof)
        // 每次平台调用也受同一上限约束：单次交付量由本层（而非平台自述）决定。
        assertEquals(1, platform.readRequests.size)
        assertEquals(SafPolicy.MAX_DELIVERY_BYTES, platform.readRequests.first().second)
    }

    @Test
    fun deliveryNeverExceedsDestinationBuffer() {
        val (platform, reader, _) = readerSetup()
        val (code, length, _) = read(reader, 2L, 0L, 4096, dstSize = 8)
        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(8, length)
        assertEquals(8, platform.readRequests.first().second)
    }

    @Test
    fun shortReadsAreFilledUpToTheRequestedLength() {
        val bytes = SafTestData.bytes(100)
        val (platform, reader, _) = readerSetup(bytes) { maxBytesPerRead = 7 }
        val (code, length, eof) = read(reader, 3L, 0L, 100, dstSize = 512)
        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(100, length)
        assertEquals(1, eof)
        // 每次请求仍是未满足的剩余长度，短读由本层补齐（而不是接受半截窗口）。
        assertEquals(15, platform.readRequests.size)
        assertEquals(100, platform.readRequests.first().second)
        assertEquals(2, platform.readRequests.last().second)
    }

    @Test
    fun eofFlagFollowsCoordinateSpaceEnd() {
        val (_, reader, _) = readerSetup()
        assertEquals(0, read(reader, 4L, 0L, 16).third)
        assertEquals(1, read(reader, 4L, content.size.toLong() - 16L, 16).third)
        val (code, length, eof) = read(reader, 4L, content.size.toLong() - 3L, 16)
        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(3, length)
        assertEquals(1, eof)
    }

    @Test
    fun eofIsReportedWhenProviderEndsEarlierThanDeclared() {
        // Provider 声称 4096 字节，实际只给了 20 字节：如实标记 EOF，而不是当作「读满」。
        val (_, reader, _) = readerSetup(SafTestData.bytes(20))
        val (code, length, eof) = read(reader, 5L, 0L, 4096, dstSize = 512)
        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(20, length)
        assertEquals(1, eof)
    }

    @Test
    fun offsetAtEndReturnsEmptyWindowWithEof() {
        val (platform, reader, _) = readerSetup()
        val (code, length, eof) = read(reader, 6L, content.size.toLong(), 64)
        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(0, length)
        assertEquals(1, eof)
        assertTrue(platform.readRequests.isEmpty())
    }

    @Test
    fun ioFailureIsReportedInsteadOfSilentZero() {
        val (_, reader, _) = readerSetup {
            failingFrom = 32L
            maxBytesPerRead = 16
        }
        val (code, length, eof) = read(reader, 7L, 0L, 256, dstSize = 256)
        assertEquals(NrErrors.NR_ERR_IO, code)
        assertEquals(32, length)
        assertEquals(0, eof)
        assertNotNull(reader.lastIoFailure)
    }

    @Test
    fun failureOnTheVeryFirstReadIsStillAnError() {
        val (_, reader, _) = readerSetup { failingFrom = 0L }
        val (code, length, _) = read(reader, 8L, 0L, 256, dstSize = 256)
        assertEquals(NrErrors.NR_ERR_IO, code)
        // 0 字节 + 错误码：调用方不可能把「权限失效」误当成「文件已到末尾」。
        assertEquals(0, length)
    }

    @Test
    fun preCancelledRequestReturnsCancelledWithoutTouchingPlatform() {
        val cancels = SafCancelRegistry()
        cancels.cancel(11L)
        val (platform, reader, _) = readerSetup(cancels = cancels)
        val (code, _, _) = read(reader, 11L, 0L, 64)
        assertEquals(NrErrors.NR_ERR_CANCELLED, code)
        assertTrue(platform.readRequests.isEmpty())
    }

    @Test
    fun cancellationDuringWindowReportsPartialDelivery() {
        val cancels = SafCancelRegistry()
        val platform = FakeSafPlatform(content).apply { maxBytesPerRead = 4 }
        val handle = platform.openReadOnlyDescriptor(SafTestData.uri())
        val cancelling = object : SafPlatform by platform {
            override fun readAt(
                handle: Long,
                offset: Long,
                dst: ByteArray,
                dstOffset: Int,
                maxLen: Int,
            ): Int {
                val read = platform.readAt(handle, offset, dst, dstOffset, maxLen)
                cancels.cancel(12L)
                return read
            }
        }
        val reader = SafWindowReader(cancelling, handle, content.size.toLong(), cancels)
        val (code, length, _) = read(reader, 12L, 0L, 64, dstSize = 64)
        assertEquals(NrErrors.NR_ERR_CANCELLED, code)
        // 取消是部分成功：已交付字节必须如实回报，不能被丢弃也不能被当成内部错误。
        assertEquals(4, length)
    }

    @Test
    fun invalidArgumentsAreRejectedWithoutTouchingPlatform() {
        val (platform, reader, _) = readerSetup()
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, read(reader, 13L, -1L, 16).first)
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, read(reader, 13L, 0L, 0).first)
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, read(reader, 13L, 0L, 16, dstSize = 0).first)
        assertTrue(platform.readRequests.isEmpty())
    }

    @Test
    fun cancelRegistryClearsFinishedRequests() {
        val cancels = SafCancelRegistry()
        cancels.tokenOf(21L)
        assertEquals(1, cancels.pendingCount())
        cancels.cancel(21L)
        assertTrue(cancels.isCancelled(21L))
        cancels.complete(21L)
        assertEquals(0, cancels.pendingCount())
        assertFalse(cancels.isCancelled(21L))
        assertFalse(cancels.tokenOf(22L).cancelled)
    }
}
