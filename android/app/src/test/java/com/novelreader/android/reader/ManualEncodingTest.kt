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
 * 手动选择编码（规格 CR2「用户手动覆盖」，ABI 第 19 项 `nr_set_encoding`）。
 *
 * 用户裁定语义：只改变「如何解释文件字节」，**永不写回原文件**；覆盖后核心立即按新编码重扫章节。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ManualEncodingTest {

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(io = StandardTestDispatcher(scope.testScheduler), scope = TestScope(scope.testScheduler))

    @Test
    fun manualEncodingIsSentToCoreAndSurfacedToUi() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        harness.viewModel.setEncoding(NrAbiConstants.NR_ENCODING_GB18030.toInt())
        harness.settle(this)

        assertEquals(
            "必须把用户选择的编码码交给核心",
            NrAbiConstants.NR_ENCODING_GB18030.toInt(),
            harness.abi.lastRequestedEncoding,
        )
        val reader = harness.viewModel.ui.value.reader
        assertEquals("界面必须显示新编码", "GB18030", reader.encodingName)
        assertFalse("手动指定后不再提示「待确认」", reader.needsEncodingConfirmation)
        // 状态行随后会被「页面状态」覆盖（含当前编码），因此这里断言的是**最终可读的编码**：
        assertTrue(
            "信息区必须显示生效编码",
            harness.viewModel.ui.value.status.contains("编码 GB18030"),
        )
        assertTrue("编码切换后仍应排出页面", reader.page != null)
        assertTrue("章节列表必须重新载入", reader.chapters.isNotEmpty())
    }

    @Test
    fun autoOptionClearsOverride() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        harness.viewModel.setEncoding(0)
        harness.settle(this)

        assertEquals("0 表示清除覆盖（回到自动检测）", 0, harness.abi.lastRequestedEncoding)
        assertEquals(
            "替身按夹具默认编码回应：UTF-8",
            "UTF-8",
            harness.viewModel.ui.value.reader.encodingName,
        )
    }

    @Test
    fun setEncodingWithoutOpenBookReportsNoBook() = runTest {
        val harness = harnessFor(this)
        harness.settle(this)

        harness.viewModel.setEncoding(NrAbiConstants.NR_ENCODING_GBK.toInt())
        harness.settle(this)

        assertEquals(ReaderStatus.NO_BOOK, harness.viewModel.ui.value.status)
        assertEquals("未打开书籍时不得调用核心", null, harness.abi.lastRequestedEncoding)
    }
}
