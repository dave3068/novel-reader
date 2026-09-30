package com.novelreader.android.reader

import com.novelreader.android.core.NrAbiConstants
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 书架「移除」（用户要求：避免为刷新缓存而卸载应用）。
 *
 * 用户裁定语义：**仅移除书架 + 清空本书本地索引（保留阅读位置），绝不删除原文件**。
 * 本用例只验证应用自身状态与持久化行为；「不删原文件」由实现约束保证
 * （端口只存标识字符串，代码内不出现任何文件写入；证据见 results/shelf-remove.md）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ShelfRemovalTest {

    /** 内存版「已移除」集合。 */
    private class FakeHiddenBooksStore : HiddenBooksStore {
        var saved: Set<String> = emptySet()
            private set
        var loads = 0
            private set

        override fun load(): Set<String> {
            loads += 1
            return saved
        }

        override fun save(identities: Set<String>) {
            saved = identities
        }
    }

    private fun harnessFor(
        scope: TestScope,
        hidden: FakeHiddenBooksStore?,
    ): ReaderHarness = ReaderHarness(
        io = StandardTestDispatcher(scope.testScheduler),
        scope = TestScope(scope.testScheduler),
        hiddenBooks = hidden,
    )

    @Test
    fun removalHidesBookAndPersistsIdentity() = runTest {
        val hidden = FakeHiddenBooksStore()
        val harness = harnessFor(this, hidden)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        val book = harness.viewModel.ui.value.shelf.books.firstOrNull()
            ?: ShelfBook(
                sourceIdentity = "content://fixture/book-1",
                displayName = "夹具书",
                bookId = 1L,
                totalBytes = 621L,
                chapterCount = 3,
            )

        harness.viewModel.removeBook(book)
        harness.settle(this)

        assertTrue(
            "移除后不得再出现在书架列表",
            harness.viewModel.ui.value.shelf.books.none { it.sourceIdentity == book.sourceIdentity },
        )
        assertTrue(
            "被移除的来源必须持久化（否则重启后又出现）",
            hidden.saved.contains(book.sourceIdentity),
        )
        assertTrue(
            "移除必须给出明确状态（且说明未删原文件）",
            harness.viewModel.ui.value.status.contains("已移除"),
        )
    }

    @Test
    fun catalogRebuildDoesNotResurrectRemovedBook() = runTest {
        val identity = "content://fixture/removed-book"
        // 模拟上次会话已移除该书：本地集合里已有该标识。
        val hidden = FakeHiddenBooksStore().apply { save(setOf(identity)) }
        val catalog = FakeCatalog(
            listOf(
                HostBookEntry(sourceIdentity = identity, displayName = "已移除的书", sourceKind = NrAbiConstants.NR_SOURCE_KIND_PLATFORM),
                HostBookEntry(
                    sourceIdentity = "content://fixture/kept-book",
                    displayName = "保留的书",
                    sourceKind = NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
                ),
            ),
        )
        val harness = ReaderHarness(
            io = StandardTestDispatcher(testScheduler),
            scope = TestScope(testScheduler),
            bookCatalog = catalog,
            hiddenBooks = hidden,
        )
        harness.settle(this)

        val identities = harness.viewModel.ui.value.shelf.books.map { it.sourceIdentity }
        assertFalse("已移除的书不得由目录重建复活", identities.contains(identity))
        assertTrue("未移除的书必须照常出现", identities.contains("content://fixture/kept-book"))
    }

    @Test
    fun reimportingRestoresVisibility() = runTest {
        val identity = "content://fixture/removed-again"
        val hidden = FakeHiddenBooksStore().apply { save(setOf(identity)) }
        val catalog = FakeCatalog(listOf(HostBookEntry(sourceIdentity = identity, displayName = "重新导入的书", sourceKind = NrAbiConstants.NR_SOURCE_KIND_PLATFORM)))
        val harness = ReaderHarness(
            io = StandardTestDispatcher(testScheduler),
            scope = TestScope(testScheduler),
            bookCatalog = catalog,
            hiddenBooks = hidden,
        )
        harness.settle(this)
        assertTrue(
            "前提：导入前应处于已移除状态",
            harness.viewModel.ui.value.shelf.books.isEmpty(),
        )

        harness.viewModel.restoreToShelf(identity)
        harness.settle(this)

        assertEquals(
            "重新导入后必须恢复显示",
            listOf(identity),
            harness.viewModel.ui.value.shelf.books.map { it.sourceIdentity },
        )
        assertFalse("恢复后不得再被持久化为已移除", hidden.saved.contains(identity))
    }

    /** 内存版宿主目录。 */
    private class FakeCatalog(private val entries: List<HostBookEntry>) : HostBookCatalog {
        override fun list(): List<HostBookEntry> = entries

        override fun rememberBookId(sourceIdentity: String, bookId: Long) = Unit
    }
}
