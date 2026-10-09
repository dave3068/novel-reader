package com.novelreader.android.reader

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 「阅读时保持屏幕常亮」的路由决策（[ScreenOnController]）。
 *
 * 覆盖三条真机验收口径：
 * 1. **仅阅读页保持常亮** —— 书架 / 设置 / 历史必须恢复系统息屏；
 * 2. **离开立即恢复、回来重新保持** —— 路由来回切换时标志跟随；
 * 3. **重复路由不重复写标志** —— 翻页 / 状态文案更新产生大量同路由快照。
 *
 * 对应的静态门禁：`scripts/verify-android-scope.py` 的 `[13]`（宿主接线形状）。
 */
class ScreenOnControllerTest {

    @Test
    fun keepsScreenOnOnlyWhileReading() {
        val calls = mutableListOf<Boolean>()
        val controller = ScreenOnController { calls += it }

        controller.onRouteChanged(ReaderRoute.READER) // 置位
        controller.onRouteChanged(ReaderRoute.SHELF) // 清位
        controller.onRouteChanged(ReaderRoute.SETTINGS) // 非阅读页：取值未变，不重复写
        controller.onRouteChanged(ReaderRoute.READER) // 重新置位
        controller.onRouteChanged(ReaderRoute.HISTORY) // 清位

        assertEquals(
            "阅读页置位、离开（书架/设置/历史）清位；取值未变时不重复写窗口标志",
            listOf(true, false, true, false),
            calls,
        )
    }

    @Test
    fun repeatedRouteStatesAreNotReapplied() {
        val calls = mutableListOf<Boolean>()
        val controller = ScreenOnController { calls += it }

        controller.onRouteChanged(ReaderRoute.READER)
        controller.onRouteChanged(ReaderRoute.READER)
        controller.onRouteChanged(ReaderRoute.SHELF)
        controller.onRouteChanged(ReaderRoute.SHELF)

        assertEquals("同一路由的重复观测不得重复写窗口标志", listOf(true, false), calls)
    }

    @Test
    fun firstObservationAlignsWindowFlagOnce() {
        // 启动可能直接落在阅读页（重建 / 直达）也可能落在书架：首次观测即对齐一次。
        // 新窗口不会继承旧窗口的标志，因此首次观测必须写入。
        val readingCalls = mutableListOf<Boolean>()
        ScreenOnController { readingCalls += it }.onRouteChanged(ReaderRoute.READER)
        assertEquals("首帧在阅读页：首次观测必须置位", listOf(true), readingCalls)

        val shelfCalls = mutableListOf<Boolean>()
        ScreenOnController { shelfCalls += it }.onRouteChanged(ReaderRoute.SHELF)
        assertEquals("首帧在书架：首次观测对齐为「不保持」（写入幂等）", listOf(false), shelfCalls)
    }

    @Test
    fun everyRouteIsClassifiedForScreenOn() {
        // 路由集合变化（如新增页面）时必须重新分类「是否保持常亮」，不允许默默沿用旧口径。
        val expected = mapOf(
            ReaderRoute.SHELF to false,
            ReaderRoute.READER to true,
            ReaderRoute.SETTINGS to false,
            ReaderRoute.HISTORY to false,
        )
        assertEquals(
            "ReaderRoute 的每个取值都必须显式分类屏幕常亮策略（新增路由时更新本表）",
            expected.keys,
            ReaderRoute.entries.toSet(),
        )
        expected.forEach { (route, keepOn) ->
            val calls = mutableListOf<Boolean>()
            ScreenOnController { calls += it }.onRouteChanged(route)
            assertEquals("路由 $route 的常亮策略应为 $keepOn", listOf(keepOn), calls)
        }
    }
}
