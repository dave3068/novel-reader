package com.novelreader.android.reader

/**
 * 「阅读时保持屏幕常亮」的路由决策（纯 Kotlin，不依赖 Android 框架，便于 JVM 单测）。
 *
 * # 需求口径（真机反馈：阅读中很容易息屏）
 *
 * 阅读页需要像视频播放器一样**不自动息屏**；离开阅读页（书架 / 设置 / 历史）或退出应用后
 * 必须**恢复系统息屏行为**。实现用窗口标志 `FLAG_KEEP_SCREEN_ON`（无需任何权限）：
 * 本类只做「当前路由下是否应保持常亮」的判定与**变化**分发，宿主
 * [com.novelreader.android.MainActivity] 把判定结果写进窗口标志。
 *
 * # 为什么不是「修改系统息屏时长」
 *
 * 延长系统超时需要 `WRITE_SETTINGS` 特殊授权并改动**全局**设置（进程被杀可能留下延长状态，
 * 且影响其它应用）；而窗口标志的作用域**天然**就是本窗口可见期间 —— 应用退到后台时窗口不可见，
 * 系统自动忽略该标志，无需宿主感知生命周期；离开阅读页时由本类显式清位，立即恢复默认息屏。
 *
 * # 去重
 *
 * UI 状态流在翻页 / 状态文案更新时会发射大量**相同路由**的快照；本类只在「保持与否」真正
 * 变化时回调一次，避免对窗口标志做无谓的重复写入。
 *
 * 静态门禁：`scripts/verify-android-scope.py` 的 `[13]`（宿主须持有「标志对 + 路由观测」，
 * 本类须以 [ReaderRoute.READER] 判定）；JVM 单测见 `ScreenOnControllerTest`。
 */
class ScreenOnController(
    /** 把「是否保持常亮」写到窗口（宿主注入真实窗口；单测注入记录器）。 */
    private val applyFlag: (Boolean) -> Unit,
) {

    /** 最近一次已应用的取值；`null` = 尚未应用过（窗口是新建的，标志必然处于关闭态）。 */
    private var lastApplied: Boolean? = null

    /**
     * 路由变化入口（幂等）。
     *
     * 仅 [ReaderRoute.READER] 保持常亮，其余路由一律恢复系统息屏；首次观测即对齐一次 ——
     * 启动/重建直接落在阅读页时标志会被重新置位（新窗口不会继承旧窗口的标志）。
     */
    fun onRouteChanged(route: ReaderRoute) {
        val keepScreenOn = route == ReaderRoute.READER
        if (keepScreenOn == lastApplied) {
            return
        }
        lastApplied = keepScreenOn
        applyFlag(keepScreenOn)
    }
}
