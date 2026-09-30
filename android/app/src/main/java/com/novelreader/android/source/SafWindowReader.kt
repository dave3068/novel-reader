package com.novelreader.android.source

import com.novelreader.android.core.NrErrors
import com.novelreader.android.core.NrSourceProvider

/**
 * 平台窗口读取器：把 Rust 给定的 `[offset, offset + maxLength)` 窗口交给 [SafPlatform]。
 *
 * 职责边界（头文件「平台窗口内容协议」/ `core/NrSourceProvider.kt`）：
 * - **只**做字节访问：不做坐标、UTF-8 边界、章节或位置判定（那些由 Rust 裁定）；
 * - 单次交付量恒 ≤ `min(maxLength, dst.size, SafPolicy.MAX_DELIVERY_BYTES)` ——
 *   即便上游误传更大的 maxLength，也不可能一次交付整本正文（门禁检查项 C11 的行为侧证明）；
 * - 短读用循环补齐（管道 / Provider 短读是常态），到达末尾则以 `out_eof = 1` 如实回报；
 * - 取消命中返回 `NR_ERR_CANCELLED`（取消是**部分成功**，绝不能被当成 INTERNAL）；
 * - 读失败返回 `NR_ERR_IO`（不静默返回 0 字节假成功）。
 */
class SafWindowReader(
    private val platform: SafPlatform,
    private val handle: Long,
    /** 坐标空间总字节数（来自描述符或元数据，均为已知值才会走窗口模式）。 */
    val totalBytes: Long,
    private val cancels: SafCancelRegistry,
    private val maxDeliveryBytes: Int = SafPolicy.MAX_DELIVERY_BYTES,
) : NrSourceProvider {

    /** 最近一次读取是否以 I/O 失败结束（宿主据此判定「权限/来源失效」并转入重新关联）。 */
    var lastIoFailure: String? = null
        private set

    override fun readWindow(
        requestId: Long,
        offset: Long,
        maxLength: Int,
        dst: ByteArray,
        outInfo: IntArray,
    ): Int {
        if (outInfo.size < 2) return NrErrors.NR_ERR_INVALID_ARGUMENT
        outInfo[0] = 0
        outInfo[1] = 0
        if (offset < 0L || maxLength <= 0 || dst.isEmpty()) {
            return NrErrors.NR_ERR_INVALID_ARGUMENT
        }
        val token = cancels.tokenOf(requestId)
        if (token.cancelled) return NrErrors.NR_ERR_CANCELLED

        // 偏移已在坐标空间末尾：合法空窗口（头文件允许 0 长度返回）。
        if (offset >= totalBytes) {
            outInfo[1] = 1
            return NrErrors.NR_ERR_OK
        }

        val limit = minOf(maxLength, dst.size, maxDeliveryBytes)
        var filled = 0
        var reachedEnd = false
        while (filled < limit) {
            if (token.cancelled) {
                outInfo[0] = filled
                return NrErrors.NR_ERR_CANCELLED
            }
            val read = platform.readAt(handle, offset + filled, dst, filled, limit - filled)
            if (read < 0) {
                lastIoFailure = "平台读取失败：offset=${offset + filled}"
                outInfo[0] = filled
                return NrErrors.NR_ERR_IO
            }
            if (read == 0) {
                // 源头提前结束（Provider 未按报告长度供字节）：如实标记 EOF，不当作成功读满。
                reachedEnd = true
                break
            }
            filled += read
        }
        lastIoFailure = null
        outInfo[0] = filled
        outInfo[1] = if (reachedEnd || offset + filled >= totalBytes) 1 else 0
        return NrErrors.NR_ERR_OK
    }
}

/**
 * 一次已打开的窗口会话：实现 [NrSourceProvider]（可直接传给 `NrEngine.create(provider = …)`），
 * 并由自己持有描述符句柄，`close()` 归还。
 *
 * 通过 `record.accessMode == WINDOW` 时使用 `NR_SOURCE_KIND_PLATFORM` 打开书籍；
 * 会话对象**不持有** Activity / Context —— 旋转或退到后台只影响宿主，不影响本对象。
 */
class SafWindowSession internal constructor(
    val record: SafSourceRecord,
    val reader: SafWindowReader,
    private val platform: SafPlatform,
    private val handle: Long,
) : NrSourceProvider, AutoCloseable {

    val totalBytes: Long get() = reader.totalBytes

    var closed: Boolean = false
        private set

    override fun readWindow(
        requestId: Long,
        offset: Long,
        maxLength: Int,
        dst: ByteArray,
        outInfo: IntArray,
    ): Int = reader.readWindow(requestId, offset, maxLength, dst, outInfo)

    /** 归还描述符；幂等（重复关闭是安全空操作）。 */
    override fun close() {
        if (closed) return
        closed = true
        platform.closeDescriptor(handle)
    }
}
