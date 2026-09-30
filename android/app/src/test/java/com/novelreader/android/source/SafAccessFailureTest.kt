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
 * 权限失效与重新关联（卡片：「权限失效进入需要重新关联，不删除书籍记录」；AR6 真机延期边界）。
 *
 * 断言重点：
 * 1. 窗口读取 I/O 失败**必须**能被转成「需要重新关联」状态，而不是静默什么都不发生；
 * 2. 转换过程中**记录与 `bookId` 都保留**（书不会从书架消失）；
 * 3. 权限恢复后记录能回到可用状态。
 */
class SafAccessFailureTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val kv = MemoryKeyValueStore()

    private val uri = SafTestData.uri()

    private fun source(platform: SafPlatform): SafBookSource =
        SafBookSource(
            platform = platform,
            store = SafSourceStore(kv),
            cache = SafCacheStore(File(folder.root, "cache")),
        )

    @Test
    fun accessFailureKeepsRecordAndBookId() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = source(platform)
        source.intake(uri)
        source.attachBookId(uri, 1234L)

        val updated = source.onAccessFailure(uri, "读取正文窗口失败：权限已失效")

        assertNotNull(updated)
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, updated!!.accessMode)
        assertEquals(1234L, updated.bookId)
        assertFalse(updated.permissionGranted)
        assertEquals("读取正文窗口失败：权限已失效", updated.lastFailure)
        assertEquals(1, source.records().size)
        assertEquals(1234L, source.recordOf(uri)!!.bookId)
    }

    @Test
    fun accessFailureForUnknownUriReturnsNullWithoutWriting() {
        val source = source(FakeSafPlatform(SafTestData.bytes(16)))
        assertNull(source.onAccessFailure(SafTestData.uri("ghost"), "权限已失效"))
        assertTrue(kv.entries.isEmpty())
    }

    @Test
    fun windowReadIoFailureCanBeTurnedIntoReassociationKeepingTheBook() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = source(platform)
        source.intake(uri)
        source.attachBookId(uri, 555L)
        val session = source.openWindowSession(uri)!!

        // 权限在打开之后失效：后续窗口读取全部 I/O 失败（而不是静默返回 0 字节）。
        platform.failingFrom = 0L
        val info = IntArray(2)
        val code = session.readWindow(1L, 0L, 64, ByteArray(64), info)
        assertEquals(NrErrors.NR_ERR_IO, code)
        assertEquals(0, info[0])
        assertNotNull(session.reader.lastIoFailure)

        val updated = source.onAccessFailure(uri, session.reader.lastIoFailure!!)
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, updated!!.accessMode)
        assertEquals(555L, updated.bookId)
        assertEquals(1, source.records().size)
        session.close()
    }

    @Test
    fun permissionRestoredRefreshesStateAndReintakeRestoresWindowMode() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096)).apply { takePersistableSucceeds = false }
        val source = source(platform)
        var ready = source.intake(uri) as SafIntakeResult.Ready
        source.attachBookId(uri, 9L)
        assertFalse(ready.record.permissionGranted)

        // 用户在系统设置里重新授权（或重新选择文件）之后：系统里出现持久读授权。
        platform.takePersistableSucceeds = true
        platform.persistedFlags = 1
        val refreshed = source.onPermissionRestored(uri)
        assertTrue(refreshed!!.permissionGranted)
        assertNull(refreshed.lastFailure)

        ready = source.intake(uri) as SafIntakeResult.Ready
        assertEquals(SafAccessMode.WINDOW, ready.mode)
        assertEquals(9L, ready.record.bookId)
        assertTrue(ready.record.permissionGranted)
        assertNull(ready.record.lastFailure)
        assertEquals(1, source.records().size)
    }

    @Test
    fun reassociationTwoStepFlowSurvivesUriChange() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = source(platform)
        val oldUri = SafTestData.uri("2024")
        val newUri = SafTestData.uri("2025")

        source.intake(oldUri)
        source.attachBookId(oldUri, 2024L)
        // 权限失效 → 记录转入需要重新关联（保留书）。
        source.onAccessFailure(oldUri, "权限已失效")
        // 用户重新选择同一个文件（URI 可能变化）→ 记录迁到新 URI，bookId 不变。
        val ready = source.reassociate(oldUri, newUri) as SafIntakeResult.Ready

        assertEquals(2024L, ready.record.bookId)
        assertEquals(newUri, ready.record.uri)
        assertFalse(ready.record.needsReassociation)
        assertEquals(1, source.records().size)
        assertNotNull(source.openWindowSession(newUri))
    }

    @Test
    fun deniedReassociationKeepsPendingRecordInsteadOfDeletingIt() {
        val platform = FakeSafPlatform(SafTestData.bytes(4096))
        val source = source(platform)
        source.intake(uri)
        source.attachBookId(uri, 31L)

        // 用户再次选择后来源仍然不可读（例如文件被移动到别的账号下）：状态保持「需要重新关联」。
        platform.metadata = null
        val result = source.intake(uri)
        assertTrue(result is SafIntakeResult.NeedsReassociation)

        assertEquals(31L, source.recordOf(uri)!!.bookId)
        assertEquals(1, source.records().size)
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, source.records().first().accessMode)
    }
}
