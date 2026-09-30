package com.novelreader.android.reader

import java.io.File

/**
 * 渲染侧（Compose）与测量侧的排版契约**静态审计器**（A-T5 / 检查项 C5）。
 *
 * 为什么用静态审计而不是行为断言：宿主机 JVM 单测里既没有 Compose 运行时，也没有 Android 排版栈
 * （`android.graphics` 只有空实现），无法真正渲染一页并量行数。渲染侧口径 —— 显式字号 / 字距 /
 * 行高、不继承 Material 默认排版值、正文不用滚动容器 —— 只能靠源码审计固定下来。
 *
 * 审计器刻意做成**可被变异输入检验**的纯函数：调用方（[PaginationContractTest]）除了在真实源码上
 * 断言「零违规」，还必须在内置变异源码上断言「至少 1 条违规」，证明这些检查不是空检查。
 */
internal object ComposeTextContractAudit {

    private const val READER_SCREEN_RELATIVE =
        "src/main/java/com/novelreader/android/ui/ReaderScreen.kt"
    private const val MEASURER_RELATIVE =
        "src/main/java/com/novelreader/android/reader/AndroidTextMeasurer.kt"

    /** 正文 `Text(...)` 调用里的锚点（唯一标识正文的那一个调用）。 */
    private const val BODY_ANCHOR = "ReaderTestTags.READER_BODY"

    /** 正文区域禁止出现的滚动容器（NFR-A4：滚动会掩盖分页错误）。 */
    private val SCROLL_TOKENS = listOf(
        "verticalScroll",
        "horizontalScroll",
        "rememberScrollState",
        "LazyColumn",
        "LazyRow",
        "LazyVerticalGrid",
        "scrollable(",
    )

    // ------------------------------------------------------------------ 源码定位

    /** 渲染侧源码（`ui/ReaderScreen.kt`）文本；找不到即失败，绝不静默跳过。 */
    fun readerScreenSource(): String = readSource(READER_SCREEN_RELATIVE)

    /** 生产测量器源码（`reader/AndroidTextMeasurer.kt`）文本。 */
    fun measurerSource(): String = readSource(MEASURER_RELATIVE)

    /**
     * 任意相对路径的源码文本。
     *
     * 用于**对照**：同一目录下的其它页面（设置页 / 书架页）确实使用了滚动容器，说明
     * [auditNoScrollContainer] 不是空检查 —— 它只在正文路径上判定为合规。
     */
    fun source(relativePath: String): String = readSource(relativePath)

    /**
     * 从单测工作目录向上查找 `android/app/` 下的相对路径。
     *
     * Gradle 的 `Test` 任务默认工作目录是模块目录（`android/app`），但这里不假设调用方式：
     * 逐级向上探测，找不到就抛异常（把试探过的路径写进消息，便于定位）。
     */
    private fun readSource(relative: String): String {
        val searched = ArrayList<String>()
        var dir: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (dir != null && searched.size < 24) {
            val candidate = File(dir, relative)
            searched.add(candidate.path)
            if (candidate.isFile) {
                return candidate.readText(Charsets.UTF_8)
            }
            dir = dir.parentFile
        }
        throw IllegalStateException(
            "找不到源码 `$relative`（工作目录 ${System.getProperty("user.dir")}）；已尝试：$searched",
        )
    }

    // ------------------------------------------------------------------ 注释剥离与调用块提取

    /** 剥离 Kotlin 注释（`//` 与可嵌套 `/* */`）：注释里写「不使用 verticalScroll」不是代码。 */
    fun stripComments(source: String): String {
        val out = StringBuilder()
        var index = 0
        while (index < source.length) {
            val current = source[index]
            val next = if (index + 1 < source.length) source[index + 1] else ' '
            if (current == '/' && next == '/') {
                while (index < source.length && source[index] != '\n') {
                    index += 1
                }
                continue
            }
            if (current == '/' && next == '*') {
                var depth = 1
                index += 2
                while (index < source.length && depth > 0) {
                    if (source[index] == '/' && index + 1 < source.length && source[index + 1] == '*') {
                        depth += 1
                        index += 2
                    } else if (source[index] == '*' && index + 1 < source.length && source[index + 1] == '/') {
                        depth -= 1
                        index += 2
                    } else {
                        index += 1
                    }
                }
                continue
            }
            out.append(current)
            index += 1
        }
        return out.toString()
    }

    /**
     * 取出包含 [anchor] 的 `Text(...)` 调用块（去掉注释后，按圆括号/花括号配平）。
     *
     * 用「调用块」而不是整文件正则：断言作用在正文那一个 `Text` 上，不会被顶栏/状态行的 `Text`
     * 或 KDoc 里的字面量掩盖。
     */
    fun bodyTextCall(source: String): String {
        val code = stripComments(source)
        val marker = "Text("
        var from = 0
        while (true) {
            val start = code.indexOf(marker, from)
            require(start >= 0) { "源码里找不到包含 `$BODY_ANCHOR` 的 `Text(...)` 调用" }
            from = start + marker.length
            val block = balanced(code, start + marker.length - 1)
            if (block != null && block.contains(BODY_ANCHOR)) {
                return block
            }
        }
    }

    /** 从 [openIndex]（必须是 `(` 或 `{`）取出配平到对应闭合符的子串。 */
    private fun balanced(source: String, openIndex: Int): String? {
        var depth = 0
        var index = openIndex
        while (index < source.length) {
            when (source[index]) {
                '(', '{' -> depth += 1
                ')', '}' -> {
                    depth -= 1
                    if (depth == 0) {
                        return source.substring(openIndex, index + 1)
                    }
                }
            }
            index += 1
        }
        return null
    }

    /** 取具名实参的值（到顶层逗号或换行为止，保留 `{}` / `()` 嵌套）。无该实参返回 `null`。 */
    fun argumentOf(callBlock: String, name: String): String? {
        val match = Regex("\\b" + Regex.escape(name) + "\\s*=").find(callBlock) ?: return null
        val out = StringBuilder()
        var depth = 0
        var index = match.range.last + 1
        while (index < callBlock.length) {
            val current = callBlock[index]
            when (current) {
                '(', '{', '[' -> depth += 1
                ')', '}', ']' -> depth -= 1
                '\n' -> if (depth == 0) return out.toString().trim()
                ',' -> if (depth == 0) return out.toString().trim()
            }
            out.append(current)
            index += 1
        }
        return out.toString().trim()
    }

    // ------------------------------------------------------------------ 审计规则

    /**
     * 审计正文 `Text(...)` 调用块，返回违规列表（空 = 合规）。
     *
     * 规则（AR3 / 子规格 design.md §排版）：
     * 1. `letterSpacing` 必须显式且为 0（不得继承 Material `bodyLarge` 的 0.5sp）；
     * 2. `lineHeight` 必须来自测量回传的 `page.lineHeightPx` 并按密度换算回 sp；
     * 3. `fontSize` 必须来自用户设置（sp，与测量侧 `sp × density` 同一像素）；
     * 4. 字重 / 字体族显式指定，逐项与测量侧同口径；整块不得出现 `MaterialTheme.typography`；
     * 5. 溢出必须是 `TextOverflow.Clip`（省略号会改变展示文本，掩盖真实页边界）。
     */
    fun auditBodyTextCall(callBlock: String): List<String> {
        val violations = ArrayList<String>()

        val letterSpacing = argumentOf(callBlock, "letterSpacing")
        if (letterSpacing == null) {
            violations.add("正文未显式设置 letterSpacing：会继承 Material 默认字距（bodyLarge = 0.5sp），与测量侧的 0 不一致")
        } else if (!isZeroSpacing(letterSpacing)) {
            violations.add("正文 letterSpacing 不是 0：`$letterSpacing`（测量侧为 ${TextLayoutContract.LETTER_SPACING_EM}f，两侧不一致会裁切末行）")
        }

        val lineHeight = argumentOf(callBlock, "lineHeight")
        if (lineHeight == null) {
            violations.add("正文未设置 lineHeight：会回落到 Material 默认行高，与测量行高不一致")
        } else {
            if (!lineHeight.contains("page.lineHeightPx")) {
                violations.add("正文 lineHeight 未使用测量回传的实际行高 `page.lineHeightPx`：`$lineHeight`")
            }
            if (!lineHeight.contains("toSp")) {
                violations.add("正文 lineHeight 未按当前密度换算回 sp：`$lineHeight`")
            }
        }

        val fontSize = argumentOf(callBlock, "fontSize")
        if (fontSize == null) {
            violations.add("正文未设置 fontSize")
        } else {
            if (!fontSize.contains("fontSizeSp")) {
                violations.add("正文 fontSize 未来自用户设置（`settings.fontSizeSp`）：`$fontSize`")
            }
            if (!fontSize.endsWith(".sp")) {
                violations.add("正文字号必须以 sp 渲染（与测量侧 `sp × density` 同像素）：`$fontSize`")
            }
        }

        val fontWeight = argumentOf(callBlock, "fontWeight")
        if (fontWeight == null) {
            violations.add("正文未显式设置 fontWeight：会继承 Material 默认字重，与测量侧的 ${TextLayoutContract.BODY_FONT_WEIGHT} 不一致")
        } else if (!fontWeight.contains("FontWeight.Normal")) {
            violations.add("正文字重必须显式 Normal（与测量侧 ${TextLayoutContract.BODY_FONT_WEIGHT} 同口径）：`$fontWeight`")
        }

        val fontFamily = argumentOf(callBlock, "fontFamily")
        if (fontFamily == null) {
            violations.add("正文未显式设置 fontFamily：会继承 Material 默认字体族，与测量侧字体不一致")
        } else if (!fontFamily.contains("FontFamily.SansSerif")) {
            violations.add("正文字体族必须显式 SansSerif（与测量侧 sans-serif 同口径）：`$fontFamily`")
        }

        val overflow = argumentOf(callBlock, "overflow")
        if (overflow == null || !overflow.contains("TextOverflow.Clip")) {
            violations.add("正文 overflow 必须是 TextOverflow.Clip（省略号会改变展示文本）：`$overflow`")
        }

        if (callBlock.contains("MaterialTheme.typography")) {
            violations.add("正文用到了 `MaterialTheme.typography` 默认排版值：字号/字距/行高必须逐项显式指定")
        }
        return violations
    }

    /** 正文区域禁止滚动容器：返回违规列表（空 = 合规）。注释已剥离。 */
    fun auditNoScrollContainer(source: String, label: String): List<String> {
        val code = stripComments(source)
        return SCROLL_TOKENS.filter { code.contains(it) }.map {
            "$label 的正文路径出现滚动容器 `$it`：分页错误会被滚动掩盖（NFR-A4）"
        }
    }

    /** 生产测量器必须从共享契约取 0 字距（不得写死其它值、也不得缺省）。 */
    fun auditMeasurerLetterSpacing(source: String): List<String> {
        val code = stripComments(source)
        val violations = ArrayList<String>()
        val assignments = Regex("letterSpacing\\s*=\\s*([^\\n]+)")
            .findAll(code)
            .map { it.groupValues[1].trim() }
            .toList()
        if (assignments.isEmpty()) {
            violations.add("生产测量器未显式设置 letterSpacing：平台默认字距会与渲染端不一致")
        }
        assignments.filterNot { it.startsWith("TextLayoutContract.LETTER_SPACING_EM") }.forEach {
            violations.add("生产测量器 letterSpacing 未取共享契约常量：`$it`")
        }
        return violations
    }

    /** 是否为 0（sp / em 单位）字距值：接受 `0.sp` / `0f.sp` / 共享契约常量引用。 */
    fun isZeroSpacing(value: String): Boolean {
        val trimmed = value.trim()
        if (trimmed.startsWith("TextLayoutContract.")) {
            // 契约常量本身由 PaginationContractTest 断言为 0。
            return true
        }
        val number = Regex("^-?\\d+(?:\\.\\d+)?f?").find(trimmed)?.value ?: return false
        val numeric = number.removeSuffix("f").toFloatOrNull() ?: return false
        val unitOk = trimmed.endsWith(".sp") || trimmed.endsWith("f")
        return numeric == 0f && unitOk
    }
}
