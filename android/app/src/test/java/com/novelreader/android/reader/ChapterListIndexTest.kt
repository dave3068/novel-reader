package com.novelreader.android.reader

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 目录浮层的「自动定位到当前章」判据（纯函数，边界逐条锁定）。
 *
 * # 真机缺陷背景
 *
 * 目录此前总是从第 1 项开始显示：章节上千时（如《深空彼岸》1436 章），
 * 用户必须手动翻很久才能找到自己正在读的位置。
 */
class ChapterListIndexTest {

    @Test
    fun locatesCurrentChapterWithContextRowsAbove() {
        // 当前第 6 章（下标 5），上方保留 2 行上下文 ⇒ 首个可见项为下标 3。
        assertEquals(3, chapterListInitialIndex(5, 10))
    }

    @Test
    fun neverGoesNegativeNearTheStartOfTheBook() {
        assertEquals("第 1 章", 0, chapterListInitialIndex(0, 10))
        assertEquals("第 2 章（上方容不下 2 行）", 0, chapterListInitialIndex(1, 10))
        assertEquals("第 3 章（恰好贴顶）", 0, chapterListInitialIndex(2, 10))
        assertEquals("第 4 章起才有上方上下文", 1, chapterListInitialIndex(3, 10))
    }

    @Test
    fun clampsWhenChapterIndexExceedsKnownChapterCount() {
        // 核心章节数变小（换编码/换索引版本重扫）时不得产生非法下标。
        assertEquals(7, chapterListInitialIndex(999, 10))
        assertEquals(0, chapterListInitialIndex(-5, 10))
    }

    @Test
    fun unknownOrEmptyChapterListStartsAtTop() {
        assertEquals(0, chapterListInitialIndex(null, 10))
        assertEquals(0, chapterListInitialIndex(3, 0))
        assertEquals(0, chapterListInitialIndex(null, 0))
    }

    @Test
    fun worksForVeryLongChapterLists() {
        // 与真实样本同量级：《深空彼岸》1436 章，读到第 1436 章时应定位到倒数第 3 项。
        assertEquals(1433, chapterListInitialIndex(1435, 1436))
        assertEquals("最后一章", 1433, chapterListInitialIndex(1436, 1436))
    }
}
