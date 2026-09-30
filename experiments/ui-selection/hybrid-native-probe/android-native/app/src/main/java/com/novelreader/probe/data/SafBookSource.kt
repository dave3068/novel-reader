package com.novelreader.probe.data

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts

/**
 * SAF（Storage Access Framework）文件访问。
 *
 * 属规格允许的 Kotlin 职责（系统集成）：只负责选择文件与**按有限窗口**读取字节，
 * 不实现任何阅读业务（分页、位置、锚点仍由共享库裁定）。
 */
class SafBookSource(
    context: Context,
    onSelected: (Uri?) -> Unit,
) {
    private val resolver: ContentResolver = context.contentResolver
    private val launcher: ActivityResultLauncher<Array<String>> =
        (context as androidx.activity.ComponentActivity).registerForActivityResult(
            ActivityResultContracts.OpenDocument(),
            onSelected,
        )

    /** 拉起系统文件选择器。 */
    fun requestBook() {
        launcher.launch(arrayOf("text/plain", "application/octet-stream"))
    }

    /** 文件大小（未知时返回 -1）。 */
    fun sizeOf(uri: Uri): Long =
        runCatching {
            resolver.openAssetFileDescriptor(uri, "r")?.use { it.length } ?: -1L
        }.getOrDefault(-1L)

    /** 展示用文件名。 */
    fun displayNameOf(uri: Uri): String =
        runCatching {
            resolver.query(uri, null, null, null, null)?.use { cursor ->
                val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (index >= 0 && cursor.moveToFirst()) cursor.getString(index) else null
            }
        }.getOrNull() ?: uri.lastPathSegment ?: uri.toString()

    /** 按字节偏移读取有限窗口。 */
    fun readChunk(uri: Uri, offset: Long, length: Int): ByteArray {
        val input = resolver.openInputStream(uri) ?: return ByteArray(0)
        return input.use { stream ->
            var remaining = offset
            while (remaining > 0) {
                val skipped = runCatching { stream.skip(remaining) }.getOrDefault(0L)
                if (skipped <= 0) break
                remaining -= skipped
            }
            val buffer = ByteArray(length.coerceAtLeast(0))
            val count = stream.read(buffer)
            if (count <= 0) ByteArray(0) else buffer.copyOf(count)
        }
    }
}
