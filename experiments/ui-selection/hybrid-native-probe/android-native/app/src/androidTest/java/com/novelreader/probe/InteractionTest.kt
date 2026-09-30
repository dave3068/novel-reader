package com.novelreader.probe

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import org.junit.Rule
import org.junit.Test

/**
 * 真机 UI 交互回归测试。
 *
 * 覆盖真机反馈的三类问题：
 *  1. 点击底部标签栏能切换页面（此前部分回调为空实现）；
 *  2. 「上一页 / 下一页」按钮有反应且页码可见变化（此前因布局溢出被挤出屏幕）；
 *  3. 正文区域左 1/3 点击 = 上一页、右 1/3 = 下一页（手势响应）。
 *
 * 运行：`gradle :app:connectedDebugAndroidTest`（需连接真机/模拟器）
 */
class InteractionTest {

    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    /** 等待阅读页首屏排版完成（共享库 + StaticLayout 是异步的）。 */
    private fun awaitFirstPage() {
        rule.waitUntil(timeoutMillis = 30_000) {
            runCatching {
                rule.onNodeWithTag("reader_status").assertTextContains("第 1 页", substring = true)
            }.isSuccess
        }
    }

    @Test
    fun bottomTabsSwitchScreens() {
        rule.onNodeWithTag("tab_settings").performClick()
        rule.onNodeWithText("阅读设置").assertIsDisplayed()

        rule.onNodeWithTag("tab_shelf").performClick()
        rule.onNodeWithText("共 ", substring = true).assertIsDisplayed()

        rule.onNodeWithTag("tab_reader").performClick()
        rule.onNodeWithTag("btn_next").assertIsDisplayed()
    }

    @Test
    fun nextPageButtonAdvancesOrdinal() {
        rule.onNodeWithTag("tab_reader").performClick()
        awaitFirstPage()

        rule.onNodeWithTag("btn_next").performClick()
        rule.waitUntil(timeoutMillis = 30_000) {
            runCatching {
                rule.onNodeWithTag("reader_status").assertTextContains("第 2 页", substring = true)
            }.isSuccess
        }
        rule.onNodeWithTag("reader_status").assertTextContains("第 2 页", substring = true)

        rule.onNodeWithTag("btn_prev").performClick()
        rule.waitUntil(timeoutMillis = 30_000) {
            runCatching {
                rule.onNodeWithTag("reader_status").assertTextContains("第 1 页", substring = true)
            }.isSuccess
        }
    }

    @Test
    fun tapZonesOnReaderAreaTurnPages() {
        rule.onNodeWithTag("tab_reader").performClick()
        awaitFirstPage()

        // 正文区域右侧 1/3 点击 = 下一页
        rule.onNodeWithTag("reader_area").performTouchInput {
            click(Offset(right - 30f, centerY))
        }
        rule.waitUntil(timeoutMillis = 30_000) {
            runCatching {
                rule.onNodeWithTag("reader_status").assertTextContains("第 2 页", substring = true)
            }.isSuccess
        }
        rule.onNodeWithTag("reader_status").assertTextContains("下一页", substring = true)

        // 正文区域左侧 1/3 点击 = 上一页
        rule.onNodeWithTag("reader_area").performTouchInput {
            click(Offset(left + 30f, centerY))
        }
        rule.waitUntil(timeoutMillis = 30_000) {
            runCatching {
                rule.onNodeWithTag("reader_status").assertTextContains("上一页", substring = true)
            }.isSuccess
        }
        rule.onNodeWithTag("reader_status").assertTextContains("上一页", substring = true)
    }

    @Test
    fun settingsSwitchRespondsToTap() {
        rule.onNodeWithTag("tab_settings").performClick()
        rule.onNodeWithTag("switch_dark").performClick()
        // 开关被真实点击后状态行会更新为模式文案
        rule.waitUntil(timeoutMillis = 15_000) {
            runCatching {
                rule.onNodeWithText("夜间模式", substring = true).assertIsDisplayed()
            }.isSuccess
        }
        rule.onNodeWithTag("btn_open_file").assertIsDisplayed()
    }
}
