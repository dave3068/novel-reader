package com.novelreader.android.reader

/**
 * 页锚点持久化端口（规格：`docs/novel-reader-v1-development-plan.md:481` ——
 * 「上一页优先使用边界栈或**持久化稀疏锚点**，不能永久保存全书页面」）。
 *
 * 背景：真机实测两处「上一页」错误置灰 ——
 * ① 切章时旧的实现清空了会话内边界栈；② 重开书籍恢复位置后边界栈为空。
 * 本端口使第二种情况也有**确定的**落点：宿主把最近访问过的少量页锚点持久化下来，
 * 下次打开同一本书时播种回边界栈。
 *
 * 契约：
 * - **不抛异常**：持久化失败只影响「重开后能否后退一步」，绝不能影响阅读主链路；
 * - [load] 在无记录或**排版签名不匹配**时返回空表 —— 宁可无锚点，也不把旧签名下的偏移当页边界；
 * - 只存锚点（章序号 + 起始字节），不存正文、不存页码。
 */
interface ReaderAnchorStore {

    /** 读取某本书在给定排版签名下的稀疏锚点（由远及近或乱序皆可，调用方会排序）。 */
    fun load(bookKey: String, signature: String): List<PageAnchor>

    /** 覆盖写入该书的稀疏锚点（实现方须保证有界、幂等）。 */
    fun save(bookKey: String, signature: String, anchors: List<PageAnchor>)
}
