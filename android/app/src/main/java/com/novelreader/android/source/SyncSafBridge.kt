package com.novelreader.android.source

import android.content.Context
import android.net.Uri
import java.io.File
import java.io.IOException

/**
 * 阶段 0 同步包与 SAF 目录之间的搬运入口（Kotlin 薄封装）。
 *
 * 真正的 ContentResolver 访问在 [SyncSafPlatform]（Java 平台层）：
 * 范围门禁把整文件流视为「整本正文通道」，同步包复制不应绕过该边界，而应由平台层承担。
 * 本类只做暂存目录管理与委托，不解析 state.json、不判断书籍/进度语义。
 */
object SyncSafBridge {

    /** 导出包在用户所选目录下的固定子目录名（与桌面端一致）。 */
    const val BUNDLE_DIR_NAME = SyncSafPlatform.BUNDLE_DIR_NAME

    /** 清空并重建暂存目录。 */
    fun resetStaging(dir: File) {
        if (dir.exists() && !dir.deleteRecursively()) {
            throw IOException("无法清理同步暂存目录：${dir.absolutePath}")
        }
        if (!dir.mkdirs() && !dir.isDirectory) {
            throw IOException("无法创建同步暂存目录：${dir.absolutePath}")
        }
    }

    fun copyLocalToTree(context: Context, treeUri: Uri, source: File) {
        SyncSafPlatform.copyLocalToTree(context, treeUri, source)
    }

    fun copyTreeToLocal(context: Context, treeUri: Uri, target: File) {
        SyncSafPlatform.copyTreeToLocal(context, treeUri, target)
    }

    fun looksLikeBundle(context: Context, treeUri: Uri): Boolean =
        SyncSafPlatform.looksLikeBundle(context, treeUri)
}
