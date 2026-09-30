package com.novelreader.android.reader

/**
 * 「已移除」书籍来源的持久化端口（书架「移除」用）。
 *
 * # 语义（用户裁定）
 *
 * > 仅移除书架 + 清空本书本地索引（保留阅读位置），**绝不删除原文件**。
 *
 * 因此本端口只保存**来源标识集合**（SAF URI 或文件路径），
 * 不涉及任何文件系统写入，也不改动核心书库。
 * 移除后若用户**重新导入**同一文件，应把它从集合中取出（否则会一直不显示）。
 */
interface HiddenBooksStore {

    /** 读取已移除的来源标识（无记录返回空集）。 */
    fun load(): Set<String>

    /** 覆盖写入已移除的来源标识。 */
    fun save(identities: Set<String>)
}
