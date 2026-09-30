package com.novelreader.android.core

/**
 * 64 位 id / 偏移的**跨边界字符串传输**工具。
 *
 * 背景：书籍 id、内容索引版本、字节偏移都是 64 位（头文件 §4）。一旦经由 JSON 数字或
 * JavaScript `Number` 传输，超过 2^53 的整数会被静默截断（`9007199254740993` →
 * `9007199254740992`），因而本层只提供字符串形态的编解码：**不得**用 `Double` 承载 id。
 *
 * 本对象不含业务判定，只做数值 ↔ 文本的精确往返。
 */
object NrWireIds {

    /** 可以精确表示的最大整数（JS/JSON 数字的安全整数上界）。 */
    const val JSON_SAFE_INTEGER: Long = 9007199254740991L

    /** 64 位 id → 十进制字符串（精确，无精度损失）。 */
    fun toWireString(id: Long): String = id.toString()

    /** 十进制字符串 → 64 位 id；空串 / 非十进制整数按 `NR_ERR_INVALID_ARGUMENT` 拒绝。 */
    fun fromWireString(text: String): Long {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) {
            throw NrException(NrErrors.NR_ERR_INVALID_ARGUMENT, message = "id 文本为空")
        }
        return trimmed.toLongOrNull()
            ?: throw NrException(NrErrors.NR_ERR_INVALID_ARGUMENT, message = "id 文本不是 64 位整数：$trimmed")
    }

    /** 该 id 是否会超出 JSON 数字的安全整数范围（超出时必须走字符串）。 */
    fun exceedsJsonSafeInteger(id: Long): Boolean = id > JSON_SAFE_INTEGER || id < -JSON_SAFE_INTEGER
}
