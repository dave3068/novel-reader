package com.novelreader.probe.reader

import android.text.StaticLayout
import android.text.TextPaint
import com.novelreader.probe.core.NrEngine

/**
 * Android 端排版适配器（只使用公开 Android 文字排版 API）。
 *
 * ## 与渲染对齐（重要）
 * 真机曾出现「一页装不下、正文可滚动」的问题，根因是**测量与渲染的尺寸口径不一致**：
 * 本类原先用 `18f` 像素测量，而 Compose 的 `Text` 用 `bodyLarge`（≈16sp ≈ 51px），
 * 于是量出的「一页」能容纳约 3 倍的字量。现在约定：
 *  - 字号一律以**像素**传入（由 UI 用 `sp × density` 换算），Compose 侧用同一像素值渲染；
 *  - 本类把实际 **行高（像素）** 通过 [Page.lineHeightPx] 回传，UI 据此设置 `lineHeight`；
 *  - 测量时预留**半行安全余量**，吸收 StaticLayout 与 Compose 在行高取整上的微小差异。
 *
 * ## 职责边界（H-R2/H-R4）
 * 只负责「一屏能放下多少字」的测量与 **UTF-8 字节偏移**换算；页边界登记、位置与
 * 锚点语义全部交回共享库（`commitPageBoundary`）。`byteWindow` 允许注入不同字节来源
 * （模拟书走共享库 `readWindow`；SAF 文件由 Android 侧按窗口提供）。
 */
class AndroidPageLayouter(
    private val engine: NrEngine,
    private val widthPx: Int,
    private val heightPx: Int,
    private val fontSizePx: Float = 54f,
    private val lineSpacingMultiplier: Float = 1.35f,
    private val layoutId: Long = 1L,
    private val byteWindow: (suspend (Int, Long, Int) -> ByteArray)? = null,
) {
    data class Page(
        val startByte: Long,
        val endByte: Long,
        val text: String,
        val lineCount: Int,
        /** 实际行高（像素），供 Compose 对齐渲染。 */
        val lineHeightPx: Float,
        val reachedEnd: Boolean,
    )

    /** 排版一页。窗口读取是有限窗口，绝不整本载入。 */
    suspend fun measurePage(chapterIndex: Int, startByte: Long, windowBytes: Int = 64 * 1024): Page {
        val bytes = byteWindow?.invoke(chapterIndex, startByte, windowBytes)
            ?: engine.readWindow(chapterIndex, startByte, windowBytes)
        val text = bytes.toString(Charsets.UTF_8)
        if (text.isEmpty()) {
            return Page(startByte, startByte, "", 0, fontSizePx * lineSpacingMultiplier, true)
        }

        val paint = TextPaint(TextPaint.ANTI_ALIAS_FLAG).apply { textSize = fontSizePx }
        // StaticLayout.Builder 是 API 23+ 的公开 API（本工程 minSdk 31）。
        val layout = StaticLayout.Builder.obtain(text, 0, text.length, paint, widthPx)
            .setIncludePad(false)
            .setLineSpacing(0f, lineSpacingMultiplier)
            .build()

        val lineHeightPx = if (layout.lineCount > 0) {
            (layout.getLineBottom(0) - layout.getLineTop(0)).toFloat().coerceAtLeast(fontSizePx)
        } else {
            fontSizePx * lineSpacingMultiplier
        }
        // 预留**整行**安全余量：宁可一页少排一行，也绝不让最后一行被裁切
        // （Compose 与 StaticLayout 在行高/字符间距取整上可能存在极小差异）
        val usableHeight = (heightPx - lineHeightPx).coerceAtLeast(lineHeightPx)

        var lastLine = -1
        for (line in 0 until layout.lineCount) {
            if (layout.getLineBottom(line) > usableHeight) break
            lastLine = line
        }
        val lineCount = (lastLine + 1).coerceAtLeast(0)
        // 至少消费一个完整字符，避免零进度死循环
        val endChar = when {
            lastLine >= 0 -> layout.getLineEnd(lastLine)
            else -> text.offsetByCodePoints(0, 1)
        }.coerceAtLeast(1).coerceAtMost(text.length)

        val endByte = startByte + Utf8OffsetMapper.byteOffsetForCharIndex(text, endChar)
        val pageText = text.substring(0, endChar)
        val reachedEnd = endChar >= text.length
        // 共享库负责页面边界登记与位置语义；Android 只提供测量结果。
        engine.commitPageBoundary(chapterIndex, startByte, endByte, layoutId, lineCount)
        return Page(startByte, endByte, pageText, lineCount, lineHeightPx, reachedEnd)
    }
}
