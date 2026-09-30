package com.novelreader.android.reader

import java.nio.charset.Charset
import java.nio.charset.StandardCharsets

/**
 * 内容编码：核心 `NR_ENCODING_*` 码 → 平台解码器 + 展示名。
 *
 * 规格 CR2（`reading-core/spec.md:38`）：核心「SHALL 支持 UTF-8、UTF-8 BOM、UTF-16 LE/BE、
 * GB18030、GBK/CP936、Big5 的识别结果和用户手动覆盖，并记录检测器版本与可信度」。
 *
 * **真机缺陷背景**：核心早已实现检测（`reading-source/src/encoding.rs`）并经 `nr_open_result.encoding`
 * 回报，但 Android 侧**完全没用上** —— 正文窗口与章节标题一律 `Charsets.UTF_8` 硬编码解码，
 * 切片层（[PageLayout]）也按 UTF-8 计算字节长度。后果有二：
 * ① 非 UTF-8 文档显示乱码；② 字符↔字节映射与核心不一致，页边界被判为
 * `NR_ERR_UTF8_BOUNDARY`（「偏移不在合法字符边界」）。
 *
 * 本文件把「核心给出的编码码」翻译成平台侧的 [Charset]，解码与字节计数**统一走这里**。
 */
enum class ContentEncoding(
    val abiCode: Long,
    val displayName: String,
    val charset: Charset,
) {
    UTF8(1L, "UTF-8", StandardCharsets.UTF_8),
    UTF8_BOM(2L, "UTF-8 BOM", StandardCharsets.UTF_8),
    UTF16_LE(3L, "UTF-16LE", StandardCharsets.UTF_16LE),
    UTF16_BE(4L, "UTF-16BE", StandardCharsets.UTF_16BE),
    GB18030(5L, "GB18030", Charset.forName("GB18030")),
    GBK(6L, "GBK", Charset.forName("GBK")),
    BIG5(7L, "Big5", Charset.forName("Big5")),
    ;

    companion object {

        /** 未识别时的兜底：UTF-8（核心在无法判定时亦按 UTF-8 解释）。 */
        val DEFAULT = UTF8

        /** 核心编码码 → 平台编码；未知码退回 [DEFAULT]，绝不抛异常。 */
        fun ofAbiCode(code: Long): ContentEncoding = byAbiCode(code) ?: DEFAULT

        fun byAbiCode(code: Long): ContentEncoding? = entries.firstOrNull { it.abiCode == code }

        /** 供日志/诊断的稳定回显（与 `nr_open_result.encoding` 一致）。 */
        fun describe(code: Long): String {
            val encoding = byAbiCode(code)
            return if (encoding == null) "未知($code)" else encoding.displayName
        }
    }
}
