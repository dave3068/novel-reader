package com.novelreader.android.source

import com.novelreader.android.reader.HiddenBooksStore

/**
 * [HiddenBooksStore] 的 Android 实现：复用 SAF 层同一份键值存储（`AndroidSafKeyValue` → `SharedPreferences`）。
 *
 * 存储格式：单键，值形如 `v1\u001f<identity1>\u001f<identity2>…`。
 * 版本前缀不匹配即视为无记录（宁可少隐藏，也不采用半截记录）；
 * 空片段一律丢弃，避免脏值把空标识也当成一本书。
 *
 * ⚠️ 只存标识字符串，**不写文件系统**：本项目硬约束是绝不删除/移动/修改用户原始文件。
 */
class HiddenBooksPrefsStore(private val kv: SafKeyValueStore) : HiddenBooksStore {

    override fun load(): Set<String> {
        val raw = kv.get(KEY) ?: return emptySet()
        val parts = raw.split(SEPARATOR)
        if (parts.firstOrNull() != VERSION) return emptySet()
        return parts.drop(1).filter { it.isNotBlank() }.toSet()
    }

    override fun save(identities: Set<String>) {
        val values = listOf(VERSION) + identities.filter { it.isNotBlank() }
        kv.put(KEY, values.joinToString(SEPARATOR))
    }

    private companion object {
        const val KEY = "hidden_books"
        const val VERSION = "v1"
        const val SEPARATOR = "\u001f"
    }
}
