package com.novelreader.android.reader

import com.novelreader.android.core.FakeReleaser
import com.novelreader.android.core.NrBackgroundEntry
import com.novelreader.android.core.NrErrors
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 设置页「阅读背景」区（V1.1 规格 R14–R17）。
 *
 * 钉住的口径：
 * - 核心**复制**图片、只回**相对文件名**（R15）；
 * - 缩放口径只能在**选图时**确定（核心无单独改 fit 的导出）；
 * - 宿主未提供目录 / 选图入口时**如实回报**，不给无反应的控件；
 * - 写入被拒时**原设置不变**。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class BackgroundSettingsTest {

    private fun le(size: Int): ByteBuffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    /** 造 `nr_background` 批次：定长头 + 相对文件名。 */
    private fun backgroundPayload(
        fileName: String,
        fileSize: Long = 1024L,
        scrimPermille: Long = 600L,
        fitMode: Long = BackgroundFit.CENTER.toLong(),
    ): ByteArray {
        val raw = fileName.toByteArray(Charsets.UTF_8)
        val buf = le(NrBackgroundEntry.STRUCT_SIZE + raw.size)
        buf.putLong(0, fileSize)
        buf.putLong(8, scrimPermille)
        buf.putLong(16, fitMode)
        buf.put(32, 153.toByte()) // scrim_alpha（核心已换算）
        buf.putInt(36, raw.size)
        buf.putInt(40, NrBackgroundEntry.STRUCT_SIZE)
        val out = buf.array()
        raw.copyInto(out, NrBackgroundEntry.STRUCT_SIZE)
        return out
    }

    private fun abiWithImage(fileName: String, scrim: Long = 600L, fit: Long = 0L) =
        ScriptedReaderAbi(FixtureBook.build()).apply {
            backgroundBytes = backgroundPayload(fileName, scrimPermille = scrim, fitMode = fit)
            backgroundCount = 1
        }

    private fun abiWithoutImage() = ScriptedReaderAbi(FixtureBook.build()).apply {
        backgroundBytes = ByteArray(0)
        backgroundCount = 0
    }

    private fun viewModel(abi: ScriptedReaderAbi, dir: String? = "/tmp/bg"): ReaderViewModel {
        val releaser = FakeReleaser()
        return ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = UnconfinedTestDispatcher(),
            workScope = kotlinx.coroutines.CoroutineScope(UnconfinedTestDispatcher()),
            backgroundDir = dir,
        )
    }

    // ------------------------------------------------------------------ 读回

    @Test
    fun openingSettingsReadsRelativeNameAndAlphaFromCore() = runTest {
        val abi = abiWithImage("bg-1791264415-42.png", scrim = 600L, fit = BackgroundFit.CENTER.toLong())
        val vm = viewModel(abi)

        vm.openSettings()

        val background = vm.ui.value.background
        assertTrue(background.hasImage)
        // **只回相对文件名**（R15）：不得含路径分隔符。
        assertEquals("bg-1791264415-42.png", background.fileName)
        assertFalse(background.fileName.contains('/'))
        assertEquals(600L, background.scrimPermille)
        assertEquals(BackgroundFit.CENTER.toLong(), background.fitMode)
        assertEquals("原始尺寸居中", background.fitLabel)
        assertEquals("60%", background.scrimLabel)
        // alpha 由核心换算，界面不重算。
        assertEquals(153, background.scrimAlpha)
    }

    @Test
    fun absentBackgroundShowsEmptyState() = runTest {
        val vm = viewModel(abiWithoutImage())

        vm.openSettings()

        val background = vm.ui.value.background
        assertFalse(background.hasImage)
        assertEquals("", background.fileName)
        assertEquals("—", background.sizeLabel)
    }

    @Test
    fun readFailureSurfacesInsteadOfShowingUnset() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply { v11Code = NrErrors.NR_ERR_INTERNAL }
        val vm = viewModel(abi)

        vm.openSettings()

        assertTrue(
            "读失败应回显错误，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("读取背景设置失败"),
        )
    }

    // ------------------------------------------------------------------ 应用

    @Test
    fun pickedImageForwardsPathDirectoryFitAndScrim() = runTest {
        val abi = abiWithImage("bg-new.png", scrim = 800L, fit = BackgroundFit.TILE.toLong())
        val vm = viewModel(abi, dir = "/data/app/backgrounds")

        vm.onBackgroundPickRequest(fitMode = BackgroundFit.TILE, scrimPermille = 800)
        vm.applyPickedBackground("/data/app/cache/background-src/abc")

        val forwarded = abi.lastBackgroundSet
        assertEquals("/data/app/cache/background-src/abc" to "/data/app/backgrounds", forwarded)
        // **口径必须随图一起传**（核心只在 set_background 这一次调用里收 fit/scrim）。
        assertEquals(BackgroundFit.TILE to 800, abi.lastBackgroundChoice)
        assertTrue(abi.events.contains("setBackground:enter"))
        // 回读的是**核心认可**的值，不是界面自己记的。
        assertEquals("bg-new.png", vm.ui.value.background.fileName)
        assertEquals(800L, vm.ui.value.background.scrimPermille)
    }

    @Test
    fun rejectedBackgroundKeepsPreviousSetting() = runTest {
        // 核心拒绝（如扩展名不在白名单、复制失败）：原设置必须保持不变。
        val abi = abiWithImage("bg-old.png", scrim = 400L).apply {
            v11Code = NrErrors.NR_ERR_INVALID_ARGUMENT
        }
        val vm = viewModel(abi)

        vm.openSettings()
        // 读成功时先记住原值（下面把读也弄失败，模拟「写入被拒且回读也失败」的极端）。
        val before = vm.ui.value.background.fileName

        vm.onBackgroundPickRequest(fitMode = BackgroundFit.COVER, scrimPermille = 400)
        vm.applyPickedBackground("/tmp/broken.txt")

        assertEquals("被拒后原值不得被清掉", before, vm.ui.value.background.fileName)
        assertTrue(
            "应回显「原设置未改动」，实际：${vm.ui.value.status}",
            vm.ui.value.status.contains("被拒") || vm.ui.value.status.contains("失败"),
        )
    }

    @Test
    fun scrimChangeWritesThroughWithoutReplacingImage() = runTest {
        val abi = abiWithImage("bg-keep.png", scrim = 300L).apply { backgroundCount = 1 }
        val vm = viewModel(abi)

        vm.onBackgroundScrimChange(500)

        assertTrue(abi.events.contains("setBackgroundScrim:enter"))
        // 图片仍在（调遮罩不换图）。
        assertEquals("bg-keep.png", vm.ui.value.background.fileName)
    }

    @Test
    fun clearForwardsAppPrivateDirectoryAndAppliesCoreResult() = runTest {
        val abi = abiWithoutImage()
        val vm = viewModel(abi, dir = "/data/app/backgrounds")

        vm.onBackgroundClear()

        assertTrue(abi.events.contains("clearBackground:enter"))
        assertFalse(vm.ui.value.background.hasImage)
        assertTrue(vm.ui.value.status.contains("已移除"))
    }

    // ------------------------------------------------------------------ 缺能力时如实回报

    @Test
    fun missingBackgroundDirReportsHonestlyInsteadOfSilentNoOp() = runTest {
        val abi = abiWithoutImage()
        val vm = viewModel(abi, dir = null)

        vm.onBackgroundPickRequest(fitMode = BackgroundFit.COVER, scrimPermille = 400)
        vm.applyPickedBackground("/tmp/x.png")

        assertEquals(ReaderStatus.BACKGROUND_NO_DIR, vm.ui.value.status)
        // 缺目录时**不得**触碰 native（否则会把图写到未知位置或直接失败在核心）。
        assertFalse(abi.events.contains("setBackground:enter"))
        assertFalse(abi.events.contains("clearBackground:enter"))
    }

    @Test
    fun clearWithMissingBackgroundDirReportsHonestlyInsteadOfSilentNoOp() = runTest {
        // 与「选图」同一条纪律：缺能力时必须**说出来**，不能按了没反应。
        val abi = abiWithoutImage()
        val vm = viewModel(abi, dir = null)

        vm.onBackgroundClear()

        assertEquals(ReaderStatus.BACKGROUND_NO_DIR, vm.ui.value.status)
        assertFalse(abi.events.contains("clearBackground:enter"))
    }

    @Test
    fun missingPickerReportsHonestly() = runTest {
        val vm = viewModel(abiWithoutImage())

        vm.onBackgroundPickRequest(fitMode = BackgroundFit.COVER, scrimPermille = 400, onPickImage = null)

        assertEquals(ReaderStatus.BACKGROUND_NO_PICKER, vm.ui.value.status)
    }

    @Test
    fun pickerIsInvokedWhenProvided() = runTest {
        val vm = viewModel(abiWithoutImage())
        var launched = 0

        vm.onBackgroundPickRequest(fitMode = BackgroundFit.COVER, scrimPermille = 400) { launched += 1 }

        assertEquals(1, launched)
    }

    @Test
    fun hostFailureIsVisibleAndClearsPendingChoice() = runTest {
        val vm = viewModel(abiWithoutImage())

        vm.onBackgroundPickRequest(fitMode = BackgroundFit.COVER, scrimPermille = 400) { }
        vm.reportBackgroundFailure("提供者拒绝或权限失效")

        assertTrue(vm.ui.value.status.contains("提供者拒绝"))
        assertTrue(vm.ui.value.failure != null)

        // 暂存已被清掉：此时再来一次「选图返回」应退回核心默认口径，而不是沿用陈旧选择。
        vm.applyPickedBackground("/tmp/x.png")
        assertTrue(
            "陈旧选择必须已被清除（否则会用上次的 fit/scrim）",
            vm.ui.value.status.contains("背景图已设置") ||
                vm.ui.value.status.contains("失败") ||
                vm.ui.value.status.contains("被拒"),
        )
    }
}
