package com.novelreader.android.reader

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
 * 重排合并与设置状态（android/checklist.md C3：重排合并、设置状态；
 * AN3B 卡片的 `--tests '*Reflow*'` 过滤目标）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReflowCoalescingTest {

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(io = StandardTestDispatcher(scope.testScheduler), scope = TestScope(scope.testScheduler))

    private fun openAndSettle(harness: ReaderHarness, scope: TestScope) {
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(scope)
    }

    @Test
    fun fontSizeSliderWritesBackSynchronously() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)

        harness.viewModel.onFontSizeChange(24f)
        // 不推进调度器：受控滑杆的 value 必须**立刻**反映新值，否则滑块会弹回旧值。
        assertEquals(24f, harness.viewModel.ui.value.settings.fontSizeSp, 0.001f)
        assertEquals(ReaderStatus.REFLOW, harness.viewModel.ui.value.status)

        harness.settle(this)
        val page = harness.viewModel.ui.value.reader.page!!
        assertEquals("重排后页面字号 = 新设置 × spToPx", 24f, page.fontSizePx, 0.001f)
    }

    @Test
    fun rapidFontSizeChangesCollapseIntoASingleReflow() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val beginsBefore = harness.abi.beginLayoutCalls
        val readsBefore = harness.abi.readWindowCalls

        // 模拟拖动：一次事件循环里连续几十次改字号
        for (size in 12..34) {
            harness.viewModel.onFontSizeChange(size.toFloat())
        }
        harness.settle(this)

        assertEquals(
            "重排请求必须合并为一次（拖动只保留最终值）",
            1,
            harness.abi.beginLayoutCalls - beginsBefore,
        )
        assertTrue(
            "重排只测量当前页 + 前向预取 ${ReaderViewModel.MAX_FORWARD_CACHE} 页",
            harness.abi.readWindowCalls - readsBefore <= 1 + ReaderViewModel.MAX_FORWARD_CACHE,
        )
        assertEquals(34f, harness.viewModel.ui.value.settings.fontSizeSp, 0.001f)
        assertEquals(34f, harness.viewModel.ui.value.reader.page!!.fontSizePx, 0.001f)
        assertTrue(harness.viewModel.ui.value.reader.forwardCacheCount <= ReaderViewModel.MAX_FORWARD_CACHE)
    }

    @Test
    fun reflowKeepsCurrentPageAnchorAndOrdinal() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        harness.viewModel.nextPage()
        harness.settle(this)
        val before = harness.viewModel.ui.value.reader.page!!
        assertEquals(ReaderHarness.PAGE_ONE_END, before.startByte)
        assertTrue(harness.viewModel.ui.value.reader.canGoPreviousPage)

        harness.viewModel.onLineSpacingChange(1.6f)
        harness.settle(this)

        val after = harness.viewModel.ui.value.reader.page!!
        assertEquals("重排必须停在原锚点，不能跳回章节开头", before.startByte, after.startByte)
        assertEquals(before.pageOrdinal, after.pageOrdinal)
        assertEquals(1.6f, harness.viewModel.ui.value.settings.lineSpacingMultiplier, 0.001f)
        // 意图变更（2026-09-28，用户实测缺陷驱动）：
        // 旧断言要求重排后「上一页」置灰。但①字节位置是合法阅读位置（阅读身份按字节偏移表达）；
        // ②用户明确要求「不在书首就应可后退」。现改为：重排仍清空旧页边界锚点（新排版边界不同），
        // 随即用本会话稀疏锚点重播，「上一页」保持可用且落点是真实读过的位置。
        assertTrue(
            "重排后「上一页」应仍可用（按字节位置重播稀疏锚点）",
            harness.viewModel.ui.value.reader.canGoPreviousPage,
        )
    }

    @Test
    fun hostPauseCancelsPendingReflowAndResumeRealignsIt() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val beginsBefore = harness.abi.beginLayoutCalls
        val pageBefore = harness.viewModel.ui.value.reader.page!!

        // 字号同步写回 + 合并式重排（作业已入队，尚未执行）
        harness.viewModel.onFontSizeChange(26f)
        assertEquals(26f, harness.viewModel.ui.value.settings.fontSizeSp, 0.001f)

        val cancelled = harness.viewModel.cancelInFlightWork("屏幕旋转")
        assertTrue("在途重排必须被取消（实际 $cancelled）", cancelled >= 1)

        harness.settle(this)
        assertEquals("被取消的重排不得执行", beginsBefore, harness.abi.beginLayoutCalls)
        assertEquals(
            "被取消的重排不得改动当前页面",
            pageBefore.fontSizePx,
            harness.viewModel.ui.value.reader.page!!.fontSizePx,
            0.001f,
        )
        assertNull("取消不是失败", harness.viewModel.ui.value.failure)

        // 回到前台：句柄仍可用，且必须按新设置重新对齐（否则界面会停在与设置不符的旧页面）
        harness.viewModel.onHostResumed()
        harness.settle(this)
        assertEquals("回到前台必须重新走一次排版登记", beginsBefore + 1, harness.abi.beginLayoutCalls)
        assertEquals(26f, harness.viewModel.ui.value.reader.page!!.fontSizePx, 0.001f)
        assertNull(harness.viewModel.ui.value.failure)
        assertFalse("取消在途任务不得销毁引擎句柄", harness.engine.isDestroyed)
        assertEquals("在途任务必须清零", 0, harness.viewModel.inFlightCount())
    }

    @Test
    fun hostResumeWithoutSpecChangeTouchesNothing() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val beginsBefore = harness.abi.beginLayoutCalls
        val readsBefore = harness.abi.readWindowCalls
        val statusBefore = harness.viewModel.ui.value.status

        harness.viewModel.onHostResumed()
        harness.settle(this)

        assertEquals("排版参数未变化时不得重新登记世代", beginsBefore, harness.abi.beginLayoutCalls)
        assertEquals("排版参数未变化时不得重读正文", readsBefore, harness.abi.readWindowCalls)
        assertEquals(statusBefore, harness.viewModel.ui.value.status)
    }

    @Test
    fun backgroundSettingsChangesMergeOnResumeAndKeepHandle() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val beginsBefore = harness.abi.beginLayoutCalls

        // 后台期间的多次设置变更（取消丢掉了在途重排）→ 回到前台必须合并成一次
        harness.viewModel.onFontSizeChange(22f)
        harness.viewModel.cancelInFlightWork("退到后台")
        harness.viewModel.onFontSizeChange(24f)
        harness.viewModel.onLineSpacingChange(1.6f)
        harness.viewModel.onHostResumed()
        harness.settle(this)

        assertEquals("回到前台的重排必须合并", beginsBefore + 1, harness.abi.beginLayoutCalls)
        val state = harness.viewModel.ui.value
        assertEquals(24f, state.settings.fontSizeSp, 0.001f)
        assertEquals(24f, state.reader.page!!.fontSizePx, 0.001f)
        assertEquals(1.6f, state.settings.lineSpacingMultiplier, 0.001f)
        assertNull(state.failure)
    }

    @Test
    fun darkThemeChangeDoesNotTouchLayout() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val beginsBefore = harness.abi.beginLayoutCalls
        val readsBefore = harness.abi.readWindowCalls

        harness.viewModel.onDarkThemeChange(true)
        harness.settle(this)

        assertTrue(harness.viewModel.ui.value.settings.darkTheme)
        assertEquals("夜间模式不参与排版签名：不得触发重排", beginsBefore, harness.abi.beginLayoutCalls)
        assertEquals(readsBefore, harness.abi.readWindowCalls)
    }

    @Test
    fun explicitRelayoutInvalidatesCurrentGenerationThenRebuilds() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)
        val layoutIdBefore = harness.viewModel.ui.value.reader.layoutId
        val beginsBefore = harness.abi.beginLayoutCalls

        harness.viewModel.requestRelayout()
        harness.settle(this)

        assertEquals("重新排版必须先失效当前世代", 1, harness.abi.invalidateCalls)
        assertEquals(beginsBefore + 1, harness.abi.beginLayoutCalls)
        assertTrue(
            "失效后必须拿到新的 layout_id",
            harness.viewModel.ui.value.reader.layoutId > layoutIdBefore,
        )
        assertNotNull(harness.viewModel.ui.value.reader.page)
    }

    @Test
    fun clampedFontSizeIgnoresOutOfRangeValues() = runTest {
        val harness = harnessFor(this)
        openAndSettle(harness, this)

        harness.viewModel.onFontSizeChange(4f)
        harness.settle(this)
        assertEquals(TypographySettings.MIN_FONT_SIZE_SP, harness.viewModel.ui.value.settings.fontSizeSp, 0.001f)

        harness.viewModel.onFontSizeChange(99f)
        harness.settle(this)
        assertEquals(TypographySettings.MAX_FONT_SIZE_SP, harness.viewModel.ui.value.settings.fontSizeSp, 0.001f)
    }
}
