package com.novelreader.android.reader

/**
 * 书架的自适应布局判定（纯逻辑，不依赖 Compose，便于 JVM 单测）。
 *
 * 2026-10-09 主人确认：
 * - **窄屏（竖屏手机）** 用「文件管理器式」下钻列表（目录在上、书籍在下，逐层进入）；
 * - **宽屏（横屏 / 平板）** 保留左右布局（分类侧栏 + 书籍列表）。
 */
object ShelfLayout {

    /**
     * 小于该宽度（dp）即用下钻列表。
     *
     * 600dp 是 Material 的 medium 断点：本应用侧栏宽 190dp，600dp 之下书籍区已不足
     * 410dp，再窄就该把导航收进单列。恰好 600dp 视为宽屏。
     */
    const val COMPACT_MAX_WIDTH_DP = 600

    /** [widthDp] 是否走窄屏下钻布局。 */
    fun isCompact(widthDp: Float): Boolean = widthDp < COMPACT_MAX_WIDTH_DP
}
