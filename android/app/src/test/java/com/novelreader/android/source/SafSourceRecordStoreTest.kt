package com.novelreader.android.source

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 来源记录持久化（卡片交付物：保存 URI、显示名、大小与权限状态；权限失效**不删记录**）。
 *
 * 断言重点：字段往返无损、损坏可检出（不返回半真记录）、重新关联保留 `bookId`。
 */
class SafSourceRecordStoreTest {

    private val kv = MemoryKeyValueStore()

    private val store = SafSourceStore(kv)

    private fun record(
        uri: String = SafTestData.uri(),
        name: String = SafTestData.NAME,
        bookId: Long? = null,
    ): SafSourceRecord = SafSourceRecord(
        uri = uri,
        displayName = name,
        sizeBytes = 2048L,
        mimeType = SafTestData.MIME_TXT,
        accessMode = SafAccessMode.WINDOW,
        permissionGranted = true,
        bookId = bookId,
    )

    @Test
    fun recordRoundTripKeepsEveryField() {
        val original = record(name = "带=等号 与\n换行.txt", bookId = 42L).copy(
            accessMode = SafAccessMode.CACHE_COPY,
            permissionGranted = false,
            cachePath = "/data/cache/saf-cache/abc.txt",
            cacheBytes = 2048L,
            lastFailure = "未取得持久读权限：重启应用后该来源需要重新关联",
        )
        store.save(original)
        assertEquals(original, store.load(original.uri))
    }

    @Test
    fun corruptValuesAreDetectedInsteadOfReturningHalfRecords() {
        assertNull(SafSourceCodec.decode(""))
        assertNull(SafSourceCodec.decode("displayName=x\nsizeBytes=10\naccessMode=WINDOW"))
        assertNull(SafSourceCodec.decode("uri=u\nsizeBytes=abc\naccessMode=WINDOW"))
        assertNull(SafSourceCodec.decode("uri=u\nsizeBytes=10\naccessMode=NOT_A_MODE"))
        assertNull(SafSourceCodec.decode("uri=\nsizeBytes=10\naccessMode=WINDOW"))
        // 未知键被忽略（向前兼容），必需字段齐备即可解出。
        assertNotNull(SafSourceCodec.decode("uri=u\nsizeBytes=10\naccessMode=WINDOW\nfutureKey=v"))
    }

    @Test
    fun corruptedEntryIsSkippedByListAndDoesNotAffectOthers() {
        store.save(record())
        kv.put("source:broken", "sizeBytes=1\naccessMode=WINDOW")
        // 前缀之外的键（例如将来的其它偏好）不能混进来源列表。
        kv.put("other:key", "uri=u\nsizeBytes=1\naccessMode=WINDOW")
        val all = store.list()
        assertEquals(1, all.size)
        assertEquals(SafTestData.uri(), all.first().uri)
    }

    @Test
    fun markNeedsReassociationKeepsRecordAndBookId() {
        store.save(record(bookId = 99L))
        val updated = store.markNeedsReassociation(SafTestData.uri(), "权限已失效，请重新关联")
        assertNotNull(updated)
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, updated!!.accessMode)
        assertTrue(updated.needsReassociation)
        assertFalse(updated.permissionGranted)
        assertEquals("权限已失效，请重新关联", updated.lastFailure)
        // 记录仍在存储里：书籍与阅读进度不会因一次权限丢失而消失。
        assertEquals(1, store.list().size)
        assertEquals(99L, store.load(SafTestData.uri())!!.bookId)
    }

    @Test
    fun markNeedsReassociationOnUnknownUriReturnsNull() {
        assertNull(store.markNeedsReassociation(SafTestData.uri("unknown"), "权限已失效"))
        assertTrue(kv.entries.isEmpty())
    }

    @Test
    fun reassociateMovesRecordToNewUriAndKeepsBookId() {
        val oldUri = SafTestData.uri("old")
        val newUri = SafTestData.uri("new")
        store.save(record(uri = oldUri, bookId = 7L).copy(accessMode = SafAccessMode.NEEDS_REASSOCIATION))
        store.save(record(uri = newUri))

        val merged = store.reassociate(oldUri, newUri)

        assertNotNull(merged)
        assertEquals(7L, merged!!.bookId)
        assertEquals(newUri, merged.uri)
        assertNull(merged.lastFailure)
        assertNull(store.load(oldUri))
        assertEquals(1, store.list().size)
        assertEquals(7L, store.findByBookId(7L)!!.bookId)
    }

    @Test
    fun reassociateWithoutPreviousRecordChangesNothing() {
        store.save(record(uri = SafTestData.uri("new")))
        assertNull(store.reassociate(SafTestData.uri("missing"), SafTestData.uri("new")))
        assertEquals(1, store.list().size)
        assertEquals(1, kv.entries.size)
    }

    @Test
    fun attachBookIdIsVisibleThroughFindByBookId() {
        store.save(record())
        val updated = store.attachBookId(SafTestData.uri(), 12345678901234L)
        assertEquals(12345678901234L, updated!!.bookId)
        assertEquals(SafTestData.uri(), store.findByBookId(12345678901234L)!!.uri)
        assertEquals(12345678901234L, store.load(SafTestData.uri())!!.bookId)
    }

    @Test
    fun cacheStateCanBeSetAndCleared() {
        store.save(record().copy(accessMode = SafAccessMode.CACHE_COPY))
        val withCache = store.updateCache(SafTestData.uri(), "/data/cache/saf-cache/x.txt", 4096L)
        assertTrue(withCache!!.hasCacheCopy)
        assertEquals(4096L, withCache.cacheBytes)

        val cleared = store.updateCache(SafTestData.uri(), null, 4096L)
        assertFalse(cleared!!.hasCacheCopy)
        assertEquals(SafPolicy.UNKNOWN_SIZE, cleared.cacheBytes)
        assertNull(cleared.cachePath)
    }

    @Test
    fun refreshPermissionDrivesVisibleFailureText() {
        store.save(record().copy(permissionGranted = false, lastFailure = "旧的失败说明"))
        assertNull(store.refreshPermission(SafTestData.uri(), true)!!.lastFailure)
        assertNotNull(store.refreshPermission(SafTestData.uri(), false)!!.lastFailure)
    }

    @Test
    fun updateAccessModeOnlyTouchesMode() {
        store.save(record(bookId = 5L))
        val updated = store.updateAccessMode(SafTestData.uri(), SafAccessMode.NEEDS_REASSOCIATION)
        assertEquals(SafAccessMode.NEEDS_REASSOCIATION, updated!!.accessMode)
        assertEquals(5L, updated.bookId)
        assertTrue(updated.permissionGranted)
    }

    @Test
    fun removeDropsTheRecord() {
        store.save(record())
        store.remove(SafTestData.uri())
        assertNull(store.load(SafTestData.uri()))
        assertTrue(store.list().isEmpty())
    }
}
