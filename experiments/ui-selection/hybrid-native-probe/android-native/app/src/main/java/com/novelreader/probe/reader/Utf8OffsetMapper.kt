package com.novelreader.probe.reader

/** Pure Kotlin UTF-8 boundary mapping; no Android dependency for JVM tests. */
object Utf8OffsetMapper {
    fun byteOffsetForCharIndex(text: String, charIndex: Int): Long {
        val safe = charIndex.coerceIn(0, text.length)
        return text.substring(0, safe).toByteArray(Charsets.UTF_8).size.toLong()
    }

    /** Returns the greatest UTF-16 index that is a complete UTF-8 code-point boundary. */
    fun charIndexForByteOffset(text: String, byteOffset: Long): Int {
        if (byteOffset <= 0L) return 0
        val bytes = text.toByteArray(Charsets.UTF_8)
        val wanted = byteOffset.coerceAtMost(bytes.size.toLong()).toInt()
        var chars = 0
        var bytesSeen = 0
        while (chars < text.length) {
            val next = chars + Character.charCount(text.codePointAt(chars))
            val nextBytes = text.substring(chars, next).toByteArray(Charsets.UTF_8).size
            if (bytesSeen + nextBytes > wanted) break
            bytesSeen += nextBytes
            chars = next
        }
        return chars
    }

    fun isLegalBoundary(text: String, byteOffset: Long): Boolean =
        byteOffsetForCharIndex(text, charIndexForByteOffset(text, byteOffset)) == byteOffset
}
