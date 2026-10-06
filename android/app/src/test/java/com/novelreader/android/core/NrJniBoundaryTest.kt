package com.novelreader.android.core

import java.lang.reflect.Method
import java.lang.reflect.Modifier
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * JNI 边界自检（JVM 可执行的部分）：
 *
 * 1. [NrCore] 的 `external fun` 声明集合必须**恰好**是头文件冻结的 18 项入口
 *    （名字逐字相同 —— JNI 短名含方法名，改名即 `UnsatisfiedLinkError`）；
 * 2. 关键签名不得退化（句柄/偏移 `long`、缓冲区入口以 `long[]` 归还槽位收尾、
 *    `nr_abi_version` 返回 `int` 等）；
 * 3. 单测注入用的 [NrAbi] 与 18 项入口一一对应（薄封装没有漏掉或多出入口）。
 *
 * 三方（头文件 / C 导出 / Kotlin 声明）的静态比对由
 * `scripts/verify-abi.py` 与 `scripts/verify-android-boundary.py` 承担。
 *
 * 注意：这里用 `Class.forName(name, false, loader)` 取类，**不触发**
 * `System.loadLibrary("nr_jni")` —— 宿主机无法加载 arm64 的 `.so`。
 */
class NrJniBoundaryTest {

    private val entryNames = listOf(
        "nr_abi_version",
        "nr_engine_create",
        "nr_engine_destroy",
        "nr_open_txt",
        "nr_close_book",
        "nr_read_window",
        "nr_submit_page_boundary",
        "nr_get_page_boundary",
        "nr_save_position",
        "nr_get_position",
        "nr_get_chapters",
        "nr_create_bookmark",
        "nr_delete_bookmark",
        "nr_invalidate_layout",
        "nr_rebuild_index",
        "nr_set_encoding",
        // ---- V1.1 分类（加性契约修订）----
        "nr_list_categories",
        "nr_create_category",
        "nr_move_book_to_category",
        "nr_delete_category",
        // ---- V1.1 阅读历史（加性契约修订）----
        "nr_list_history",
        "nr_clear_history_entry",
        "nr_clear_history_for_book",
        // ---- V1.1 自定义配色（加性契约修订）----
        "nr_get_theme_base",
        "nr_set_theme_base",
        "nr_set_theme_enabled",
        "nr_get_theme_token",
        "nr_get_theme_contrast",
        // ---- V1.1 全局背景图（加性契约修订）----
        "nr_get_background",
        "nr_set_background",
        "nr_set_background_scrim",
        "nr_clear_background",
        "nr_category_move_targets",
        "nr_rename_category",
        "nr_move_category",
        "nr_list_books_in_scope",
        "nr_list_book_categories",
        "nr_cancel_request",
        "nr_get_last_error",
        "nr_release_buffer",
    )

    private val seamNames = listOf(
        "abiVersion",
        "engineCreate",
        "engineDestroy",
        "openTxt",
        "closeBook",
        "readWindow",
        "submitPageBoundary",
        "getPageBoundary",
        "savePosition",
        "getPosition",
        "getChapters",
        "createBookmark",
        "deleteBookmark",
        "invalidateLayout",
        "rebuildIndex",
        "setEncoding",
        // ---- V1.1 分类（加性契约修订）----
        "listCategories",
        "createCategory",
        "moveBookToCategory",
        "deleteCategory",
        // ---- V1.1 阅读历史（加性契约修订）----
        "listHistory",
        "clearHistoryEntry",
        "clearHistoryForBook",
        // ---- V1.1 自定义配色（加性契约修订）----
        "getThemeBase",
        "setThemeBase",
        "setThemeEnabled",
        "getThemeToken",
        "getThemeContrast",
        // ---- V1.1 全局背景图（加性契约修订）----
        "getBackground",
        "setBackground",
        "setBackgroundScrim",
        "clearBackground",
        "categoryMoveTargets",
        "renameCategory",
        "moveCategory",
        "listBooksInScope",
        "listBookCategories",
        "cancelRequest",
        "getLastError",
        "releaseBuffer",
    )

    private fun nativeMethods(): List<Method> =
        Class.forName("com.novelreader.android.core.NrCore", false, javaClass.classLoader)
            .declaredMethods
            .filter { Modifier.isNative(it.modifiers) }

    private fun nativeMethod(name: String): Method = nativeMethods().first { it.name == name }

    @Test
    fun nrCoreDeclaresExactlyTheFrozenEntrySet() {
        val natives = nativeMethods()
        assertEquals(
            "external fun 声明数必须为 40（18 项冻结 + nr_set_encoding + V1.1 分类 4 + 历史 3 + 配色 5 + 背景 4 + 作用域书籍 1 + 书籍分类映射 1 + 分类改名/移动 2 + 移动目标 1 项）",
            40,
            natives.size,
        )
        assertEquals(entryNames.toSet(), natives.map { it.name }.toSet())
    }

    @Test
    fun abiVersionReturnsIntWithoutHandle() {
        val abiVersion = nativeMethod("nr_abi_version")
        assertEquals(Int::class.javaPrimitiveType, abiVersion.returnType)
        assertEquals(0, abiVersion.parameterCount)
    }

    @Test
    fun handleAndOffsetParametersStayLong() {
        val create = nativeMethod("nr_engine_create")
        assertEquals(Long::class.javaPrimitiveType, create.returnType)
        assertEquals(3, create.parameterCount)
        assertEquals(Long::class.javaPrimitiveType, create.parameterTypes[0])
        assertEquals(ByteArray::class.java, create.parameterTypes[1])
        assertEquals(NrSourceProvider::class.java, create.parameterTypes[2])

        assertEquals(Long::class.javaPrimitiveType, nativeMethod("nr_engine_destroy").parameterTypes[0])
        assertEquals(Long::class.javaPrimitiveType, nativeMethod("nr_read_window").parameterTypes[2])
        assertEquals(Long::class.javaPrimitiveType, nativeMethod("nr_read_window").parameterTypes[3])
        assertEquals(Long::class.javaPrimitiveType, nativeMethod("nr_rebuild_index").parameterTypes[1])
        assertEquals(Long::class.javaPrimitiveType, nativeMethod("nr_cancel_request").parameterTypes[1])

        val release = nativeMethod("nr_release_buffer")
        assertEquals(Int::class.javaPrimitiveType, release.returnType)
        assertEquals(3, release.parameterCount)
        assertTrue(release.parameterTypes.all { it == Long::class.javaPrimitiveType })
    }

    @Test
    fun bufferEntriesEndWithReleaseSlotsAndReturnNullableBytes() {
        for (name in listOf("nr_read_window", "nr_get_position", "nr_get_chapters", "nr_get_last_error")) {
            val target = nativeMethod(name)
            assertEquals("$name 必须返回 ByteArray?", ByteArray::class.java, target.returnType)
            assertEquals(
                "$name 末位必须是 LongArray 归还槽位",
                LongArray::class.java,
                target.parameterTypes.last(),
            )
        }
    }

    @Test
    fun structuredOutParametersUseCallerOwnedArrays() {
        val submit = nativeMethod("nr_submit_page_boundary")
        assertEquals(LongArray::class.java, submit.parameterTypes[8])
        assertEquals(ByteArray::class.java, submit.parameterTypes[9])
        assertEquals(ByteArray::class.java, submit.parameterTypes[10])

        assertEquals(IntArray::class.java, nativeMethod("nr_save_position").parameterTypes[6])
        assertEquals(IntArray::class.java, nativeMethod("nr_delete_bookmark").parameterTypes[2])
        assertEquals(LongArray::class.java, nativeMethod("nr_invalidate_layout").parameterTypes[2])
    }

    @Test
    fun seamMirrorsNativeEntrySet() {
        val declared = NrAbi::class.java.declaredMethods
            .map { it.name.substringBefore('$') }
            .toSet()
        assertEquals(
            "NrAbi 接口方法数必须为 40（与 external fun 一一对应）",
            40,
            declared.size,
        )
        assertEquals(seamNames.toSet(), declared)
    }
}
