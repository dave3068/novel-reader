package com.novelreader.android.reader

import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrErrorKind
import com.novelreader.android.core.NrErrors
import com.novelreader.android.source.SafTestData
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 宿主接线（`MainActivity` + `SafHost` 所依赖的两个 ViewModel 入口）。
 *
 * 覆盖此前**无人认领**的那一环：把宿主拿到的来源（SAF 平台窗口 / 本地文件）以正确的
 * 数据源种类与坐标空间长度交给核心。断言落在 ScriptedReaderAbi 记录的 `openTxt` 参数上，
 * 而不是界面文案 —— 界面是结果，不是依据。
 *
 * 对应的静态门禁：`scripts/verify-android-scope.py` 的 `[7]`（MainActivity 必须挂载外壳并注册导入）。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HostWiringTest {

    private fun harnessFor(scope: TestScope): ReaderHarness =
        ReaderHarness(io = StandardTestDispatcher(scope.testScheduler), scope = TestScope(scope.testScheduler))

    @Test
    fun platformBookOpensWithPlatformKindAndHostSuppliedLength() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()

        harness.viewModel.openPlatformBook(SafTestData.uri(), PLATFORM_BYTES, SafTestData.NAME)
        harness.settle(this)

        assertEquals(
            "SAF 可定位来源必须以平台窗口通道打开",
            NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
            harness.abi.lastOpenKind,
        )
        assertEquals(
            "平台窗口必须传宿主给出的坐标空间长度（不猜测、不裁剪）",
            PLATFORM_BYTES,
            harness.abi.lastOpenTotalBytes,
        )
        assertEquals(ReaderRoute.READER, harness.viewModel.ui.value.route)
        assertEquals(SafTestData.NAME, harness.viewModel.ui.value.reader.displayName)
    }

    @Test
    fun fileBookKeepsFileKindAndZeroLength() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        assertEquals(
            "本地文件源必须保持文件通道（回归）",
            NrAbiConstants.NR_SOURCE_KIND_FILE,
            harness.abi.lastOpenKind,
        )
        assertEquals("文件源应以文件实际长度为准（传 0）", 0L, harness.abi.lastOpenTotalBytes)
    }

    @Test
    fun platformBookRejectsInvalidArgumentsWithoutOpening() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.settle(this)

        harness.viewModel.openPlatformBook(sourceIdentity = "", totalBytes = 0L, displayName = null)
        harness.settle(this)

        assertEquals("参数无效时不得发起打开", -1, harness.abi.lastOpenKind)
        val failure = harness.viewModel.ui.value.failure
        assertNotNull("无效参数必须留下可见失败", failure)
        assertEquals(NrErrors.NR_ERR_INVALID_ARGUMENT, failure!!.rawCode)
        assertEquals(NrErrorKind.CALLER, failure.kind)
    }

    @Test
    fun hostFailureIsSurfacedAndLeavesNoStuckLoading() = runTest {
        val harness = harnessFor(this)
        harness.settle(this)

        harness.viewModel.reportHostFailure("导入被拒绝：MIME 为 image/png（非纯文本）")

        val state = harness.viewModel.ui.value
        assertEquals(NrErrors.NR_ERR_IO, state.failure?.rawCode)
        assertEquals("失败后不得停在「加载中」", false, state.shelf.loading)
        assertTrue("失败原因必须可见", state.status.startsWith("导入失败"))
    }

    @Test
    fun shelfPlatformBookReopensThroughHostResolver() = runTest {
        val calls = mutableListOf<String>()
        val resolver = HostSourceResolver { uri ->
            calls += uri
            HostOpenTarget(
                kind = NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
                identity = uri,
                totalBytes = RENEWED_BYTES,
                displayName = SafTestData.NAME,
            )
        }
        val harness = ReaderHarness(
            io = StandardTestDispatcher(testScheduler),
            scope = TestScope(testScheduler),
            hostResolver = resolver,
        )
        harness.measureViewport()

        harness.viewModel.openBook(platformShelfBook())
        harness.settle(this)

        assertEquals("SAF 平台来源必须经宿主重新解析（描述符不跨会话存活）", listOf(SafTestData.uri()), calls)
        assertEquals(
            "必须以平台窗口通道打开（把 content:// 当文件路径会得到 NR_ERR_6）",
            NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
            harness.abi.lastOpenKind,
        )
        assertEquals("必须以宿主重新给出的长度打开", RENEWED_BYTES, harness.abi.lastOpenTotalBytes)
        assertEquals(
            "书架条目必须记住数据源种类，否则下次打开又会退化成文件通道",
            NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
            harness.viewModel.ui.value.shelf.books.first().sourceKind,
        )
    }

    @Test
    fun shelfFileBookKeepsFileChannelAndSkipsResolver() = runTest {
        var calls = 0
        val harness = ReaderHarness(
            io = StandardTestDispatcher(testScheduler),
            scope = TestScope(testScheduler),
            hostResolver = HostSourceResolver { calls++; error("文件来源不得调用宿主解析") },
        )
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)

        assertEquals("本地文件来源不得触发宿主解析", 0, calls)
        assertEquals(NrAbiConstants.NR_SOURCE_KIND_FILE, harness.abi.lastOpenKind)
        assertEquals(0L, harness.abi.lastOpenTotalBytes)
        assertEquals(
            NrAbiConstants.NR_SOURCE_KIND_FILE,
            harness.viewModel.ui.value.shelf.books.first().sourceKind,
        )
    }

    @Test
    fun shelfPlatformBookResolverFailureIsSurfacedAndDoesNotOpen() = runTest {
        val harness = ReaderHarness(
            io = StandardTestDispatcher(testScheduler),
            scope = TestScope(testScheduler),
            hostResolver = HostSourceResolver { error("来源已失效，需要重新关联") },
        )
        harness.measureViewport()
        harness.settle(this)

        harness.viewModel.openBook(platformShelfBook())
        harness.settle(this)

        assertEquals("宿主解析失败时不得发起打开", -1, harness.abi.lastOpenKind)
        assertNotNull("失败必须留下可见原因", harness.viewModel.ui.value.failure)
        assertEquals("失败后不得停在「加载中」", false, harness.viewModel.ui.value.shelf.loading)
    }

    private fun platformShelfBook() = ShelfBook(
        sourceIdentity = SafTestData.uri(),
        displayName = SafTestData.NAME,
        bookId = 7L,
        totalBytes = 4L,
        chapterCount = 3,
        sourceKind = NrAbiConstants.NR_SOURCE_KIND_PLATFORM,
    )

    companion object {
        private const val PLATFORM_BYTES = 987_654L

        /** 宿主重新解析时给出的长度（与首次导入不同，用于证明用的是**新**长度）。 */
        private const val RENEWED_BYTES = 1_234_567L
    }
}
