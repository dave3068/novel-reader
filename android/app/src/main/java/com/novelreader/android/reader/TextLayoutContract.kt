package com.novelreader.android.reader

import kotlin.math.abs

/**
 * Android 排版口径契约（AR3 / A-T5；子规格 `android/design.md` §排版）。
 *
 * 本对象是「测量侧（[AndroidTextMeasurer]）」与「渲染侧（`ui/ReaderScreen.kt`）」两份代码必须共同
 * 遵守的**唯一数值口径**。任何一侧单独改动，都会让每页行数不一致：测量多排一行 → 渲染装不下、
 * 末行被裁切；测量少排一行 → 每页底部留白。因此这些数值只在这里定义一次，两侧都引用它。
 *
 * 四条约定：
 * 1. **字号统一用像素参与测量**：`fontSizePx = fontSizeSp × spToPx`，其中
 *    `spToPx = density × fontScale`（含系统字体缩放）。渲染端用**同一个** sp 值渲染
 *    （Compose 的 `sp → px` 换算与该公式一致），因此两侧落在同一像素值上（见 [fontSizePx]）；
 * 2. **字距两侧一致且必须为 0**：测量侧 `TextPaint.letterSpacing = ` [LETTER_SPACING_EM]，
 *    渲染侧 `letterSpacing = ` [LETTER_SPACING_SP]` .sp`。不得继承 Material `bodyLarge` 自带的
 *    0.5sp —— 那会让每行少放几个字、比测量多出一行而被裁切（字号越小影响越大）；
 * 3. **行高以测量结果为准**：实测行高（像素）经 [PageState.lineHeightPx] 回传，渲染端用
 *    [lineHeightSp] 换算成 `lineHeight` 设置，不得使用 Material 默认行高（如 `bodyLarge` 的 24sp）；
 * 4. **段距与首行缩进固定 0**：桌面端已验证「测量侧叠加段距/缩进、渲染侧不叠加」会造成每页少排
 *    若干行（见 `novel-reader-ui-env` 技能与开发方案 §8.3）。
 *
 * 本对象只做换算与常量声明：断行、页边界与合法性裁定仍由 Rust `reading-core` + 平台测量承担。
 */
object TextLayoutContract {

    /** 渲染端字距（sp）：0 = 不施加字符间距，与 [LETTER_SPACING_EM] 等值（不同单位、同一语义）。 */
    const val LETTER_SPACING_SP = 0f

    /** 测量端字距（em，`TextPaint.letterSpacing` 的单位）：0 = 不施加字符间距。 */
    const val LETTER_SPACING_EM = 0f

    /** 段距（sp）：固定 0，与 [TypographySettings.PARAGRAPH_SPACING_SP] 同值。 */
    const val PARAGRAPH_SPACING_SP = TypographySettings.PARAGRAPH_SPACING_SP

    /** 首行缩进（sp）：固定 0，与 [TypographySettings.FIRST_LINE_INDENT_SP] 同值。 */
    const val FIRST_LINE_INDENT_SP = TypographySettings.FIRST_LINE_INDENT_SP

    /** 正文字重：400（渲染端必须显式指定，不得继承 Material 默认值）。 */
    const val BODY_FONT_WEIGHT = TypographySettings.DEFAULT_FONT_WEIGHT

    /** 字号（用户可见 sp）→ 测量用像素（`spToPx` 含系统字体缩放）。 */
    fun fontSizePx(fontSizeSp: Float, spToPx: Float): Float = fontSizeSp * spToPx

    /** 边距（用户可见 dp）→ 像素（`dpToPx` 不含字体缩放，与字号换算刻意分开）。 */
    fun marginPx(marginDp: Float, dpToPx: Float): Float = marginDp * dpToPx

    /** 实测行高（像素）→ 渲染端 `lineHeight` 的 sp 值（`px ÷ spToPx`）。 */
    fun lineHeightSp(lineHeightPx: Float, spToPx: Float): Float =
        if (spToPx <= 0f || !spToPx.isFinite()) 0f else lineHeightPx / spToPx

    /** 渲染端 sp 行高 → 实际像素（用于校验「渲染与测量同口径」）。 */
    fun lineHeightPx(lineHeightSp: Float, spToPx: Float): Float = lineHeightSp * spToPx

    /** 换算因子是否可用于排版：零 / 负 / 非有限值都是非法口径（零视口是非法签名）。 */
    fun isValidScale(scale: Float): Boolean = scale > 0f && scale.isFinite()

    /** 行高经「像素 → sp → 像素」往返后是否仍是同一像素值（容差 0.001px）。 */
    fun roundTripsLineHeight(lineHeightPx: Float, spToPx: Float): Boolean =
        abs(lineHeightPx(lineHeightSp(lineHeightPx, spToPx), spToPx) - lineHeightPx) < 0.001f

    /** 字号经「像素 → sp → 像素」往返后是否仍是同一像素值（容差 0.001px）。 */
    fun roundTripsFontSize(fontSizePx: Float, spToPx: Float): Boolean =
        abs(lineHeightPx(lineHeightSp(fontSizePx, spToPx), spToPx) - fontSizePx) < 0.001f
}
