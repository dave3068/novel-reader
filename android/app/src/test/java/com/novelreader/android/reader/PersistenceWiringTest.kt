package com.novelreader.android.reader

import com.novelreader.android.source.MemoryKeyValueStore
import com.novelreader.android.source.ReaderSettingsPrefsStore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 重启持久化回归（真机缺陷：退出应用后书架被清空、字体与夜间模式被重置）。
 *
 * 机制：宿主侧持久化（SAF 记录 → 书架目录；KV → 排版设置）。
 * 断言落在「状态与端口调用」上，不看界面文案。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PersistenceWiringTest {

    private fun harnessFor(
        scope: TestScope,
        settingsStore: ReaderSettingsStore? = null,
        bookCatalog: HostBookCatalog? = null,
    ): ReaderHarness = ReaderHarness(
        io = StandardTestDispatcher(scope.testScheduler),
        scope = TestScope(scope.testScheduler),
        settingsStore = settingsStore,
        bookCatalog = bookCatalog,
    )

    @Test
    fun persistedSettingsAreRestoredOnStartup() = runTest {
        val store = MemorySettingsStore(
            ReaderSettingsSnapshot(
                fontSizeSp = 26f,
                lineSpacingMultiplier = 1.8f,
                horizontalMarginDp = 32f,
                verticalMarginDp = 40f,
                darkTheme = true,
            ),
        )
        val harness = harnessFor(this, settingsStore = store)
        harness.settle(this)

        val settings = harness.viewModel.ui.value.settings
        assertEquals(26f, settings.fontSizeSp, 0.001f)
        assertEquals(1.8f, settings.lineSpacingMultiplier, 0.001f)
        assertEquals(32f, settings.horizontalMarginDp, 0.001f)
        assertEquals(40f, settings.verticalMarginDp, 0.001f)
        assertTrue("夜间模式必须恢复（真机缺陷回归）", settings.darkTheme)
    }

    @Test
    fun settingsChangesArePersisted() = runTest {
        val store = MemorySettingsStore(null)
        val harness = harnessFor(this, settingsStore = store)
        harness.settle(this)

        harness.viewModel.onFontSizeChange(24f)
        harness.settle(this)
        assertEquals(24f, store.saved.last().fontSizeSp, 0.001f)

        harness.viewModel.onDarkThemeChange(true)
        harness.settle(this)
        assertTrue("夜间模式开关必须落库（真机缺陷回归）", store.saved.last().darkTheme)
        assertEquals("落库快照必须包含同期字号", 24f, store.saved.last().fontSizeSp, 0.001f)
    }

    @Test
    fun shelfIsRebuiltFromHostCatalog() = runTest {
        val catalog = MemoryBookCatalog(
            listOf(
                HostBookEntry("/tmp/a.txt", "甲书.txt", com.novelreader.android.core.NrAbiConstants.NR_SOURCE_KIND_FILE),
                HostBookEntry("content://d/1", "乙书.txt", com.novelreader.android.core.NrAbiConstants.NR_SOURCE_KIND_PLATFORM),
            ),
        )
        val harness = harnessFor(this, bookCatalog = catalog)
        harness.settle(this)

        val books = harness.viewModel.ui.value.shelf.books
        assertEquals("重启后书架必须由宿主目录重建（真机缺陷回归）", 2, books.size)
        assertEquals(listOf("甲书.txt", "乙书.txt"), books.map { it.displayName })
        assertEquals(
            com.novelreader.android.core.NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
            books.last().sourceKind,
        )
    }

    @Test
    fun openedBookIdIsReportedBackToHost() = runTest {
        val catalog = MemoryBookCatalog(emptyList())
        val harness = harnessFor(this, bookCatalog = catalog)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        assertEquals("打开成功后必须回填 book_id，宿主记录才能关联核心进度", 1, catalog.remembered.size)
        assertEquals("/tmp/novel-reader-fixture.txt", catalog.remembered.first().first)
        assertTrue("book_id 必须来自核心回执", catalog.remembered.first().second > 0L)
    }

    @Test
    fun shelfEntriesAreNotOverwrittenByCatalogPlaceholders() = runTest {
        val catalog = MemoryBookCatalog(
            listOf(
                HostBookEntry(
                    "/tmp/novel-reader-fixture.txt",
                    "已打开.txt",
                    com.novelreader.android.core.NrAbiConstants.NR_SOURCE_KIND_FILE,
                ),
            ),
        )
        val harness = harnessFor(this, bookCatalog = catalog)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        val entry = harness.viewModel.ui.value.shelf.books.first { it.sourceIdentity == "/tmp/novel-reader-fixture.txt" }
        assertTrue("打开过的条目必须保留真实章节数，不被目录占位值覆盖", entry.chapterCount > 0)
        assertTrue("打开过的条目必须保留真实 bookId", entry.bookId > 0L)
    }

    @Test
    fun settingsStoreRoundTripAndRejectsBrokenRecords() {
        val kv = MemoryKeyValueStore()
        val store = ReaderSettingsPrefsStore(kv)

        assertNull("无记录时返回 null（调用方保持默认值）", store.load())

        store.save(
            ReaderSettingsSnapshot(
                fontSizeSp = 21.5f,
                lineSpacingMultiplier = 1.25f,
                horizontalMarginDp = 12f,
                verticalMarginDp = 14f,
                darkTheme = false,
            ),
        )
        val loaded = store.load()
        assertEquals(21.5f, loaded!!.fontSizeSp, 0.001f)
        assertEquals(1.25f, loaded.lineSpacingMultiplier, 0.001f)
        assertEquals(12f, loaded.horizontalMarginDp, 0.001f)
        assertEquals(14f, loaded.verticalMarginDp, 0.001f)
        assertFalse(loaded.darkTheme)

        kv.put("reader.settings.v1", "v1;fontSizeSp=18.0")
        assertNull("半截记录必须视为无记录，不得采用", store.load())
    }

    /** 内存版设置端口。 */
    private class MemorySettingsStore(initial: ReaderSettingsSnapshot?) : ReaderSettingsStore {
        private var current: ReaderSettingsSnapshot? = initial
        val saved = mutableListOf<ReaderSettingsSnapshot>()
        override fun load(): ReaderSettingsSnapshot? = current
        override fun save(snapshot: ReaderSettingsSnapshot) {
            current = snapshot
            saved += snapshot
        }
    }

    /** 内存版书架目录端口。 */
    private class MemoryBookCatalog(private val entries: List<HostBookEntry>) : HostBookCatalog {
        val remembered = mutableListOf<Pair<String, Long>>()
        override fun list(): List<HostBookEntry> = entries
        override fun rememberBookId(sourceIdentity: String, bookId: Long) {
            remembered += sourceIdentity to bookId
        }
    }
}
