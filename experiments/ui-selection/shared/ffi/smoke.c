// NDK 交叉编译冒烟测试：验证 Android 工具链可编译 Rust 所需 C 库/静态库
// 编译命令（在 NDK clang 环境中）：
//   $NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android31-clang \
//     -shared -fPIC -o libsmoke.so smoke.c
#include <stdint.h>
#include <string.h>

// 简单函数：UTF-8 字节偏移映射冒烟
int64_t nr_utf8_next_boundary(const char* s, int64_t len, int64_t pos) {
    if (pos < 0 || pos >= len) return -1;
    // 跳过后续连续字节（10xxxxxx），停在合法字符边界
    int64_t i = pos + 1;
    while (i < len && (s[i] & 0xC0) == 0x80) i++;
    return i;
}

// 返回窗口内容（供 JNI 层调用验证）
int nr_read_window(const char* src, int64_t srclen, char* out, int64_t* outlen) {
    if (!src || !out || !outlen) return -1;
    int64_t n = *outlen < srclen ? *outlen : srclen;
    memcpy(out, src, n);
    *outlen = n;
    return 0;
}
