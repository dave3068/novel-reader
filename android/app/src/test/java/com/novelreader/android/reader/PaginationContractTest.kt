package com.novelreader.android.reader

import com.novelreader.android.core.NrAbiConstants
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Android 排版口径与页边界契约（A-T5 / android/checklist.md **C5**；AR3、AR4）。
 *
 * 本文件是「测量侧 ↔ 渲染侧」契约的固化点，逐条对应卡片完成定义：
 *
 * | 契约 | 断言位置 |
 * |---|---|
 * | 字号统一用像素参与测量（`sp × density`），测量行高回传渲染端 | [fontSizeUsesPixelConversionSpTimesDensityAndMatchesCoreSignature]、[measuredLineHeightIsEchoedBackForTheRendererWithPixelRoundTrip] |
 * | Compose 显式 `letterSpacing = 0.sp` 且与测量一致，关键属性不依赖 Material 默认值 | [renderSideBodyStyleIsExplicitAndMatchesTheMeasurementContract]、[measurerSideLetterSpacingComesFromTheSharedContract] |
 * | 测量预留整行安全余量（宁可少排一行，不裁切） | [fittingLineCountKeepsWholeLineSafetyMarginSoTheLastLineIsNeverClipped] |
 * | 向核心提交合法 UTF-8 页末字节 | [pageBoundaryAlwaysLandsOnALegalUtf8CodePointBoundary]、[committedPageBoundariesFormALegalUtf8Chain] |
 * | 过期结果不覆盖当前页面 | [stalePrefetchedPageNeverEntersTheForwardCache]（分页路径）+ `StaleLayoutResultTest`（5 条用例，落在同一条必需验证命令 `--tests '*Layout*'` 的过滤范围内） |
 * | 正文路径无滚动容器（伪分页） | [readerBodyNeverUsesAScrollContainer] |
 *
 * 渲染侧（`ui/ReaderScreen.kt`）无法在 JVM 上真正渲染，因此那两条契约用**源码静态审计**固定，
 * 并且每条审计都带**变异反向用例**（见 [renderSideBodyStyleIsExplicitAndMatchesTheMeasurementContract]
 * 与 [readerBodyNeverUsesAScrollContainer]）：变异的源码必须被判出违规，证明审计不是空检查。
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PaginationContractTest {

    private fun harnessFor(scope: TestScope, measurer: FakeTextMeasurer = FakeTextMeasurer()): ReaderHarness =
        ReaderHarness(
            io = StandardTestDispatcher(scope.testScheduler),
            scope = TestScope(scope.testScheduler),
            measurer = measurer,
        )

    // ---------------------------------------------------------------- 像素口径

    /**
     * 字号只以像素参与测量：`fontSizePx = fontSizeSp × spToPx`（`spToPx` 含系统字体缩放），
     * 且 13 项排版签名里的字号/额外行距是同一像素值的 milli-px 表达；渲染端用同一 sp 值渲染，
     * 因此 `页面像素字号 ÷ spToPx` 必须回到用户设置值。
     */
    @Test
    fun fontSizeUsesPixelConversionSpTimesDensityAndMatchesCoreSignature() = runTest {
        val harness = harnessFor(this)
        // 模拟 420dpi + 系统字体缩放：spToPx = 2.75，dpToPx = 2.625（两者刻意不同）
        harness.measureViewport(widthPx = 400, heightPx = 140, spToPx = 2.75f, dpToPx = 2.625f)
        harness.openAndLayout()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        val settings = state.settings

        val begin = harness.abi.boundarySubmissions.first { it.phase == NrAbiConstants.NR_SUBMIT_BEGIN_LAYOUT }
        val fields = begin.layoutFields
        assertNotNull("BEGIN_LAYOUT 必须携带 13 项排版签名", fields)
        assertEquals(NrAbiConstants.NR_LAYOUT_FIELDS_SLOTS, fields!!.size)

        // 内容区宽 = 400 − 2×20dp×2.625 = 295；内容区高 = round(140 − 2×16dp×2.625) = 56
        assertEquals(295L, fields[0])
        assertEquals(56L, fields[1])
        // 字号：18sp × 2.75 px/sp = 49.5 px → 49500 milli-px
        assertEquals(49_500L, fields[4])
        // 额外行距：(1.35 − 1) × 49.5 px = 17.325 px → 17325 milli-px
        assertEquals(17_325L, fields[5])
        // 段距 / 首行缩进固定 0（两侧渲染能力未对齐前不参与排版）
        assertEquals(Math.round(TextLayoutContract.PARAGRAPH_SPACING_SP * 2.75f * 1000f).toLong(), fields[6])
        assertEquals(Math.round(TextLayoutContract.FIRST_LINE_INDENT_SP * 2.75f * 1000f).toLong(), fields[7])

        val page = state.reader.page!!
        assertEquals(
            "页面必须记录本次测量真正使用的像素字号",
            49.5f,
            page.fontSizePx,
            0.0001f,
        )
        assertEquals(
            "渲染端 sp 与测量像素必须同口径",
            settings.fontSizeSp,
            page.fontSizePx / state.spToPx,
            0.0001f,
        )
        assertEquals(
            TextLayoutContract.fontSizePx(settings.fontSizeSp, state.spToPx),
            page.fontSizePx,
            0.0001f,
        )
        // 可用高度 56px、实测行高 27px ⇒ 预算 56 − 27 = 29 ⇒ 每页 1 行
        assertEquals(1, page.lineCount)
        // 第一行 = 章节标题行 + 1 个正文汉字（夹具测量器每行 10 字符）：8 个标题字符 20B + '\n' 1B + '中' 3B
        assertEquals(24L, page.endByte)
        assertTrue(isUtf8Boundary(harness.book.bytes, page.endByte))
    }

    /**
     * 测量得到的**实际行高**必须回传渲染端：页面记录的是测量器给出的小数值（不是「字号×倍数」的推算值），
     * 且「像素 → sp → 像素」往返必须还原同一像素（渲染端 `lineHeight` 与测量同口径）。
     */
    @Test
    fun measuredLineHeightIsEchoedBackForTheRendererWithPixelRoundTrip() = runTest {
        val harness = harnessFor(this, measurer = FakeTextMeasurer(charsPerLine = 10, lineHeightPx = 31.5f))
        harness.measureViewport(widthPx = 400, heightPx = 140, spToPx = 2.75f, dpToPx = 1f)
        harness.openAndLayout()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        val page = state.reader.page!!
        assertEquals("页面必须回传测量器的实测行高，而不是推算值", 31.5f, page.lineHeightPx, 0.0001f)
        assertTrue(
            "行高必须在「像素 → sp → 像素」往返后还原（渲染端 lineHeight 与测量同口径）",
            TextLayoutContract.roundTripsLineHeight(page.lineHeightPx, state.spToPx),
        )
        val lineHeightSp = TextLayoutContract.lineHeightSp(page.lineHeightPx, state.spToPx)
        assertTrue("渲染端 lineHeight 必须为正，否则会被 Material 默认行高接管", lineHeightSp > 0f)
        assertEquals(
            page.lineHeightPx,
            TextLayoutContract.lineHeightPx(lineHeightSp, state.spToPx),
            0.001f,
        )

        // 可用高度 140 − 2×16 = 108；预算 108 − 31.5 = 76.5 ⇒ 2 行（3×31.5 = 94.5 > 76.5）
        assertEquals(2, page.lineCount)
        assertTrue(
            "行数×行高不得超过可用高度（否则末行被裁切）",
            page.lineCount * page.lineHeightPx <= PageLayout.usableContentHeightPx(
                TextLayoutSpec(
                    viewportWidthPx = 400,
                    viewportHeightPx = 140,
                    fontSizePx = TextLayoutContract.fontSizePx(18f, 2.75f),
                    lineSpacingMultiplier = 1.35f,
                    horizontalMarginPx = 0f,
                    verticalMarginPx = 16f,
                ),
            ),
        )
    }

    // ---------------------------------------------------------------- 安全余量

    /**
     * 整行安全余量（检查项 C5「小字号末行不裁切」）。
     *
     * 判据：`行数 × 行高 ≤ 可用高度 − 一整行`。因此即使渲染端因平台度量差异比测量**多排一行**，
     * 该行也仍在可视区内 —— 这是「宁可少排一行，不裁切」的可验证含义。
     */
    @Test
    fun fittingLineCountKeepsWholeLineSafetyMarginSoTheLastLineIsNeverClipped() {
        data class Case(val fontSizeSp: Float, val spToPx: Float, val viewportHeightPx: Int)

        val cases = listOf(
            // 最小字号 12sp：最容易被挤出末行
            Case(TypographySettings.MIN_FONT_SIZE_SP, 1f, 140),
            Case(TypographySettings.MIN_FONT_SIZE_SP, 2.625f, 120),
            // 默认与最大字号 × 常见/高分屏密度
            Case(TypographySettings.DEFAULT_FONT_SIZE_SP, 2.75f, 640),
            Case(TypographySettings.MAX_FONT_SIZE_SP, 2f, 400),
        )

        for (case in cases) {
            val fontSizePx = TextLayoutContract.fontSizePx(case.fontSizeSp, case.spToPx)
            // 平台实测行高 ≈ 字号 × 行距倍数（StaticLayout + setLineSpacing 的近似）；取小数以覆盖非整数行高
            val lineHeightPx = fontSizePx * TypographySettings.DEFAULT_LINE_SPACING_MULTIPLIER + 0.5f
            val spec = TextLayoutSpec(
                viewportWidthPx = 400,
                viewportHeightPx = case.viewportHeightPx,
                fontSizePx = fontSizePx,
                lineSpacingMultiplier = TypographySettings.DEFAULT_LINE_SPACING_MULTIPLIER,
                horizontalMarginPx = 0f,
                verticalMarginPx = 0f,
            )
            val measured = MeasuredText(
                lineCount = 500,
                lineHeightPx = lineHeightPx,
                lineEnds = (1..500).toList(),
            )
            val fitted = PageLayout.fittingLineCount(measured, spec)
            val available = PageLayout.usableContentHeightPx(spec)

            assertTrue(
                "字号 ${case.fontSizeSp}sp（${case.spToPx}px/sp，可用 ${available}px，行高 ${lineHeightPx}px）至少要排下一行",
                fitted >= 1,
            )
            assertTrue(
                "字号 ${case.fontSizeSp}sp：已排行数必须装进可用高度（fitted=$fitted）",
                fitted * lineHeightPx <= available,
            )
            assertTrue(
                "字号 ${case.fontSizeSp}sp：整行安全余量必须成立 —— 渲染端多排一行也不裁切" +
                    "（fitted=$fitted 行高=$lineHeightPx 可用=$available）",
                (fitted + 1) * lineHeightPx <= available,
            )
            // 小字号也要保证「装下的行」真的能切出来，且页末落在合法边界
            val text = "中".repeat(500)
            val slice = PageLayout.slice(
                windowStartByte = 0L,
                text = text,
                measured = measured,
                lineCount = fitted,
            )
            assertEquals("页内行数必须等于装下的行数", fitted, slice.lineCount)
            assertEquals("页文本必须是前 fitted 行覆盖的字符", fitted, slice.text.length)
            assertEquals("页末偏移 = 页文本的 UTF-8 字节数", (fitted * 3).toLong(), slice.endByte)
            assertTrue(isUtf8Boundary(text.toByteArray(Charsets.UTF_8), slice.endByte))
        }

        // 极端情况：视口比一整行还矮 → 0 行（宁可少排一行、不裁切），但翻页必须仍能前进一个完整码点
        val tinySpec = TextLayoutSpec(
            viewportWidthPx = 400,
            viewportHeightPx = 20,
            fontSizePx = TextLayoutContract.fontSizePx(18f, 1f),
            lineSpacingMultiplier = 1.35f,
            horizontalMarginPx = 0f,
            verticalMarginPx = 0f,
        )
        val tinyMeasured = MeasuredText(5, 27f, listOf(10, 20, 30, 40, 50))
        assertEquals(0, PageLayout.fittingLineCount(tinyMeasured, tinySpec))
        val tinySlice = PageLayout.slice(0L, "中文正文", tinyMeasured, 0)
        // 一个汉字 = 一个码点 = 3 字节；零行可用时也只消费这一个码点
        assertEquals("中", tinySlice.text)
        assertEquals(3L, tinySlice.endByte)
    }

    // ---------------------------------------------------------------- UTF-8 页末

    /**
     * 页末字节必须落在合法 UTF-8 码点边界上，且「页文本重新编码 == 原始字节区间」
     * （半字、半个代理对、替换字符都说明页边界非法）。
     */
    @Test
    fun pageBoundaryAlwaysLandsOnALegalUtf8CodePointBoundary() {
        val samples = listOf(
            "中文正文".repeat(6),
            "abc中😀文\n".repeat(6),
            "😀😀😀😀",
            "a😀b中c\n",
            "中文\r\n正文\n",
            "中".repeat(40),
        )

        for (text in samples) {
            val bytes = text.toByteArray(Charsets.UTF_8)
            val measured = MeasuredText(1, 27f, listOf(text.length / 2))
            var previousEnd = 0L
            for (lines in 0..6) {
                val slice = PageLayout.slice(
                    windowStartByte = WINDOW_START,
                    text = text,
                    measured = measured,
                    lineCount = lines,
                )
                val endOffset = slice.endByte - WINDOW_START
                assertTrue(
                    "页末必须落在合法 UTF-8 边界（lines=$lines end=$endOffset text=${text.take(4)}…）",
                    isUtf8Boundary(bytes, endOffset),
                )
                assertEquals(
                    "页末偏移必须等于页文本的 UTF-8 字节数（lines=$lines）",
                    Utf8Offsets.byteLengthOfPrefix(text, slice.text.length).toLong(),
                    endOffset,
                )
                assertEquals(
                    "页文本必须等于原始字节区间的解码结果（无半字/无替换字符）",
                    String(bytes.copyOfRange(0, endOffset.toInt()), Charsets.UTF_8),
                    slice.text,
                )
                assertTrue(
                    "页文本长度必须落在码点边界上（无半个代理对）",
                    Utf8Offsets.isCodePointBoundary(text, slice.text.length),
                )
                assertTrue("翻页必须至少消费一个完整码点", slice.endByte > slice.startByte)
                assertTrue("行数增加不得让页末回退", slice.endByte >= previousEnd)
                previousEnd = slice.endByte
            }
        }
    }

    /** 行末落在代理对中间时，页边界必须回退到整码点（4 字节），不得切出半个 emoji。 */
    @Test
    fun pageBoundaryNeverSplitsASurrogatePair() {
        val text = "😀中文"
        val slice = PageLayout.slice(
            windowStartByte = 0L,
            text = text,
            measured = MeasuredText(1, 27f, listOf(1)),
            lineCount = 1,
        )
        assertEquals("代理对必须整体消费（4 字节）", 4L, slice.endByte)
        assertEquals("😀", slice.text)
        assertEquals(1, slice.text.codePointCount(0, slice.text.length))
    }

    /** 提交给核心的页末与界面展示的页必须同源：每条登记的边界都合法，且同世代内首尾相接。 */
    @Test
    fun committedPageBoundariesFormALegalUtf8Chain() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        harness.viewModel.nextPage()
        harness.viewModel.nextPage()
        harness.settle(this)

        harness.assertContiguousChain()
        val records = harness.abi.commitRecords
        assertTrue("翻页后必须登记多页边界（实际 ${records.size}）", records.size > 1)
        for (record in records) {
            assertEquals("只有 COMMIT 阶段才携带页边界", NrAbiConstants.NR_SUBMIT_COMMIT, record.phase)
            assertTrue("页末必须严格前进", record.endByte > record.startByte)
            assertTrue(
                "每条页末都必须落在合法 UTF-8 边界（${record.startByte}→${record.endByte}）",
                isUtf8Boundary(harness.book.bytes, record.endByte),
            )
            assertTrue(
                "页起点也必须是合法边界（${record.startByte}）",
                isUtf8Boundary(harness.book.bytes, record.startByte),
            )
        }

        val page = harness.viewModel.ui.value.reader.page!!
        assertTrue(
            "界面展示的页必须已在核心登记（${page.startByte}→${page.endByte}）",
            records.any { it.startByte == page.startByte && it.endByte == page.endByte },
        )
        assertEquals(
            "展示文本必须等于同字节区间的真实解码结果",
            String(harness.book.bytes.copyOfRange(page.startByte.toInt(), page.endByte.toInt()), Charsets.UTF_8),
            page.text,
        )
        assertEquals(
            "已登记页数必须等于真实提交记录数（过期结果不计入）",
            records.size.toLong(),
            harness.abi.registeredPageCount,
        )
    }

    // ---------------------------------------------------------------- 过期结果

    /**
     * 被核心判为过期的预取页**不得进入前向缓存**（AR4「过期结果不覆盖当前状态」在分页路径上的表现）。
     *
     * 夹具场景：打开后预取 `[83,176)`、`[176,207)` 两页；再翻一页时预取链走到**章节 1 第 1 页**
     * （`207`），这一页是本次唯一的真实测量 —— 脚本让核心判它过期，于是：
     * - 该页不得被缓存（否则下一次翻页会把一个从未登记的过期结果当成可见页）；
     * - 已测量过的 `[176,207)` 仍应复用命中，因此前向缓存恰好剩 1 页。
     */
    @Test
    fun stalePrefetchedPageNeverEntersTheForwardCache() = runTest {
        val harness = harnessFor(this)
        harness.measureViewport()
        harness.openAndLayout()
        harness.settle(this)
        assertEquals(
            "打开后应预取满 ${ReaderViewModel.MAX_FORWARD_CACHE} 页",
            ReaderViewModel.MAX_FORWARD_CACHE,
            harness.viewModel.ui.value.reader.forwardCacheCount,
        )

        harness.abi.commitStatusScript.addLast(NrAbiConstants.NR_BOUNDARY_STALE_LAYOUT_ID)
        harness.viewModel.nextPage()
        harness.settle(this)

        val state = harness.viewModel.ui.value
        assertNull("过期不是失败：不得残留错误状态", state.failure)
        val page = state.reader.page!!
        assertEquals("翻页后必须落在已登记的下一页", ReaderHarness.PAGE_ONE_END, page.startByte)
        assertEquals(ReaderHarness.PAGE_TWO_END, page.endByte)
        assertTrue(
            "可见页必须是核心已登记的页边界",
            harness.abi.commitRecords.any { it.startByte == page.startByte && it.endByte == page.endByte },
        )
        assertEquals(
            "过期预取结果不得进入前向缓存（过期结果不得被当成可翻到的页）",
            1,
            state.reader.forwardCacheCount,
        )
        assertTrue(
            "过期预取页（章节 1 起点 = 章节 0 字节长度）不得被登记进当前世代",
            harness.abi.commitRecords.none { it.startByte == ReaderHarness.CHAPTER_ZERO_BYTES },
        )
    }

    // ---------------------------------------------------------------- 渲染侧契约（静态审计）

    /**
     * 渲染侧正文必须逐项显式指定关键排版属性，且与测量侧同口径；带变异反向用例。
     *
     * 这条断言覆盖「不得依赖 `bodyLarge` 等默认字距/行高」与「不得混用像素与 sp 口径」两项禁止事项。
     */
    @Test
    fun renderSideBodyStyleIsExplicitAndMatchesTheMeasurementContract() {
        val source = ComposeTextContractAudit.readerScreenSource()
        val bodyCall = ComposeTextContractAudit.bodyTextCall(source)

        val violations = ComposeTextContractAudit.auditBodyTextCall(bodyCall)
        assertEquals("正文排版属性必须逐项显式且与测量同口径，实际违规：$violations", emptyList<String>(), violations)

        // 变异反向用例：每个变异都必须被判出违规（证明审计非空检查）。
        val mutations = listOf(
            "letterSpacing = 0.sp" to "letterSpacing = 0.5.sp",
            "fontWeight = FontWeight.Normal" to "fontWeight = FontWeight.Bold",
            "page.lineHeightPx" to "18f",
            "state.settings.fontSizeSp.sp" to "16.sp",
            "overflow = TextOverflow.Clip" to "overflow = TextOverflow.Ellipsis",
            "fontFamily = FontFamily.SansSerif" to "fontFamily = FontFamily.Serif",
        )
        for ((target, replacement) in mutations) {
            assertTrue("反向用例前提不足：正文调用块里没有 `$target`", bodyCall.contains(target))
            val mutated = bodyCall.replace(target, replacement)
            assertTrue("变异未生效：`$target` → `$replacement`", mutated != bodyCall)
            val detected = ComposeTextContractAudit.auditBodyTextCall(mutated)
            assertTrue(
                "变异 `$target` → `$replacement` 未被判出违规（审计存在盲区）",
                detected.isNotEmpty(),
            )
        }

        // Material 默认排版值一律不得出现在正文调用块里。
        val withMaterialDefault = bodyCall.replace("style = TextStyle(", "style = MaterialTheme.typography.bodyLarge.copy(")
        assertTrue(
            "引用 MaterialTheme.typography 必须被判出违规",
            ComposeTextContractAudit.auditBodyTextCall(withMaterialDefault).isNotEmpty(),
        )
    }

    /**
     * 正文路径不得出现滚动容器（NFR-A4：滚动会掩盖分页错误）；同时用同目录的其它页面做对照，
     * 证明审计确实能识别真实的滚动容器，而不是「任何文件都判合规」。
     */
    @Test
    fun readerBodyNeverUsesAScrollContainer() {
        val audit = ComposeTextContractAudit
        val readerScreen = audit.readerScreenSource()
        val violations = audit.auditNoScrollContainer(readerScreen, "ReaderScreen.kt")
        assertEquals("正文路径不得出现滚动容器，实际违规：$violations", emptyList<String>(), violations)

        // 对照 1：真实存在滚动容器的页面必须被判出（证明审计非空检查）
        val settingsScreen = audit.source("src/main/java/com/novelreader/android/ui/SettingsScreen.kt")
        assertTrue(
            "设置页使用 verticalScroll，审计必须判出（否则审计为空检查）",
            audit.auditNoScrollContainer(settingsScreen, "SettingsScreen.kt").isNotEmpty(),
        )

        // 对照 2：变异源码（给正文加滚动容器）必须被判出
        val mutated = readerScreen + "\nprivate fun Mutated() = Text(modifier = Modifier.verticalScroll(rememberScrollState()))\n"
        assertTrue(
            "给正文加滚动容器后必须被判出违规",
            audit.auditNoScrollContainer(mutated, "ReaderScreen.kt(mutated)").isNotEmpty(),
        )
    }

    /** 测量侧字距必须来自共享契约常量（0），变异成非零必须被判出。 */
    @Test
    fun measurerSideLetterSpacingComesFromTheSharedContract() {
        val source = ComposeTextContractAudit.measurerSource()
        val violations = ComposeTextContractAudit.auditMeasurerLetterSpacing(source)
        assertEquals("测量侧字距必须取共享契约常量，实际违规：$violations", emptyList<String>(), violations)
        assertTrue(
            "测量器必须显式设置 0 字距（与渲染端 letterSpacing = 0.sp 同口径）",
            ComposeTextContractAudit.stripComments(source).contains("TextLayoutContract.LETTER_SPACING_EM"),
        )

        val mutated = source.replace(
            "letterSpacing = TextLayoutContract.LETTER_SPACING_EM",
            "letterSpacing = 0.5f",
        )
        assertTrue("变异未生效", mutated != source)
        assertTrue(
            "测量侧字距被改成非零后必须被判出违规",
            ComposeTextContractAudit.auditMeasurerLetterSpacing(mutated).isNotEmpty(),
        )
    }

    /** 契约常量本身：字距/段距/缩进恒为 0，字重 400，非法换算因子必须被拒绝。 */
    @Test
    fun contractPinsZeroSpacingAndRejectsIllegalPixelScales() {
        assertEquals(0f, TextLayoutContract.LETTER_SPACING_SP, 0f)
        assertEquals(0f, TextLayoutContract.LETTER_SPACING_EM, 0f)
        assertEquals(TypographySettings.PARAGRAPH_SPACING_SP, TextLayoutContract.PARAGRAPH_SPACING_SP, 0f)
        assertEquals(TypographySettings.FIRST_LINE_INDENT_SP, TextLayoutContract.FIRST_LINE_INDENT_SP, 0f)
        assertEquals(400, TextLayoutContract.BODY_FONT_WEIGHT)
        assertEquals(
            TypographySettings.DEFAULT_FONT_WEIGHT,
            TextLayoutContract.BODY_FONT_WEIGHT,
        )

        assertTrue(TextLayoutContract.isValidScale(2.625f))
        assertFalse(TextLayoutContract.isValidScale(0f))
        assertFalse(TextLayoutContract.isValidScale(-1f))
        assertFalse(TextLayoutContract.isValidScale(Float.NaN))
        assertFalse(TextLayoutContract.isValidScale(Float.POSITIVE_INFINITY))

        // 非法换算因子不得产生 Infinity/NaN 行高（那会让渲染端 lineHeight 变成非法值）
        assertEquals(0f, TextLayoutContract.lineHeightSp(27f, 0f), 0f)
        assertEquals(0f, TextLayoutContract.lineHeightSp(27f, Float.NaN), 0f)

        // 字号与行高口径一致：像素 → sp → 像素
        assertTrue(TextLayoutContract.roundTripsFontSize(49.5f, 2.75f))
        assertTrue(TextLayoutContract.roundTripsLineHeight(31.5f, 2.75f))
        assertTrue(TextLayoutContract.roundTripsLineHeight(21.5f, 3.5f))
    }

    private companion object {
        /** 页末偏移的断言用非零窗口起点，确保「页末是绝对偏移」而不是「相对窗口的字符数」。 */
        const val WINDOW_START = 3L
    }
}
