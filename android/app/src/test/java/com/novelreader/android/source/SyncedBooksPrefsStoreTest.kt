package com.novelreader.android.source

import com.novelreader.android.reader.SyncedBookEntry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SyncedBooksPrefsStoreTest {

    @Test
    fun roundTripsAndIgnoresMalformedLines() {
        val kv = MemoryKeyValueStore()
        val store = SyncedBooksPrefsStore(kv)
        val entries = listOf(
            SyncedBookEntry(1L, "中文书名\t带制表符", "/data/user/0/app/files/state/sync-books/a.txt", 123L),
            SyncedBookEntry(2L, "第二本", "/data/user/0/app/files/state/sync-books/b.txt", 0L),
        )
        store.save(entries)
        assertEquals(entries, store.load())

        // 版本不匹配 / 字段缺失的行必须丢弃，不得把半截记录当成可打开来源。
        kv.put("synced_books", "v1\nbad\n3\t!!!\t!!!\t0\nv1\n4\tZm9v\tL3RtcC9mb28udHh0\t9")
        val loaded = store.load()
        assertEquals(1, loaded.size)
        assertEquals(4L, loaded[0].bookId)
        assertEquals("foo", loaded[0].displayName)
        assertEquals("/tmp/foo.txt", loaded[0].sourceLocator)
        assertEquals(9L, loaded[0].sourceSize)
    }

    @Test
    fun skipsInvalidEntriesOnSave() {
        val kv = MemoryKeyValueStore()
        val store = SyncedBooksPrefsStore(kv)
        store.save(
            listOf(
                SyncedBookEntry(0L, "无 id", "/tmp/a.txt", 1L),
                SyncedBookEntry(1L, "空路径", " ", 1L),
                SyncedBookEntry(2L, "正常", "/tmp/b.txt", 2L),
            ),
        )
        val loaded = store.load()
        assertEquals(1, loaded.size)
        assertEquals(2L, loaded[0].bookId)
        assertTrue(loaded[0].sourceLocator.endsWith("b.txt"))
    }
}
