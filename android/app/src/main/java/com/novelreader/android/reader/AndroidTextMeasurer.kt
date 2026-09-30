package com.novelreader.android.reader

import android.graphics.Typeface
import android.text.StaticLayout
import android.text.TextPaint

/**
 * 生产文本测量器：只使用公开 Android 排版 API（`StaticLayout` + `TextPaint`）。
 *
 * 与渲染端（`ui/ReaderScreen.kt`）的口径约定：
 * - 字号以**像素**传入（`sp × spToPx`），渲染端用同一 sp 值渲染 ⇒ 同像素；
 * - `letterSpacing = 0`，渲染端显式 `letterSpacing = 0.sp`（不继承 Material `bodyLarge` 的 0.5sp，
 *   否则每行少放几个字、比测量多出一行而被裁切）；
 * - 实测行高（像素）随 [MeasuredText.lineHeightPx] 回传，渲染端据此设置 `lineHeight`；
 * - 换行宽度 = 视口宽 − 左右边距；换行/断行由平台排版引擎完成，本类不实现断行算法。
 *
 * 不持有 Activity/Compose 状态，可在任意线程调用（由 ViewModel 串行化）。
 */
class AndroidTextMeasurer : TextMeasurer {

    override fun measure(text: String, spec: TextLayoutSpec): MeasuredText {
        val fallbackLineHeightPx = (spec.fontSizePx * spec.lineSpacingMultiplier).coerceAtLeast(0f)
        if (text.isEmpty() || !spec.isValid) {
            return MeasuredText(0, fallbackLineHeightPx, emptyList())
        }
        val paint = TextPaint(TextPaint.ANTI_ALIAS_FLAG).apply {
            textSize = spec.fontSizePx
            // 与 Compose 渲染端 letterSpacing = 0.sp 同口径（见 [TextLayoutContract]）：sp 字距在
            // 像素口径下必须显式归零，否则每行少放几个字、比测量多出一行而被裁切。
            letterSpacing = TextLayoutContract.LETTER_SPACING_EM
            typeface = Typeface.create(
                spec.fontFamily,
                if (spec.fontWeight >= BOLD_WEIGHT) Typeface.BOLD else Typeface.NORMAL,
            )
        }
        val layout = StaticLayout.Builder
            .obtain(text, 0, text.length, paint, spec.contentWidthPx)
            // 不额外叠加行距：行距只通过 setLineSpacing 的倍率表达，避免两侧各自叠加一次。
            .setIncludePad(false)
            .setLineSpacing(0f, spec.lineSpacingMultiplier)
            .build()
        if (layout.lineCount <= 0) {
            return MeasuredText(0, fallbackLineHeightPx, emptyList())
        }
        val measuredLineHeightPx = (layout.getLineBottom(0) - layout.getLineTop(0))
            .toFloat()
            .coerceAtLeast(spec.fontSizePx)
        val lineEnds = ArrayList<Int>(layout.lineCount)
        for (line in 0 until layout.lineCount) {
            lineEnds.add(layout.getLineEnd(line))
        }
        return MeasuredText(layout.lineCount, measuredLineHeightPx, lineEnds)
    }

    private companion object {
        const val BOLD_WEIGHT = 700
    }
}
