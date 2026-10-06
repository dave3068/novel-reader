/*
 * nr_jni.c — Android JNI 薄适配层（AN1 交付物）
 *
 * 职责：把 Kotlin 侧 `com.novelreader.android.core.NrCore` 的 `external fun nr_*` 声明
 *       一一映射到 `include/novel_reader.h` 冻结的 18 项 C ABI 入口。
 *
 * 设计约束（与子规格 android/spec.md AR2、design.md 一致）：
 *   1. **零业务逻辑**：本文件只做参数装箱/拆箱、错误码透传与资源释放，不做章节、位置、
 *      分页、缓存失效或书签判定；所有语义由 Rust `reading-core` 裁定。
 *   2. **请求参数用基本类型**：结构体入参（engine_config / open_request / page_boundary_submission /
 *      position / bookmark_input）在 C 侧组装，Kotlin 只传字段值，避免跨语言指针与对齐歧义。
 *   3. **结果结构体用调用方预分配的 ByteArray**：C 侧 `memcpy` 整个 `#[repr(C)]` 结构体
 *      （小端、无内部填充，尺寸由下方 `_Static_assert` 冻结），Kotlin 用
 *      `ByteBuffer.order(LITTLE_ENDIAN)` 顺序解析。字段偏移与尺寸见 `android/jni/README.md`。
 *   4. **缓冲区所有权显式**：返回正文/标题/指纹/消息的入口把 Rust 分配的缓冲区复制进
 *      Java 数组，并把 `(ptr, len, capacity)` 写入调用方的 `LongArray(3)`；调用方随后必须
 *      调用 `nr_release_buffer(ptr, len, capacity)` 归还 —— 与头文件 §5 的所有权契约逐字对应。
 *      若调用方未提供 `LongArray(3)`，本层立即归还（绝不泄露），但业务路径不允许这样用。
 *   5. **平台窗口回调**：引擎创建时若传入 provider 对象，本层保存其全局引用，并把
 *      `read_window` 回调转发给 Kotlin 的 `NrSourceProvider.readWindow(...)`；
 *      回调在调用线程同步执行（Rust 侧有引擎互斥锁保证串行）。
 *   6. **64 位精确**：所有 id/偏移/长度都走 `jlong`/`uint64_t`，不经过 double 或 32 位。
 *
 * 说明：本文件同时可在宿主机（x86_64 Linux）编译，用于 `android/jni/host-smoke` 的
 *       JNI 层自检；`__ANDROID__` 之外的日志走 stderr。
 */

#include <jni.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "novel_reader.h"

#if defined(__ANDROID__)
#include <android/log.h>
#define NR_LOG(...) __android_log_print(ANDROID_LOG_WARN, "nr_jni", __VA_ARGS__)
#else
#include <stdio.h>
#define NR_LOG(...)                       \
    do {                                  \
        fputs("[nr_jni] ", stderr);       \
        fprintf(stderr, __VA_ARGS__);     \
        fputc('\n', stderr);              \
    } while (0)
#endif

/* JNI 符号名前缀：Kotlin 包名 + 类名。
 * 变更此宏必须同步 Kotlin 声明位置（android/jni/README.md §1）。
 *
 * ⚠ JNI 名称转义（已实测）：JNI 短名 = "Java_" + 类名（'.'→'_'）+ "_" + 方法名，
 * 且**方法名中的下划线必须转义为 `_1`**。因此 Kotlin 的
 * `external fun nr_abi_version()` 对应的 C 符号是
 * `Java_com_novelreader_android_core_NrCore_nr_1abi_1version`（不是 `..._nr_abi_version`），
 * 用 HotSpot 验证时未转义的符号会报 UnsatisfiedLinkError。下面 18 项定义全部使用转义名。 */
#define NR_JNI(name) Java_com_novelreader_android_core_NrCore_##name

/* 平台窗口回调的 Kotlin 侧方法名与签名（android/jni/README.md §3）。
 * readWindow(long requestId, long offset, int maxLength, byte[] dst, int[] outInfo): int
 *   → J=long, I=int, [B=byte[], [I=int[] ⇒ (JJI[B[I)I */
#define NR_JNI_PROVIDER_METHOD "readWindow"
#define NR_JNI_PROVIDER_SIG "(JJI[B[I)I"

/* 结果结构体尺寸冻结：Kotlin 解析表与本层 memcpy 依据同一份布局；任何 Rust 侧
 * 字段增删都会在编译期失败，强制同步更新 android/jni/README.md 与 Kotlin 解析代码。 */
_Static_assert(sizeof(nr_open_result) == 56, "nr_open_result 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_window) == 48, "nr_window 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_boundary_result) == 80, "nr_boundary_result 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_position_result) == 80, "nr_position_result 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_chapter_batch) == 40, "nr_chapter_batch 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_category) == 48, "nr_category 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_category_batch) == 32, "nr_category_batch 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_history) == 72, "nr_history 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_history_batch) == 32, "nr_history_batch 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_theme_base) == 56, "nr_theme_base 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_theme_contrast_result) == 24, "nr_theme_contrast_result 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_background) == 48, "nr_background 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_background_batch) == 32, "nr_background_batch 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_book_category) == 16, "nr_book_category 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_id_list) == 32, "nr_id_list 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_category_choice) == 40, "nr_category_choice 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_category_choice_batch) == 32, "nr_category_choice_batch 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_book_category_batch) == 32, "nr_book_category_batch 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_rebuild_result) == 40, "nr_rebuild_result 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_error_info) == 32, "nr_error_info 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_layout_signature) == 72, "nr_layout_signature 布局变化，请同步 Kotlin 解析");
_Static_assert(sizeof(nr_buffer) == 24, "nr_buffer 布局变化，请同步 Kotlin 解析");
_Static_assert(NR_ABI_VERSION == 1u, "ABI 版本变化：必须整链同步（头文件/Rust/Kotlin/JNI）");

/* ============================ 工具函数 ============================ */

/* 把调用方预分配的结果 ByteArray 当作出参结构体：容量不足报 NR_ERR_INVALID_ARGUMENT。 */
static int32_t jni_write_out(JNIEnv *env, jbyteArray out, const void *src, size_t size) {
    if (out == NULL || src == NULL) {
        return NR_ERR_INVALID_ARGUMENT;
    }
    if ((size_t)(*env)->GetArrayLength(env, out) < size) {
        NR_LOG("出参 ByteArray 容量不足：需要 %zu 字节", size);
        return NR_ERR_INVALID_ARGUMENT;
    }
    (*env)->SetByteArrayRegion(env, out, 0, (jsize)size, (const jbyte *)src);
    return (*env)->ExceptionCheck(env) ? NR_ERR_INTERNAL : NR_ERR_OK;
}

/* 读取 jlongArray/ jintArray 的出参槽位（长度不足返回 0，由调用方决定是否失败）。 */
static int32_t jni_write_long_out(JNIEnv *env, jlongArray out, jsize index, jlong value) {
    if (out == NULL || (*env)->GetArrayLength(env, out) <= index) {
        return NR_ERR_INVALID_ARGUMENT;
    }
    (*env)->SetLongArrayRegion(env, out, index, 1, &value);
    return (*env)->ExceptionCheck(env) ? NR_ERR_INTERNAL : NR_ERR_OK;
}

static int32_t jni_write_int_out(JNIEnv *env, jintArray out, jsize index, jint value) {
    if (out == NULL || (*env)->GetArrayLength(env, out) <= index) {
        return NR_ERR_INVALID_ARGUMENT;
    }
    (*env)->SetIntArrayRegion(env, out, index, 1, &value);
    return (*env)->ExceptionCheck(env) ? NR_ERR_INTERNAL : NR_ERR_OK;
}

/* 读取 jlongArray 的 13 项排版字段（BEGIN_LAYOUT 用）。 */
static int32_t jni_read_layout_fields(JNIEnv *env, jlongArray fields, nr_layout_signature *sig,
                                      const uint8_t *font_family, size_t font_family_len) {
    if (fields == NULL || (*env)->GetArrayLength(env, fields) < 13) {
        NR_LOG("排版签名需要 13 项 LongArray");
        return NR_ERR_INVALID_ARGUMENT;
    }
    jlong raw[13];
    (*env)->GetLongArrayRegion(env, fields, 0, 13, raw);
    if ((*env)->ExceptionCheck(env)) {
        return NR_ERR_INTERNAL;
    }
    memset(sig, 0, sizeof(*sig));
    sig->viewport_width = (uint32_t)raw[0];
    sig->viewport_height = (uint32_t)raw[1];
    sig->direction = (uint32_t)raw[2];
    sig->font_weight = (uint32_t)raw[3];
    sig->font_size_milli = (uint32_t)raw[4];
    sig->line_spacing_milli = (uint32_t)raw[5];
    sig->paragraph_spacing_milli = (uint32_t)raw[6];
    sig->first_line_indent_milli = (uint32_t)raw[7];
    sig->margin_horizontal_milli = (uint32_t)raw[8];
    sig->margin_vertical_milli = (uint32_t)raw[9];
    sig->algorithm_version = (uint32_t)raw[10];
    sig->reserved0 = (uint32_t)raw[11];
    sig->platform_font_fingerprint = (uint64_t)raw[12];
    sig->font_family_utf8 = font_family;
    sig->font_family_len = font_family_len;
    return NR_ERR_OK;
}

/* ============================ 已交付缓冲区登记表 ============================
 * C ABI 的「重复归还是安全 no-op」依赖 Rust 归还时把调用方结构体置零；跨 JNI 时
 * Kotlin 侧持有的是 (ptr, len, capacity) 三个值，归还后不会被零化，直接二次调用会
 * 变成对同一指针的二次释放（崩溃）。因此本层登记所有已交付的缓冲区：
 *   - 命中登记：按登记的权威结构体归还，归还成功后移除登记；
 *   - 未命中且三字段全 0：安全 no-op（返回 0），与头文件 §5 一致；
 *   - 未命中且 ptr 非 0：判定为重复/伪造归还，返回 NR_ERR_INVALID_ARGUMENT，
 *     且**不把可能已释放的指针交给 Rust**。
 */
typedef struct nr_jni_outstanding {
    nr_buffer buffer;
    struct nr_jni_outstanding *next;
} nr_jni_outstanding;

static nr_jni_outstanding *g_outstanding = NULL;
static pthread_mutex_t g_outstanding_lock = PTHREAD_MUTEX_INITIALIZER;

static void jni_outstanding_add(const nr_buffer *buffer) {
    if (buffer->ptr == NULL) {
        /* 空缓冲区无需归还，也不登记（否则会与 (0,0,0) 的安全 no-op 分支冲突）。 */
        return;
    }
    nr_jni_outstanding *entry = (nr_jni_outstanding *)malloc(sizeof(*entry));
    if (entry == NULL) {
        NR_LOG("缓冲区登记失败（内存不足）：该次交付将不可归还");
        return;
    }
    entry->buffer = *buffer;
    pthread_mutex_lock(&g_outstanding_lock);
    entry->next = g_outstanding;
    g_outstanding = entry;
    pthread_mutex_unlock(&g_outstanding_lock);
}

/* 查找（不移除）；找到返回 1 并把权威结构体写入 out。 */
static int jni_outstanding_find(size_t ptr, nr_buffer *out) {
    int found = 0;
    pthread_mutex_lock(&g_outstanding_lock);
    for (nr_jni_outstanding *entry = g_outstanding; entry != NULL; entry = entry->next) {
        if ((size_t)entry->buffer.ptr == ptr) {
            *out = entry->buffer;
            found = 1;
            break;
        }
    }
    pthread_mutex_unlock(&g_outstanding_lock);
    return found;
}

static void jni_outstanding_remove(size_t ptr) {
    pthread_mutex_lock(&g_outstanding_lock);
    nr_jni_outstanding **cursor = &g_outstanding;
    while (*cursor != NULL) {
        nr_jni_outstanding *entry = *cursor;
        if ((size_t)entry->buffer.ptr == ptr) {
            *cursor = entry->next;
            free(entry);
            break;
        }
        cursor = &entry->next;
    }
    pthread_mutex_unlock(&g_outstanding_lock);
}

/* 把 Rust 分配的缓冲区复制进 Java 数组，并把 (ptr,len,capacity) 交给调用方显式归还。
 * 返回：Java 数组（调用方持有）；失败返回 NULL（错误码经 nr_get_last_error 查询）。
 * out_handle 为 NULL/过短时立即归还，保证本层不泄露。 */
static jbyteArray jni_take_buffer(JNIEnv *env, nr_buffer *buf, jlongArray out_handle) {
    if (buf->len > (size_t)INT32_MAX) {
        NR_LOG("缓冲区过大，拒绝跨边界交付：len=%zu", buf->len);
        nr_release_buffer(buf);
        return NULL;
    }
    jbyteArray result = (*env)->NewByteArray(env, (jsize)buf->len);
    if (result == NULL) {
        nr_release_buffer(buf);
        if ((*env)->ExceptionCheck(env)) {
            (*env)->ExceptionClear(env);
        }
        return NULL;
    }
    if (buf->len > 0 && buf->ptr != NULL) {
        (*env)->SetByteArrayRegion(env, result, 0, (jsize)buf->len, (const jbyte *)buf->ptr);
    }
    jlong handle[3] = {(jlong)(uintptr_t)buf->ptr, (jlong)buf->len, (jlong)buf->capacity};
    if (out_handle != NULL &&
        jni_write_long_out(env, out_handle, 0, handle[0]) == NR_ERR_OK &&
        jni_write_long_out(env, out_handle, 1, handle[1]) == NR_ERR_OK &&
        jni_write_long_out(env, out_handle, 2, handle[2]) == NR_ERR_OK) {
        /* 归还槽位写成功后才登记：此后必须由调用方调用 nr_release_buffer 归还。 */
        jni_outstanding_add(buf);
    } else {
        /* 无归还槽位：立即归还，绝不把 Rust 内存的所有权悬空。 */
        nr_release_buffer(buf);
    }
    return result;
}

/* ByteArray -> UTF-8 指针 + 长度（借用，不复制内容所有权）。 */
static const uint8_t *jni_borrow_bytes(JNIEnv *env, jbyteArray array, jsize *out_len,
                                       jbyte **out_pinned) {
    if (array == NULL) {
        if (out_len != NULL) {
            *out_len = 0;
        }
        return NULL;
    }
    jsize len = (*env)->GetArrayLength(env, array);
    jbyte *pinned = len > 0 ? (*env)->GetByteArrayElements(env, array, NULL) : NULL;
    if (out_len != NULL) {
        *out_len = len;
    }
    if (out_pinned != NULL) {
        *out_pinned = pinned;
    }
    return (const uint8_t *)pinned;
}

static void jni_release_bytes(JNIEnv *env, jbyteArray array, jbyte *pinned) {
    if (pinned != NULL) {
        (*env)->ReleaseByteArrayElements(env, array, pinned, JNI_ABORT);
    }
}

/* ============================ 平台数据源回调 ============================ */

typedef struct nr_jni_provider {
    JavaVM *vm;
    jobject ref;              /* 平台 provider 对象的全局引用 */
    jmethodID read_window;    /* NrSourceProvider.readWindow(JJI[BI)I */
    uint64_t engine;          /* 绑定的引擎句柄 */
    struct nr_jni_provider *next;
} nr_jni_provider;

static nr_jni_provider *g_providers = NULL;
static pthread_mutex_t g_providers_lock = PTHREAD_MUTEX_INITIALIZER;

static void jni_provider_attach(nr_jni_provider *entry) {
    pthread_mutex_lock(&g_providers_lock);
    entry->next = g_providers;
    g_providers = entry;
    pthread_mutex_unlock(&g_providers_lock);
}

/* 解绑并释放：返回被解绑的条目数（0 表示该引擎没有 provider）。 */
static int jni_provider_detach(JNIEnv *env, uint64_t engine) {
    int removed = 0;
    pthread_mutex_lock(&g_providers_lock);
    nr_jni_provider **cursor = &g_providers;
    while (*cursor != NULL) {
        nr_jni_provider *entry = *cursor;
        if (entry->engine == engine) {
            *cursor = entry->next;
            pthread_mutex_unlock(&g_providers_lock);
            if (env != NULL && entry->ref != NULL) {
                (*env)->DeleteGlobalRef(env, entry->ref);
            }
            free(entry);
            removed++;
            pthread_mutex_lock(&g_providers_lock);
            cursor = &g_providers;
            continue;
        }
        cursor = &entry->next;
    }
    pthread_mutex_unlock(&g_providers_lock);
    return removed;
}

/* 平台窗口回调：转发给 Kotlin `NrSourceProvider.readWindow`。
 * 契约（头文件 §平台窗口内容协议）：成功时 out_len ≤ max_len，超长一律按内部错误拒绝。 */
static int32_t jni_read_window(void *user, uint64_t request_id, uint64_t offset, uint32_t max_len,
                               uint8_t *dst, uint32_t *out_len, int32_t *out_eof) {
    nr_jni_provider *entry = (nr_jni_provider *)user;
    if (entry == NULL || entry->ref == NULL || out_len == NULL || out_eof == NULL) {
        return NR_ERR_INTERNAL;
    }
    JNIEnv *env = NULL;
    if ((*entry->vm)->GetEnv(entry->vm, (void **)&env, JNI_VERSION_1_6) != JNI_OK || env == NULL) {
        NR_LOG("平台回调线程未附加 JVM");
        return NR_ERR_INTERNAL;
    }
    jbyteArray scratch = (*env)->NewByteArray(env, (jsize)max_len);
    jintArray info = (*env)->NewIntArray(env, 2);
    if (scratch == NULL || info == NULL) {
        if (scratch != NULL) {
            (*env)->DeleteLocalRef(env, scratch);
        }
        if (info != NULL) {
            (*env)->DeleteLocalRef(env, info);
        }
        if ((*env)->ExceptionCheck(env)) {
            (*env)->ExceptionClear(env);
        }
        return NR_ERR_INTERNAL;
    }
    jint code = (*env)->CallIntMethod(env, entry->ref, entry->read_window, (jlong)request_id,
                                      (jlong)offset, (jint)max_len, scratch, info);
    if ((*env)->ExceptionCheck(env)) {
        /* Kotlin 数据源异常：清理挂起异常并映射为 I/O 失败（绝不把异常带进 Rust）。 */
        (*env)->ExceptionDescribe(env);
        (*env)->ExceptionClear(env);
        code = NR_ERR_IO;
    }
    jint values[2] = {0, 0};
    if (code == NR_ERR_OK) {
        (*env)->GetIntArrayRegion(env, info, 0, 2, values);
        if ((*env)->ExceptionCheck(env)) {
            (*env)->ExceptionClear(env);
            code = NR_ERR_INTERNAL;
        }
    }
    uint32_t len = (uint32_t)values[0];
    if (code == NR_ERR_OK) {
        if (len > max_len) {
            NR_LOG("平台回报超长窗口：out_len=%u > max_len=%u（拒绝采信）", len, max_len);
            code = NR_ERR_INTERNAL;
        } else {
            if (len > 0 && dst != NULL) {
                (*env)->GetByteArrayRegion(env, scratch, 0, (jsize)len, (jbyte *)dst);
                if ((*env)->ExceptionCheck(env)) {
                    (*env)->ExceptionClear(env);
                    code = NR_ERR_INTERNAL;
                }
            }
            *out_len = len;
            *out_eof = values[1] ? 1 : 0;
        }
    }
    (*env)->DeleteLocalRef(env, scratch);
    (*env)->DeleteLocalRef(env, info);
    return code;
}

/* ============================ 18 项入口 ============================ */

/* 1. nr_abi_version() -> uint32_t */
JNIEXPORT jint JNICALL NR_JNI(nr_1abi_1version)(JNIEnv *env, jobject self) {
    (void)env;
    (void)self;
    return (jint)nr_abi_version();
}

/* 2. nr_engine_create(config, out_engine)：provider 为 null 时按文件数据源创建。 */
JNIEXPORT jlong JNICALL NR_JNI(nr_1engine_1create)(JNIEnv *env, jobject self, jlong max_window_bytes,
                                                 jbyteArray db_path, jobject provider) {
    (void)self;
    jsize db_len = 0;
    jbyte *db_pinned = NULL;
    const uint8_t *db_bytes = jni_borrow_bytes(env, db_path, &db_len, &db_pinned);

    /* provider 条目必须在引擎存续期内保持有效地址（Rust 侧保存 user 指针），
     * 因此先分配堆上的登记项，再把它的地址交给 Rust —— 绝不能用栈上局部变量的地址。 */
    nr_jni_provider *entry = NULL;
    nr_source_provider source;
    memset(&source, 0, sizeof(source));
    if (provider != NULL) {
        JavaVM *vm = NULL;
        if ((*env)->GetJavaVM(env, &vm) != JNI_OK || vm == NULL) {
            NR_LOG("无法获取 JavaVM");
            jni_release_bytes(env, db_path, db_pinned);
            return 0;
        }
        jclass cls = (*env)->GetObjectClass(env, provider);
        jmethodID mid = cls != NULL ? (*env)->GetMethodID(env, cls, NR_JNI_PROVIDER_METHOD,
                                                          NR_JNI_PROVIDER_SIG)
                                    : NULL;
        if (cls != NULL) {
            (*env)->DeleteLocalRef(env, cls);
        }
        if (mid == NULL) {
            NR_LOG("provider 缺少 %s%s：与 android/jni/README.md §3 不一致",
                   NR_JNI_PROVIDER_METHOD, NR_JNI_PROVIDER_SIG);
            if ((*env)->ExceptionCheck(env)) {
                (*env)->ExceptionClear(env);
            }
            jni_release_bytes(env, db_path, db_pinned);
            return 0;
        }
        jobject ref = (*env)->NewGlobalRef(env, provider);
        entry = (nr_jni_provider *)malloc(sizeof(*entry));
        if (ref == NULL || entry == NULL) {
            if (ref != NULL) {
                (*env)->DeleteGlobalRef(env, ref);
            }
            free(entry);
            jni_release_bytes(env, db_path, db_pinned);
            return 0;
        }
        entry->vm = vm;
        entry->ref = ref;
        entry->read_window = mid;
        entry->engine = 0;
        entry->next = NULL;
        source.user = entry;
        source.read_window = jni_read_window;
    }

    nr_engine_config config;
    memset(&config, 0, sizeof(config));
    config.struct_size = (uint32_t)sizeof(nr_engine_config);
    config.abi_version = NR_ABI_VERSION;
    config.max_window_bytes = (uint64_t)max_window_bytes;
    config.provider = provider != NULL ? &source : NULL;
    config.db_path_utf8 = db_len > 0 ? db_bytes : NULL;
    config.db_path_len = db_len > 0 ? (size_t)db_len : 0;

    uint64_t engine = 0;
    int32_t code = nr_engine_create(&config, &engine);
    jni_release_bytes(env, db_path, db_pinned);
    if (code != NR_ERR_OK || engine == 0) {
        if (entry != NULL) {
            (*env)->DeleteGlobalRef(env, entry->ref);
            free(entry);
        }
        NR_LOG("nr_engine_create 失败：code=%d", (int)code);
        return 0;
    }
    if (entry != NULL) {
        entry->engine = engine;
        jni_provider_attach(entry);
    }
    return (jlong)engine;
}

/* 3. nr_engine_destroy(engine) */
JNIEXPORT jint JNICALL NR_JNI(nr_1engine_1destroy)(JNIEnv *env, jobject self, jlong engine) {
    (void)self;
    int32_t code = nr_engine_destroy((uint64_t)engine);
    if (code == NR_ERR_OK) {
        jni_provider_detach(env, (uint64_t)engine);
    }
    return (jint)code;
}

/* 4. nr_open_txt(engine, request, out_result) */
JNIEXPORT jint JNICALL NR_JNI(nr_1open_1txt)(JNIEnv *env, jobject self, jlong engine, jint kind,
                                           jlong total_bytes, jbyteArray identity,
                                           jbyteArray display_name, jbyteArray out_result) {
    (void)self;
    jsize identity_len = 0, name_len = 0;
    jbyte *identity_pinned = NULL, *name_pinned = NULL;
    const uint8_t *identity_bytes = jni_borrow_bytes(env, identity, &identity_len, &identity_pinned);
    const uint8_t *name_bytes = jni_borrow_bytes(env, display_name, &name_len, &name_pinned);

    nr_open_request request;
    memset(&request, 0, sizeof(request));
    request.struct_size = (uint32_t)sizeof(nr_open_request);
    request.kind = (uint32_t)kind;
    request.total_bytes = (uint64_t)total_bytes;
    request.identity_utf8 = identity_len > 0 ? identity_bytes : NULL;
    request.identity_len = identity_len > 0 ? (size_t)identity_len : 0;
    request.display_name_utf8 = name_len > 0 ? name_bytes : NULL;
    request.display_name_len = name_len > 0 ? (size_t)name_len : 0;

    nr_open_result result;
    memset(&result, 0, sizeof(result));
    int32_t code = nr_open_txt((uint64_t)engine, &request, &result);
    if (code == NR_ERR_OK) {
        code = jni_write_out(env, out_result, &result, sizeof(result));
    }
    jni_release_bytes(env, identity, identity_pinned);
    jni_release_bytes(env, display_name, name_pinned);
    return (jint)code;
}

/* 5. nr_close_book(engine, book_id) */
JNIEXPORT jint JNICALL NR_JNI(nr_1close_1book)(JNIEnv *env, jobject self, jlong engine,
                                             jlong book_id) {
    (void)env;
    (void)self;
    return (jint)nr_close_book((uint64_t)engine, (uint64_t)book_id);
}

/* 6. nr_read_window(engine, chapter_index, offset, length, out_window, out_buffer) -> bytes */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1read_1window)(JNIEnv *env, jobject self, jlong engine,
                                                    jint chapter_index, jlong offset, jlong length,
                                                    jbyteArray out_window, jlongArray out_buffer) {
    (void)self;
    nr_window window;
    nr_buffer bytes;
    memset(&window, 0, sizeof(window));
    memset(&bytes, 0, sizeof(bytes));
    int32_t code = nr_read_window((uint64_t)engine, (uint32_t)chapter_index, (uint64_t)offset,
                                  (uint64_t)length, &window, &bytes);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_window, &window, sizeof(window)) != NR_ERR_OK) {
        nr_release_buffer(&bytes);
        return NULL;
    }
    return jni_take_buffer(env, &bytes, out_buffer);
}

/* 7. nr_submit_page_boundary(engine, submission, out_result) */
JNIEXPORT jint JNICALL NR_JNI(nr_1submit_1page_1boundary)(
    JNIEnv *env, jobject self, jlong engine, jint phase, jlong content_index_version,
    jlong layout_id, jlong request_id, jlong total_bytes, jlong start_byte, jlong end_byte,
    jlongArray layout_fields, jbyteArray font_family, jbyteArray out_result) {
    (void)self;
    jsize font_len = 0;
    jbyte *font_pinned = NULL;
    const uint8_t *font_bytes = jni_borrow_bytes(env, font_family, &font_len, &font_pinned);

    nr_layout_signature signature;
    memset(&signature, 0, sizeof(signature));
    int32_t code = NR_ERR_OK;
    const nr_layout_signature *signature_ptr = NULL;
    if (phase == (jint)NR_SUBMIT_BEGIN_LAYOUT) {
        code = jni_read_layout_fields(env, layout_fields, &signature,
                                      font_len > 0 ? font_bytes : NULL,
                                      font_len > 0 ? (size_t)font_len : 0);
        if (code != NR_ERR_OK) {
            jni_release_bytes(env, font_family, font_pinned);
            return (jint)code;
        }
        signature_ptr = &signature;
    }

    nr_page_boundary_submission submission;
    memset(&submission, 0, sizeof(submission));
    submission.struct_size = (uint32_t)sizeof(nr_page_boundary_submission);
    submission.phase = (uint32_t)phase;
    submission.content_index_version = (uint64_t)content_index_version;
    submission.layout_id = (uint64_t)layout_id;
    submission.request_id = (uint64_t)request_id;
    submission.total_bytes = (uint64_t)total_bytes;
    submission.start_byte = (uint64_t)start_byte;
    submission.end_byte = (uint64_t)end_byte;
    submission.signature = signature_ptr;

    nr_boundary_result result;
    memset(&result, 0, sizeof(result));
    code = nr_submit_page_boundary((uint64_t)engine, &submission, &result);
    if (code == NR_ERR_OK) {
        code = jni_write_out(env, out_result, &result, sizeof(result));
    }
    jni_release_bytes(env, font_family, font_pinned);
    return (jint)code;
}

/* 8. nr_get_page_boundary(engine, layout_id, chapter_index, start_byte, out_result) */
JNIEXPORT jint JNICALL NR_JNI(nr_1get_1page_1boundary)(JNIEnv *env, jobject self, jlong engine,
                                                    jlong layout_id, jint chapter_index,
                                                    jlong start_byte, jbyteArray out_result) {
    (void)self;
    nr_boundary_result result;
    memset(&result, 0, sizeof(result));
    int32_t code = nr_get_page_boundary((uint64_t)engine, (uint64_t)layout_id,
                                        (uint32_t)chapter_index, (uint64_t)start_byte, &result);
    if (code == NR_ERR_OK) {
        code = jni_write_out(env, out_result, &result, sizeof(result));
    }
    return (jint)code;
}

/* 9. nr_save_position(engine, position, out_changed) */
JNIEXPORT jint JNICALL NR_JNI(nr_1save_1position)(JNIEnv *env, jobject self, jlong engine,
                                                jint chapter_index, jlong book_id,
                                                jlong content_unit_id, jlong original_byte_offset,
                                                jbyteArray fingerprint, jintArray out_changed) {
    (void)self;
    jsize fingerprint_len = 0;
    jbyte *fingerprint_pinned = NULL;
    const uint8_t *fingerprint_bytes =
        jni_borrow_bytes(env, fingerprint, &fingerprint_len, &fingerprint_pinned);

    nr_position position;
    memset(&position, 0, sizeof(position));
    position.struct_size = (uint32_t)sizeof(nr_position);
    position.chapter_index = (uint32_t)chapter_index;
    position.book_id = (uint64_t)book_id;
    position.content_unit_id = (uint64_t)content_unit_id;
    position.original_byte_offset = (uint64_t)original_byte_offset;
    position.nearby_fingerprint_utf8 = fingerprint_len > 0 ? fingerprint_bytes : NULL;
    position.nearby_fingerprint_len = fingerprint_len > 0 ? (size_t)fingerprint_len : 0;

    int32_t changed = 0;
    int32_t code = nr_save_position((uint64_t)engine, &position, &changed);
    if (code == NR_ERR_OK && out_changed != NULL) {
        code = jni_write_int_out(env, out_changed, 0, (jint)changed);
    }
    jni_release_bytes(env, fingerprint, fingerprint_pinned);
    return (jint)code;
}

/* 10. nr_get_position(engine, book_id, out_result, out_buffer) -> 指纹字节 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1get_1position)(JNIEnv *env, jobject self, jlong engine,
                                                     jlong book_id, jbyteArray out_result,
                                                     jlongArray out_buffer) {
    (void)self;
    nr_position_result result;
    memset(&result, 0, sizeof(result));
    int32_t code = nr_get_position((uint64_t)engine, (uint64_t)book_id, &result);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_result, &result, sizeof(result)) != NR_ERR_OK) {
        nr_release_buffer(&result.nearby_fingerprint);
        return NULL;
    }
    return jni_take_buffer(env, &result.nearby_fingerprint, out_buffer);
}

/* 11. nr_get_chapters(engine, book_id, start_sequence, max_count, out_batch, out_buffer) */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1get_1chapters)(JNIEnv *env, jobject self, jlong engine,
                                                     jlong book_id, jint start_sequence,
                                                     jint max_count, jbyteArray out_batch,
                                                     jlongArray out_buffer) {
    (void)self;
    nr_chapter_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_get_chapters((uint64_t)engine, (uint64_t)book_id, (uint32_t)start_sequence,
                                   (uint32_t)max_count, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_batch, &batch, sizeof(batch)) != NR_ERR_OK) {
        nr_release_buffer(&batch.bytes);
        return NULL;
    }
    return jni_take_buffer(env, &batch.bytes, out_buffer);
}

/* 12. nr_create_bookmark(engine, input, out_bookmark_id) */
JNIEXPORT jint JNICALL NR_JNI(nr_1create_1bookmark)(JNIEnv *env, jobject self, jlong engine,
                                                  jint chapter_index, jlong book_id,
                                                  jlong content_unit_id, jlong byte_offset,
                                                  jbyteArray title, jlongArray out_bookmark_id) {
    (void)self;
    jsize title_len = 0;
    jbyte *title_pinned = NULL;
    const uint8_t *title_bytes = jni_borrow_bytes(env, title, &title_len, &title_pinned);

    nr_bookmark_input input;
    memset(&input, 0, sizeof(input));
    input.struct_size = (uint32_t)sizeof(nr_bookmark_input);
    input.chapter_index = (uint32_t)chapter_index;
    input.book_id = (uint64_t)book_id;
    input.content_unit_id = (uint64_t)content_unit_id;
    input.byte_offset = (uint64_t)byte_offset;
    input.title_utf8 = title_len > 0 ? title_bytes : NULL;
    input.title_len = title_len > 0 ? (size_t)title_len : 0;

    uint64_t bookmark_id = 0;
    int32_t code = nr_create_bookmark((uint64_t)engine, &input, &bookmark_id);
    if (code == NR_ERR_OK) {
        code = jni_write_long_out(env, out_bookmark_id, 0, (jlong)bookmark_id);
    }
    jni_release_bytes(env, title, title_pinned);
    return (jint)code;
}

/* 13. nr_delete_bookmark(engine, bookmark_id, out_deleted) */
JNIEXPORT jint JNICALL NR_JNI(nr_1delete_1bookmark)(JNIEnv *env, jobject self, jlong engine,
                                                  jlong bookmark_id, jintArray out_deleted) {
    (void)self;
    int32_t deleted = 0;
    int32_t code = nr_delete_bookmark((uint64_t)engine, (uint64_t)bookmark_id, &deleted);
    if (code == NR_ERR_OK && out_deleted != NULL) {
        code = jni_write_int_out(env, out_deleted, 0, (jint)deleted);
    }
    return (jint)code;
}

/* 14. nr_invalidate_layout(engine, layout_id, out_removed) */
JNIEXPORT jint JNICALL NR_JNI(nr_1invalidate_1layout)(JNIEnv *env, jobject self, jlong engine,
                                                    jlong layout_id, jlongArray out_removed) {
    (void)self;
    uint64_t removed = 0;
    int32_t code = nr_invalidate_layout((uint64_t)engine, (uint64_t)layout_id, &removed);
    if (code == NR_ERR_OK && out_removed != NULL) {
        code = jni_write_long_out(env, out_removed, 0, (jlong)removed);
    }
    return (jint)code;
}

/* 15. nr_rebuild_index(engine, book_id, request_id, out_result) */
JNIEXPORT jint JNICALL NR_JNI(nr_1rebuild_1index)(JNIEnv *env, jobject self, jlong engine,
                                                jlong book_id, jlong request_id,
                                                jbyteArray out_result) {
    (void)self;
    nr_rebuild_result result;
    memset(&result, 0, sizeof(result));
    int32_t code = nr_rebuild_index((uint64_t)engine, (uint64_t)book_id, (uint64_t)request_id,
                                    &result);
    if (code == NR_ERR_OK || code == NR_ERR_CANCELLED) {
        /* 取消是"部分成功"：结果结构体仍然有效，必须写回（partial = 1，不落库）。 */
        int32_t write_code = jni_write_out(env, out_result, &result, sizeof(result));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* 19. nr_set_encoding(engine, book_id, encoding_code, out_result)
 *     手动指定内容编码：只改变解释方式，不写回原文件；成功后返回新的索引版本与章节数。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1set_1encoding)(JNIEnv *env, jobject self, jlong engine,
                                                jlong book_id, jint encoding_code,
                                                jbyteArray out_result) {
    (void)self;
    nr_set_encoding_result result;
    memset(&result, 0, sizeof(result));
    int32_t code = nr_set_encoding((uint64_t)engine, (uint64_t)book_id, (uint32_t)encoding_code,
                                   &result);
    if (code == NR_ERR_OK) {
        int32_t write_code = jni_write_out(env, out_result, &result, sizeof(result));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* ---- 分类树（V1.1；加性契约修订）----
 * 注意 JNI 名字转义：每个 `_` 必须写成 `_1`（否则能编译、运行时才
 * UnsatisfiedLinkError；scripts/build-android-libs.sh 会 grep 这个错误）。 */

/* nr_list_categories(engine, out_batch): jbyteArray
 *     一次性返回整棵树；缓冲区按 `nr_get_chapters` 同一套流程归还。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1list_1categories)(JNIEnv *env, jobject self,
                                                          jlong engine, jbyteArray out_batch,
                                                          jlongArray out_buffer) {
    (void)self;
    nr_category_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_list_categories((uint64_t)engine, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_batch, &batch, sizeof(batch)) != NR_ERR_OK) {
        nr_release_buffer(&batch.bytes);
        return NULL;
    }
    return jni_take_buffer(env, &batch.bytes, out_buffer);
}

/* nr_create_category(engine, parent_id, name, out_category_id): jint
 *     parent_id 传 -1 = 根下；同父同名由核心拒绝。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1create_1category)(JNIEnv *env, jobject self, jlong engine,
                                                    jlong parent_id, jbyteArray name,
                                                    jlongArray out_category_id) {
    (void)self;
    jsize name_len = 0;
    jbyte *name_pinned = NULL;
    const uint8_t *name_bytes = jni_borrow_bytes(env, name, &name_len, &name_pinned);

    int64_t category_id = 0;
    int32_t code = nr_create_category((uint64_t)engine, (int64_t)parent_id, name_bytes,
                                      (uint32_t)name_len, &category_id);
    if (name_pinned != NULL) {
        (*env)->ReleaseByteArrayElements(env, name, name_pinned, 0);
    }
    if (code == NR_ERR_OK && out_category_id != NULL) {
        int32_t write_code = jni_write_out(env, out_category_id, &category_id,
                                           sizeof(category_id));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* nr_move_book_to_category(engine, book_id, category_id): jint
 *     category_id 传 -1 = 未分类（根）。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1move_1book_1to_1category)(JNIEnv *env, jobject self,
                                                           jlong engine, jlong book_id,
                                                           jlong category_id) {
    (void)self;
    (void)env;
    return (jint)nr_move_book_to_category((uint64_t)engine, (int64_t)book_id,
                                          (int64_t)category_id);
}

/* nr_delete_category(engine, category_id, destination, target_id, out_moved_count): jint
 *     destination 必须显式给出；传 0（未选去向）核心会拒绝且零副作用。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1delete_1category)(JNIEnv *env, jobject self, jlong engine,
                                                   jlong category_id, jint destination,
                                                   jlong target_id, jintArray out_moved_count) {
    (void)self;
    uint32_t moved = 0;
    int32_t code = nr_delete_category((uint64_t)engine, (int64_t)category_id,
                                      (int32_t)destination, (int64_t)target_id, &moved);
    if (code == NR_ERR_OK && out_moved_count != NULL) {
        int32_t write_code = jni_write_out(env, out_moved_count, &moved, sizeof(moved));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* ---- 阅读历史（V1.1；加性契约修订）----
 * 与分类同样注意 JNI 名字转义（`_` → `_1`）。 */

/* nr_list_history(engine, book_id, limit, grouped, out_batch): jbyteArray
 *     book_id > 0 限定单书；limit > 0 限量；grouped != 0 走按书聚合。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1list_1history)(JNIEnv *env, jobject self,
                                                      jlong engine, jlong book_id,
                                                      jint limit, jint grouped,
                                                      jbyteArray out_batch,
                                                      jlongArray out_buffer) {
    (void)self;
    nr_history_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_list_history((uint64_t)engine, (int64_t)book_id, (int32_t)limit,
                                   (int32_t)grouped, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_batch, &batch, sizeof(batch)) != NR_ERR_OK) {
        nr_release_buffer(&batch.bytes);
        return NULL;
    }
    return jni_take_buffer(env, &batch.bytes, out_buffer);
}

/* nr_clear_history_entry(engine, entry_id, out_removed): jint
 *     out_removed 写回 1 = 已清除、0 = 该 id 不存在。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1clear_1history_1entry)(JNIEnv *env, jobject self,
                                                         jlong engine, jlong entry_id,
                                                         jintArray out_removed) {
    (void)self;
    uint32_t removed = 0;
    int32_t code =
        nr_clear_history_entry((uint64_t)engine, (int64_t)entry_id, &removed);
    if (code == NR_ERR_OK && out_removed != NULL) {
        int32_t write_code = jni_write_out(env, out_removed, &removed, sizeof(removed));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* nr_clear_history_for_book(engine, book_id, out_removed_count): jint
 *     只删流水：不动书籍 / 进度 / 书签。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1clear_1history_1for_1book)(JNIEnv *env, jobject self,
                                                           jlong engine, jlong book_id,
                                                           jintArray out_removed_count) {
    (void)self;
    uint32_t removed = 0;
    int32_t code =
        nr_clear_history_for_book((uint64_t)engine, (int64_t)book_id, &removed);
    if (code == NR_ERR_OK && out_removed_count != NULL) {
        int32_t write_code = jni_write_out(env, out_removed_count, &removed, sizeof(removed));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* ---- 自定义配色（V1.1；加性契约修订）----
 * 派生与对比度**全在核心**（桌面与 Android 同一函数），本层只搬运。 */

/* nr_get_theme_base(engine, out_base): jint
 *     只回 4 个基色与 enabled；四项各 12 字节，全 0 = 未设置。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1get_1theme_1base)(JNIEnv *env, jobject self,
                                                     jlong engine,
                                                     jbyteArray out_base) {
    (void)self;
    nr_theme_base base;
    memset(&base, 0, sizeof(base));
    int32_t code = nr_get_theme_base((uint64_t)engine, &base);
    if (code == NR_ERR_OK) {
        int32_t write_code = jni_write_out(env, out_base, &base, sizeof(base));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* nr_set_theme_base(engine, which, value): jint
 *     which：0=正文 1=背景 2=主色 3=强调；value 长度 0 = 清除该项。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1set_1theme_1base)(JNIEnv *env, jobject self,
                                                     jlong engine, jint which,
                                                     jbyteArray value) {
    (void)self;
    jsize value_len = 0;
    jbyte *value_pinned = NULL;
    const uint8_t *value_bytes = jni_borrow_bytes(env, value, &value_len, &value_pinned);
    int32_t code = nr_set_theme_base((uint64_t)engine, (uint32_t)which, value_bytes,
                                     (uint32_t)value_len);
    if (value_pinned != NULL) {
        (*env)->ReleaseByteArrayElements(env, value, value_pinned, 0);
    }
    return (jint)code;
}

/* nr_set_theme_enabled(engine, enabled): jint */
JNIEXPORT jint JNICALL NR_JNI(nr_1set_1theme_1enabled)(JNIEnv *env, jobject self,
                                                        jlong engine, jint enabled) {
    (void)self;
    (void)env;
    return (jint)nr_set_theme_enabled((uint64_t)engine, (uint32_t)enabled);
}

/* nr_get_theme_token(engine, which, dark, outRGBA): jint
 *     写 4 字节 RGBA；越界索引由核心拒绝。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1get_1theme_1token)(JNIEnv *env, jobject self,
                                                      jlong engine, jint which,
                                                      jint dark, jbyteArray out_rgba) {
    (void)self;
    uint8_t rgba[4];
    memset(rgba, 0, sizeof(rgba));
    int32_t code = nr_get_theme_token((uint64_t)engine, (uint32_t)which, (uint32_t)dark, rgba);
    if (code == NR_ERR_OK) {
        int32_t write_code = jni_write_out(env, out_rgba, rgba, sizeof(rgba));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* nr_get_theme_contrast(engine, outResult): jint
 *     比值为整数 × 100（如 4.54:1 ⇒ 454）；passes=0 时核心已保留用户基色。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1get_1theme_1contrast)(JNIEnv *env, jobject self,
                                                        jlong engine,
                                                        jbyteArray out_result) {
    (void)self;
    nr_theme_contrast_result result;
    memset(&result, 0, sizeof(result));
    int32_t code = nr_get_theme_contrast((uint64_t)engine, &result);
    if (code == NR_ERR_OK) {
        int32_t write_code = jni_write_out(env, out_result, &result, sizeof(result));
        if (write_code != NR_ERR_OK) {
            return (jint)write_code;
        }
    }
    return (jint)code;
}

/* ---- 全局背景图（V1.1；加性契约修订）----
 * 核心**复制**图片并只回相对文件名（R14–R16）；本层不碰文件。 */

/* 读背景批次并取回字节缓冲（4 个入口共用的收尾）。
 * 返回 `NULL` 表示失败；`*out_buffer` 由 Kotlin 侧归还。 */
/* 静态辅助（**不用** `NR_JNI(...)` —— 那是给导出符号用的 JNI 名字转义宏）。
 * 本函数不出现在 `.so` 导出表里，故直接用 C 名字。 */
static jbyteArray nr_background_result(JNIEnv *env, jbyteArray out_batch,
                                       nr_background_batch *batch, jlongArray out_buffer) {
    if (jni_write_out(env, out_batch, batch, sizeof(*batch)) != NR_ERR_OK) {
        nr_release_buffer(&batch->bytes);
        return NULL;
    }
    return jni_take_buffer(env, &batch->bytes, out_buffer);
}

/* nr_get_background(engine, outBatch, outBuffer): jbyteArray
 *     只回相对文件名（R15）；fileNameLen = 0 表示未设置。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1get_1background)(JNIEnv *env, jobject self,
                                                         jlong engine,
                                                         jbyteArray out_batch,
                                                         jlongArray out_buffer) {
    (void)self;
    nr_background_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_get_background((uint64_t)engine, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    return nr_background_result(env, out_batch, &batch, out_buffer);
}

/* nr_set_background(engine, srcPath, destDir, fitMode, scrimPermille, outBatch, outBuffer)
 *     fitMode < 0 或 scrimPermille < 0 = 用默认值。
 *     destDir 由调用方给（Android 传 app-private 目录）—— 核心不猜平台数据目录。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1set_1background)(JNIEnv *env, jobject self,
                                                         jlong engine,
                                                         jbyteArray src_path,
                                                         jbyteArray dest_dir,
                                                         jint fit_mode,
                                                         jint scrim_permille,
                                                         jbyteArray out_batch,
                                                         jlongArray out_buffer) {
    (void)self;
    jsize src_len = 0, dir_len = 0;
    jbyte *src_pinned = NULL, *dir_pinned = NULL;
    const uint8_t *src_bytes = jni_borrow_bytes(env, src_path, &src_len, &src_pinned);
    const uint8_t *dir_bytes = jni_borrow_bytes(env, dest_dir, &dir_len, &dir_pinned);

    nr_background_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_set_background((uint64_t)engine, src_bytes, (uint32_t)src_len,
                                     dir_bytes, (uint32_t)dir_len, (int32_t)fit_mode,
                                     (int32_t)scrim_permille, &batch);
    if (src_pinned != NULL) {
        (*env)->ReleaseByteArrayElements(env, src_path, src_pinned, 0);
    }
    if (dir_pinned != NULL) {
        (*env)->ReleaseByteArrayElements(env, dest_dir, dir_pinned, 0);
    }
    if (code != NR_ERR_OK) {
        return NULL;
    }
    return nr_background_result(env, out_batch, &batch, out_buffer);
}

/* nr_set_background_scrim(engine, scrimPermille, outBatch, outBuffer): jbyteArray
 *     只调遮罩强度（不换图）；permille 必须在 0..=1000。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1set_1background_1scrim)(JNIEnv *env, jobject self,
                                                                jlong engine,
                                                                jint scrim_permille,
                                                                jbyteArray out_batch,
                                                                jlongArray out_buffer) {
    (void)self;
    nr_background_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_set_background_scrim((uint64_t)engine, (int32_t)scrim_permille, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    return nr_background_result(env, out_batch, &batch, out_buffer);
}

/* nr_clear_background(engine, destDir, outBatch, outBuffer): jbyteArray
 *     **只删核心生成的文件**（bg-* 前缀）；**不动用户原图**。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1clear_1background)(JNIEnv *env, jobject self,
                                                           jlong engine,
                                                           jbyteArray dest_dir,
                                                           jbyteArray out_batch,
                                                           jlongArray out_buffer) {
    (void)self;
    jsize dir_len = 0;
    jbyte *dir_pinned = NULL;
    const uint8_t *dir_bytes = jni_borrow_bytes(env, dest_dir, &dir_len, &dir_pinned);

    nr_background_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_clear_background((uint64_t)engine, dir_bytes, (uint32_t)dir_len, &batch);
    if (dir_pinned != NULL) {
        (*env)->ReleaseByteArrayElements(env, dest_dir, dir_pinned, 0);
    }
    if (code != NR_ERR_OK) {
        return NULL;
    }
    return nr_background_result(env, out_batch, &batch, out_buffer);
}

/* ---- 移动 / 删除目标（V1.1）---- */

/* nr_category_move_targets(engine, source_id, out_batch, out_buffer): jbyteArray
 *     含「未分类」（id = -1）；自身与全部子孙 selectable = 0（不隐藏，只标记）。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1category_1move_1targets)(JNIEnv *env, jobject self,
                                                                 jlong engine,
                                                                 jlong source_id,
                                                                 jbyteArray out_batch,
                                                                 jlongArray out_buffer) {
    (void)self;
    nr_category_choice_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_category_move_targets((uint64_t)engine, (int64_t)source_id, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_batch, &batch, sizeof(batch)) != NR_ERR_OK) {
        nr_release_buffer(&batch.bytes);
        return NULL;
    }
    return jni_take_buffer(env, &batch.bytes, out_buffer);
}

/* ---- 分类改名 / 移动（V1.1）---- */

/* nr_rename_category(engine, category_id, name, name_len): jint
 *     空名 / 坏 UTF-8 由核心拒绝（返回 INVALID_ARGUMENT）。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1rename_1category)(JNIEnv *env, jobject self,
                                                    jlong engine,
                                                    jlong category_id,
                                                    jbyteArray name) {
    (void)self;
    jsize name_len = 0;
    jbyte *name_pinned = NULL;
    const uint8_t *name_bytes = jni_borrow_bytes(env, name, &name_len, &name_pinned);

    int32_t code =
        nr_rename_category((uint64_t)engine, (int64_t)category_id, name_bytes, (uint32_t)name_len);
    if (name_pinned != NULL) {
        (*env)->ReleaseByteArrayElements(env, name, name_pinned, 0);
    }
    return (jint)code;
}

/* nr_move_category(engine, category_id, new_parent_id): jint
 *     new_parent_id = -1 表示移到根层；移入自身或子孙必须拒绝。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1move_1category)(JNIEnv *env, jobject self,
                                                  jlong engine,
                                                  jlong category_id,
                                                  jlong new_parent_id) {
    (void)env;
    (void)self;
    return (jint)nr_move_category((uint64_t)engine, (int64_t)category_id, (int64_t)new_parent_id);
}

/* ---- 作用域书籍 id（V1.1）---- */

/* nr_list_books_in_scope(engine, scope_kind, category_id, out_batch, out_buffer): jbyteArray
 *     scope_kind: 0 = 全部、1 = 仅未分类、2 = 指定分类（含子孙）。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1list_1books_1in_1scope)(JNIEnv *env, jobject self,
                                                                jlong engine,
                                                                jint scope_kind,
                                                                jlong category_id,
                                                                jbyteArray out_batch,
                                                                jlongArray out_buffer) {
    (void)self;
    nr_id_list batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code =
        nr_list_books_in_scope((uint64_t)engine, (int32_t)scope_kind, (int64_t)category_id, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_batch, &batch, sizeof(batch)) != NR_ERR_OK) {
        nr_release_buffer(&batch.bytes);
        return NULL;
    }
    return jni_take_buffer(env, &batch.bytes, out_buffer);
}

/* ---- 书籍分类映射（V1.1；筛选前置）---- */

/* nr_list_book_categories(engine, out_batch, out_buffer): jbyteArray
 *     只含在架书籍；category_id = -1 表示未分类。行内无字符串。 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1list_1book_1categories)(JNIEnv *env, jobject self,
                                                                jlong engine,
                                                                jbyteArray out_batch,
                                                                jlongArray out_buffer) {
    (void)self;
    nr_book_category_batch batch;
    memset(&batch, 0, sizeof(batch));
    int32_t code = nr_list_book_categories((uint64_t)engine, &batch);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_batch, &batch, sizeof(batch)) != NR_ERR_OK) {
        nr_release_buffer(&batch.bytes);
        return NULL;
    }
    return jni_take_buffer(env, &batch.bytes, out_buffer);
}

/* 16. nr_cancel_request(engine, request_id) */
JNIEXPORT jint JNICALL NR_JNI(nr_1cancel_1request)(JNIEnv *env, jobject self, jlong engine,
                                                 jlong request_id) {
    (void)env;
    (void)self;
    return (jint)nr_cancel_request((uint64_t)engine, (uint64_t)request_id);
}

/* 17. nr_get_last_error(engine, out_info, out_buffer) -> 消息字节 */
JNIEXPORT jbyteArray JNICALL NR_JNI(nr_1get_1last_1error)(JNIEnv *env, jobject self, jlong engine,
                                                       jbyteArray out_info, jlongArray out_buffer) {
    (void)self;
    nr_error_info info;
    memset(&info, 0, sizeof(info));
    int32_t code = nr_get_last_error((uint64_t)engine, &info);
    if (code != NR_ERR_OK) {
        return NULL;
    }
    if (jni_write_out(env, out_info, &info, sizeof(info)) != NR_ERR_OK) {
        nr_release_buffer(&info.message);
        return NULL;
    }
    return jni_take_buffer(env, &info.message, out_buffer);
}

/* 18. nr_release_buffer(ptr, len, capacity)：按头文件 §5 归还 Rust 分配的缓冲区。
 *     (0,0,0) 是安全 no-op（返回 0）；只有本层登记过的缓冲区才允许归还，
 *     重复/伪造归还会被拒绝为 NR_ERR_INVALID_ARGUMENT（不会双重释放）。 */
JNIEXPORT jint JNICALL NR_JNI(nr_1release_1buffer)(JNIEnv *env, jobject self, jlong ptr, jlong len,
                                                 jlong capacity) {
    (void)env;
    (void)self;
    if (ptr == 0 && len == 0 && capacity == 0) {
        nr_buffer empty;
        memset(&empty, 0, sizeof(empty));
        return (jint)nr_release_buffer(&empty);
    }
    nr_buffer recorded;
    memset(&recorded, 0, sizeof(recorded));
    if (!jni_outstanding_find((size_t)(uintptr_t)ptr, &recorded)) {
        NR_LOG("归还未登记（重复或伪造）的缓冲区：ptr=%lld", (long long)ptr);
        return NR_ERR_INVALID_ARGUMENT;
    }
    if ((size_t)len != recorded.len || (size_t)capacity != recorded.capacity) {
        NR_LOG("归还字段与登记不一致：登记 len=%zu cap=%zu，调用 len=%lld cap=%lld", recorded.len,
               recorded.capacity, (long long)len, (long long)capacity);
        return NR_ERR_INVALID_ARGUMENT;
    }
    int32_t code = nr_release_buffer(&recorded);
    jni_outstanding_remove((size_t)(uintptr_t)ptr);
    return (jint)code;
}
