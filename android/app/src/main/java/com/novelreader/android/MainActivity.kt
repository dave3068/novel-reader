package com.novelreader.android

import android.content.Context
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.ViewModelProvider
import com.novelreader.android.core.NrAbiConstants
import com.novelreader.android.reader.HostOpenTarget
import com.novelreader.android.reader.HostSourceResolver
import com.novelreader.android.reader.ReaderViewModel
import com.novelreader.android.source.AndroidSafKeyValue
import com.novelreader.android.source.HiddenBooksPrefsStore
import com.novelreader.android.source.ReaderAnchorPrefsStore
import com.novelreader.android.source.ReaderSettingsPrefsStore
import com.novelreader.android.source.SafHost
import com.novelreader.android.source.SafWindowProviderHolder
import com.novelreader.android.ui.NovelReaderApp
import java.io.File

/**
 * 应用唯一 Activity：挂载三页面外壳 [NovelReaderApp] 并完成宿主接线。
 *
 * # 本文件承担的三件事（此前无人认领的「宿主接线」缺口）
 *
 * 1. **挂载应用外壳**：`setContent { NovelReaderApp(...) }`。此前本 Activity 只渲染 AN1 的
 *    构建标记页（白屏 + 居中标题），装机后既无书架也无任何可交互控件；
 * 2. **注册系统文件选择器**：`ActivityResultContracts.OpenDocument` → [SafHost]；
 *    持久读权限由 `SafBookSource.intake` 内部取得（`takePersistableReadPermission`），本层不重复申请；
 * 3. **给引擎接上平台窗口来源**：把进程级 [SafWindowProviderHolder.shared] 传给
 *    `productionFactory`，使 SAF 可定位来源能按窗口读取；文件源与私有缓存副本走既有入口。
 *
 * 构建标记（A-T7 的真机核对项）由 [NovelReaderApp] 在书架页以角标形式显示，未丢失。
 *
 * targetSdk 36 强制 edge-to-edge：这里调用 `enableEdgeToEdge()`，各页面自行处理自身 insets。
 */
class MainActivity : ComponentActivity() {

    private lateinit var safHost: SafHost

    /**
     * 稀疏锚点持久化（宿主侧）。
     *
     * 复用 SAF 层同一份 KV：使重开书籍恢复位置后「上一页」仍有确定落点
     * （规格：「上一页优先使用边界栈或持久化稀疏锚点」）。
     */
    // ⚠️ 必须延迟初始化：Activity 的**属性初始化器在 attachBaseContext() 之前**执行，
    // 那时 ContextWrapper.mBase 仍为 null，访问 applicationContext 会抛 NPE 导致启动闪退
    // （真机实测：点图标即退出）。by lazy 的首次访问发生在 onCreate 内，届时上下文已就绪。
    private val anchorStore by lazy { ReaderAnchorPrefsStore(AndroidSafKeyValue(applicationContext)) }

    /** 排版设置持久化（宿主侧）：使字号 / 行距 / 边距 / 夜间模式在重启后仍生效。 */
    private val settingsStore by lazy { ReaderSettingsPrefsStore(AndroidSafKeyValue(applicationContext)) }

    /** 书架「移除」的持久化集合（只存来源标识，绝不触碰用户文件）。 */
    private val hiddenBooksStore by lazy { HiddenBooksPrefsStore(AndroidSafKeyValue(applicationContext)) }

    /**
     * 把 SAF 宿主适配成 ViewModel 需要的 [HostSourceResolver]（书架再次打开 SAF 来源时使用）。
     *
     * 只做类型适配：会话重建 / 缓存复用等判断全部留在 [SafHost]。
     */
    private val hostSourceResolver = HostSourceResolver { uri ->
        val target = safHost.resolveExisting(uri)
        HostOpenTarget(
            kind = target.kind,
            identity = target.identity,
            totalBytes = target.totalBytes,
            displayName = target.displayName,
        )
    }

    /** 阅读 ViewModel：Activity 级作用域（旋转 / 重建不丢状态），引擎带落盘路径与平台来源转发器。 */
    private val readerViewModel: ReaderViewModel by lazy {
        ViewModelProvider(
            this,
            ReaderViewModel.productionFactory(
                dbPath = readerDbPath(this),
                provider = SafWindowProviderHolder.shared,
                hostResolver = hostSourceResolver,
                anchorStore = anchorStore,
                settingsStore = settingsStore,
                bookCatalog = safHost,
                hiddenBooks = hiddenBooksStore,
            ),
        )[ReaderViewModel::class.java]
    }

    /**
     * 系统文件选择器：结果交给 [SafHost] 做 intake 与通道选择，
     * 再按返回的 kind 调用对应打开入口（平台窗口 / 文件源）。
     */
    private val importLauncher =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            if (uri == null) return@registerForActivityResult
            // 重新导入同一文件 ⇒ 把来源从「已移除」集合取出（移除 → 重导入 → 恢复显示）。
            readerViewModel.restoreToShelf(uri.toString())
            safHost.open(
                uri = uri.toString(),
                // MIME 不在这里读取：`SafPolicy` 会从文档元数据推导（平台桥接在 Java 侧内部完成），
                // 宿主直接触碰 ContentResolver 会被 `[3]` 整本正文通道门禁正确拦下。
                reportedMimeType = null,
                onOpened = { target ->
                    if (target.kind == NrAbiConstants.NR_SOURCE_KIND_PLATFORM) {
                        readerViewModel.openPlatformBook(
                            sourceIdentity = target.identity,
                            totalBytes = target.totalBytes,
                            displayName = target.displayName,
                        )
                    } else {
                        readerViewModel.openBook(target.identity, target.displayName)
                    }
                },
                onFailure = readerViewModel::reportHostFailure,
            )
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        safHost = SafHost(applicationContext)
        setContent {
            NovelReaderApp(
                onImportRequest = { importLauncher.launch(IMPORT_MIME_TYPES) },
                buildMarker = BuildConfig.BUILD_MARKER,
                viewModel = readerViewModel,
            )
        }
    }

    override fun onStop() {
        // 与桌面端「退出即保存」同口径（crates/desktop-app/src/ui.rs 的 save_current(..., "退出")）：
        // 进入后台前落盘当前位置，否则「读到章中退出再入」会退回上一次保存点。
        readerViewModel.onHostPaused()
        super.onStop()
    }

    override fun onDestroy() {
        // 只关闭 SAF 执行器：已打开书籍的窗口会话须跨 Activity 重建存活（见 SafHost 文档）。
        safHost.close()
        super.onDestroy()
    }

    companion object {

        /**
         * 选择器过滤。
         *
         * 加入 `application/octet-stream` 是因为部分文档提供者不为 `.txt` 标注文本 MIME；
         * 是否可作为 TXT 导入仍由 `SafPolicy.validate` 按 MIME + 扩展名裁定，非 TXT 会被明确拒绝并提示。
         */
        private val IMPORT_MIME_TYPES = arrayOf("text/plain", "text/*", "application/octet-stream")

        /** 引擎的 SQLite 落盘路径（应用私有目录）；不传则引擎用内存库、进度不可持久化。 */
        private fun readerDbPath(context: Context): ByteArray {
            val dir = File(context.filesDir, "state").apply { mkdirs() }
            return File(dir, "reader.db").absolutePath.toByteArray(Charsets.UTF_8)
        }
    }
}
