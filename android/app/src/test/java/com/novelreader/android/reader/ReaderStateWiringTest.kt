package com.novelreader.android.reader

import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrErrorKind
import com.novelreader.android.core.NrErrors
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 三页面与真实状态接线（A-T3）：路由、控件回调、过期/被拒结果的处置、缓冲区归还。
 *
 * 「空回调清零」由 `scripts/verify-android-scope.py` 的 `[3b]` 静态门禁保证；
 * 本文件证明同一批入口在真实状态上的**行为**（不是占位实现）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReaderStateWiringTest {

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(io = StandardTestDispatcher(scope.testScheduler), scope = TestScope(scope.testScheduler))

    private fun openAndSettle(harness: ReaderHarness, scope: TestScope) {
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(scope)
    }

    @Test
    fun routeSwitchesFollowRealStateOnly() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val viewModel = harness.viewModel

        viewModel.openSettings()
        assertEquals(ReaderRoute.SETTINGS, viewModel.ui.value.route)

        viewModel.openShelf()
        assertEquals(ReaderRoute.SHELF, viewModel.ui.value.route)
        assertEquals("书架页仍保留真实阅读状态", 901L, viewModel.ui.value.reader.bookId)

        viewModel.openReader()
        assertEquals(ReaderRoute.READER, viewModel.ui.value.route)

        viewModel.closeBook()
        harness.settle(this)
        assertEquals(ReaderRoute.SHELF, viewModel.ui.value.route)
        assertNull("关闭书籍后阅读状态必须清空", viewModel.ui.value.reader.bookId)
        assertTrue(viewModel.ui.value.reader.chapters.isEmpty())

        // 未打开书籍时进入阅读页：不得假装成功，必须保留真实状态并给出提示
        viewModel.openReader()
        assertEquals(ReaderRoute.SHELF, viewModel.ui.value.route)
        assertEquals(ReaderStatus.NO_BOOK, viewModel.ui.value.status)
    }

    @Test
    fun staleBoundarySubmissionIsDiscardedAndRetried() = runTest {
        val harness = harnessFor(this)
        // 第一次 COMMIT 被判为过期（排版世代已变）
        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_LAYOUT_ID)
        openAndSettle(harness, this)

        assertEquals("过期结果必须触发重新登记排版世代", 2, harness.abi.beginLayoutCalls)
        assertTrue("过期提交之后必须重试", harness.abi.commitCalls >= 2)
        val page = harness.viewModel.ui.value.reader.page
        assertNotNull("重试后必须拿到页面", page)
        assertEquals(0L, page!!.startByte)
        assertEquals(ReaderHarness.PAGE_ONE_END, page.endByte)
        assertEquals(
            "页面文本必须来自同一段真实字节",
            String(harness.book.bytes.copyOfRange(0, page.endByte.toInt()), Charsets.UTF_8),
            page.text,
        )
        assertNull(harness.viewModel.ui.value.failure)
    }

    @Test
    fun rejectedBoundaryPublishesFailureAndDoesNotFakeSuccess() = runTest {
        val harness = harnessFor(this)
        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_REJECTED)
        openAndSettle(harness, this)

        val rejected = harness.viewModel.ui.value
        assertNull("被拒的页边界不得发布成页面", rejected.reader.page)
        val failure = rejected.failure
        assertNotNull("被拒必须有错误状态", failure)
        assertTrue(rejected.status.contains("rejection"))

        // 失败不得粘住：下一次排版（脚本已耗尽）必须恢复正常
        harness.viewModel.requestRelayout()
        harness.settle(this)
        val recovered = harness.viewModel.ui.value
        assertNotNull(recovered.reader.page)
        assertNull(recovered.failure)
    }

    @Test
    fun rejectedPrefetchCommitKeepsCurrentPageIntact() = runTest {
        val harness = harnessFor(this)
        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_COMMITTED)
        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_REJECTED)
        openAndSettle(harness, this)

        val state = harness.viewModel.ui.value
        val page = state.reader.page
        assertNotNull("当前页必须已发布（第一次提交成功）", page)
        assertEquals(0L, page!!.startByte)
        assertEquals(ReaderHarness.PAGE_ONE_END, page.endByte)
        assertEquals("被拒的预取页不得进入缓存", 0, state.reader.forwardCacheCount)
        assertTrue(harness.abi.commitCalls >= 2)
    }

    @Test
    fun openFailureMapsToErrorKindWithoutFakingSuccess() = runTest {
        val harness = harnessFor(this)
        harness.abi.openTxtCode = NrErrors.NR_ERR_BOOK_NOT_FOUND
        openAndSettle(harness, this)

        val state = harness.viewModel.ui.value
        assertEquals("打开失败不得登记书架条目", 0, state.shelf.books.size)
        assertFalse(state.shelf.loading)
        val failure = state.failure
        assertNotNull(failure)
        assertEquals(NrErrors.NR_ERR_BOOK_NOT_FOUND, failure!!.rawCode)
        assertEquals(NrErrorKind.NOT_FOUND, failure.kind)
        assertTrue(state.status.contains(messageFor(NrErrorKind.NOT_FOUND)))
        assertNull(state.reader.page)
    }

    @Test
    fun everyReturnedBufferIsReleasedExactlyOnce() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        harness.viewModel.nextPage()
        harness.viewModel.nextChapter()
        harness.viewModel.savePositionNow()
        harness.settle(this)

        val released = harness.releaser.calls
        assertEquals("同一块缓冲区不得重复归还", released.size, released.distinct().size)
        assertEquals(
            "窗口 + 章节批次 + 位置指纹 三类缓冲区各归还一次",
            harness.abi.readWindowCalls + harness.abi.getChaptersCalls + 1,
            released.size,
        )
    }

    @Test
    fun indexRebuildRefreshesChaptersAndRelayouts() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val chaptersBefore = harness.viewModel.ui.value.reader.chapters.size

        harness.viewModel.rebuildIndex()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertEquals(chaptersBefore, state.reader.chapters.size)
        assertFalse(state.reader.indexRebuilding)
        assertTrue(
            "索引重建后必须按新世代重新排版（状态回显本次动作）",
            state.status.contains("索引重建后重排"),
        )
        assertEquals("索引重建后必须按新索引版本重新登记排版世代", 2, harness.abi.beginLayoutCalls)
        assertNotNull(state.reader.page)
    }

    @Test
    fun restorePositionStartsAtSavedOffset() = runTest {
        val harness = harnessFor(this)
        val chapterOneStart = harness.book.chapters[1].startByte
        // 标题行 21 字节 + 一个正文行 31 字节 ⇒ 落在合法 UTF-8 边界上
        val savedOffset = chapterOneStart + 52L
        harness.abi.positionFound = 1L
        harness.abi.positionChapterIndex = 1L
        harness.abi.positionOffset = savedOffset
        openAndSettle(harness, this)

        val reader = harness.viewModel.ui.value.reader
        assertEquals(1, reader.page!!.chapterIndex)
        assertEquals(savedOffset, reader.page!!.startByte)
        assertEquals(savedOffset, reader.lastSavedByteOffset)
        assertTrue("恢复成功后书架必须显示真实进度", harness.viewModel.ui.value.shelf.books.first().hasProgress)
    }
}
