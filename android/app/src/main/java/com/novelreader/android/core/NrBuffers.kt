package com.novelreader.android.core

/**
 * 归还 Rust 分配缓冲区时使用的函数类型。
 *
 * 生产实现是 [JNI]（透传 `NrCore.nr_release_buffer`）。抽成函数类型的唯一理由是让
 * 「恰好归还一次 / 重复归还 / 空凭据 no-op / native 返回失败码」这些语义能在 JVM 单测里
 * 被真实断言（宿主机无法加载 arm64 的 `libnr_jni.so`）。
 */
fun interface NrReleaser {

    /** 归还 `(ptr, len, capacity)`，返回 `NR_ERR_*`。 */
    fun release(ptr: Long, len: Long, capacity: Long): Int

    companion object {
        /** 生产实现：调用 JNI 入口（lambda 体只在真正归还时才触发 [NrCore] 初始化）。 */
        val JNI: NrReleaser = NrReleaser { ptr, len, capacity -> NrCore.nr_release_buffer(ptr, len, capacity) }
    }
}

/**
 * Rust 分配缓冲区的归还凭据（头文件 §5 / README §4）。
 *
 * 语义（与 C 层逐条对应）：
 * - **每个缓冲区只允许归还一次**：第一次成功后 `released = true`；之后再次调用不触碰
 *   native，直接返回 `NR_ERR_INVALID_ARGUMENT`（与 C 层拒绝重复归还的码一致，且比 C 层
 *   更早拦住「对已释放指针二次释放」的窗口）；
 * - `(0, 0, 0)` 是**安全 no-op**（返回 0；零长度结果不登记缓冲区）；
 * - native 返回非 0 时保持未归还状态，可由调用方诊断 / 决定是否重试；
 * - 结构体内嵌的 `(ptr, len, capacity)` 槽位与 `outBuffer` 是**同一份值**，因此归还只允许
 *   经本对象进行（由 [requireSameSlots] 断言两者一致，不一致按内部错误拒绝）。
 */
class NrBufferHandle private constructor(
    val ptr: Long,
    val len: Long,
    val capacity: Long,
) {

    /** 是否已成功归还（或已因空凭据而 no-op）。 */
    var released: Boolean = false
        private set

    /** 空凭据：`(0,0,0)`。 */
    val isEmpty: Boolean
        get() = ptr == 0L && len == 0L && capacity == 0L

    /**
     * 归还缓冲区；返回 `NR_ERR_*`（不抛异常）。
     *
     * 重复归还返回 `NR_ERR_INVALID_ARGUMENT` 且不再调用 native。
     */
    fun release(releaser: NrReleaser = NrReleaser.JNI): Int {
        if (released) {
            return NrErrors.NR_ERR_INVALID_ARGUMENT
        }
        if (isEmpty) {
            released = true
            return NrErrors.NR_ERR_OK
        }
        val code = releaser.release(ptr, len, capacity)
        if (code == NrErrors.NR_ERR_OK) {
            released = true
        }
        return code
    }

    /** 归还缓冲区；失败时抛 [NrException]。 */
    fun releaseOrThrow(releaser: NrReleaser = NrReleaser.JNI) {
        val code = release(releaser)
        if (code != NrErrors.NR_ERR_OK) {
            throw NrException(code, message = "缓冲区归还失败（ptr=$ptr len=$len capacity=$capacity）")
        }
    }

    override fun toString(): String =
        "NrBufferHandle(ptr=$ptr, len=$len, capacity=$capacity, released=$released)"

    companion object {

        /** 空凭据常量（可安全归还）。 */
        val EMPTY: NrBufferHandle = NrBufferHandle(0L, 0L, 0L)

        /** 从 `outBuffer: LongArray` 构造凭据；槽位不足按 `NR_ERR_INVALID_ARGUMENT` 拒绝。 */
        fun fromSlots(slots: LongArray): NrBufferHandle {
            if (slots.size < NrAbiConstants.NR_BUFFER_SLOTS) {
                throw NrException(
                    NrErrors.NR_ERR_INVALID_ARGUMENT,
                    message = "outBuffer 必须是 LongArray(${NrAbiConstants.NR_BUFFER_SLOTS})，实际 ${slots.size}",
                )
            }
            return NrBufferHandle(slots[0], slots[1], slots[2])
        }

        /**
         * 断言结构体内嵌槽位与 `outBuffer` 是同一份值。
         *
         * 结构体与 `outBuffer` 都描述 Rust 分配的同一块内存；两者不一致说明跨边界布局已错位，
         * 此时归还任一指针都可能释放错误内存，因此按内部错误拒绝。
         */
        fun requireSameSlots(handle: NrBufferHandle, structPtr: Long, structLen: Long, structCapacity: Long) {
            if (handle.ptr != structPtr || handle.len != structLen || handle.capacity != structCapacity) {
                throw NrException(
                    NrErrors.NR_ERR_INTERNAL,
                    message = "结构体内嵌缓冲区槽位与 outBuffer 不一致："
                        + "struct=($structPtr,$structLen,$structCapacity) out=(${handle.ptr},${handle.len},${handle.capacity})",
                )
            }
        }

        /**
         * 断言归还凭据的长度与结果自述长度一致。
         *
         * `nr_window.len` / `nr_chapter_batch.bytes.len` 与凭据的 `len` 是同一份值；不一致说明
         * 出参槽位没写成功（例如调用方给的 `outBuffer` 过短）或跨边界布局已错位。
         */
        fun requireLenMatches(handle: NrBufferHandle, expectedLen: Long) {
            if (handle.len != expectedLen) {
                throw NrException(
                    NrErrors.NR_ERR_INTERNAL,
                    message = "缓冲区长度自述不一致：凭据 len=${handle.len}，结果 len=$expectedLen",
                )
            }
        }
    }
}
