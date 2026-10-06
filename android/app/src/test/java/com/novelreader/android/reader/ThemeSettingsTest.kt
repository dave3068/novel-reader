package com.novelreader.android.reader

import com.novelreader.android.core.FakeReleaser
import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.core.NrErrors
import com.novelreader.android.core.NrThemeBase
import com.novelreader.android.core.NrThemeContrastResult
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 设置页「自定义配色」区的状态接线（V1.1 规格 §3.3 / §3.6）。
 *
 * 覆盖的重点是**界面与核心的一致性**：界面只显示核心认可的值，被拒的写入不改动显示。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ThemeSettingsTest {

    private fun le(size: Int): ByteBuffer = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    /** 造 `nr_theme_base`（56 字节）：四项各 12 字节定长槽，全 0 = 未设置。 */
    private fun baseBytes(
        enabled: Boolean,
        foreground: String = "",
        background: String = "",
        primary: String = "",
        accent: String = "",
    ): ByteArray {
        val buf = le(NrThemeBase.STRUCT_SIZE)
        buf.putInt(0, NrThemeBase.STRUCT_SIZE)
        buf.putInt(4, if (enabled) 1 else 0)
        listOf(foreground to 8, background to 20, primary to 32, accent to 44).forEach { (text, at) ->
            text.toByteArray(Charsets.UTF_8).copyInto(buf.array(), at)
        }
        return buf.array()
    }

    /** 造 `nr_theme_contrast_result`（24 字节）。 */
    private fun contrastBytes(ratioX100: Int, passes: Boolean): ByteArray {
        val buf = le(NrThemeContrastResult.STRUCT_SIZE)
        buf.putInt(0, NrThemeContrastResult.STRUCT_SIZE)
        buf.putInt(4, ratioX100)
        buf.putInt(8, if (passes) 1 else 0)
        return buf.array()
    }

    private fun viewModel(abi: ScriptedReaderAbi): ReaderViewModel {
        val releaser = FakeReleaser()
        return ReaderViewModel(
            engine = engineOn(abi, releaser),
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = UnconfinedTestDispatcher(),
            workScope = kotlinx.coroutines.CoroutineScope(UnconfinedTestDispatcher()),
        )
    }

    // ------------------------------------------------------------------ 纯状态

    @Test
    fun contrastLabelRendersIntegerRatioWithoutFloatingPoint() {
        assertEquals("4.54 : 1", ThemeState(contrastX100 = 454).contrastLabel)
        assertEquals("21.00 : 1", ThemeState(contrastX100 = 2100).contrastLabel)
        assertEquals("1.05 : 1", ThemeState(contrastX100 = 105).contrastLabel)
        // 未读取时给占位，不显示 0.00（免得被当成「对比度为 0」）。
        assertEquals("尚未读取", ThemeState(contrastX100 = 0).contrastLabel)
    }

    @Test
    fun baseOfMapsWhichToTheRightSlot() {
        val state = ThemeState(
            foreground = "#111111",
            background = "#222222",
            primary = "#333333",
            accent = "#444444",
        )
        assertEquals("#111111", state.baseOf(ThemeBase.FOREGROUND))
        assertEquals("#222222", state.baseOf(ThemeBase.BACKGROUND))
        assertEquals("#333333", state.baseOf(ThemeBase.PRIMARY))
        assertEquals("#444444", state.baseOf(ThemeBase.ACCENT))
        assertEquals("", state.baseOf(99))
        assertTrue(state.hasAnyBase)
        assertFalse(ThemeState().hasAnyBase)
    }

    @Test
    fun baseLabelsCoverAllFourIndicesInOrder() {
        assertEquals(4, ThemeBase.LABELS.size)
        assertEquals(ThemeBase.FOREGROUND, 0)
        assertEquals(ThemeBase.ACCENT, ThemeBase.LABELS.lastIndex)
    }

    // ------------------------------------------------------------------ 门面接线

    @Test
    fun openingSettingsReadsBaseColorsAndContrastFromCore() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            themeBaseBytes = baseBytes(
                enabled = true,
                foreground = "#222222",
                background = "#fdfdfd",
                primary = "#1890ff",
                accent = "#d4380d",
            )
            themeContrastBytes = contrastBytes(ratioX100 = 454, passes = true)
        }
        val vm = viewModel(abi)

        vm.openSettings()

        val theme = vm.ui.value.theme
        assertTrue(theme.enabled)
        assertEquals("#222222", theme.foreground)
        assertEquals("#fdfdfd", theme.background)
        assertEquals("#1890ff", theme.primary)
        assertEquals("#d4380d", theme.accent)
        assertEquals(454, theme.contrastX100)
        assertTrue(theme.contrastPasses)
        assertTrue(abi.events.contains("getThemeBase:enter"))
        assertTrue(abi.events.contains("getThemeContrast:enter"))
    }

    @Test
    fun disabledThemeWithNoBaseColorsShowsEmptyState() = runTest {
        // 核心回的**未设置**状态：全 0 槽 ⇒ 界面应为空串（据此显示「尚未设置」）。
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            themeBaseBytes = baseBytes(enabled = false)
            themeContrastBytes = contrastBytes(ratioX100 = 2100, passes = true)
        }
        val vm = viewModel(abi)

        vm.openSettings()

        val theme = vm.ui.value.theme
        assertFalse(theme.enabled)
        assertEquals("", theme.foreground)
        assertFalse("未设置任何基色时应显示空态", theme.hasAnyBase)
    }

    @Test
    fun enablingThemeWritesThroughAndReReads() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            // 核心在写入后返回「已启用」——界面必须回显**核心的值**，而不是乐观自更新。
            themeBaseBytes = baseBytes(enabled = true, primary = "#1890ff")
            themeContrastBytes = contrastBytes(ratioX100 = 700, passes = true)
        }
        val vm = viewModel(abi)

        vm.onThemeEnabledChange(true)

        assertEquals(1, abi.lastThemeEnabled)
        assertTrue(vm.ui.value.theme.enabled)
        assertTrue(abi.events.contains("getThemeBase:enter"))
    }

    @Test
    fun baseColorWriteForwardsWhichAndValue() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            themeBaseBytes = baseBytes(enabled = true, primary = "#ff0000")
            themeContrastBytes = contrastBytes(ratioX100 = 500, passes = true)
        }
        val vm = viewModel(abi)

        vm.onThemeBaseChange(ThemeBase.PRIMARY, "  #ff0000  ")

        // 值被 trim 后透传（核心侧还要做格式校验）。
        assertEquals(ThemeBase.PRIMARY to "#ff0000", abi.lastThemeBaseWrite)
        assertEquals("#ff0000", vm.ui.value.theme.primary)
    }

    @Test
    fun rejectedWriteKeepsUiShowingTheValueCoreActuallyHolds() = runTest {
        // 核心拒绝非法色值并**保留原值**：界面必须回读核心的旧值，
        // 不能把用户输入的非法串留在输入框里（否则界面与核心不一致）。
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            themeBaseBytes = baseBytes(enabled = true, primary = "#1890ff")
            themeContrastBytes = contrastBytes(ratioX100 = 454, passes = true)
            themeWriteCode = NrErrors.NR_ERR_INVALID_ARGUMENT
        }
        val vm = viewModel(abi)

        vm.onThemeBaseChange(ThemeBase.PRIMARY, "nope")

        assertEquals(ThemeBase.PRIMARY to "nope", abi.lastThemeBaseWrite)
        assertEquals(
            "被拒后界面必须显示核心认可的值",
            "#1890ff",
            vm.ui.value.theme.primary,
        )
        assertTrue(vm.ui.value.status.contains("被拒"))
    }

    @Test
    fun readFailureSurfacesInsteadOfSilentlyShowingEmpty() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            v11Code = NrErrors.NR_ERR_INTERNAL
        }
        val vm = viewModel(abi)

        vm.openSettings()

        // 读失败必须给出错误回显，**不得**静默显示成「未设置」。
        assertTrue(
            "读失败应有错误回显，实际状态：${vm.ui.value.status}",
            vm.ui.value.status.contains("读取配色失败"),
        )
    }

    @Test
    fun contrastBelowThresholdIsReportedButColorsAreKept() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build()).apply {
            themeBaseBytes = baseBytes(enabled = true, foreground = "#f0f0f0", background = "#fdfdfd")
            themeContrastBytes = contrastBytes(ratioX100 = 105, passes = false)
        }
        val vm = viewModel(abi)

        vm.openSettings()

        val theme = vm.ui.value.theme
        assertFalse("低对比度应如实回报不达标", theme.contrastPasses)
        // 但基色**保留**（核心 R12：只提示，不强改）。
        assertEquals("#f0f0f0", theme.foreground)
        assertEquals("#fdfdfd", theme.background)
    }

    @Test
    fun engineDestroyedByHostDoesNotBreakSettingsScreen() = runTest {
        val abi = ScriptedReaderAbi(FixtureBook.build())
        val releaser = FakeReleaser()
        val engine = engineOn(abi, releaser)
        val vm = ReaderViewModel(
            engine = engine,
            releaser = releaser,
            measurer = FakeTextMeasurer(),
            io = UnconfinedTestDispatcher(),
            workScope = kotlinx.coroutines.CoroutineScope(UnconfinedTestDispatcher()),
        )
        engine.destroy()

        vm.openSettings()

        // 句柄已销毁时给错误回显，不崩溃（宿主生命周期取消后仍可能进设置页）。
        assertTrue(vm.ui.value.status.contains("读取配色失败"))
        assertEquals(NrAbiConstants.NR_DEFAULT_WINDOW_BYTES > 0, true)
    }
}
