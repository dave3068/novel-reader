package com.novelreader.android.core;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;

/**
 * 宿主机 JNI 层自检（AN1）：真实加载 libnr_jni_host.so 并调用 18 项入口。
 *
 * 覆盖：ABI 版本、引擎创建/销毁（含重复销毁）、文件数据源打开、窗口读取 +
 * 缓冲区归还（含重复归还拒绝）、章节批次、页边界 BEGIN/COMMIT 与非法 UTF-8 边界、
 * 位置保存/恢复、书签增删、索引重建、取消、最近错误、平台（SAF 语义）窗口回调
 * 与超长回报拒绝。
 *
 * 用法：java -cp <classes> com.novelreader.android.core.HostSmoke <libnr_jni_host.so> <工作目录>
 * 退出码：0 = 全部断言通过；1 = 存在失败断言。
 */
public final class HostSmoke {

    private static final int NR_ERR_OK = 0;
    private static final int NR_ERR_INVALID_ARGUMENT = 1;
    private static final int NR_ERR_ENGINE_DESTROYED = 9;
    private static final int NR_SOURCE_KIND_FILE = 1;
    private static final int NR_SOURCE_KIND_PLATFORM = 2;
    private static final int NR_SUBMIT_BEGIN_LAYOUT = 1;
    private static final int NR_SUBMIT_COMMIT = 2;
    private static final int NR_BOUNDARY_LAYOUT_READY = 100;
    private static final int NR_BOUNDARY_COMMITTED = 0;
    private static final int NR_BOUNDARY_DUPLICATE_IDENTICAL = 1;
    private static final int NR_BOUNDARY_REJECTED = 2;

    private static final String FIXTURE_TEXT =
            "第一章 起风\n"
            + "\n"
            + "这是一段中文正文，用于验证 JNI 层与 Rust 核心的窗口读取、分页边界与进度保存。\n"
            + "第二行正文，用于确定合法 UTF-8 页边界的换行符。\n"
            + "\n"
            + "第二章 落雨\n"
            + "\n"
            + "又一段正文，确保章节索引至少识别出两章。\n"
            + "第三行正文。\n"
            + "\n"
            + "第三章 归途\n"
            + "\n"
            + "最后一段正文。\n";

    private static int checks = 0;
    private static int failures = 0;

    private static void check(String label, boolean ok) {
        checks++;
        if (ok) {
            System.out.println("NR-HOST ok   " + label);
        } else {
            failures++;
            System.out.println("NR-HOST FAIL " + label);
        }
    }

    private static void note(String text) {
        System.out.println("NR-HOST note " + text);
    }

    private static ByteBuffer le(byte[] bytes) {
        return ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN);
    }

    private static int indexOf(byte[] haystack, byte needle) {
        for (int i = 0; i < haystack.length; i++) {
            if (haystack[i] == needle) {
                return i;
            }
        }
        return -1;
    }

    /** 三字节 UTF-8 起首字节之后的第一个位置：一定落在字符中间（非法页边界）。 */
    private static int midOfThreeByteChar(byte[] bytes) {
        for (int i = 0; i + 1 < bytes.length; i++) {
            int b = bytes[i] & 0xFF;
            if (b >= 0xE0 && b <= 0xEF) {
                return i + 1;
            }
        }
        return -1;
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 2) {
            System.err.println("用法: HostSmoke <libnr_jni_host.so> <工作目录>");
            System.exit(2);
        }
        Path so = Paths.get(args[0]).toAbsolutePath();
        Path work = Files.createDirectories(Paths.get(args[1]));
        System.load(so.toString());
        note("已加载 " + so);

        Path fixture = work.resolve("smoke.txt");
        Files.write(fixture, FIXTURE_TEXT.getBytes(StandardCharsets.UTF_8));
        byte[] fileBytes = Files.readAllBytes(fixture);
        long fileLen = fileBytes.length;
        Path dbPath = work.resolve("smoke.sqlite");

        // ---------------- 1. ABI 版本 ----------------
        int abi = NrCore.nr_abi_version();
        check("nr_abi_version() == 1（实际 " + abi + "）", abi == 1);

        // ---------------- 2. 引擎（文件数据源） ----------------
        long engine = NrCore.nr_engine_create(65536L, dbPath.toString().getBytes(StandardCharsets.UTF_8), null);
        check("nr_engine_create(db_path, 无 provider) 返回非 0 句柄", engine != 0);

        // ---------------- 3. 打开 TXT ----------------
        byte[] open = new byte[56];
        int rcOpen = NrCore.nr_open_txt(engine, NR_SOURCE_KIND_FILE, 0L,
                fixture.toString().getBytes(StandardCharsets.UTF_8), null, open);
        check("nr_open_txt(FILE) == 0（实际 " + rcOpen + "）", rcOpen == NR_ERR_OK);
        ByteBuffer openBuf = le(open);
        long bookId = openBuf.getLong(0);
        long totalBytes = openBuf.getLong(8);
        long indexVersion = openBuf.getLong(16);
        int contentUnit = openBuf.getInt(24);
        int encoding = openBuf.getInt(28);
        int chapters = openBuf.getInt(44);
        int indexFromCache = openBuf.getInt(48);
        note("open_result: book_id=" + bookId + " total_bytes=" + totalBytes + " index_version="
                + indexVersion + " content_unit=" + contentUnit + " encoding=" + encoding
                + " chapters=" + chapters + " index_from_cache=" + indexFromCache);
        check("book_id != 0（64 位未截断）", bookId != 0);
        check("total_bytes == 文件长度（" + totalBytes + " vs " + fileLen + "）", totalBytes == fileLen);
        check("content_unit_id == 1（NR_CONTENT_UNIT_SINGLE）", contentUnit == 1);
        check("chapters >= 2", chapters >= 2);

        // ---------------- 4. 窗口读取 + 缓冲区归还 ----------------
        byte[] window = new byte[48];
        long[] buffer = new long[3];
        byte[] bytes = NrCore.nr_read_window(engine, 0, 0L, 4096L, window, buffer);
        check("nr_read_window 返回正文（非 null）", bytes != null);
        ByteBuffer winBuf = le(window);
        long winOffset = winBuf.getLong(0);
        long winEnd = winBuf.getLong(8);
        int winLen = winBuf.getInt(32);
        int droppedLeading = winBuf.getInt(36);
        int trimmedTail = winBuf.getInt(40);
        int eof = winBuf.getInt(44);
        note("window: offset=" + winOffset + " end=" + winEnd + " len=" + winLen + " dropped="
                + droppedLeading + " trimmed=" + trimmedTail + " eof=" + eof);
        check("窗口 len == 返回数组长度", bytes != null && winLen == bytes.length);
        check("窗口 end_byte == offset + len", winEnd == winOffset + winLen);
        check("窗口 offset == 请求 offset", winOffset == 0L);
        check("正文为合法 UTF-8 中文", bytes != null
                && new String(bytes, StandardCharsets.UTF_8).contains("这是一段中文正文"));
        check("缓冲区登记 ptr 非 0 且 len > 0", buffer[0] != 0 && buffer[1] > 0);
        check("归还槽位 len == 返回数组长度", bytes != null && buffer[1] == bytes.length);
        int release1 = NrCore.nr_release_buffer(buffer[0], buffer[1], buffer[2]);
        check("nr_release_buffer 首次归还 == 0（实际 " + release1 + "）", release1 == NR_ERR_OK);
        int release2 = NrCore.nr_release_buffer(buffer[0], buffer[1], buffer[2]);
        check("重复归还被拒绝为 NR_ERR_INVALID_ARGUMENT（未二次释放）",
                release2 == NR_ERR_INVALID_ARGUMENT);
        check("nr_release_buffer(0,0,0) 是安全 no-op",
                NrCore.nr_release_buffer(0L, 0L, 0L) == NR_ERR_OK);
        check("归还字段不一致被拒绝",
                NrCore.nr_release_buffer(buffer[0], buffer[1], buffer[2] + 1) == NR_ERR_INVALID_ARGUMENT);

        // ---------------- 5. 章节批次 ----------------
        byte[] batch = new byte[40];
        long[] chapterBuffer = new long[3];
        byte[] chapterBytes = NrCore.nr_get_chapters(engine, bookId, 0, 8, batch, chapterBuffer);
        check("nr_get_chapters 返回批次字节（非 null）", chapterBytes != null);
        ByteBuffer batchBuf = le(batch);
        int count = batchBuf.getInt(0);
        int batchIndexVersion = batchBuf.getInt(4);
        int totalChapters = batchBuf.getInt(8);
        note("chapter_batch: count=" + count + " total=" + totalChapters + " index_version="
                + batchIndexVersion + " bytes=" + (chapterBytes == null ? -1 : chapterBytes.length));
        check("批次 count >= 1", count >= 1);
        check("批次 total_chapters >= 2", totalChapters >= 2);
        check("批次字节长度 == 登记 len",
                chapterBytes != null && chapterBuffer[1] == chapterBytes.length);
        String firstTitle = "";
        if (chapterBytes != null && count >= 1) {
            ByteBuffer first = le(Arrays.copyOfRange(chapterBytes, 0, 40));
            int titleOffset = first.getInt(24);
            int titleLen = first.getInt(28);
            note("章节条目: sequence=" + first.getInt(0) + " start=" + first.getLong(8)
                    + " end=" + first.getLong(16) + " title_offset=" + titleOffset
                    + " title_len=" + titleLen);
            if (titleOffset >= 0 && titleLen > 0 && titleOffset + titleLen <= chapterBytes.length) {
                firstTitle = new String(chapterBytes, titleOffset, titleLen, StandardCharsets.UTF_8);
            }
        }
        note("第一章标题 = " + firstTitle);
        check("第一章标题包含「第一章」", firstTitle.contains("第一章"));
        check("章节批次缓冲区归还 == 0",
                chapterBuffer[0] == 0
                        ? true
                        : NrCore.nr_release_buffer(chapterBuffer[0], chapterBuffer[1], chapterBuffer[2]) == 0);

        // ---------------- 6. 页边界 BEGIN / COMMIT ----------------
        byte[] boundary = new byte[80];
        long[] layoutFields = new long[13];
        layoutFields[0] = 1080;   // viewport_width
        layoutFields[1] = 720;    // viewport_height
        layoutFields[2] = 0;      // direction 横排
        layoutFields[3] = 400;    // font_weight
        layoutFields[4] = 18000;  // font_size_milli (18.0 px)
        layoutFields[5] = 1400;   // line_spacing_milli
        layoutFields[6] = 0;      // paragraph_spacing_milli
        layoutFields[7] = 0;      // first_line_indent_milli
        layoutFields[8] = 0;      // margin_horizontal_milli
        layoutFields[9] = 0;      // margin_vertical_milli
        layoutFields[10] = 1;     // algorithm_version
        layoutFields[11] = 0;     // reserved0
        layoutFields[12] = 0;     // platform_font_fingerprint
        byte[] fontFamily = "sans-serif".getBytes(StandardCharsets.UTF_8);
        int rcBegin = NrCore.nr_submit_page_boundary(engine, NR_SUBMIT_BEGIN_LAYOUT, indexVersion,
                0L, 0L, totalBytes, 0L, 0L, layoutFields, fontFamily, boundary);
        ByteBuffer beginBuf = le(boundary);
        int beginStatus = beginBuf.getInt(0);
        int beginError = beginBuf.getInt(8);
        long layoutId = beginBuf.getLong(16);
        long requestId = beginBuf.getLong(24);
        note("BEGIN_LAYOUT: rc=" + rcBegin + " status=" + beginStatus + " error=" + beginError
                + " layout_id=" + layoutId + " request_id=" + requestId);
        check("BEGIN_LAYOUT == 0", rcBegin == NR_ERR_OK);
        check("BEGIN_LAYOUT status == " + NR_BOUNDARY_LAYOUT_READY, beginStatus == NR_BOUNDARY_LAYOUT_READY);
        check("layout_id / request_id 非 0", layoutId != 0 && requestId != 0);

        int newline = bytes == null ? -1 : indexOf(bytes, (byte) '\n');
        long pageEnd = newline >= 0 ? winOffset + newline + 1 : -1;
        check("窗口内找到换行符（可用于合法页末）", pageEnd > 0);
        byte[] commit = new byte[80];
        int rcCommit = NrCore.nr_submit_page_boundary(engine, NR_SUBMIT_COMMIT, indexVersion, layoutId,
                requestId, totalBytes, 0L, pageEnd, null, null, commit);
        int commitStatus = le(commit).getInt(0);
        int commitError = le(commit).getInt(8);
        long committedEnd = le(commit).getLong(56);
        note("COMMIT(0," + pageEnd + "): rc=" + rcCommit + " status=" + commitStatus + " error="
                + commitError + " end_byte=" + committedEnd);
        check("COMMIT(合法 UTF-8 页末) == 0", rcCommit == NR_ERR_OK);
        check("COMMIT status 属于已登记（0/1）",
                commitStatus == NR_BOUNDARY_COMMITTED || commitStatus == NR_BOUNDARY_DUPLICATE_IDENTICAL);

        byte[] committed = new byte[80];
        int rcGet = NrCore.nr_get_page_boundary(engine, layoutId, 0, 0L, committed);
        int getStatus = le(committed).getInt(0);
        long gotEnd = le(committed).getLong(56);
        note("GET_PAGE_BOUNDARY: rc=" + rcGet + " status=" + getStatus + " end_byte=" + gotEnd);
        check("nr_get_page_boundary == 0", rcGet == NR_ERR_OK);
        check("查询到的页末与提交一致", gotEnd == pageEnd);

        int badEnd = midOfThreeByteChar(bytes);
        byte[] bad = new byte[80];
        int rcBad = NrCore.nr_submit_page_boundary(engine, NR_SUBMIT_COMMIT, indexVersion, layoutId,
                requestId, totalBytes, 0L, winOffset + badEnd, null, null, bad);
        int badStatus = le(bad).getInt(0);
        int badError = le(bad).getInt(8);
        note("COMMIT(非法 UTF-8 边界 @" + (winOffset + badEnd) + "): rc=" + rcBad + " status="
                + badStatus + " error=" + badError + " rejection=" + le(bad).getInt(4));
        check("非法 UTF-8 边界被拒绝（rc != 0 或 status == REJECTED）",
                rcBad != NR_ERR_OK || badStatus == NR_BOUNDARY_REJECTED);

        // ---------------- 7. 位置保存 / 恢复 ----------------
        int[] changed = new int[1];
        int rcSave = NrCore.nr_save_position(engine, 0, bookId, 1L, pageEnd, null, changed);
        check("nr_save_position == 0（实际 " + rcSave + "）", rcSave == NR_ERR_OK);
        check("out_changed == 1", changed[0] == 1);
        byte[] position = new byte[80];
        long[] positionBuffer = new long[3];
        byte[] fingerprint = NrCore.nr_get_position(engine, bookId, position, positionBuffer);
        ByteBuffer posBuf = le(position);
        note("position: found=" + posBuf.getInt(4) + " chapter=" + posBuf.getInt(8)
                + " offset=" + posBuf.getLong(40) + " fingerprint="
                + (fingerprint == null ? -1 : fingerprint.length));
        check("nr_get_position 返回非 null", fingerprint != null);
        check("found == 1", posBuf.getInt(4) == 1);
        check("恢复 offset == 保存 offset", posBuf.getLong(40) == pageEnd);
        check("恢复 chapter_index == 0", posBuf.getInt(8) == 0);
        check("位置结果内嵌缓冲区字段与归还槽位一致",
                posBuf.getLong(64) == positionBuffer[1] && posBuf.getLong(72) == positionBuffer[2]);
        if (positionBuffer[0] != 0) {
            check("位置指纹缓冲区归还 == 0",
                    NrCore.nr_release_buffer(positionBuffer[0], positionBuffer[1], positionBuffer[2]) == 0);
        } else {
            check("位置指纹为空（无需归还）", true);
        }

        // ---------------- 8. 书签 ----------------
        long[] bookmarkId = new long[1];
        int rcBookmark = NrCore.nr_create_bookmark(engine, 0, bookId, 1L, pageEnd,
                "冒烟书签".getBytes(StandardCharsets.UTF_8), bookmarkId);
        check("nr_create_bookmark == 0 且 id != 0（实际 rc=" + rcBookmark + " id=" + bookmarkId[0] + "）",
                rcBookmark == NR_ERR_OK && bookmarkId[0] != 0);
        int[] deleted = new int[1];
        int rcDelete = NrCore.nr_delete_bookmark(engine, bookmarkId[0], deleted);
        check("nr_delete_bookmark == 0 且 deleted == 1",
                rcDelete == NR_ERR_OK && deleted[0] == 1);

        // ---------------- 9. 索引重建 / 取消 / 最近错误 ----------------
        byte[] rebuild = new byte[40];
        int rcRebuild = NrCore.nr_rebuild_index(engine, bookId, 4242L, rebuild);
        ByteBuffer rebuildBuf = le(rebuild);
        note("rebuild: rc=" + rcRebuild + " partial=" + rebuildBuf.getInt(4) + " chapters="
                + rebuildBuf.getInt(8) + " scanned=" + rebuildBuf.getLong(16));
        check("nr_rebuild_index == 0", rcRebuild == NR_ERR_OK);
        check("rebuild.partial == 0", rebuildBuf.getInt(4) == 0);
        check("nr_cancel_request(未启动的 request_id) == 0",
                NrCore.nr_cancel_request(engine, 12345L) == NR_ERR_OK);
        check("nr_cancel_request(0) 被拒绝为 NR_ERR_INVALID_ARGUMENT",
                NrCore.nr_cancel_request(engine, 0L) == NR_ERR_INVALID_ARGUMENT);
        byte[] errorInfo = new byte[32];
        long[] errorBuffer = new long[3];
        byte[] message = NrCore.nr_get_last_error(engine, errorInfo, errorBuffer);
        int lastErrorCode = le(errorInfo).getInt(0);
        note("last_error: code=" + lastErrorCode + " message="
                + (message == null ? "null" : new String(message, StandardCharsets.UTF_8)));
        check("nr_get_last_error 返回非 null", message != null);
        check("last_error 记录了最近一次失败（非法 UTF-8 边界的 NR_ERR_UTF8_BOUNDARY=3）",
                lastErrorCode == 3);
        check("last_error.has_message == 1 且消息非空",
                le(errorInfo).getInt(4) == 1 && message != null && message.length > 0);
        if (errorBuffer[0] != 0) {
            NrCore.nr_release_buffer(errorBuffer[0], errorBuffer[1], errorBuffer[2]);
        }

        // ---------------- 10. 平台（SAF 语义）数据源 ----------------
        Provider provider = new Provider(fileBytes);
        long platformEngine = NrCore.nr_engine_create(65536L, null, provider);
        check("nr_engine_create(provider) 返回非 0 句柄", platformEngine != 0);
        byte[] platformOpen = new byte[56];
        int rcPlatformOpen = NrCore.nr_open_txt(platformEngine, NR_SOURCE_KIND_PLATFORM, fileLen,
                "content://smoke/book-1".getBytes(StandardCharsets.UTF_8),
                "smoke.txt".getBytes(StandardCharsets.UTF_8), platformOpen);
        check("nr_open_txt(PLATFORM) == 0（实际 " + rcPlatformOpen + "）", rcPlatformOpen == NR_ERR_OK);
        check("平台回调至少被调用一次（calls=" + provider.calls + "）", provider.calls > 0);
        long platformBook = le(platformOpen).getLong(0);
        byte[] platformWindow = new byte[48];
        long[] platformBuffer = new long[3];
        byte[] platformBytes = NrCore.nr_read_window(platformEngine, 0, 0L, 4096L, platformWindow,
                platformBuffer);
        check("平台数据源窗口读取非 null", platformBytes != null);
        check("平台与文件数据源读取结果一致",
                platformBytes != null && bytes != null
                        && Arrays.equals(platformBytes, Arrays.copyOf(bytes, platformBytes.length)));
        if (platformBuffer[0] != 0) {
            NrCore.nr_release_buffer(platformBuffer[0], platformBuffer[1], platformBuffer[2]);
        }
        provider.overReport = true;
        byte[] overWindow = new byte[48];
        long[] overBuffer = new long[3];
        byte[] overBytes = NrCore.nr_read_window(platformEngine, 0, 0L, 4096L, overWindow, overBuffer);
        byte[] overError = new byte[32];
        long[] overErrorBuffer = new long[3];
        NrCore.nr_get_last_error(platformEngine, overError, overErrorBuffer);
        int overCode = le(overError).getInt(0);
        note("平台超长回报：返回=" + (overBytes == null ? "null" : overBytes.length + "B")
                + " last_error=" + overCode + "（期望被拒绝）");
        check("平台回报超长窗口被拒绝（返回 null 且 last_error != 0）",
                overBytes == null && overCode != NR_ERR_OK);
        if (overErrorBuffer[0] != 0) {
            NrCore.nr_release_buffer(overErrorBuffer[0], overErrorBuffer[1], overErrorBuffer[2]);
        }

        // ---------------- 11. 关闭与销毁 ----------------
        check("nr_engine_destroy(platformEngine) == 0",
                NrCore.nr_engine_destroy(platformEngine) == NR_ERR_OK);
        check("重复销毁 platformEngine 返回 NR_ERR_ENGINE_DESTROYED",
                NrCore.nr_engine_destroy(platformEngine) == NR_ERR_ENGINE_DESTROYED);
        check("nr_close_book == 0", NrCore.nr_close_book(engine, bookId) == NR_ERR_OK);
        check("nr_engine_destroy(engine) == 0", NrCore.nr_engine_destroy(engine) == NR_ERR_OK);
        byte[] afterDestroy = new byte[80];
        check("销毁后调用返回 NR_ERR_ENGINE_DESTROYED",
                NrCore.nr_get_page_boundary(engine, layoutId, 0, 0L, afterDestroy)
                        == NR_ERR_ENGINE_DESTROYED);

        System.out.println("HOST-SMOKE RESULT: " + (checks - failures) + "/" + checks
                + " 项通过（失败 " + failures + "）");
        System.out.println("HOST-SMOKE EXIT: " + (failures == 0 ? 0 : 1));
        if (failures != 0) {
            System.exit(1);
        }
    }

    /** SAF 语义的窗口提供者：把文件字节按窗口交给 Rust。 */
    static final class Provider implements NrSourceProvider {
        private final byte[] data;
        int calls = 0;
        boolean overReport = false;

        Provider(byte[] data) {
            this.data = data;
        }

        @Override
        public int readWindow(long requestId, long offset, int maxLength, byte[] dst, int[] outInfo) {
            calls++;
            if (offset >= data.length) {
                outInfo[0] = 0;
                outInfo[1] = 1;
                return 0;
            }
            int len = (int) Math.min(maxLength, data.length - offset);
            System.arraycopy(data, (int) offset, dst, 0, len);
            outInfo[0] = overReport ? maxLength + 1 : len;
            outInfo[1] = (offset + len >= data.length) ? 1 : 0;
            return 0;
        }
    }
}
