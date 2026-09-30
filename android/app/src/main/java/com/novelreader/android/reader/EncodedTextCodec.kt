package com.novelreader.android.reader

import java.nio.ByteBuffer
import java.nio.CharBuffer
import java.nio.charset.Charset
import java.nio.charset.CodingErrorAction

/**
 * 解码结果：文本 + **每个字符之后的源字节偏移**。
 *
 * `prefixByteEnds[i]` = 前 `i` 个字符在源文件中占用的字节数（`prefixByteEnds[0] == 0`）。
 * 调用方据此把「页末字符下标」精确换算成「原始字节偏移」。
 */
class DecodedText internal constructor(
    val text: String,
    val prefixByteEnds: LongArray,
) {

    /** 前 [charCount] 个字符占用的源字节数（越界按末尾处理）。 */
    fun byteLengthOfPrefix(charCount: Int): Long =
        prefixByteEnds[charCount.coerceIn(0, prefixByteEnds.size - 1)]

    /**
     * 开头连续替换字符（U+FFFD）的个数，上限 [maxSkip]。
     *
     * **为什么会有替换字符**：核心的窗口对齐 `align_to_utf8_boundaries` 是 UTF-8 专属，
     * 在 GB18030/GBK/Big5 文档上会把窗口首字节**切在字符中间** ⇒ 平台解码时得到替换字符
     * （真机表现为「首行出现一个/三个乱码字符」）。
     *
     * 平台自己解码，正好能识别并**跳过**它 —— 跳过之后才是真正的字符边界，
     * 这样提交给核心的页边界不会落在字符内部（真机「首行乱码时无法『上一页』」的根因）。
     */
    fun leadingReplacementCount(maxSkip: Int = MAX_LEADING_SKIP): Int {
        var count = 0
        while (count < maxSkip && count < text.length && text[count] == REPLACEMENT_CHARACTER) {
            count += 1
        }
        return count
    }

    /** 跳过头部的 [skip] 个字符，返回**重新基准化**的解码结果（文本与字节映射一致右移）。 */
    fun afterSkipping(skip: Int): DecodedText {
        if (skip <= 0) return this
        val end = skip.coerceAtMost(text.length)
        val base = prefixByteEnds[end]
        val rebased = LongArray(text.length - end + 1) { index -> prefixByteEnds[end + index] - base }
        return DecodedText(text.substring(end), rebased)
    }

    companion object {
        /** 单窗口头部最多跳过的替换字符数（核心对齐最多切断 3 字节）。 */
        const val MAX_LEADING_SKIP = 3
        const val REPLACEMENT_CHARACTER = '\uFFFD'
    }
}

/**
 * 按**源字节**精确解码，并给出「字符 → 源字节」映射。
 *
 * # 为什么必须精确
 *
 * 「先解码、再重新编码求长度」是不可靠的：变长编码存在多种合法表达
 * （GB18030 的 2 字节与 4 字节形式、Big5 的异体映射、不可映射字符的替换），
 * 重新编码得到的字节数可能**不同于源文件**。而核心的坐标空间是原始文件字节，
 * 页边界按同一份字节判定 ⇒ 长度算错即被判为 `NR_ERR_UTF8_BOUNDARY`
 * （真机实测：「偏移不在合法字符边界」，并在翻页时反复出现）。
 *
 * # 做法
 *
 * 只**向前消费源字节**：用增量解码器逐字符推进，记录每个字符消费的源字节数。
 * 因而不依赖任何「往返一致」假设，UTF-8 / UTF-16 / GB18030 / GBK / Big5 一视同仁。
 *
 * 不变量：`prefixByteEnds.last() <= raw.size`（尾部若为非法/未完成序列则不完整计入；
 * 核心的窗口裁剪 `nr_window.trimmed_tail` 通常已把这类尾部去掉）。
 */
object EncodedTextCodec {

    /**
     * 单次解码输出缓冲 = **1 个字符**。
     *
     * 必须是 1：否则一次调用会产出多个字符，而调用方只能拿到一个「输入已消费字节数」，
     * 无法为每个字符分别记录终点（曾用 4 字符缓冲 → 映射错位，被 11 项测试抓出）。
     * 代理对由两次调用自然完成：第一次消费全部 4 字节并产出高代理，第二次产出低代理、
     * 不再消费字节 ⇒ 两者共享同一字节终点（它们之间本就不是码点边界）。
     */
    private const val OUTPUT_CAPACITY = 1

    /** 防御上限：连续无进展的轮次超过此值即停止，绝不无限循环。 */
    private const val MAX_STALL = 8

    fun decode(raw: ByteArray, charset: Charset): DecodedText {
        if (raw.isEmpty()) return DecodedText("", longArrayOf(0L))

        val decoder = charset.newDecoder()
            .onMalformedInput(CodingErrorAction.REPLACE)
            .onUnmappableCharacter(CodingErrorAction.REPLACE)
        val input = ByteBuffer.wrap(raw)
        val builder = StringBuilder(raw.size / 2 + 8)
        val ends = ArrayList<Long>(raw.size / 2 + 8)
        ends.add(0L)
        val out = CharBuffer.allocate(OUTPUT_CAPACITY)

        var consumed = 0L
        var stall = 0
        while (input.hasRemaining()) {
            out.clear()
            val before = input.position()
            val result = decoder.decode(input, out, true)
            val advanced = input.position() - before
            if (advanced > 0) consumed += advanced

            out.flip()
            // flip() 之后 remaining 即本轮产出的字符数（Java 的 length() 在 Kotlin 里是属性）
            val produced = out.remaining()
            if (produced > 0) {
                builder.append(out[0])
                ends.add(consumed)
                stall = 0
                continue
            }

            stall += 1
            when {
                result.isError -> {
                    // 非法序列：以替换字符消费 1 字节，保证前进（不静默丢内容）。
                    input.position(before + 1)
                    consumed = (before + 1).toLong()
                    builder.append('\uFFFD')
                    ends.add(consumed)
                    stall = 0
                }

                result.isUnderflow -> break   // 输入已不足以构成一个字符
                stall > MAX_STALL -> break
            }
        }
        return DecodedText(builder.toString(), ends.toLongArray())
    }
}
