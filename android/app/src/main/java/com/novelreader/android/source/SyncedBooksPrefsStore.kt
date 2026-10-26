package com.novelreader.android.source

import com.novelreader.android.reader.SyncedBookEntry
import com.novelreader.android.reader.SyncedBooksStore
import java.util.Base64

/**
 * 同步书持久化（阶段 0）：复用 SAF 层同一份键值存储。
 *
 * 格式（单键，行分隔）：
 * `v1\n<bookId>\t<base64(displayName)>\t<base64(sourceLocator)>\t<sourceSize>`
 *
 * 只存应用自己的副本路径；不写用户原始文件。解析失败的行一律丢弃，
 * 宁可少显示一本书，也不把半截记录当成可打开来源。
 */
class SyncedBooksPrefsStore(private val kv: SafKeyValueStore) : SyncedBooksStore {

    override fun load(): List<SyncedBookEntry> {
        val raw = kv.get(KEY) ?: return emptyList()
        val lines = raw.split(LINE)
        if (lines.firstOrNull() != VERSION) return emptyList()
        return lines.drop(1).mapNotNull(::parse)
    }

    override fun save(entries: List<SyncedBookEntry>) {
        val lines = mutableListOf(VERSION)
        for (entry in entries.filter { it.bookId > 0 && it.sourceLocator.isNotBlank() }) {
            val name = Base64.getEncoder().encodeToString(entry.displayName.toByteArray(Charsets.UTF_8))
            val path = Base64.getEncoder().encodeToString(entry.sourceLocator.toByteArray(Charsets.UTF_8))
            lines += "${entry.bookId}\t$name\t$path\t${entry.sourceSize}"
        }
        kv.put(KEY, lines.joinToString(LINE))
    }

    private fun parse(line: String): SyncedBookEntry? {
        val fields = line.split(TAB)
        if (fields.size != 4) return null
        val id = fields[0].toLongOrNull()?.takeIf { it > 0 } ?: return null
        val name = runCatching {
            Base64.getDecoder().decode(fields[1]).toString(Charsets.UTF_8)
        }.getOrNull() ?: return null
        val path = runCatching {
            Base64.getDecoder().decode(fields[2]).toString(Charsets.UTF_8)
        }.getOrNull()?.takeIf { it.isNotBlank() } ?: return null
        val size = fields[3].toLongOrNull()?.coerceAtLeast(0) ?: 0
        return SyncedBookEntry(id, name, path, size)
    }

    private companion object {
        const val KEY = "synced_books"
        const val VERSION = "v1"
        const val LINE = "\n"
        const val TAB = "\t"
    }
}
