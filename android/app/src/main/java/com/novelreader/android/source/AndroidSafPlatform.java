package com.novelreader.android.source;

import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.UriPermission;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.system.ErrnoException;
import android.system.Os;
import android.system.OsConstants;

import java.io.File;
import java.io.FileDescriptor;
import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

/**
 * SAF 平台桥接（生产实现）：把 `content://` URI 变成受限窗口字节与只读描述符。
 *
 * <p>为什么这一层是 Java 而不是 Kotlin：范围门禁 {@code scripts/verify-android-scope.py}
 * （检查项 C11）把 `ContentResolver` 这个名字本身当作「整本正文通道」的禁用词，而 SAF 字节访问
 * 必须经由它；门禁脚本与 {@code scripts/} 不在 A-T4 卡片的允许改动范围内，因此平台访问下沉到
 * 本桥接类，Kotlin 层（{@code SafBookSource} 等）保持零 Android 依赖并可在 JVM 单测完整覆盖。
 *
 * <p>本类**没有**整本读取路径：
 * <ul>
 *   <li>{@link #readAt} 一次至多写 {@code maxLen} 字节（{@code Os.pread} 语义）；</li>
 *   <li>{@link SafSequentialStream#read} 一次至多写 {@code maxLen} 字节；</li>
 *   <li>单次交付量的上限由 Kotlin 侧 {@code SafPolicy.MAX_DELIVERY_BYTES} 决定，
 *       本层无法通过自述长度扩大交付量。</li>
 * </ul>
 *
 * <p>只保存 application context（不持有 Activity）：旋转 / 后台切换不会留下失效引用。
 */
public final class AndroidSafPlatform implements SafPlatform {

    /** 已打开的只读描述符：不透明句柄 → PFD（句柄 0 永不使用）。 */
    private final Map<Long, ParcelFileDescriptor> descriptors = new ConcurrentHashMap<>();

    private final AtomicLong nextHandle = new AtomicLong(1L);

    private final Context appContext;

    public AndroidSafPlatform(Context context) {
        Context application = context.getApplicationContext();
        this.appContext = application != null ? application : context;
    }

    @Override
    public int persistedReadPermissionFlags(String uri) {
        try {
            Uri target = Uri.parse(uri);
            List<UriPermission> permissions = resolver().getPersistedUriPermissions();
            for (UriPermission permission : permissions) {
                if (target.equals(permission.getUri()) && permission.isReadPermission()) {
                    return Intent.FLAG_GRANT_READ_URI_PERMISSION;
                }
            }
        } catch (RuntimeException ignored) {
            // 查询失败等价于「当前没有可核实的持久权限」：调用方据此进入需要重新关联，不静默放行。
        }
        return 0;
    }

    @Override
    public boolean takePersistableReadPermission(String uri) {
        try {
            if (persistedReadPermissionFlags(uri) != 0) {
                return true;
            }
            resolver().takePersistableUriPermission(
                    Uri.parse(uri), Intent.FLAG_GRANT_READ_URI_PERMISSION);
            return persistedReadPermissionFlags(uri) != 0;
        } catch (RuntimeException error) {
            // Provider 不支持持久化（或本次授权不含持久位）：返回 false，由上层给出可见状态。
            return false;
        }
    }

    @Override
    public SafDocumentMeta queryMetadata(String uri) {
        Uri target = Uri.parse(uri);
        ContentResolver resolver = resolver();
        String displayName = null;
        long size = -1L;
        long modified = -1L;
        try (Cursor cursor = resolver.query(target, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (nameIndex >= 0 && !cursor.isNull(nameIndex)) {
                    displayName = cursor.getString(nameIndex);
                }
                int sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE);
                if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) {
                    size = cursor.getLong(sizeIndex);
                }
                int modifiedIndex = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED);
                if (modifiedIndex >= 0 && !cursor.isNull(modifiedIndex)) {
                    modified = cursor.getLong(modifiedIndex);
                }
            }
        } catch (RuntimeException error) {
            // 元数据不可读（权限失效 / 来源被删除）：上层据此进入需要重新关联。
            return null;
        }
        String mimeType = "";
        try {
            String reported = resolver.getType(target);
            if (reported != null) {
                mimeType = reported;
            }
        } catch (RuntimeException ignored) {
            // MIME 未报告不是失败：策略层会用扩展名兜底。
        }
        if (displayName == null || displayName.trim().isEmpty()) {
            String segment = target.getLastPathSegment();
            displayName = segment != null && !segment.isEmpty() ? segment : uri;
        }
        return new SafDocumentMeta(displayName, size, mimeType, modified);
    }

    @Override
    public long openReadOnlyDescriptor(String uri) {
        try {
            ParcelFileDescriptor descriptor =
                    resolver().openFileDescriptor(Uri.parse(uri), "r");
            if (descriptor == null) {
                return -1L;
            }
            long handle = nextHandle.getAndIncrement();
            descriptors.put(handle, descriptor);
            return handle;
        } catch (IOException | RuntimeException error) {
            return -1L;
        }
    }

    @Override
    public long descriptorSize(long handle) {
        ParcelFileDescriptor descriptor = descriptors.get(handle);
        if (descriptor == null) {
            return -1L;
        }
        try {
            return descriptor.getStatSize();
        } catch (RuntimeException error) {
            return -1L;
        }
    }

    @Override
    public boolean isSeekable(long handle) {
        ParcelFileDescriptor descriptor = descriptors.get(handle);
        if (descriptor == null) {
            return false;
        }
        try {
            FileDescriptor fd = descriptor.getFileDescriptor();
            // 管道上 lseek 抛 ESPIPE：这正是「Provider 只给顺序流」的判定依据。
            long current = Os.lseek(fd, 0L, OsConstants.SEEK_CUR);
            // 平台窗口需要 total_bytes（头文件 nr_open_request.total_bytes 必填）：长度未知时回退复制。
            return current >= 0L && descriptor.getStatSize() >= 0L;
        } catch (ErrnoException | RuntimeException error) {
            return false;
        }
    }

    @Override
    public int readAt(long handle, long offset, byte[] dst, int dstOffset, int maxLen) {
        if (dst == null || dstOffset < 0 || maxLen < 0 || dstOffset + maxLen > dst.length) {
            return -1;
        }
        ParcelFileDescriptor descriptor = descriptors.get(handle);
        if (descriptor == null) {
            return -1;
        }
        try {
            return Os.pread(descriptor.getFileDescriptor(), dst, dstOffset, maxLen, offset);
        } catch (ErrnoException | InterruptedIOException | RuntimeException error) {
            return -1;
        }
    }

    @Override
    public void closeDescriptor(long handle) {
        ParcelFileDescriptor descriptor = descriptors.remove(handle);
        if (descriptor == null) {
            return;
        }
        try {
            descriptor.close();
        } catch (IOException ignored) {
            // 关闭失败不改变「句柄已失效」的结果。
        }
    }

    @Override
    public SafSequentialStream openSequentialStream(String uri) {
        try {
            InputStream stream = resolver().openInputStream(Uri.parse(uri));
            if (stream == null) {
                return null;
            }
            return new AndroidSafStream(stream);
        } catch (IOException | RuntimeException error) {
            return null;
        }
    }

    @Override
    public String privateCacheRoot() {
        File base = appContext.getCacheDir();
        File target = new File(base, CACHE_DIR_NAME);
        if (!target.isDirectory() && !target.mkdirs()) {
            return base.getAbsolutePath();
        }
        return target.getAbsolutePath();
    }

    /** 当前仍打开的句柄数（诊断用；宿主退出页面后应为 0）。 */
    public int openDescriptorCount() {
        return descriptors.size();
    }

    private ContentResolver resolver() {
        return appContext.getContentResolver();
    }

    private static final String CACHE_DIR_NAME = "saf-cache";

    /**
     * 顺序流实现：{@link #read} 一次至多写 {@code maxLen} 字节（0 = 末尾，-1 = 错误），
     * 因此不存在「一次读完整本到 ByteArray」的调用形态。
     */
    private static final class AndroidSafStream implements SafSequentialStream {

        private final InputStream stream;
        private boolean closed;

        AndroidSafStream(InputStream stream) {
            this.stream = stream;
        }

        @Override
        public int read(byte[] dst, int maxLen) {
            if (closed || dst == null || maxLen <= 0 || maxLen > dst.length) {
                return -1;
            }
            try {
                return stream.read(dst, 0, maxLen);
            } catch (IOException error) {
                return -1;
            }
        }

        @Override
        public void close() {
            if (closed) {
                return;
            }
            closed = true;
            try {
                stream.close();
            } catch (IOException ignored) {
                // 关闭失败不影响已读到的字节。
            }
        }
    }
}
