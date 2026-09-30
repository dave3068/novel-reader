package com.novelreader.android.core;

/**
 * 平台窗口内容协议（宿主机自检副本；生产 Kotlin 接口见 android/jni/README.md §3）。
 *
 * 只负责字节访问：Rust 决定读取区间与上限，实现者只回报实际写入长度与是否到达末尾。
 */
public interface NrSourceProvider {

    /**
     * 把 [offset, offset + maxLength) 的字节写入 dst，并回报：
     * outInfo[0] = out_len（实际写入长度，必须 &lt;= maxLength）；outInfo[1] = out_eof（0/1）。
     *
     * @return NR_ERR_* 错误码（0 = 成功）
     */
    int readWindow(long requestId, long offset, int maxLength, byte[] dst, int[] outInfo);
}
