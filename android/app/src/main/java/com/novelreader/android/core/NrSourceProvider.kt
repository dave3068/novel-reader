package com.novelreader.android.core

/**
 * 平台窗口内容协议（头文件「平台窗口内容协议」/ README §3）。
 *
 * 实现者（Android SAF 数据源）只负责把 `[offset, offset + maxLength)` 的字节写进 `dst`，
 * **不做**坐标、UTF-8 边界、章节或位置判定 —— 读取区间与裁剪一律由 Rust 核心裁定
 * （子规格范围外：「Kotlin 复制 Rust 的章节、位置、分页、缓存失效或书签业务」）。
 *
 * 约束：
 * - 必须**同步返回**；
 * - 不得回调任何 `nr_*` 入口（同一引擎会重入死锁，头文件 §6）；
 * - 实现类缺少 `readWindow(JJI[B[I)I` 会让 `nr_engine_create` 直接返回 0（不静默降级）；
 * - 抛出的异常会被 C 层清理并映射为 `NR_ERR_IO`，绝不把 Java 异常带进 Rust；
 * - `outInfo[0] > maxLength`（试图一次交付整本正文）会被拒绝为 `NR_ERR_INTERNAL`：
 *   单次交付量由 Rust 裁定，平台无法通过自述长度扩大交付量。
 */
interface NrSourceProvider {

    /**
     * 把 `[offset, offset + maxLength)` 的字节写入 [dst]，并回报：
     * - `outInfo[0] = out_len`（实际写入长度，必须 ≤ `maxLength`）
     * - `outInfo[1] = out_eof`（0/1，1 = 已到达内容末尾）
     *
     * 返回 `NR_ERR_*` 错误码（0 = 成功）；允许返回 0 长度（例如 offset 已到末尾）。
     */
    fun readWindow(requestId: Long, offset: Long, maxLength: Int, dst: ByteArray, outInfo: IntArray): Int
}
