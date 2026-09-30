package com.novelreader.android.reader

import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrErrorKind
import com.novelreader.android.core.NrErrors
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 过期结果丢弃（AR4 / 子规格 AC-A4「过期结果不会覆盖当前状态」；android/checklist.md C3）。
 *
 * 三种过期状态都来自核心 `BoundaryCommitStatus`，语义各不相同：
 * - `STALE_LAYOUT_ID`：排版世代已变（设置/视口变化后核心切换了世代）；
 * - `STALE_REQUEST_ID`：本次请求已被更新的请求取代（重排合并）；
 * - `STALE_CONTENT_INDEX_VERSION`：内容索引版本已变（重建章节索引）。
 *
 * 共同契约（本文件逐条断言）：
 * 1. 过期结果**绝不**当作页面发布（不进入 [`ReaderState.page`]，也不进前向缓存）；
 * 2. 必须按核心给出的当前世代重新登记（`BEGIN_LAYOUT`）并在 [`ReaderViewModel.MAX_STALE_RETRY`]
 *    次之内重试；
 * 3. 重试耗尽后如实回报失败（`NR_ERR_LAYOUT_NOT_FOUND` + [`NrErrorKind.LAYOUT`]），
 *    而**当前页面保持原样**：既不清空、也不被过期结果覆盖。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class StaleLayoutResultTest {

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(io = StandardTestDispatcher(scope.testScheduler), scope = TestScope(scope.testScheduler))

    private fun openAndSettle(harness: ReaderHarness, scope: TestScope) {
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(scope)
    }

    /** 切章总是走「真实测量」路径（不查前向缓存），因此过期脚本的落到哪一次提交是确定的。 */
    private fun nextChapterText(harness: ReaderHarness): String {
        val start = harness.book.chapters[1].startByte.toInt()
        val end = start + (ReaderHarness.PAGE_ONE_END - 0L).toInt()
        return String(harness.book.bytes.copyOfRange(start, end), Charsets.UTF_8)
    }

    @Test
    fun staleRequestIdIsDiscardedAndRetriedAgainstTheCurrentGeneration() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val beginsBefore = harness.abi.beginLayoutCalls

        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_REQUEST_ID)
        harness.viewModel.nextChapter()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertEquals(
            "过期结果必须触发重新登记排版世代（请求已被取代）",
            beginsBefore + 1,
            harness.abi.beginLayoutCalls,
        )
        assertNull("过期不是失败：重试成功后不得残留错误状态", state.failure)
        val page = state.reader.page
        assertNotNull(page)
        assertEquals("重试后必须排出目标章", 1, page!!.chapterIndex)
        assertEquals(harness.book.chapters[1].startByte, page.startByte)
        assertEquals("重试后的页面必须来自真实字节", nextChapterText(harness), page.text)
    }

    @Test
    fun staleContentIndexVersionIsDiscardedAndRetried() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val beginsBefore = harness.abi.beginLayoutCalls
        val previous = harness.viewModel.ui.value.reader.page!!
        val requestIdBefore = harness.viewModel.ui.value.reader.requestId

        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_CONTENT_INDEX_VERSION)
        harness.viewModel.nextChapter()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertEquals("内容索引版本过期必须重新登记世代", beginsBefore + 1, harness.abi.beginLayoutCalls)
        assertTrue(
            "重新登记必须拿到新的 request_id（旧的已过期）",
            state.reader.requestId > requestIdBefore,
        )
        assertNull(state.failure)
        val page = state.reader.page!!
        assertEquals(1, page.chapterIndex)
        assertEquals(
            "被丢弃的旧页不得留在展示状态里",
            false,
            page.startByte == previous.startByte && page.chapterIndex == previous.chapterIndex,
        )
    }

    @Test
    fun staleDuringPageTurnNeverBecomesTheVisiblePageUntilItIsReMeasured() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        // 两条过期：第一条被丢弃，第二条命中重试成功
        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_LAYOUT_ID)
        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_REQUEST_ID)

        harness.viewModel.nextChapter()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertNull(state.failure)
        assertEquals(1, state.reader.page!!.chapterIndex)
        assertEquals(harness.book.chapters[1].startByte, state.reader.page!!.startByte)
        // 被丢弃的两次提交绝不进入已登记边界（核心侧只有重试成功的那一条）
        val chapterOneStart = harness.book.chapters[1].startByte
        assertEquals(
            "过期提交不得被登记进当前世代",
            1,
            harness.abi.commitRecords.count { it.startByte == chapterOneStart },
        )
    }

    @Test
    fun exhaustedStaleRetriesReportFailureAndNeverTouchTheCurrentPage() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val before = harness.viewModel.ui.value.reader.page!!
        assertEquals(0, before.chapterIndex)

        val observed = mutableListOf<PageState?>()
        val collector = backgroundScope.launch { harness.viewModel.ui.collect { observed.add(it.reader.page) } }
        runCurrent()

        repeat(ReaderViewModel.MAX_STALE_RETRY + 1) {
            harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_LAYOUT_ID)
        }
        harness.viewModel.nextChapter()
        harness.settle(this)
        collector.cancel()

        val state = harness.viewModel.ui.value
        val failure = state.failure
        assertNotNull("重试耗尽必须如实回报失败，不得静默通过", failure)
        assertEquals(NrErrors.NR_ERR_LAYOUT_NOT_FOUND, failure!!.rawCode)
        assertEquals(NrErrorKind.LAYOUT, failure.kind)
        assertTrue("状态必须说明是世代过期（${state.status}）", state.status.contains("过期"))

        val page = state.reader.page
        assertNotNull("过期结果绝不允许把当前页面清空", page)
        assertEquals("当前页面起点不得被覆盖", before.startByte, page!!.startByte)
        assertEquals("当前页面终点不得被覆盖", before.endByte, page.endByte)
        assertEquals("当前页面正文不得被覆盖", before.text, page.text)
        assertEquals("当前页面章节不得被覆盖", before.chapterIndex, page.chapterIndex)
        assertTrue(
            "任何中间状态都不得把页面换成过期结果（观察到 ${observed.size} 个状态）",
            observed.all { it == null || (it.startByte == before.startByte && it.endByte == before.endByte) },
        )
    }

    @Test
    fun stalePrefetchCommitIsDiscardedAndReMeasuredWithoutBreakingTheChain() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val before = harness.viewModel.ui.value.reader.page!!
        val chapterOneStart = harness.book.chapters[1].startByte

        // 过期脚本会被「翻页后的预取」消费：预取失败不得进入缓存，也不得影响随后真实测量的结果
        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_LAYOUT_ID)
        harness.viewModel.nextPage()
        harness.viewModel.nextPage()
        harness.viewModel.nextPage()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertNull("过期不是失败", state.failure)
        assertTrue(
            "被丢弃的过期结果不得让前向缓存越界",
            state.reader.forwardCacheCount <= ReaderViewModel.MAX_FORWARD_CACHE,
        )
        val page = state.reader.page
        assertNotNull(page)
        assertTrue(
            "页面只能前进，不得回退到过期结果对应的锚点之前",
            page!!.startByte >= before.startByte,
        )
        assertEquals("预取过期后重测必须照常前进到下一章", chapterOneStart, page.startByte)
        assertEquals("过期提交不得被登记进当前世代", 1, harness.abi.commitRecords.count { it.startByte == chapterOneStart })
        assertEquals(
            "已登记边界数必须等于真实提交记录数（过期提交不计入）",
            harness.abi.commitRecords.size.toLong(),
            harness.abi.registeredPageCount,
        )
    }
}
