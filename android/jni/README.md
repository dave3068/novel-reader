# android/jni —— JNI 薄适配层契约（AN1 冻结）

本目录是「Kotlin `external fun nr_*` 声明」与「`include/novel_reader.h` 冻结 18 项 C ABI」
之间的唯一适配层。Kotlin 侧只做装箱/拆箱与错误映射，不复制任何章节、位置、分页、缓存失效
或书签业务（子规格 `android/spec.md` AR2 / design.md）。

- 实现：`nr_jni.c`（C11，无 C++、无 CMake、无业务逻辑）
- 链接：由 `scripts/build-android-libs.sh` 把 Rust 静态库 `libreading_ffi.a`
  以 `--whole-archive` 静态链进来，产出**自包含** `libnr_jni.so`，无需其它 `.so`
- 尺寸冻结：`nr_jni.c` 内 `_Static_assert(sizeof(...))` 与 §4 表格一致
  （Rust 侧字段增删会在 C 编译期失败，强制同步本文件与 Kotlin 解析代码）

---

## §1 命名契约

| 项 | 值 |
|---|---|
| Kotlin 包名（冻结） | `com.novelreader.android.core` |
| Kotlin 承载类型（冻结） | `object NrCore` |
| JNI 符号前缀 | `Java_com_novelreader_android_core_NrCore_` |
| 库名 | `libnr_jni.so`（`System.loadLibrary("nr_jni")`） |

**JNI 名称转义（实测，必须遵守）**：JNI 短名 = `Java_` + 类名（`.`→`_`）+ `_` + 方法名，
其中**方法名里的下划线要转义为 `_1`**。因此 Kotlin 的 `external fun nr_abi_version()`
对应的 C 符号是 `Java_com_novelreader_android_core_NrCore_nr_1abi_1version`，
而**不是** `..._NrCore_nr_abi_version`（后者在 HotSpot 上实测直接
`UnsatisfiedLinkError: no implementation found`）。
`android/jni/nr_jni.c` 的 18 项定义全部使用转义名，`scripts/build-android-libs.sh`
会同时校验「转义名 = 18 项」且「未转义名 = 0 项」，防止回退。

因此 Kotlin 必须放在 `android/app/src/main/java/com/novelreader/android/core/NrCore.kt`，
包名与类名都不可更改，否则 JNI 符号无法解析（`UnsatisfiedLinkError`）。
18 项声明的函数名必须与头文件入口名**逐字相同**（`scripts/verify-abi.py` 逐项校验）。

## §2 18 项 Kotlin `external` 声明（可直接复制）

```kotlin
package com.novelreader.android.core

object NrCore {
    init { System.loadLibrary("nr_jni") }

    external fun nr_abi_version(): Int
    external fun nr_engine_create(maxWindowBytes: Long, dbPath: ByteArray?, provider: NrSourceProvider?): Long
    external fun nr_engine_destroy(engine: Long): Int
    external fun nr_open_txt(engine: Long, kind: Int, totalBytes: Long, identity: ByteArray,
                             displayName: ByteArray?, outResult: ByteArray): Int
    external fun nr_close_book(engine: Long, bookId: Long): Int
    external fun nr_read_window(engine: Long, chapterIndex: Int, offset: Long, length: Long,
                                outWindow: ByteArray, outBuffer: LongArray): ByteArray?
    external fun nr_submit_page_boundary(engine: Long, phase: Int, contentIndexVersion: Long,
                                         layoutId: Long, requestId: Long, totalBytes: Long,
                                         startByte: Long, endByte: Long, layoutFields: LongArray?,
                                         fontFamily: ByteArray?, outResult: ByteArray): Int
    external fun nr_get_page_boundary(engine: Long, layoutId: Long, chapterIndex: Int,
                                      startByte: Long, outResult: ByteArray): Int
    external fun nr_save_position(engine: Long, chapterIndex: Int, bookId: Long, contentUnitId: Long,
                                  originalByteOffset: Long, fingerprint: ByteArray?,
                                  outChanged: IntArray?): Int
    external fun nr_get_position(engine: Long, bookId: Long, outResult: ByteArray,
                                 outBuffer: LongArray): ByteArray?
    external fun nr_get_chapters(engine: Long, bookId: Long, startSequence: Int, maxCount: Int,
                                 outBatch: ByteArray, outBuffer: LongArray): ByteArray?
    external fun nr_create_bookmark(engine: Long, chapterIndex: Int, bookId: Long, contentUnitId: Long,
                                    byteOffset: Long, title: ByteArray?, outBookmarkId: LongArray): Int
    external fun nr_delete_bookmark(engine: Long, bookmarkId: Long, outDeleted: IntArray?): Int
    external fun nr_invalidate_layout(engine: Long, layoutId: Long, outRemoved: LongArray?): Int
    external fun nr_rebuild_index(engine: Long, bookId: Long, requestId: Long, outResult: ByteArray): Int
    external fun nr_cancel_request(engine: Long, requestId: Long): Int
    external fun nr_get_last_error(engine: Long, outInfo: ByteArray, outBuffer: LongArray): ByteArray?
    external fun nr_release_buffer(ptr: Long, len: Long, capacity: Long): Int
}
```

约定（与 C 层逐条对应）：

1. **返回值**：`Int` 一律是 `NR_ERR_*` 错误码（0 = 成功）。**禁止用错误文本判断分支**
   （头文件 §2）；错误文本只能通过 `nr_get_last_error` 取展示用字符串。
   返回 `Long` 的两项：`nr_abi_version`（版本号）、`nr_engine_create`（句柄，**0 = 失败**）。
   返回 `ByteArray?` 的四项（`nr_read_window` / `nr_get_position` / `nr_get_chapters` /
   `nr_get_last_error`）：`null` = 调用失败（用 `nr_get_last_error` 取码），
   **零长度结果是空数组而不是 `null`**。
2. **字符串一律 `ByteArray`（UTF-8 字节）**，不使用 `String`：跨边界与头文件
   「UTF-8 指针 + 长度，不依赖 NUL」一致，且可在 JVM 单测里直接构造。
3. **出参结构体是调用方预分配的 `ByteArray`**（§4 给尺寸与字段偏移）；容量不足时
   C 层返回 `NR_ERR_INVALID_ARGUMENT`，不会越界写。Kotlin 用
   `ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)` 解析。
4. **缓冲区所有权显式**：四个产出缓冲区（正文 / 指纹 / 章节批次字节 / 错误消息）的入口
   把 Rust 分配区的 `(ptr, len, capacity)` 写入 `outBuffer: LongArray(3)`；
   调用方消费字节后**必须**调用 `nr_release_buffer(ptr, len, capacity)` 归还
   （头文件 §5）。语义细节：
   - 每个缓冲区**只允许归还一次**；结构体内嵌的 `(ptr, len, capacity)` 字段与
     `outBuffer` 是同一份值，**不得重复归还**；
   - `(0, 0, 0)` 是安全 no-op（返回 0）；
   - 重复归还或未登记的指针会被拒绝为 `NR_ERR_INVALID_ARGUMENT`（C 层内部登记已交付
     缓冲区，绝不会对同一指针二次释放）；
   - 未提供 `outBuffer` 时 C 层立即归还，不会泄漏（但业务路径必须提供）。
5. **64 位**：所有 id、偏移、长度、缓冲区字段都是 `Long`；禁止 `Int` 截断或浮点转换。
6. `external fun` 不允许带默认参数值（Kotlin 语言限制），可空参数一律显式传 `null`。

## §3 平台数据源回调（SAF）

`nr_engine_create` 的 `provider` 非空时，C 层保存其全局引用并把 Rust 的 `read_window`
回调转发给 Kotlin：

```kotlin
package com.novelreader.android.core

/**
 * 平台窗口内容协议（头文件「平台窗口内容协议」）；实现者只负责 ContentResolver 字节访问，
 * 不做坐标/边界/位置判定（由 Rust 裁定）。
 */
interface NrSourceProvider {
    /**
     * 把 [offset, offset + maxLength) 的字节写入 dst，并回报：
     *   outInfo[0] = out_len（实际写入长度，必须 <= maxLength）
     *   outInfo[1] = out_eof（0/1，1 = 已到达内容末尾）
     * 返回 NR_ERR_* 错误码（0 = 成功）。允许返回 0 长度（例如 offset 已到末尾）。
     * 必须同步返回；不得回调任何 nr_* 入口（会重入同一引擎死锁）。
     */
    fun readWindow(requestId: Long, offset: Long, maxLength: Int, dst: ByteArray, outInfo: IntArray): Int
}
```

C 层行为（`jni_read_window`）：

- 方法名与签名冻结为 `readWindow(JJI[B[I)I`；若实现类缺少该方法，`nr_engine_create`
  直接失败并返回 `0`（日志 `provider 缺少 readWindow...`），不会静默降级。
- Kotlin 抛出的异常会被 `ExceptionDescribe` + `ExceptionClear` 清理，并映射为
  `NR_ERR_IO`（绝不把 Java 异常带进 Rust）。
- `outInfo[0] > maxLength`（试图一次交付整本正文）会被拒绝为 `NR_ERR_INTERNAL`，
  与头文件「单次交付量由 Rust 裁定」一致。

## §4 结构体字节布局（小端，`_Static_assert` 已在 C 层冻结）

`nr_open_result`（56 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 / 8 / 16 | u64 | `book_id` / `total_bytes` / `content_index_version` |
| 24 / 28 / 32 / 36 | u32 | `content_unit_id` / `encoding` / `encoding_confidence` / `encoding_score` |
| 40 / 44 / 48 / 52 | u32 | `needs_encoding_confirmation` / `chapters` / `index_from_cache` / `reserved0` |

`nr_window`（48 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 / 8 / 16 / 24 | u64 | `offset` / `end_byte` / `request_id` / `total_bytes` |
| 32 / 36 / 40 / 44 | u32 | `len` / `dropped_leading` / `trimmed_tail` / `eof` |

`nr_boundary_result`（80 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 / 4 / 8 / 12 | u32 | `status` / `rejection` / `error_code`(i32) / `reserved0` |
| 16 … 72（8 项 u64） | u64 | `layout_id` / `request_id` / `stale_layout_id` / `stale_request_id` / `start_byte` / `end_byte` / `total_bytes` / `page_count` |

`nr_position_result`（80 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 / 4 / 8 / 12 / 16 / 20 | u32 | `struct_size` / `found` / `chapter_index` / `restored` / `fallback_to_chapter_start` / `reserved0` |
| 24 / 32 / 40 / 48 | u64 | `book_id` / `content_unit_id` / `original_byte_offset` / `updated_at` |
| 56 / 64 / 72 | u64 | `nearby_fingerprint` 的 `ptr` / `len` / `capacity`（**归还槽位**） |

`nr_chapter_batch`（40 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 / 4 / 8 / 12 | u32 | `count` / `index_version` / `total_chapters` / `reserved0` |
| 16 / 24 / 32 | u64 | `bytes` 的 `ptr` / `len` / `capacity`（**归还槽位**） |

`nr_rebuild_result`（40 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 / 4 / 8 / 12 | u32 | `struct_size` / `partial` / `chapters` / `index_from_cache` |
| 16 / 24 / 32 | u64 | `scanned_bytes` / `total_bytes` / `content_index_version` |

`nr_error_info`（32 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 | i32 | `code` |
| 4 | u32 | `has_message` |
| 8 / 16 / 24 | u64 | `message` 的 `ptr` / `len` / `capacity`（**归还槽位**） |

章节批次字节（`nr_get_chapters` 的返回数组）布局（头文件 `nr_chapter_batch` 注释）：

```text
[ nr_chapter; count ] 之后紧接全部标题的 UTF-8 字节
nr_chapter = 40 字节：u32 sequence, u32 rule_id, u64 start_byte, u64 end_byte,
                       u32 title_offset, u32 title_len, u32 index_version, u32 reserved0
title_offset 相对批次字节起点（即返回数组下标 0）
```

`layoutFields: LongArray(13)`（`nr_submit_page_boundary` 的 BEGIN_LAYOUT 阶段，
顺序与头文件 `nr_layout_signature` 完全一致）：

| 下标 | 字段 | 下标 | 字段 |
|---|---|---|---|
| 0 | `viewport_width` | 7 | `first_line_indent_milli` |
| 1 | `viewport_height` | 8 | `margin_horizontal_milli` |
| 2 | `direction` | 9 | `margin_vertical_milli` |
| 3 | `font_weight` | 10 | `algorithm_version` |
| 4 | `font_size_milli` | 11 | `reserved0` |
| 5 | `line_spacing_milli` | 12 | `platform_font_fingerprint`（u64） |
| 6 | `paragraph_spacing_milli` | | |

`fontFamily: ByteArray?` 是排版签名里的字体族 UTF-8 字节（BEGIN_LAYOUT 必填）；
`phase = NR_SUBMIT_COMMIT` 时 `layoutFields` / `fontFamily` 可传 `null`。

## §5 构建与校验

```bash
# 交叉编译 Rust 静态库 + 链接自包含 libnr_jni.so（产出 android/app/src/main/jniLibs/<abi>/）
bash scripts/build-android-libs.sh --abi arm64-v8a

# Gradle 会（在 .so 缺失/过期时）自动调用上面的脚本，再打包 APK
cd android && ./gradlew :app:assembleDebug

# 宿主机 JNI 层自检（真实加载 .so 并调用 18 项入口，见 host-smoke/）
bash scripts/build-android-libs.sh --host-smoke
```

禁止事项（子规格「禁止事项」）：不得引入 CMake/`externalNativeBuild`；
不得把 `.so` 手工提交为唯一来源（必须可由脚本重建）；不得省略 `ndkVersion`。
