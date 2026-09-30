package com.novelreader.android.source

import com.novelreader.android.core.NrErrors
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

/**
 * 导入流程与模式判定（AR1 + NFR-A2；checklist C4 的「URI 元数据 / 权限状态」部分）。
 *
 * 断言重点：
 * - 可定位 + 长度已知 → `WINDOW`（`NR_SOURCE_KIND_PLATFORM` 的前提）；
 * - 不可定位或长度未知 → `CACHE_COPY`；
 * - 权限不可持久化 → **可见说明**（不是静默成功、也不是静默失败）；
 * - 权限 / 来源失效 → `NEEDS_REASSOCIATION` 且**记录保留**（书不会消失）。
 */
class SafIntakeFlowTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val kv = MemoryKeyValueStore()

    private val uri = SafTestData.uri()

    private fun sourceFor(platform: SafPlatform): SafBookSource =
        SafBookSource(
            platform = platform,
            store = SafSourceStore(kv),
            cache = SafCacheStore(File(folder.root, "cache")),
        )

    private fun readThrough(session: SafWindowSession, requestId: Long, length: Int): Pair<Int, Int> {
        val dst = ByteArray(length)
        val info = IntArray(2)
        val code = session.readWindow(requestId, 0L, length, dst, info)
        return code to info[0]
    }

    @Test
    fun seekableSourceWithKnownLengthUsesPlatformWindow() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = sourceFor(platform)

        val result = source.intake(uri)

        assertTrue(result is SafIntakeResult.Ready)
        val ready = result as SafIntakeResult.Ready
        assertEquals(SafAccessMode.WINDOW, ready.mode)
        assertEquals(uri, ready.record.uri)
        assertEquals(SafTestData.NAME, ready.record.displayName)
        assertEquals(4096L, ready.record.sizeBytes)
        assertTrue(ready.record.permissionGranted)
        assertNull(ready.record.lastFailure)
        // 探测用的描述符必须归还：不留下无人关闭的句柄。
        assertEquals(1, platform.openDescriptorCalls)
        assertEquals(1, platform.closedDescriptorCalls)
        assertNotNull(source.recordOf(uri))
    }

    @Test
    fun nonSeekableProviderFallsBackToPrivateCacheCopy() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply { seekable = false }
        val result = sourceFor(platform).intake(uri)

        val ready = result as SafIntakeResult.Ready
        assertEquals(SafAccessMode.CACHE_COPY, ready.mode)
        assertEquals(SafAccessMode.CACHE_COPY, ready.record.accessMode)
        assertFalse(ready.record.hasCacheCopy)
    }

    @Test
    fun unknownLengthFallsBackToPrivateCacheCopy() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply {
            descriptorSize = -1L
            metadata = SafDocumentMeta(SafTestData.NAME, -1L, SafTestData.MIME_TXT)
        }
        val ready = sourceFor(platform).intake(uri) as SafIntakeResult.Ready
        // 平台窗口要求 total_bytes：长度未知就只能先复制（复制完成后才知道真实长度）。
        assertEquals(SafAccessMode.CACHE_COPY, ready.mode)
        assertEquals(-1L, ready.record.sizeBytes)
    }

    @Test
    fun emptyFileIsRejectedAndNothingIsPersisted() {
        val platform = FakeSafPlatform(ByteArray(0))
        val result = sourceFor(platform).intake(uri)

        val rejected = result as SafIntakeResult.Rejected
        assertEquals(SafRejectionReason.EMPTY_FILE, rejected.reason)
        assertFalse(rejected.requiresReassociation)
        assertTrue(kv.entries.isEmpty())
        assertTrue(rejected.detail.isNotEmpty())
    }

    @Test
    fun nonTextDocumentIsRejectedWithObservableReason() {
        val platform = FakeSafPlatform(SafTestData.bytes(64)).apply {
            metadata = SafDocumentMeta("封面.png", 64L, "image/png")
        }
        val rejected = sourceFor(platform).intake(uri) as SafIntakeResult.Rejected
        assertEquals(SafRejectionReason.NOT_TEXT, rejected.reason)
        assertTrue(rejected.detail.contains("image/png"))
        assertTrue(kv.entries.isEmpty())
    }

    @Test
    fun blankUriIsRejected() {
        val rejected = sourceFor(FakeSafPlatform()).intake("   ") as SafIntakeResult.Rejected
        assertEquals(SafRejectionReason.METADATA_UNAVAILABLE, rejected.reason)
        assertTrue(kv.entries.isEmpty())
    }

    @Test
    fun metadataUnavailableKeepsExistingRecordForReassociation() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = sourceFor(platform)
        source.intake(uri)
        source.attachBookId(uri, 42L)

        platform.metadata = null
        val result = source.intake(uri)

        val pending = result as SafIntakeResult.NeedsReassociation
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, pending.record.accessMode)
        assertEquals(42L, pending.record.bookId)
        assertEquals("元数据不可读：权限或来源已失效", pending.detail)
        // 「不删除书籍记录」：记录仍在，书架项与阅读进度都还在。
        assertEquals(1, source.records().size)
        assertEquals(42L, source.recordOf(uri)!!.bookId)
    }

    @Test
    fun descriptorOpenFailureLeadsToReassociation() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply { descriptorOpenable = false }
        val result = sourceFor(platform).intake(uri)
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, (result as SafIntakeResult.NeedsReassociation).record.accessMode)
        assertTrue(result.detail.contains("无法打开只读描述符"))
    }

    @Test
    fun permissionThatCannotBePersistedIsReportedNotHidden() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply { takePersistableSucceeds = false }
        val ready = sourceFor(platform).intake(uri) as SafIntakeResult.Ready

        assertFalse(ready.record.permissionGranted)
        assertNotNull(ready.record.lastFailure)
        assertTrue(ready.record.lastFailure!!.contains("重新关联"))
        // 本次仍可用：只是重启后需要重新关联，因此不能当成导入失败。
        assertEquals(SafAccessMode.WINDOW, ready.mode)
    }

    @Test
    fun reassociateMovesRecordToNewUriAndKeepsBookId() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = sourceFor(platform)
        val oldUri = SafTestData.uri("old")
        val newUri = SafTestData.uri("new")

        source.intake(oldUri)
        source.attachBookId(oldUri, 77L)
        source.onAccessFailure(oldUri, "权限已失效，请重新关联")

        val result = source.reassociate(oldUri, newUri)

        val ready = result as SafIntakeResult.Ready
        assertEquals(newUri, ready.record.uri)
        assertEquals(77L, ready.record.bookId)
        assertFalse(ready.record.needsReassociation)
        assertNull(ready.record.lastFailure)
        assertEquals(1, source.records().size)
        assertNull(source.recordOf(oldUri))
        assertNotNull(source.recordOf(newUri))
    }

    @Test
    fun reassociateWithoutPreviousUriBehavesLikePlainIntake() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = sourceFor(platform)
        val ready = source.reassociate(null, uri) as SafIntakeResult.Ready
        assertEquals(SafAccessMode.WINDOW, ready.mode)
        assertNull(ready.record.bookId)
    }

    @Test
    fun openWindowSessionOpensDescriptorAndCloseReleasesItExactlyOnce() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = sourceFor(platform)
        source.intake(uri)

        val session = source.openWindowSession(uri)
        assertNotNull(session)
        assertEquals(4096L, session!!.totalBytes)
        assertEquals(2, platform.openDescriptorCalls)
        val (code, length) = readThrough(session, 31L, 64)
        assertEquals(NrErrors.NR_ERR_OK, code)
        assertEquals(64, length)

        session.close()
        assertEquals(2, platform.closedDescriptorCalls)
        session.close()
        assertEquals(2, platform.closedDescriptorCalls)
        assertTrue(session.closed)
    }

    @Test
    fun openWindowSessionDeniedDescriptorMarksRecordForReassociation() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = sourceFor(platform)
        source.intake(uri)
        source.attachBookId(uri, 5L)
        platform.descriptorOpenable = false

        assertNull(source.openWindowSession(uri))

        val record = source.recordOf(uri)!!
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, record.accessMode)
        assertEquals(5L, record.bookId)
        assertEquals(1, source.records().size)
    }

    @Test
    fun openWindowSessionIsNotOfferedForCacheCopyRecords() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply { seekable = false }
        val source = sourceFor(platform)
        source.intake(uri)
        val opens = platform.openDescriptorCalls

        assertNull(source.openWindowSession(uri))

        assertEquals(opens, platform.openDescriptorCalls)
        assertEquals(SafAccessMode.CACHE_COPY, source.recordOf(uri)!!.accessMode)
    }

    @Test
    fun cancelledWindowRequestIsDeliveredToTheReader() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = sourceFor(platform)
        source.intake(uri)
        val session = source.openWindowSession(uri)!!

        source.cancelWindowRequest(99L)
        val (code, _) = readThrough(session, 99L, 64)
        assertEquals(NrErrors.NR_ERR_CANCELLED, code)

        source.completeWindowRequest(99L)
        val (retryCode, length) = readThrough(session, 99L, 64)
        assertEquals(NrErrors.NR_ERR_OK, retryCode)
        assertEquals(64, length)
        session.close()
    }

    @Test
    fun fallbackCopyStoresCachePathInTheRecord() {
        val content = SafTestData.bytes(200 * 1024)
        val platform = FakeSafPlatform(content).apply { seekable = false }
        val source = sourceFor(platform)
        val ready = source.intake(uri) as SafIntakeResult.Ready

        val outcome = source.startFallbackCopy(ready.record)

        val completed = outcome as SafCopyOutcome.Completed
        assertEquals(content.size.toLong(), completed.bytes)
        assertTrue(File(completed.cachePath).exists())
        val record = source.recordOf(uri)!!
        assertEquals(completed.cachePath, record.cachePath)
        assertEquals(content.size.toLong(), record.cacheBytes)
        assertTrue(record.hasCacheCopy)
        assertEquals(SafAccessMode.CACHE_COPY, record.accessMode)
        // 复制的流必须被关闭，且每次请求都不超过块大小。
        val stream = platform.lastStream!!
        assertTrue(stream.closed)
        assertTrue(stream.requestedLengths.all { it == SafPolicy.COPY_CHUNK_BYTES })
    }

    @Test
    fun fallbackCopyThatCannotOpenStreamMarksAccessFailureAndKeepsRecordClean() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply {
            seekable = false
            streamOpenable = false
        }
        val source = sourceFor(platform)
        val ready = source.intake(uri) as SafIntakeResult.Ready
        source.attachBookId(uri, 8L)

        val outcome = source.startFallbackCopy(ready.record)

        assertTrue(outcome is SafCopyOutcome.Failed)
        assertEquals(0L, (outcome as SafCopyOutcome.Failed).bytesCopied)
        val record = source.recordOf(uri)!!
        // 记录里不写假路径；同时进入「需要重新关联」，不静默失败。
        assertNull(record.cachePath)
        assertFalse(record.hasCacheCopy)
        assertTrue(record.needsReassociation)
        assertEquals(8L, record.bookId)
    }

    @Test
    fun purgeCacheClearsRecordCacheState() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply { seekable = false }
        val source = sourceFor(platform)
        val ready = source.intake(uri) as SafIntakeResult.Ready
        val completed = source.startFallbackCopy(ready.record) as SafCopyOutcome.Completed

        assertTrue(source.purgeCache(uri))

        assertFalse(File(completed.cachePath).exists())
        val record = source.recordOf(uri)!!
        assertNull(record.cachePath)
        assertFalse(record.hasCacheCopy)
    }

    @Test
    fun createFactoryUsesPlatformPrivateCacheRoot() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply {
            seekable = false
            cacheRootPath = File(folder.root, "platform-cache").absolutePath
        }
        val source = SafBookSource.create(platform, kv)
        val ready = source.intake(uri) as SafIntakeResult.Ready

        val completed = source.startFallbackCopy(ready.record) as SafCopyOutcome.Completed

        assertTrue(completed.cachePath.startsWith(platform.cacheRootPath))
        assertTrue(File(completed.cachePath).exists())
    }
}
