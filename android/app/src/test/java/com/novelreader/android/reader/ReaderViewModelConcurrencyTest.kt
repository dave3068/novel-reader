package com.novelreader.android.reader

import com.novelreader.android.core.FakeReleaser
import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrErrorKind
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.runBlocking
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
 * 状态串行化与并发重填回归（android/checklist.md C3；AN3B 的 `--tests '*Concurrency*'` 目标）。
 *
 * 覆盖的真实缺陷模式（novel-reader-ui-env 技能记录的桌面/真机原型问题）：
 * 多个挂起入口并发重填预读缓存 → 缓存被重复填充（实测「预读 9 页」而应为 3）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReaderViewModelConcurrencyTest {

    @Test
    fun concurrentPageTurnsStaySerializedOnRealThreads() = runBlocking {
        val book = FixtureBook.build()
        val abi = ScriptedReaderAbi(book).apply { readWindowDelayMillis = 2L }
        val releaser = FakeReleaser()
        val viewModel = ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = Dispatchers.Default,
            workScope = this,
        )
        viewModel.onViewportMeasured(ReaderHarness.VIEWPORT_WIDTH_PX, ReaderHarness.VIEWPORT_HEIGHT_PX, 1f, 1f)
        viewModel.openBook("/tmp/fixture.txt", "fixture.txt")
        assertTrue("首屏必须排出", awaitUntil { viewModel.ui.value.reader.page != null })

        // 6 次并发翻页（真实线程 + 真实 I/O 延迟，把竞态窗口放大）
        repeat(TURNS) { viewModel.nextPage() }
        assertTrue(
            "6 次翻页后应停在第 ${TURNS + 1} 页",
            awaitUntil { viewModel.ui.value.reader.page?.pageOrdinal == TURNS + 1 },
        )

        assertEquals("同一时刻只允许一个入口进入 native 调用", 1, abi.maxInFlight)
        val ordered = abi.commitRecords.filter { it.layoutId == 1L }.map { it.startByte }.sorted()
        check(ordered.size >= TURNS + 1) { "本世代已登记页数不足：$ordered" }
        val page = viewModel.ui.value.reader.page!!
        assertEquals("第 ${TURNS + 1} 页的起点", ordered[TURNS], page.startByte)
        assertEquals(EXPECTED_PAGE_SEVEN_START, page.startByte)
        assertTrue(
            "前向缓存不得超过上限 ${ReaderViewModel.MAX_FORWARD_CACHE}",
            viewModel.ui.value.reader.forwardCacheCount <= ReaderViewModel.MAX_FORWARD_CACHE,
        )
        assertTrue("每次读取窗口都必须归还缓冲区", releaser.calls.size >= abi.readWindowCalls)
    }

    @Test
    fun concurrentMixedStateChangesKeepCacheBoundedAndChainContiguous() = runTest {
        val harness = ReaderHarness(io = StandardTestDispatcher(testScheduler), scope = TestScope(testScheduler))
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        // 20 轮混合操作：翻页 / 改字号 / 保存位置 / 回翻 —— 全部并发投递到同一状态机
        repeat(20) { index ->
            harness.viewModel.nextPage()
            harness.viewModel.onFontSizeChange(12f + (index % 20))
            harness.viewModel.savePositionNow()
            harness.viewModel.previousPage()
        }
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertTrue(
            "并发重填不得让前向缓存越界（回归：原型曾出现预读 9 页）",
            state.reader.forwardCacheCount <= ReaderViewModel.MAX_FORWARD_CACHE,
        )
        harness.assertContiguousChain()
        val page = state.reader.page
        assertNotNull(page)
        assertTrue(page!!.startByte < harness.book.bytes.size)
        assertTrue(page.endByte > page.startByte)
        assertTrue(page.endByte <= harness.book.bytes.size)
        assertFalse("并发操作结束后不得停留在失败状态", state.failure != null && state.reader.page == null)
    }

    @Test
    fun cancelRequestIsReachableWhileIndexRebuildHoldsTheGate() = runBlocking {
        val book = FixtureBook.build()
        val abi = ScriptedReaderAbi(book).apply { rebuildDelayMillis = 1_000L }
        val releaser = FakeReleaser()
        val viewModel = ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = Dispatchers.Default,
            workScope = this,
        )
        viewModel.onViewportMeasured(ReaderHarness.VIEWPORT_WIDTH_PX, ReaderHarness.VIEWPORT_HEIGHT_PX, 1f, 1f)
        viewModel.openBook("/tmp/fixture.txt", "fixture.txt")
        assertTrue(awaitUntil { viewModel.ui.value.reader.page != null })

        viewModel.rebuildIndex()
        assertTrue("重建必须已经开始", awaitUntil { abi.events.contains("rebuildIndex:enter") })
        assertTrue("重建期间状态必须可见", viewModel.ui.value.reader.indexRebuilding)

        viewModel.cancelIndexRebuild()
        assertTrue("取消必须在长任务运行期间发出", awaitUntil { abi.cancelRequests.isNotEmpty() })

        assertTrue("重建结束后状态复位", awaitUntil { viewModel.ui.value.reader.indexRebuilding.not() })
        val rebuildExit = abi.events.indexOf("rebuildIndex:exit")
        val cancelEnter = abi.events.indexOf("cancelRequest:enter")
        assertTrue("取消时序：取消必须发生在重建返回之前（$cancelEnter < $rebuildExit）", cancelEnter in 0 until rebuildExit)
        assertEquals(ReaderStatus.INDEX_CANCELLED, viewModel.ui.value.status)
    }

    @Test
    fun pageTurnsAndReflowDoNotInterleaveAroundTheGate() = runTest {
        val harness = ReaderHarness(io = StandardTestDispatcher(testScheduler), scope = TestScope(testScheduler))
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        val readsBefore = harness.abi.readWindowCalls
        val beginsBefore = harness.abi.beginLayoutCalls

        repeat(10) { harness.viewModel.nextPage() }
        harness.viewModel.onFontSizeChange(30f)
        repeat(10) { harness.viewModel.nextPage() }
        harness.settle(this)

        // 合并后的重排只有一次；翻页读取量受「每页 1 次 + 预取 2 次」约束
        assertTrue(
            "重排合并：BEGIN_LAYOUT 增量 ≤ 1",
            harness.abi.beginLayoutCalls - beginsBefore <= 1,
        )
        assertTrue(
            "窗口读取量必须与页数同阶，而不是每次请求各读一遍",
            harness.abi.readWindowCalls - readsBefore <= (10 + 10) * (1 + ReaderViewModel.MAX_FORWARD_CACHE),
        )
        assertTrue(harness.viewModel.ui.value.reader.forwardCacheCount <= ReaderViewModel.MAX_FORWARD_CACHE)
        harness.assertContiguousChain()
        assertEquals(harness.abi.commitRecords.size.toLong(), harness.abi.registeredPageCount)
    }

    @Test
    fun concurrentChapterPageReflowSaveEntriesShareOneGate() = runBlocking {
        val book = FixtureBook.build()
        val abi = ScriptedReaderAbi(book).apply { readWindowDelayMillis = 2L }
        val releaser = FakeReleaser()
        val viewModel = ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = Dispatchers.Default,
            workScope = this,
        )
        viewModel.onViewportMeasured(ReaderHarness.VIEWPORT_WIDTH_PX, ReaderHarness.VIEWPORT_HEIGHT_PX, 1f, 1f)
        viewModel.openBook("/tmp/fixture.txt", "fixture.txt")
        assertTrue("首屏必须排出", awaitUntil { viewModel.ui.value.reader.page != null })

        // 切章 / 翻页 / 重排 / 保存位置 全部并发投递（真实线程 + 真实 I/O 延迟放大竞态窗口）
        viewModel.nextChapter()
        viewModel.nextPage()
        viewModel.previousPage()
        viewModel.onFontSizeChange(20f)
        viewModel.savePositionNow()
        viewModel.nextChapter()
        viewModel.onLineSpacingChange(1.5f)
        viewModel.previousChapter()
        viewModel.savePositionNow()

        assertTrue(
            "并发入口必须在有限时间内收敛（在途任务清零）",
            awaitUntil(timeoutMillis = 15_000L) { viewModel.inFlightCount() == 0 },
        )
        assertEquals("同一时刻只允许一个入口进入 native 调用", 1, abi.maxInFlight)

        val state = viewModel.ui.value
        assertNull("并发入口不得留下失败状态", state.failure)
        assertEquals(20f, state.settings.fontSizeSp, 0.001f)
        assertEquals(1.5f, state.settings.lineSpacingMultiplier, 0.001f)
        val page = state.reader.page
        assertNotNull("并发操作结束后必须有可见页面", page)
        assertEquals("已发布页面必须反映最终排版设置", state.settings.fontSizeSp, page!!.fontSizePx, 0.001f)
        assertTrue(
            "并发重填不得让前向缓存越界",
            state.reader.forwardCacheCount <= ReaderViewModel.MAX_FORWARD_CACHE,
        )
        assertNotNull("保存位置入口必须真实落进度", state.reader.lastSavedByteOffset)
        // 同一世代内的页边界链必须首尾相接：并发入口不得插入空洞或重叠
        abi.commitRecords.groupBy { it.layoutId }.forEach { (layoutId, records) ->
            val ordered = records.sortedBy { it.startByte }
            ordered.zipWithNext { previous, next ->
                assertTrue(
                    "世代 $layoutId 页边界链断裂：${previous.startByte}→${previous.endByte} 之后是 ${next.startByte}",
                    previous.endByte == next.startByte,
                )
            }
        }
    }

    @Test
    fun hostPauseCancelsLongRunningWorkButKeepsTheEngineHandle() = runBlocking {
        val book = FixtureBook.build()
        val abi = ScriptedReaderAbi(book).apply { rebuildDelayMillis = 800L }
        val releaser = FakeReleaser()
        val engine = engineOn(abi, releaser)
        val viewModel = ReaderViewModel(
            engine = engine,
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = Dispatchers.Default,
            workScope = this,
        )
        viewModel.onViewportMeasured(ReaderHarness.VIEWPORT_WIDTH_PX, ReaderHarness.VIEWPORT_HEIGHT_PX, 1f, 1f)
        viewModel.openBook("/tmp/fixture.txt", "fixture.txt")
        assertTrue("首屏必须排出", awaitUntil { viewModel.ui.value.reader.page != null })
        val before = viewModel.ui.value.reader.page!!

        viewModel.rebuildIndex()
        assertTrue("索引重建必须进入 native", awaitUntil { abi.events.contains("rebuildIndex:enter") })
        assertTrue("在途任务必须只有索引重建一个", awaitUntil { viewModel.inFlightCount() == 1 })

        // 宿主动作（旋转 / 退到后台）：必须能在 gate 被长任务持有期间立即返回
        val startedAt = System.currentTimeMillis()
        val cancelled = viewModel.cancelInFlightWork("屏幕旋转")
        val elapsed = System.currentTimeMillis() - startedAt
        assertTrue("必须如实报告被取消的在途任务数（实际 $cancelled）", cancelled >= 1)
        assertTrue(
            "取消不得等待 gate（耗时 ${elapsed}ms，重建占用 ${abi.rebuildDelayMillis}ms）",
            elapsed < abi.rebuildDelayMillis / 2,
        )

        assertTrue("在途任务必须收敛", awaitUntil { viewModel.inFlightCount() == 0 })
        val paused = viewModel.ui.value
        assertFalse("取消后不得残留「正在重建」标志", paused.reader.indexRebuilding)
        assertNull("取消不是业务失败（不得写 ReaderFailure）", paused.failure)
        assertTrue(
            "状态必须回显取消原因与「句柄保留」",
            paused.status.contains("屏幕旋转") && paused.status.contains("句柄保留"),
        )
        assertEquals("取消不得改动当前页面", before.startByte, paused.reader.page!!.startByte)

        // 句柄保留：既没有销毁调用，后续 native 入口也照常可用
        assertFalse("取消在途任务不得销毁引擎句柄", engine.isDestroyed)
        assertTrue(
            "取消在途任务不得调用 nr_engine_destroy（实际事件 ${abi.events}）",
            abi.events.none { it.startsWith("engineDestroy") },
        )
        abi.rebuildDelayMillis = 0L
        val savedBefore = abi.savedPositions.size
        viewModel.savePositionNow()
        assertTrue("句柄仍可用：保存位置必须落到 native", awaitUntil { abi.savedPositions.size > savedBefore })
        viewModel.nextChapter()
        assertTrue("句柄仍可用：切章必须排出新章", awaitUntil { viewModel.ui.value.reader.page?.chapterIndex == 1 })
        assertNull("取消之后的正常操作不得带出失败状态", viewModel.ui.value.failure)
    }

    @Test
    fun hostPauseDropsInFlightChapterTurnWithoutOverwritingThePage() = runBlocking {
        val book = FixtureBook.build()
        val abi = ScriptedReaderAbi(book)
        val releaser = FakeReleaser()
        val viewModel = ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = Dispatchers.Default,
            workScope = this,
        )
        viewModel.onViewportMeasured(ReaderHarness.VIEWPORT_WIDTH_PX, ReaderHarness.VIEWPORT_HEIGHT_PX, 1f, 1f)
        viewModel.openBook("/tmp/fixture.txt", "fixture.txt")
        assertTrue("首屏必须排出", awaitUntil { viewModel.ui.value.reader.page != null })
        // 「首屏可见」不等于「打开流程结束」（首屏发布后还有前向预取）：必须等打开任务真正落定，
        // 否则下面等待的窗口读取可能来自打开流程的预取，而不是本次切章。
        assertTrue("打开流程必须完全落定", awaitUntil { viewModel.inFlightCount() == 0 })
        val before = viewModel.ui.value.reader.page!!
        assertEquals(0, before.chapterIndex)

        // 让下一次切章的测量停在「已读窗口、尚未提交」的窗口里
        abi.readWindowDelayMillis = 600L
        val readsBefore = abi.readWindowCalls
        viewModel.nextChapter()
        assertTrue(
            "切章测量必须进入 native 窗口（且在途任务只有它一个）",
            awaitUntil { viewModel.inFlightCount() == 1 && abi.readWindowCalls > readsBefore },
        )

        val cancelled = viewModel.cancelInFlightWork("退到后台")
        assertEquals("只应取消这一个在途切章", 1, cancelled)
        assertTrue("在途任务必须收敛", awaitUntil { viewModel.inFlightCount() == 0 })

        val after = viewModel.ui.value
        assertEquals("被取消的切章不得改动当前页面", before.startByte, after.reader.page!!.startByte)
        assertEquals(before.chapterIndex, after.reader.page!!.chapterIndex)
        assertEquals(before.text, after.reader.page!!.text)
        assertNull("取消不得被映射成业务失败", after.failure)
        assertTrue("状态必须回显取消原因", after.status.contains("退到后台"))
        val chapterOneStart = book.chapters[1].startByte
        assertTrue(
            "被取消的测量结果绝不允许提交给核心",
            abi.boundarySubmissions.none {
                it.phase == NrAbiConstants.NR_SUBMIT_COMMIT && it.startByte == chapterOneStart
            },
        )

        // 句柄保留：重试切章照常成功
        abi.readWindowDelayMillis = 0L
        viewModel.nextChapter()
        assertTrue("句柄仍可用：切章必须排出新章", awaitUntil { viewModel.ui.value.reader.page?.chapterIndex == 1 })
        assertEquals(chapterOneStart, viewModel.ui.value.reader.page!!.startByte)
        assertNull(viewModel.ui.value.failure)
    }

    @Test
    fun hostPauseDuringOpenClearsTransientFlagsAndRepublishIsPossible() = runBlocking {
        val book = FixtureBook.build()
        // 用**可控闸门**取代 400ms 墙钟窗口：负载再高也不会错过「在途」状态。
        val gate = ScriptedReaderAbi.OpenTxtGate()
        val abi = ScriptedReaderAbi(book).apply { openTxtGate = gate }
        val releaser = FakeReleaser()
        val engine = engineOn(abi, releaser)
        val viewModel = ReaderViewModel(
            engine = engine,
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = Dispatchers.Default,
            workScope = this,
        )
        viewModel.onViewportMeasured(ReaderHarness.VIEWPORT_WIDTH_PX, ReaderHarness.VIEWPORT_HEIGHT_PX, 1f, 1f)
        viewModel.openBook("/tmp/fixture.txt", "fixture.txt")
        // ⚠️ 必须用**挂起式**等待：`runBlocking` 的调度与 `gate.awaitEntered()` 的阻塞调用互斥，
        // 阻塞等待会让打开协程根本没机会运行（这正是加固过程中被抓出的真实陷阱）。
        // 由于闸门会**一直扣住** openTxt 直到 release()，「已进入」这一事件一旦出现就必然持续存在，
        // 因此这里的等待是确定性的：不存在需要抢的墙钟窗口。
        assertTrue("打开必须真的进入 native", awaitUntil { abi.events.contains("openTxt:enter") })
        assertFalse("闸门扣住期间不得收到 openTxt 回执", abi.events.contains("openTxt:exit"))
        assertTrue("必须留下 openTxt 进入事件", abi.events.contains("openTxt:enter"))
        // 此刻**确定性地**处于「已进入 native、尚未拿到回执」：不依赖任何墙钟窗口。
        assertEquals("打开在途时只能有 1 个在途任务", 1, viewModel.inFlightCount())
        assertTrue("打开中必须显示书架 loading", viewModel.ui.value.shelf.loading)
        assertNull("打开中不得有页面", viewModel.ui.value.reader.page)

        val cancelled = viewModel.cancelInFlightWork("退到后台")
        assertEquals("只应取消这一个在途打开", 1, cancelled)
        // 放行 native 返回：取消语义（丢弃回执、不写 failure）才被真正检验。
        gate.release()
        assertTrue("在途任务必须收敛", awaitUntil { viewModel.inFlightCount() == 0 })

        val paused = viewModel.ui.value
        assertFalse("取消后不得残留书架 loading 标志", paused.shelf.loading)
        assertNull("取消不是失败：不得写 ReaderFailure", paused.failure)
        assertNull("取消的半途打开不得登记成页面", paused.reader.page)
        assertTrue("状态必须回显取消原因", paused.status.contains("退到后台"))

        // 句柄保留：重新打开照常排出首屏
        // 解除闸门，重新打开照常排出首屏。
        abi.openTxtGate = null
        viewModel.openBook("/tmp/fixture.txt", "fixture.txt")
        assertTrue("句柄仍可用：重新打开必须排出首屏", awaitUntil(timeoutMillis = 10_000L) { viewModel.ui.value.reader.page != null })
        assertNull(viewModel.ui.value.failure)
        assertFalse(viewModel.ui.value.shelf.loading)
        // 收尾用**有界收敛等待**而非固定时刻断言：重开后的预取可能仍在途，
        // 断言「最终归零」才是真正要保证的语义（固定时刻断言会随负载抖动）。
        assertTrue("在途任务必须最终归零", awaitUntil { viewModel.inFlightCount() == 0 })
    }

    private companion object {
        const val TURNS = 6

        /**
         * 夹具（3 章 × 1 标题行 + 6 正文行、3 行/页）的页起点（手算）：
         * 章节 0 = `[0,207)` → 页 `0, 83, 176`；章节 1 = `[207,414)` → 页 `207, 290, 383`；
         * 章节 2 = `[414,621)` → 页 `414, 497, 590`。
         *
         * 因此从第 1 页连续翻 6 次 ⇒ 第 7 页起点 = 414（章节 2 起点）。
         */
        const val EXPECTED_PAGE_SEVEN_START = 414L
    }
}
