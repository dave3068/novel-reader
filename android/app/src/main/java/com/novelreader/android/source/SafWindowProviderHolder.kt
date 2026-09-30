package com.novelreader.android.source

import com.novelreader.android.core.NrErrors
import com.novelreader.android.core.NrSourceProvider

/**
 * 稳定转发的 [NrSourceProvider]。
 *
 * # 为什么需要它
 *
 * 引擎在 `nr_engine_create` 时即固定 provider（`NrEngine.create(provider = …)`），
 * 而 SAF 窗口会话是**每次打开书籍**才产生（[SafBookSource.openWindowSession]）。
 * 若把会话对象直接交给引擎，就必须每打开一本书重建一次引擎 —— 那会丢掉进度与索引缓存。
 *
 * 因此引擎持有本转发器，宿主在打开书籍时再 [bind] 当前会话。
 *
 * # 语义约束（与 `NrSourceProvider` 头文件一致）
 *
 * - 未绑定会话时返回 [NrErrors.NR_ERR_IO]，并在 `outInfo` 中声明「0 字节 / 已到末尾」——
 *   **绝不伪造字节**，也不静默降级；
 * - [readWindow] 同步返回，且不得回调任何 `nr_*` 入口；
 * - 本类不持有描述符：会话的打开与归还由 [SafHost] 负责。
 */
class SafWindowProviderHolder : NrSourceProvider {

    @Volatile
    private var session: SafWindowSession? = null

    /**
     * 绑定当前会话；传 `null` 表示当前没有可用的平台窗口来源。
     *
     * 绑定新会话时会**归还**上一个会话持有的描述符（幂等；同一对象重复绑定不动作）。
     */
    fun bind(next: SafWindowSession?) {
        val previous = session
        if (previous === next) return
        session = next
        if (previous != null) previous.close()
    }

    /** 当前绑定的会话（诊断用；不转让所有权）。 */
    fun boundSession(): SafWindowSession? = session

    override fun readWindow(
        requestId: Long,
        offset: Long,
        maxLength: Int,
        dst: ByteArray,
        outInfo: IntArray,
    ): Int {
        val current = session
        if (current == null) {
            outInfo[0] = 0
            outInfo[1] = 1
            return NrErrors.NR_ERR_IO
        }
        return current.readWindow(requestId, offset, maxLength, dst, outInfo)
    }

    companion object {

        /**
         * 进程级单例。
         *
         * 引擎在创建时固定 provider，而 Activity 会因旋转 / 重建被销毁重建 ——
         * 若每次重建都换一个转发器，已打开书籍的平台窗口读取会在旋转后失效。
         * 故转发器必须**跨 Activity 生命周期存活**。
         */
        val shared: SafWindowProviderHolder = SafWindowProviderHolder()
    }
}
