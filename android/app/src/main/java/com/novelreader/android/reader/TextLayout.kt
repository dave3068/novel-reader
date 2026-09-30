package com.novelreader.android.reader


/**
 * 排版测量的像素参数。
 *
 * 口径（AR3 / 子规格 design.md §排版）：
 * - 所有尺寸都是**像素**：`fontSizePx = fontSizeSp × spToPx`、`horizontalMarginPx = dp × dpToPx`；
 * - 视口是**含边距**的正文区域（Compose 用 `onSizeChanged` 汇报），内容区在此基础上减去边距；
 * - 默认字体权重 400（常规体），渲染端不得依赖 Material 默认字重/字距/行高。
 */
data class TextLayoutSpec(
    val viewportWidthPx: Int,
    val viewportHeightPx: Int,
    val fontSizePx: Float,
    val lineSpacingMultiplier: Float,
    val horizontalMarginPx: Float,
    val verticalMarginPx: Float,
    val fontFamily: String = TypographySettings.DEFAULT_FONT_FAMILY,
    val fontWeight: Int = TypographySettings.DEFAULT_FONT_WEIGHT,
) {
    /** 文本可用宽度（像素），至少 1px。 */
    val contentWidthPx: Int
        get() = (viewportWidthPx - 2f * horizontalMarginPx).toInt().coerceAtLeast(1)

    /** 文本可用高度（像素），可为 0（此时不排版任何行）。 */
    val contentHeightPx: Float
        get() = (viewportHeightPx - 2f * verticalMarginPx).coerceAtLeast(0f)

    /** 是否为可测量的排版参数（零视口/零字号是非法签名）。 */
    val isValid: Boolean
        get() = viewportWidthPx > 0 && viewportHeightPx > 0 && fontSizePx > 0f

    /**
     * 排版签名里的「额外行距」（毫像素）：`(行距倍数 - 1) × 字号`。
     *
     * 与桌面端 `line_spacing_milli` 同口径（桌面 `line_advance = font_size + line_spacing`），
     * 因此两端的签名含义一致，缓存键不会因为平台不同而含义漂移。
     */
    val extraLeadingMilli: Long
        get() = milliPx(((lineSpacingMultiplier - 1f) * fontSizePx).coerceAtLeast(0f))

    /** 字号（毫像素，u32 口径）。 */
    val fontSizeMilli: Long
        get() = milliPx(fontSizePx)
}

/** 浮点像素 → 千分之一像素整数（跨语言无浮点歧义）。 */
fun milliPx(value: Float): Long = Math.round(value * 1000.0).toLong().coerceAtLeast(0L)

/** 平台文本测量结果（只描述「排成了多少行、每行结束在哪个字符下标、行高多少像素」）。 */
data class MeasuredText(
    val lineCount: Int,
    val lineHeightPx: Float,
    /** 每行结束的**字符下标**（不含），长度等于 [lineCount]。 */
    val lineEnds: List<Int>,
)

/**
 * 平台文本测量器：唯一允许出现在 Kotlin 的排版职责（AR3）。
 *
 * 生产实现见 [AndroidTextMeasurer]（公开 Android 排版 API）；JVM 单测注入确定性替身。
 * 章节识别、位置校验、页边界合法性、缓存失效**都不在这里**——由 Rust `reading-core` 裁定。
 */
interface TextMeasurer {
    fun measure(text: String, spec: TextLayoutSpec): MeasuredText
}

/** 窗口内切出的一页正文。 */
data class PageSlice(
    /** 页起始字节（坐标空间内，等于窗口数据起点）。 */
    val startByte: Long,
    /** 页结束字节（位于合法 UTF-8 字符边界）。 */
    val endByte: Long,
    val text: String,
    val lineCount: Int,
    /** 是否已消费完整个窗口（不代表已到整本末尾，还需看坐标是否越过总长）。 */
    val reachedWindowEnd: Boolean,
)

/**
 * UTF-8 字节偏移换算：字符下标 → 字节数。
 *
 * 这不是核心业务：核心做的是「字节偏移是否落在合法字符边界 / 是否越界 / 是否与章节一致」的
 * **裁定**（`reading-index::boundary`），本对象只把平台测量得到的**字符下标**换算成字节数，
 * 供 `nr_submit_page_boundary` 使用。窗口字节来自核心的有界读取，两端已按字符边界裁剪，
 * 因此按字符逐点累加得到的偏移必然落在合法边界上。
 */
object Utf8Offsets {

    /** 文本前 [charCount] 个字符的 UTF-8 字节数（下标越界按文本末尾处理）。 */
    fun byteLengthOfPrefix(text: String, charCount: Int): Int {
        val limit = charCount.coerceIn(0, text.length)
        var bytes = 0
        var index = 0
        while (index < limit) {
            val code = text[index].code
            when {
                code < 0x80 -> {
                    bytes += 1
                    index += 1
                }
                code < 0x800 -> {
                    bytes += 2
                    index += 1
                }
                Character.isHighSurrogate(text[index]) &&
                    index + 1 < text.length &&
                    Character.isLowSurrogate(text[index + 1]) -> {
                    // 代理对 = 一个码点 = 4 字节
                    bytes += 4
                    index += 2
                }
                else -> {
                    bytes += 3
                    index += 1
                }
            }
        }
        return bytes
    }

    /** [charIndex] 是否落在码点边界上（不在代理对中间）。 */
    fun isCodePointBoundary(text: String, charIndex: Int): Boolean {
        if (charIndex <= 0 || charIndex >= text.length) {
            return true
        }
        val next = text[charIndex]
        return Character.isLowSurrogate(next).not()
    }

    /** 至少消费一个完整码点的下标（保证翻页必然前进，避免零进度死循环）。 */
    fun nextCodePointIndex(text: String, charIndex: Int): Int {
        if (text.isEmpty()) {
            return 0
        }
        var index = charIndex.coerceIn(0, text.length - 1)
        if (Character.isHighSurrogate(text[index]) && index + 1 < text.length) {
            index += 1
        }
        return (index + 1).coerceAtMost(text.length)
    }
}

/**
 * 页内切分（页边界数学，C3 检查项）。
 *
 * 只做两件事，且都是平台侧必须做的事：
 * 1. 按可用高度决定**能完整放下的行数**，并预留**整行**安全余量（宁可少排一行，不裁切）；
 * 2. 把「放下的字符」换算成合法 UTF-8 结束字节，交给核心登记。
 */
object PageLayout {

    /** 可用内容高度（像素）。 */
    fun usableContentHeightPx(spec: TextLayoutSpec): Float = spec.contentHeightPx

    /**
     * 能完整放下的行数。
     *
     * 预算 = 可用高度 − 一整行（安全余量）；按实测行高逐行累加判断，
     * 因此 `行数 × 行高 ≤ 可用高度` 必然成立，末行不会被裁切。
     */
    fun fittingLineCount(measured: MeasuredText, spec: TextLayoutSpec): Int {
        if (measured.lineCount <= 0 || measured.lineHeightPx <= 0f) {
            return 0
        }
        val budget = usableContentHeightPx(spec) - measured.lineHeightPx
        if (budget <= 0f) {
            return 0
        }
        var fitted = 0
        while (fitted < measured.lineCount && (fitted + 1) * measured.lineHeightPx <= budget) {
            fitted += 1
        }
        return fitted
    }

    /**
     * 从窗口文本切出一页。
     *
     * [windowStartByte] 必须是窗口**数据**的起始字节（`nr_window.offset + dropped_leading`）。
     * [lineCount] ≤ 0 时至少消费一个完整码点：零长度页会让翻页永远不前进。
     */
    fun slice(
        windowStartByte: Long,
        text: String,
        measured: MeasuredText,
        lineCount: Int,
        prefixByteEnds: LongArray? = null,
    ): PageSlice {
        if (text.isEmpty()) {
            return PageSlice(windowStartByte, windowStartByte, "", 0, true)
        }
        val visibleLines = lineCount.coerceAtMost(measured.lineEnds.size).coerceAtLeast(0)
        var endChar = if (visibleLines > 0) {
            measured.lineEnds[visibleLines - 1]
        } else {
            0
        }
        endChar = endChar.coerceIn(0, text.length)
        if (endChar <= 0 || Utf8Offsets.isCodePointBoundary(text, endChar).not()) {
            endChar = Utf8Offsets.nextCodePointIndex(text, 0)
        }
        endChar = endChar.coerceAtLeast(1).coerceAtMost(text.length)
        // 字节偏移优先取**解码时记录的精确映射**（[EncodedTextCodec]）：
        // 核心的坐标空间是原始文件字节，页边界按同一份字节判定，
        // 用「解码后重新编码」推算长度会在变长编码上算错（真机 NR_ERR_UTF8_BOUNDARY）。
        // 未提供映射时（单测的 UTF-8 夹具）退回 UTF-8 字节计数。
        val prefixBytes = prefixByteEnds?.get(endChar.coerceIn(0, prefixByteEnds.size - 1))
            ?: Utf8Offsets.byteLengthOfPrefix(text, endChar).toLong()
        val endByte = windowStartByte + prefixBytes
        return PageSlice(
            startByte = windowStartByte,
            endByte = endByte,
            text = text.substring(0, endChar),
            lineCount = visibleLines,
            reachedWindowEnd = endChar >= text.length,
        )
    }
}
