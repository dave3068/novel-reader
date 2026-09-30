package com.novelreader.android.core;

/**
 * 宿主机 JNI 层自检用的声明（AN1）。
 *
 * 声明清单与 Kotlin `object NrCore` 完全相同（符号名只看 「包名 + 类名 + 方法名」，
 * 与静态/实例方法无关），因此这份 Java 副本可以真实链接 android/jni/nr_jni.c 导出的
 * `Java_com_novelreader_android_core_NrCore_nr_*` 符号，用于在没有 Android 设备的情况下
 * 验证：参数装箱/拆箱、结构体字节布局、缓冲区归还、平台窗口回调与错误码透传。
 *
 * 生产代码在 android/app/src/main/java/...（Kotlin）；本文件只服务于宿主机自检。
 */
public final class NrCore {

    private NrCore() {}

    public static native int nr_abi_version();

    public static native long nr_engine_create(long maxWindowBytes, byte[] dbPath,
                                               NrSourceProvider provider);

    public static native int nr_engine_destroy(long engine);

    public static native int nr_open_txt(long engine, int kind, long totalBytes, byte[] identity,
                                         byte[] displayName, byte[] outResult);

    public static native int nr_close_book(long engine, long bookId);

    public static native byte[] nr_read_window(long engine, int chapterIndex, long offset,
                                               long length, byte[] outWindow, long[] outBuffer);

    public static native int nr_submit_page_boundary(long engine, int phase, long contentIndexVersion,
                                                     long layoutId, long requestId, long totalBytes,
                                                     long startByte, long endByte, long[] layoutFields,
                                                     byte[] fontFamily, byte[] outResult);

    public static native int nr_get_page_boundary(long engine, long layoutId, int chapterIndex,
                                                  long startByte, byte[] outResult);

    public static native int nr_save_position(long engine, int chapterIndex, long bookId,
                                              long contentUnitId, long originalByteOffset,
                                              byte[] fingerprint, int[] outChanged);

    public static native byte[] nr_get_position(long engine, long bookId, byte[] outResult,
                                                long[] outBuffer);

    public static native byte[] nr_get_chapters(long engine, long bookId, int startSequence,
                                                int maxCount, byte[] outBatch, long[] outBuffer);

    public static native int nr_create_bookmark(long engine, int chapterIndex, long bookId,
                                                long contentUnitId, long byteOffset, byte[] title,
                                                long[] outBookmarkId);

    public static native int nr_delete_bookmark(long engine, long bookmarkId, int[] outDeleted);

    public static native int nr_invalidate_layout(long engine, long layoutId, long[] outRemoved);

    public static native int nr_rebuild_index(long engine, long bookId, long requestId,
                                              byte[] outResult);

    public static native int nr_cancel_request(long engine, long requestId);

    public static native byte[] nr_get_last_error(long engine, byte[] outInfo, long[] outBuffer);

    public static native int nr_release_buffer(long ptr, long len, long capacity);
}
