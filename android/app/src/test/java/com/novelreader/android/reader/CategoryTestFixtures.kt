package com.novelreader.android.reader

import com.novelreader.android.core.NrBookCategoryEntry
import com.novelreader.android.core.NrCategoryEntry

/**
 * 分类相关测试夹具（`CategorySidebarTest` / `CategoryDrillTest` 共用；避免两份编码器）。
 */

/** 造 `nr_category` 批次：**所有表头在前、全部名称在后**（不是逐行交错）。 */
fun categoryBatch(vararg rows: Triple<Long, Long, Pair<Int, String>>): ByteArray {
    val names = rows.map { it.third.second.toByteArray(Charsets.UTF_8) }
    val headerSize = NrCategoryEntry.STRUCT_SIZE * rows.size
    val total = headerSize + names.sumOf { it.size }
    val buf = java.nio.ByteBuffer.allocate(total).order(java.nio.ByteOrder.LITTLE_ENDIAN)
    var cursor = headerSize
    rows.forEachIndexed { index, row ->
        val base = index * NrCategoryEntry.STRUCT_SIZE
        buf.putLong(base, row.first)
        buf.putLong(base + 8, row.second)
        buf.putLong(base + 16, 1L)
        buf.putLong(base + 24, 3L)
        buf.putInt(base + 32, row.third.first)
        buf.putInt(base + 36, cursor)
        buf.putInt(base + 40, names[index].size)
        cursor += names[index].size
    }
    val out = buf.array()
    var at = headerSize
    names.forEach { it.copyInto(out, at); at += it.size }
    return out
}

/** 造 `nr_book_categories` 载荷（每行 16 字节：bookId, categoryId；`-1` = 未分类）。 */
fun bookCategoryBatch(vararg pairs: Pair<Long, Long>): ByteArray {
    val buf = java.nio.ByteBuffer
        .allocate(pairs.size * NrBookCategoryEntry.STRUCT_SIZE)
        .order(java.nio.ByteOrder.LITTLE_ENDIAN)
    pairs.forEachIndexed { index, (bookId, categoryId) ->
        val base = index * NrBookCategoryEntry.STRUCT_SIZE
        buf.putLong(base, bookId)
        buf.putLong(base + 8, categoryId)
    }
    return buf.array()
}
