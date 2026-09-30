package com.novelreader.android.reader

/**
 * 阅读页的**交互判定**（纯逻辑，不依赖 Compose，便于 JVM 单测）。
 *
 * 规格依据（`docs/novel-reader-v1-development-plan.md`）：
 * - `:620`「左 1/3 点击上一页，右 1/3 点击下一页，**中部显示/隐藏菜单**」；
 * - `:273`「桌面鼠标/键盘与 **Android 手势**」。
 *
 * 真机缺陷背景：PoC 阶段有手势、正式 V1 工程**从未移植**，且规格要求未被任何卡片认领
 * （见 `results/reader-interaction.md`）。
 */
enum class ReaderTapZone {
    PREVIOUS_PAGE,
    TOGGLE_CHROME,
    NEXT_PAGE,
}

/** 滑动手势的翻页方向（[NONE] = 位移不足，不翻页）。 */
enum class ReaderSwipeDirection {
    NONE,
    PREVIOUS_PAGE,
    NEXT_PAGE,
}

object ReaderInteraction {

    /** 横滑翻页的最小位移（dp）。低于该值视为误触，不翻页。 */
    const val SWIPE_THRESHOLD_DP = 48f

    /**
     * 点击分带：左 1/3 上一页、右 1/3 下一页、中部切换界面。
     *
     * 宽度非法（<= 0）时一律判为 [ReaderTapZone.TOGGLE_CHROME] —— 宁可只切换界面，
     * 也不在未知宽度下误触翻页。
     */
    fun tapZoneFor(x: Float, widthPx: Int): ReaderTapZone {
        if (widthPx <= 0) return ReaderTapZone.TOGGLE_CHROME
        val clamped = x.coerceIn(0f, (widthPx - 1).toFloat())
        val third = widthPx / 3f
        return when {
            clamped < third -> ReaderTapZone.PREVIOUS_PAGE
            clamped >= third * 2f -> ReaderTapZone.NEXT_PAGE
            else -> ReaderTapZone.TOGGLE_CHROME
        }
    }

    /**
     * 横滑方向：**向左划 = 下一页，向右划 = 上一页**（与常见阅读器一致）。
     *
     * [travelX] 为本次手势的横向总位移（像素，右为正）；位移绝对值不足 [thresholdPx] 时返回
     * [ReaderSwipeDirection.NONE]（不翻页）。判定为拖动的手势不再走点击分支，二者互斥。
     */
    fun swipePageFor(travelX: Float, thresholdPx: Float): ReaderSwipeDirection {
        if (thresholdPx <= 0f) return ReaderSwipeDirection.NONE
        if (kotlin.math.abs(travelX) < thresholdPx) return ReaderSwipeDirection.NONE
        return if (travelX < 0f) ReaderSwipeDirection.NEXT_PAGE else ReaderSwipeDirection.PREVIOUS_PAGE
    }
}
