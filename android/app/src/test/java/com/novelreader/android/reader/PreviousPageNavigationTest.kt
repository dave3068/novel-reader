package com.novelreader.android.reader

import com.novelreader.android.source.MemoryKeyValueStore
import com.novelreader.android.source.ReaderAnchorPrefsStore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 「上一页」可达性回归（真机缺陷）。
 *
 * 真机实测两处错误置灰：
 * ① 点「下一章」后置灰（旧实现切章时清空了边界栈）；
 * ② 退出重进、恢复位置后置灰（边界栈是会话内的，恢复时为空）。
 *
 * 判定依据是「是否可后退 + 退回的落点」，不是界面文案。
 * 规格依据：`docs/novel-reader-v1-development-plan.md:481` ——
 * 「上一页优先使用边界栈或**持久化稀疏锚点**，不能永久保存全书页面」。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PreviousPageNavigationTest {

    private fun harnessFor(scope: TestScope, store: ReaderAnchorStore? = null): ReaderHarness =
        ReaderHarness(
            io = StandardTestDispatcher(scope.testScheduler),
            scope = TestScope(scope.testScheduler),
            anchorStore = store,
        )

    @Test
    fun chapterJumpKeepsPreviousPageReachable() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        harness.viewModel.nextPage()
        harness.settle(this)
        harness.viewModel.nextPage()
        harness.settle(this)
        val beforeJump = requireNotNull(harness.viewModel.ui.value.reader.page) { "夹具前进两页后应有页面" }
        assertTrue("夹具前进两页后应已离开书首", beforeJump.startByte > 0L)

        harness.viewModel.nextChapter()
        harness.settle(this)
        val atNextChapter = requireNotNull(harness.viewModel.ui.value.reader.page) { "切章后应有页面" }
        assertTrue("应已切到后一章", atNextChapter.chapterIndex > beforeJump.chapterIndex)
        assertTrue(
            "切章后「上一页」必须可用（真机缺陷回归：旧实现此处清空了边界栈）",
            harness.viewModel.ui.value.reader.canGoPreviousPage,
        )

        harness.viewModel.previousPage()
        harness.settle(this)
        val back = requireNotNull(harness.viewModel.ui.value.reader.page) { "后退后应有页面" }
        assertEquals("应回到切章前的原页（即上一章的最后一页）", beforeJump.chapterIndex, back.chapterIndex)
        assertEquals(beforeJump.startByte, back.startByte)
    }

    @Test
    fun bookStartKeepsPreviousPageDisabled() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        val first = requireNotNull(harness.viewModel.ui.value.reader.page) { "打开后应有页面" }
        assertEquals(0, first.chapterIndex)
        assertEquals(0L, first.startByte)
        assertFalse("全书第一页必须置灰（唯一应置灰的场景）", harness.viewModel.ui.value.reader.canGoPreviousPage)
    }

    @Test
    fun pagingPersistsSparseAnchors() = runTest {
        val store = MemoryAnchorStore()
        val harness = harnessFor(this, store)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        assertEquals("首帧（第 1 页）应已记录锚点", 1, store.saved.size)
        harness.viewModel.nextPage()
        harness.settle(this)

        val lastSaved = store.saved.values.last().last()
        assertTrue("锚点表应随翻页增长", lastSaved.size >= 2)
        assertTrue("锚点应落在合法字节上", lastSaved.all { it.startByte >= 0L && it.chapterIndex >= 0 })
    }

    @Test
    fun restoredPositionSeedsPreviousPageFromPersistedAnchors() = runTest {
        val store = MemoryAnchorStore()
        val harness = harnessFor(this, store)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        assertTrue("打开后应已产生持久化锚点", store.saved.isNotEmpty())

        // 模拟「退出应用后重进」：位置由核心给出，落点由宿主持久化的稀疏锚点提供
        store.loadResult = listOf(PageAnchor(chapterIndex = 0, startByte = 0L))
        harness.abi.positionFound = 1L
        harness.abi.positionChapterIndex = 0L
        harness.abi.positionOffset = 83L

        harness.viewModel.closeBook()
        harness.settle(this)
        harness.viewModel.openBook("/tmp/novel-reader-fixture.txt", "fixture.txt")
        harness.settle(this)

        val restored = requireNotNull(harness.viewModel.ui.value.reader.page) { "恢复后应有页面" }
        assertEquals("应恢复到核心给出的偏移", 83L, restored.startByte)
        assertTrue(
            "恢复位置后「上一页」必须可用（真机缺陷回归：旧实现恢复后栈为空即置灰）",
            harness.viewModel.ui.value.reader.canGoPreviousPage,
        )
    }

    @Test
    fun persistedAnchorsAreScopedByLayoutSignature() {
        val store = ReaderAnchorPrefsStore(MemoryKeyValueStore())
        store.save("/book.txt", "sig-A", listOf(PageAnchor(1, 10L), PageAnchor(1, 20L)))

        assertEquals(2, store.load("/book.txt", "sig-A").size)
        assertTrue(
            "排版签名不一致必须返回空表（旧边界在新排版里不再是边界）",
            store.load("/book.txt", "sig-B").isEmpty(),
        )
        assertTrue("未记录的书返回空表", store.load("/other.txt", "sig-A").isEmpty())
    }

    /** 内存版锚点端口（记录调用，供断言）。 */
    private class MemoryAnchorStore : ReaderAnchorStore {

        /** bookKey|signature → 历次保存的锚点表。 */
        val saved = LinkedHashMap<String, MutableList<List<PageAnchor>>>()

        var loadResult: List<PageAnchor> = emptyList()

        override fun load(bookKey: String, signature: String): List<PageAnchor> = loadResult

        override fun save(bookKey: String, signature: String, anchors: List<PageAnchor>) {
            saved.getOrPut("$bookKey|$signature") { mutableListOf() } += anchors.toList()
        }
    }
}
