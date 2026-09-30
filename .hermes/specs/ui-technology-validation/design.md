# UI 技术验证共享设计

## 1. 目录与所有权

```text
experiments/ui-selection/
├── shared/
│   ├── rust-reading-core/
│   ├── ffi/include/novel_reader_probe.h
│   ├── contracts/
│   └── fixtures/
├── hybrid-native-probe/
│   ├── desktop-slint/
│   └── android-native/
├── slint-probe/
├── flutter-probe/
├── compose-probe/
├── qt-probe/
├── scripts/
└── results/{raw,normalized}/
```

`shared/` 只由 `shared-foundation` 修改；`hybrid-native-probe` 只修改其桌面和 Android 子目录；四个对照候选各自隔离。候选不得复制共享 Rust 业务逻辑或写入其他候选构建缓存。

## 2. 候选与阶段门禁

1. **Foundation**：共享 Rust 阅读业务库、直接 Rust API、C ABI、夹具和编排脚本通过。
2. **2A 优先候选**：先完成 hybrid-native 的桌面 Slint 分页、Android 原生 Compose/View 分页源码、薄 JNI/C ABI 和共享业务边界检查；当前环境只对桌面运行门禁，Android 状态必须是`未验证`。
3. **2B 对照尖峰**：P2 的源码/桌面证据完成后，Slint、Flutter、Compose、Qt 实现分页尖峰。
4. **候选结果**：每个平台能力只能为`通过`、`有条件通过`、`不通过`、`未验证`。子规格已实现只表示验证程序和证据链完整。
5. **条件完整原型**：仅分页门槛通过的端实现共同三页面。优先候选桌面与 Android 可使用不同原生交互，不要求页边界相等。
6. **当前 Linux 门禁**：hybrid-native 的桌面端和四个对照候选均必须 Release 编译并在 X11 写入就绪 JSON。

## 3. 共享 Rust 阅读业务边界

共享库只实现验证所需的 UI 无关行为：有限窗口、模拟书架/分类/章节/历史、内容位置、分页锚点、页面边界登记、布局失效、缓存策略、取消、错误和事件。它不实现正式数据库、ZIP、网络或最终字形布局。

公开头文件包含：

```c
nr_engine_create(); nr_engine_destroy(); nr_open_book();
nr_read_window(); nr_release_buffer(); nr_get_book_batch();
nr_get_chapter_batch(); nr_save_position();
nr_commit_page_boundary(); nr_invalidate_layout();
nr_subscribe_events(); nr_cancel_request();
```

桌面 hybrid 端调用同一 Rust API；Android 与其他跨语言候选使用 C ABI/JNI。字符串采用 UTF-8 指针加长度，跨边界无语言对象，正文仅有限窗口传输，回调包含请求 ID 且可取消。Android Kotlin 代码只做 UI、排版、线程切换、系统集成和适配，禁止重实现共享业务。

## 4. 分页与跨 UI 语义

每端实现：

```text
layout_page(window, width, height, font, font_size,
            line_spacing, paragraph_spacing, first_line_indent)
 -> consumed_utf8_bytes, visible_line_count, last_line_metrics, layout_time
```

`consumed_utf8_bytes` 在 UTF-8 合法边界；EOF 外必须前进。测试覆盖中文、繁体、标点、中英数、Emoji/组合字符、缺字、超长行、空行、混合换行和首行缩进。

桌面与 Android 允许因字体/排版引擎得出不同页末字节；要求是每端无丢字/重字/拆分/死循环，内容位置与分页锚点能通过共享库恢复相同的原文附近位置。不能以“跨端页码相同”作为验收条件。

## 5. 统一 UI 语义

通过分页门槛的端实现书架（6 层/1000 类/10000 书、虚拟化、搜索）、阅读（三页缓存、翻页、菜单、页码、章节、书签、重排、三种背景）和配置（字体、尺寸、间距、边距、颜色、背景、模式）。Android 可采用平台导航、系统返回、SAF、触摸目标与无障碍语义；桌面可采用键鼠/窗口惯例。自动化检查稳定状态而不是物理点击/拖动。

## 6. 编排与结果

```text
bootstrap-linux.sh --check|--install
build-linux.sh --release <all|hybrid-native|slint|flutter|compose|qt>
test-shared.sh --release
test-pagination.sh <candidate> --pages 1000 [--platform desktop|android]
verify-hybrid-native-contract.py --check-jni --check-no-business-duplication
verify-ui-contract.py --candidate NAME [--platform desktop|android]
smoke-linux.sh --backend x11|wayland --timeout SEC <all|candidate>
verify-results.py --phase pagination --candidates ...
verify-delivery.py
```

`smoke-linux.sh` 等待候选原子写入 `ready.json`，校验候选、平台、框架版本、后端、PID 和事件循环后退出并清理进程树。结果除通用字段外，hybrid-native 记录 `rust_business_coverage`、`platform_specific_loc`、`duplicated_ui_loc`、Android JNI 边界检查与 Android 实测状态。

## 7. 当前环境策略

- 只验收 Linux x86_64 X11 Release 构建/启动，不要求安装包。
- Rust/Cargo 1.98.0 已验证；系统 glibc 2.28、用户兼容层为 glibc 2.41。
- 明确需要新 glibc 时使用 `env -u LD_PRELOAD GLIBC_HOME="$HOME/.local/glibc" "$HOME/.local/bin/glibc-wrap" <binary> ...`；禁止全局 `LD_LIBRARY_PATH`。
- Hermes 导出 SQLite `LD_PRELOAD`，兼容启动必须清除并记录环境。
- 用户 glibc 已通过 Rust、C++、NSS、X11 smoke；这不替代各候选框架启动门禁。
- Android/Windows/Wayland 指南必须写明未在当前环境实测。
