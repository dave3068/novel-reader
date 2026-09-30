package com.novelreader.android.source

import com.novelreader.android.reader.PageAnchor
import com.novelreader.android.reader.ReaderAnchorStore

/**
 * [ReaderAnchorStore] 的 Android 实现：复用 SAF 层同一份键值存储（`AndroidSafKeyValue` → `SharedPreferences`）。
 *
 * 存储格式：单键单书，值为 `signature#chapter:start;chapter:start;…`。
 * 读取时**校验签名**：排版签名不一致（改了字号/行距/视口）时返回空表，
 * 因为旧签名下的页边界在新排版里不再是边界。
 *
 * 本类只做编解码与读写，不含任何阅读业务判断；锚点数量上限由调用方（ViewModel）保证。
 */
class ReaderAnchorPrefsStore(private val kv: SafKeyValueStore) : ReaderAnchorStore {

    override fun load(bookKey: String, signature: String): List<PageAnchor> {
        val raw = kv.get(keyFor(bookKey)) ?: return emptyList()
        val separator = raw.indexOf(SIGNATURE_SEPARATOR)
        if (separator <= 0) return emptyList()
        if (raw.substring(0, separator) != signature) return emptyList()
        return raw.substring(separator + 1)
            .split(ANCHOR_SEPARATOR)
            .mapNotNull(::decodeAnchor)
    }

    override fun save(bookKey: String, signature: String, anchors: List<PageAnchor>) {
        val body = anchors.joinToString(ANCHOR_SEPARATOR.toString()) {
            "${it.chapterIndex}$FIELD_SEPARATOR${it.startByte}"
        }
        kv.put(keyFor(bookKey), "$signature$SIGNATURE_SEPARATOR$body")
    }

    private fun decodeAnchor(text: String): PageAnchor? {
        val parts = text.split(FIELD_SEPARATOR)
        if (parts.size != 2) return null
        val chapterIndex = parts[0].toIntOrNull() ?: return null
        val startByte = parts[1].toLongOrNull() ?: return null
        if (chapterIndex < 0 || startByte < 0L) return null
        return PageAnchor(chapterIndex, startByte)
    }

    private fun keyFor(bookKey: String): String = "$KEY_PREFIX$bookKey"

    private companion object {
        const val KEY_PREFIX = "reader.anchor.v1."
        const val SIGNATURE_SEPARATOR = '#'
        const val ANCHOR_SEPARATOR = ';'
        const val FIELD_SEPARATOR = ':'
    }
}
