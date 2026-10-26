package com.novelreader.android.source;

import android.content.ContentResolver;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * 阶段 0 同步包的 SAF 字节搬运（Java 平台层）。
 *
 * <p>与 {@link AndroidSafPlatform} 同一分层理由：范围门禁把 ContentResolver/整文件流
 * 视为「整本正文通道」的禁用词，而同步包需要复制非正文文件（state.json、books/*.txt）。
 * 因此平台访问下沉到本类；Kotlin 侧只做暂存目录管理与调度。
 *
 * <p>本类只搬运用户明确选择的同步包目录，不扫描其他目录、不解析 state.json。
 */
public final class SyncSafPlatform {

    public static final String BUNDLE_DIR_NAME = "novel-reader-sync";

    private static final String MIME_DIR = DocumentsContract.Document.MIME_TYPE_DIR;
    private static final String MIME_JSON = "application/json";
    private static final String MIME_TEXT = "text/plain";

    private SyncSafPlatform() {}

    /** 把本地暂存目录复制到 SAF 树下的 novel-reader-sync/（存在则覆盖同名文件）。 */
    public static void copyLocalToTree(Context context, Uri treeUri, File source)
            throws IOException {
        ContentResolver resolver = context.getContentResolver();
        String rootId = DocumentsContract.getTreeDocumentId(treeUri);
        Uri rootUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, rootId);
        String bundleId = findChildId(resolver, treeUri, rootId, BUNDLE_DIR_NAME);
        Uri bundleUri;
        if (bundleId != null) {
            bundleUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, bundleId);
        } else {
            bundleUri = DocumentsContract.createDocument(resolver, rootUri, MIME_DIR, BUNDLE_DIR_NAME);
            if (bundleUri == null) {
                throw new IOException("无法在所选目录创建 " + BUNDLE_DIR_NAME + "/");
            }
        }
        copyLocalDir(resolver, treeUri, bundleUri, source);
    }

    /** 把 SAF 树（同步包目录）复制到本地暂存目录。 */
    public static void copyTreeToLocal(Context context, Uri treeUri, File target)
            throws IOException {
        if (!target.isDirectory() && !target.mkdirs()) {
            throw new IOException("无法创建同步暂存目录：" + target.getAbsolutePath());
        }
        String rootId = DocumentsContract.getTreeDocumentId(treeUri);
        copyTreeDir(context.getContentResolver(), treeUri, rootId, target);
    }

    /** 判断所选树根是否含 state.json（只用于提示，不解析内容）。 */
    public static boolean looksLikeBundle(Context context, Uri treeUri) {
        String rootId = DocumentsContract.getTreeDocumentId(treeUri);
        return findChildId(context.getContentResolver(), treeUri, rootId, "state.json") != null;
    }

    private static void copyLocalDir(ContentResolver resolver, Uri treeUri, Uri parentUri, File source)
            throws IOException {
        File[] children = source.listFiles();
        if (children == null) {
            return;
        }
        java.util.Arrays.sort(children, (a, b) -> a.getName().compareTo(b.getName()));
        for (File child : children) {
            String parentId = DocumentsContract.getDocumentId(parentUri);
            String existing = findChildId(resolver, treeUri, parentId, child.getName());
            if (child.isDirectory()) {
                Uri childUri;
                if (existing != null) {
                    childUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, existing);
                } else {
                    childUri = DocumentsContract.createDocument(resolver, parentUri, MIME_DIR, child.getName());
                    if (childUri == null) {
                        throw new IOException("无法创建目录：" + child.getName());
                    }
                }
                copyLocalDir(resolver, treeUri, childUri, child);
            } else {
                Uri fileUri;
                if (existing != null) {
                    fileUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, existing);
                } else {
                    String mime = child.getName().endsWith(".json") ? MIME_JSON : MIME_TEXT;
                    fileUri = DocumentsContract.createDocument(resolver, parentUri, mime, child.getName());
                    if (fileUri == null) {
                        throw new IOException("无法创建文件：" + child.getName());
                    }
                }
                OutputStream output = resolver.openOutputStream(fileUri, "wt");
                if (output == null) {
                    throw new IOException("无法写入文件：" + child.getName());
                }
                try (OutputStream out = output; InputStream in = new FileInputStream(child)) {
                    copy(in, out);
                }
            }
        }
    }

    private static void copyTreeDir(ContentResolver resolver, Uri treeUri, String parentId, File target)
            throws IOException {
        Uri childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentId);
        String[] projection = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        };
        try (Cursor cursor = resolver.query(childrenUri, projection, null, null, null)) {
            if (cursor == null) {
                throw new IOException("无法列出所选目录（提供者拒绝或权限失效）");
            }
            while (cursor.moveToNext()) {
                String documentId = cursor.getString(0);
                String name = cursor.getString(1);
                String mime = cursor.getString(2);
                if (name == null) {
                    continue;
                }
                if (name.indexOf('/') >= 0 || name.indexOf('\\') >= 0 || name.equals(".") || name.equals("..")) {
                    throw new IOException("同步包含非法文件名：" + name);
                }
                File child = new File(target, name);
                if (MIME_DIR.equals(mime)) {
                    if (!child.isDirectory() && !child.mkdirs()) {
                        throw new IOException("无法创建目录：" + name);
                    }
                    copyTreeDir(resolver, treeUri, documentId, child);
                } else {
                    Uri uri = DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId);
                    InputStream input = resolver.openInputStream(uri);
                    if (input == null) {
                        throw new IOException("无法读取文件：" + name);
                    }
                    try (InputStream in = input; OutputStream out = new FileOutputStream(child)) {
                        copy(in, out);
                    }
                }
            }
        }
        return;
    }

    private static String findChildId(ContentResolver resolver, Uri treeUri, String parentId, String name) {
        Uri childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentId);
        String[] projection = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        };
        try (Cursor cursor = resolver.query(childrenUri, projection, null, null, null)) {
            if (cursor == null) {
                return null;
            }
            while (cursor.moveToNext()) {
                if (name.equals(cursor.getString(1))) {
                    return cursor.getString(0);
                }
            }
        }
        return null;
    }

    private static void copy(InputStream in, OutputStream out) throws IOException {
        byte[] buffer = new byte[64 * 1024];
        int read;
        while ((read = in.read(buffer)) != -1) {
            out.write(buffer, 0, read);
        }
    }
}
