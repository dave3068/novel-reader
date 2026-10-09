#!/usr/bin/env python3
"""verify-android-scope.py — Android 生产 Kotlin 的范围门禁（子规格检查项 C11）。

用途（android/spec.md「范围外」/「禁止事项」）：

  证明 Kotlin 生产代码**没有**把核心业务搬进来、也**没有**把整本正文搬过边界：

  [1] 生产 Kotlin 文件集存在（`**/core/` 非空，且含薄封装的四个关键文件）；
  [2] Kotlin **不得复制**章节识别 / 位置校验 / 分页 / 缓存失效 / 书签业务
      （词法层面扫描这些算法的原语与专用标识符；判定依据：这些语义由 Rust `reading-core` 裁定，
      Kotlin 只做装箱/拆箱与编排）；
  [3] Kotlin **不得整本正文传递**：出现整文件读取（`readBytes` / `readText` / `openInputStream`
      / `BufferedReader` / `copyTo` 等）或把整书长度降为 32 位（`totalBytes.toInt()`）即失败；
  [3b] Compose/阅读层（`**/ui/**`、`**/reader/**`）**不得用空回调占位**：控件必须接真实状态
      （形如 `onX = {}` / `= { }` / `-> {}` 的空 lambda 即失败；确实无操作时必须带
      `scope-allow:` 注释说明理由）。`ui/` 与 `reader/` 尚未建立时显式 SKIP；
  [4] `core/` 不得持有 Activity / Compose / ViewModel / Context 状态（值类型入参，不传平台状态）；
  [5] 生产 Kotlin 不得引用 `experiments/` 验证探针路径；
  [6] 正向对照（始终执行）：同一批扫描规则作用在内置 fixture 上必须命中 —— 证明第 2/3/4 项
      不是空检查；
  [7] 宿主接线：`MainActivity.kt` 必须**挂载应用外壳**（`NovelReaderApp(...)`）、注册系统文件
      选择器（`ActivityResultContracts.OpenDocument()`）、把平台来源转发器交给引擎工厂、并传入
      落盘 `dbPath`；同时保留装机可核对的构建标记。反向对照：仅显示标题的占位页必须被判为失败。
      （背景：占位页曾使装机后停在白屏、界面无任何可交互控件 —— 该缺陷由本检查负责防复发。）；
  [11] 编码贯通：`ReaderViewModel.kt` 必须按核心 `nr_open_result.encoding` 解码正文窗口
      （`EncodedTextCodec.decode`，字节映射来自源字节而非重编码），页边界切片必须使用该映射；
      章节标题必须按 **UTF-8** 解码（头文件 §3 字符串约定）；`ReaderScreen.kt` 必须显示编码名。
      反向对照：硬编码 `Charsets.UTF_8` 解码的写法必须被判为失败。
      （背景：核心已实现检测/覆盖并经 ABI 回报，Android 侧未使用 —— 真机实测乱码 + `NR_ERR_UTF8_BOUNDARY`。）；
  [10] 阅读交互层：`ReaderScreen.kt` 必须提供手势翻页（1/3 分带点击 + 横滑，且复用
      `ReaderInteraction` 的纯函数）、中部点击切换界面（沉浸模式）、以及章节目录入口与跳转回调。
      反向对照：只有按钮、没有手势与目录的阅读页必须被判为失败。
      （背景：PoC 有手势而正式工程从未移植，规格 `:620`/`:611` 无人认领 —— 真机实测翻页手势无效。）；
  [9] 宿主 Context 时序：`MainActivity.kt` 的**属性初始化器**不得在构造期访问
      `applicationContext`（Activity 构造早于 `attachBaseContext`，`mBase` 为 null → NPE 闪退）。
      反向对照：`by lazy` 写法必须被判为通过。
      （背景：真机实测「点图标即退出」，根因即此。）；
  [8] 阅读页视口回传必须挂在**始终存在**的正文容器上（`.onSizeChanged` 先于 `page == null` 分支）。
      反向对照：把回传挂进「已有页」分支的死锁写法必须被判为失败。
      （背景：真机实测长期停在「正在排版…（等待正文区域尺寸）」，根因即此死锁。）；
  [13] 阅读时保持屏幕常亮：`MainActivity.kt` 必须持有 `FLAG_KEEP_SCREEN_ON` 的**路由驱动**开关
      （`addFlags` / `clearFlags` 成对，且由 `ui.route` 观测驱动），决策必须落在纯 Kotlin 的
      `ScreenOnController.kt`（仅 `ReaderRoute.READER` 保持常亮，其余路由恢复系统息屏）。
      反向对照：占位页与「只置位、不按路由、从不恢复」的写法必须被判为失败。
      （背景：真机反馈「阅读中很容易息屏」；视频类应用的常亮做法即此。）；
  [14] 书架分类下钻（窄屏，2026-10-09 文件管理器式）：`ShelfScreen.kt` 必须保留**自适应**
      两分支（复用纯函数 `ShelfLayout.isCompact`：窄屏 [CategoryDrill]、宽屏 [CategorySidebar]）
      并接上「逐级返回」与「书籍移动」回调；`CategoryDrill.kt` 必须把**目录排在书籍之前**下发、
      注册系统返回键与「返回上一级」行、在窄屏归一化「全部书籍」位置。
      反向对照：只有侧栏、或把书籍排在目录之前的形状必须被判为失败。
      （背景：左右布局在竖屏手机上过于拥挤 —— 主人真机反馈。）；

`--check-no-whole-file-read` 语义：**只**执行第 3 项（整本正文传递门禁），其余检查打印显式 SKIP，
用于聚焦复核该门禁。默认（不带标志）执行全部检查，第 3 项同样是阻断项。

退出码：0 = 全部通过；1 = 存在违规；2 = 脚本自身前提缺失（Kotlin 目录缺失或无生产源码）。

局限说明：词法扫描不能替代代码审查，它证明的是「生产 Kotlin 里没有出现核心业务算法的原语、
整文件读取通道与整书长度的 32 位降级」。行为级证明由 `android/app/src/test/java` 的 JVM 单测
与 `scripts/verify-android-boundary.py` 承担。
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

CORE_DIR_NAME = "core"
CORE_DEP_RELATIVE = (
    "com/novelreader/android/core/NrCore.kt",
    "com/novelreader/android/core/NrEngine.kt",
    "com/novelreader/android/core/NrBuffers.kt",
    "com/novelreader/android/core/NrError.kt",
)

# [2] 核心阅读业务的原语/专用标识符（大小写不敏感；注释已剥离）。
FORBIDDEN_BUSINESS_TOKENS = (
    # 章节识别
    "chapterregex",
    "chapter_pattern",
    "chapterrule",
    "splitbychapter",
    "splitchapters",
    "detectchapters",
    "chaptertitles",
    "章节识别",
    "章节规则",
    # 位置校验 / 锚点恢复
    "fingerprintmatch",
    "restoreposition",
    "nearbyfingerprintmatch",
    "位置校验",
    "锚点恢复",
    # 分页 / 断行
    "computepage",
    "breakpages",
    "measurepage",
    "paginate(",
    "layouttext(",
    "分页算法",
    "页边界计算",
    # 缓存失效
    "invalidatecache",
    "cachekey",
    "分页缓存键",
    "失效缓存",
    # 书签规则
    "bookmarkdao",
    "bookmarkrule",
    "书签规则",
)

# 上述 token 中需要词边界匹配的短标识符（避免命中无辜子串）。
WORD_BOUNDED_TOKENS = ("cachekey",)

# 章节正则字面量（例如 `Regex("第[0-9]+章")`）：识别规则属于核心。
CHAPTER_PATTERN_RE = re.compile(r"第[^\"']{0,6}[0-9\\d][^\"']{0,6}章")

# [3] 整本正文通道 / 整书长度 32 位降级。
FORBIDDEN_WHOLE_FILE_TOKENS = (
    "readbytes(",
    "readallbytes(",
    "readtext(",
    "readlines(",
    "openinputstream(",
    "fileinputstream(",
    "contentresolver",
    "bufferedreader",
    ".copyto(",
    ".available()",
    "files.newinputstream",
)
WHOLE_FILE_DOWNCAST_RE = re.compile(r"totalBytes\s*\.\s*toInt\s*\(\s*\)")

# [4] core/ 不得持有的平台状态（UI 层自己用这些，核心层只接受值类型）。
FORBIDDEN_PLATFORM_STATE_TOKENS = (
    "androidx.compose",
    "android.app.activity",
    "componentactivity",
    "viewmodel",
    "android.content.context",
    "localcontext",
    "savedstatehandle",
)

# [3b] 空回调占位（Compose/阅读层必须接真实状态）。扫描范围：路径中含 `ui/` 或 `reader/` 的文件。
UI_LAYER_DIR_NAMES = ("ui", "reader")
EMPTY_LAMBDA_RE = re.compile(r"[=:]\s*\{\s*\}|->\s*\{\s*\}")
SCOPE_ALLOW_MARKER = "scope-allow:"

# [5] 探针路径。
PROBE_PATH_TOKEN = "experiments/"

FAILURES: list[str] = []
SKIPS: list[str] = []
PASSES = [0]


def ok(message: str) -> None:
    PASSES[0] += 1
    print(f"  PASS  {message}")


def fail(message: str) -> None:
    FAILURES.append(message)
    print(f"  FAIL  {message}")


def skip(message: str) -> None:
    SKIPS.append(message)
    print(f"  SKIP  {message}")


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def strip_kotlin_comments(text: str) -> str:
    """剥离 Kotlin 注释（`//...` 与可嵌套的 `/* ... */`），保留代码与字符串字面量。

    注释不是可执行代码：边界说明文档里出现「章节识别」等词不应被判成违规；但字符串字面量
    可能是真正生效的模式串（例如章节正则），因此保留。
    """
    out: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        char = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if char == "/" and nxt == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if char == "/" and nxt == "*":
            depth = 1
            i += 2
            while i < n and depth > 0:
                if text[i] == "/" and i + 1 < n and text[i + 1] == "*":
                    depth += 1
                    i += 2
                elif text[i] == "*" and i + 1 < n and text[i + 1] == "/":
                    depth -= 1
                    i += 2
                else:
                    i += 1
            continue
        out.append(char)
        i += 1
    return "".join(out)


def kotlin_files(directory: Path) -> dict[Path, str]:
    """生产 Kotlin 文件 → 去注释代码（相对路径无关）。"""
    files: dict[Path, str] = {}
    if not directory.is_dir():
        return files
    for path in sorted(directory.rglob("*.kt")):
        files[path] = strip_kotlin_comments(path.read_text(encoding="utf-8", errors="replace"))
    return files


def core_files(files: dict[Path, str]) -> dict[Path, str]:
    return {path: text for path, text in files.items() if path.parent.name == CORE_DIR_NAME}


def scan_tokens(files: dict[Path, str], tokens: tuple[str, ...]) -> list[tuple[Path, int, str]]:
    hits: list[tuple[Path, int, str]] = []
    for path, text in files.items():
        for lineno, line in enumerate(text.splitlines(), start=1):
            lowered = line.lower()
            for token in tokens:
                if token in WORD_BOUNDED_TOKENS:
                    if re.search(rf"\b{re.escape(token)}\b", lowered):
                        hits.append((path, lineno, token))
                elif token in lowered:
                    hits.append((path, lineno, token))
    return hits


def scan_regex(files: dict[Path, str], pattern: re.Pattern[str]) -> list[tuple[Path, int, str]]:
    hits: list[tuple[Path, int, str]] = []
    for path, text in files.items():
        for lineno, line in enumerate(text.splitlines(), start=1):
            match = pattern.search(line)
            if match:
                hits.append((path, lineno, match.group(0)))
    return hits


# ------------------------------------------------------------------ 检查


def check_fileset(files: dict[Path, str], core: dict[Path, str]) -> None:
    print("[1] 生产 Kotlin 文件集")
    if not files:
        fail("未找到任何生产 Kotlin 文件（扫描范围无效）")
        return
    if not core:
        fail(f"未找到 `{CORE_DIR_NAME}/` 目录下的 Kotlin 文件：薄封装不存在")
        return
    ok(f"生产 Kotlin {len(files)} 个文件，其中 `{CORE_DIR_NAME}/` 下 {len(core)} 个")

    rels = sorted(rel(path) for path in core)
    missing = [dep for dep in CORE_DEP_RELATIVE if not any(item.endswith(dep) for item in rels)]
    if missing:
        fail(f"`{CORE_DIR_NAME}/` 缺少薄封装关键文件: {missing}（实际 {rels}）")
    else:
        ok(f"薄封装关键文件齐备（{len(CORE_DEP_RELATIVE)} 个）")


def check_business_not_copied(core: dict[Path, str]) -> None:
    print("[2] Kotlin 不得复制章节/位置/分页/缓存失效/书签业务")
    if not core:
        fail("扫描范围为空，检查无效")
        return
    hits = scan_tokens(core, FORBIDDEN_BUSINESS_TOKENS) + scan_regex(core, CHAPTER_PATTERN_RE)
    if hits:
        for path, lineno, token in hits[:20]:
            fail(
                f"{rel(path)}:{lineno} 出现核心业务原语 `{token}`；"
                "章节/位置/分页/缓存失效/书签语义由 Rust `reading-core` 裁定，Kotlin 只做装箱与编排"
            )
    else:
        ok(
            f"`{CORE_DIR_NAME}/` {len(core)} 个文件无章节/位置/分页/缓存失效/书签业务原语"
            f"（{len(FORBIDDEN_BUSINESS_TOKENS)} 条规则 + 章节正则）"
        )


def check_no_whole_file_read(files: dict[Path, str]) -> None:
    print("[3] 不得整本正文传递（整文件读取 / 整书长度 32 位降级）")
    if not files:
        fail("扫描范围为空，检查无效")
        return
    hits = scan_tokens(files, FORBIDDEN_WHOLE_FILE_TOKENS) + scan_regex(files, WHOLE_FILE_DOWNCAST_RE)
    if hits:
        for path, lineno, token in hits[:20]:
            fail(
                f"{rel(path)}:{lineno} 出现整本正文通道 `{token}`；"
                "正文字节只能以受限窗口形式跨边界（头文件「平台窗口内容协议」）"
            )
    else:
        ok(
            f"{len(files)} 个生产 Kotlin 文件无整文件读取、无整书长度 32 位降级"
            f"（{len(FORBIDDEN_WHOLE_FILE_TOKENS)} 条通道规则 + toInt 降级规则）"
        )


def check_no_empty_callback_placeholders(files: dict[Path, str]) -> None:
    print("[3b] Compose/阅读层不得用空回调占位（控件必须接真实状态）")
    layer = {
        path: text
        for path, text in files.items()
        if any(path.parent.name == name or name in path.parts for name in UI_LAYER_DIR_NAMES)
    }
    if not layer:
        skip(
            "尚未建立 `ui/` 与 `reader/` 生产 Kotlin 代码：空回调占位检查当前无对象"
            "（该层实现后自动生效，见 AN3 卡片完成定义）"
        )
        return
    hits: list[tuple[Path, int, str]] = []
    for path, text in layer.items():
        raw_lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for lineno, line in enumerate(text.splitlines(), start=1):
            match = EMPTY_LAMBDA_RE.search(line)
            if not match:
                continue
            source_line = raw_lines[lineno - 1] if lineno - 1 < len(raw_lines) else ""
            if SCOPE_ALLOW_MARKER in source_line:
                continue
            hits.append((path, lineno, match.group(0)))
    if hits:
        for path, lineno, token in hits[:20]:
            fail(
                f"{rel(path)}:{lineno} 出现空回调占位 `{token}`；"
                "控件必须接真实状态（确实无操作时用 `scope-allow:` 说明理由）"
            )
    else:
        ok(f"`ui/`+`reader/` 层 {len(layer)} 个文件无空回调占位")


def check_core_has_no_platform_state(core: dict[Path, str]) -> None:
    print("[4] core/ 不得持有 Activity / Compose / ViewModel 状态")
    if not core:
        fail("扫描范围为空，检查无效")
        return
    hits = scan_tokens(core, FORBIDDEN_PLATFORM_STATE_TOKENS)
    if hits:
        for path, lineno, token in hits[:20]:
            fail(
                f"{rel(path)}:{lineno} 引用平台状态 `{token}`；"
                "核心层只接受值类型参数，UI 状态不得进入（子规格「禁止事项」）"
            )
    else:
        ok(f"`{CORE_DIR_NAME}/` {len(core)} 个文件无 Compose/Activity/ViewModel/Context 依赖")


def check_probe_isolation(files: dict[Path, str]) -> None:
    print("[5] 探针隔离（生产 Kotlin 不得引用 experiments/）")
    hits = [
        (path, lineno)
        for path, text in files.items()
        for lineno, line in enumerate(text.splitlines(), start=1)
        if PROBE_PATH_TOKEN in line
    ]
    if hits:
        for path, lineno in hits[:20]:
            fail(f"{rel(path)}:{lineno} 引用验证探针路径 `{PROBE_PATH_TOKEN}`")
    else:
        ok(f"生产 Kotlin 未引用 `{PROBE_PATH_TOKEN}` 探针")


MAIN_ACTIVITY_NAME = "MainActivity.kt"

# [13] 阅读常亮的决策文件（纯 Kotlin；宿主只做窗口标志写入）。
SCREEN_ON_CONTROLLER_NAME = "ScreenOnController.kt"

# [9] 「构造期访问上下文」形态：单行属性初始化器里出现 applicationContext / this，
# 且**不是** by lazy（延迟初始化）——真机实测该写法会在 attachBaseContext 之前抛 NPE 闪退。
INITIALIZER_USES_CONTEXT = re.compile(
    r"private val \w+\s*=\s*(?!by lazy)[^\n]*(?:applicationContext|\bthis\b)"
)


def check_host_wiring(files: dict[Path, str]) -> None:
    print("[7] 宿主接线：MainActivity 必须挂载应用外壳、注册系统导入并接上平台来源")
    activity = next((path for path in files if path.name == MAIN_ACTIVITY_NAME), None)
    if activity is None:
        skip(f"[7] 未找到 {MAIN_ACTIVITY_NAME}（宿主接线无从校验）")
        return

    text = files[activity].lower()
    required = (
        ("挂载应用外壳 NovelReaderApp(...)", "novelreaderapp("),
        ("注册系统文件选择器 OpenDocument", "activityresultcontracts.opendocument()"),
        ("把平台来源转发器交给引擎工厂", "safwindowproviderholder"),
        ("传入引擎落盘路径 dbPath", "productionfactory("),
        ("保留装机可核对的构建标记", "buildmarker"),
    )
    missing = [label for label, token in required if token not in text]
    if missing:
        fail(
            "[7] MainActivity.kt 宿主接线缺失："
            + "; ".join(missing)
            + "（App 会停在占位页：界面无书架、无可交互控件）"
        )
    else:
        ok("[7] MainActivity 挂载外壳 + OpenDocument + 平台来源转发器 + dbPath + 构建标记均在位")

    stub = (
        "class mainactivity : componentactivity() { "
        "override fun oncreate() { setcontent { text(\"novel reader v1\") } } }"
    )
    if any(token in stub for token in (token for _, token in required)):
        fail("[7] 反向对照失效：占位页形态竟然通过宿主接线检查")
    else:
        ok("[7] 反向对照：仅显示标题的占位页会被本检查判为失败")


def check_reader_viewport_reporting(files: dict[Path, str]) -> None:
    print("[8] 阅读页视口回传：必须挂在始终存在的正文容器上（防排版死锁）")
    screen = next((path for path in files if path.name == "ReaderScreen.kt"), None)
    if screen is None:
        skip("[8] 未找到 ReaderScreen.kt（视口回传无从校验）")
        return

    text = files[screen]
    # 判据必须用「调用点」而非裸标识符：`import ...layout.onSizeChanged` 也含 `.onSizeChanged`，
    # 用裸标识符会永远命中 import 行而让本检查失去意义（曾经的缺陷，由变异测试抓出）。
    call_at = text.find(".onSizeChanged {")
    branch_at = text.find("page == null")
    if call_at < 0:
        fail(
            "[8] ReaderScreen.kt 未回传正文区域尺寸（`.onSizeChanged` 缺失）："
            "排版将永远等不到视口（长期停在「正在排版…（等待正文区域尺寸）」）"
        )
    elif 0 <= branch_at < call_at:
        fail(
            "[8] ReaderScreen.kt 的视口回传位于「page == null」之后 —— 若它被挂在依赖排版结果的节点上，"
            "会形成死锁：没有视口→排不出页→回传节点不存在→视口永远不来"
        )
    else:
        ok("[8] 视口回传位于正文容器上（先于 page 判空分支，与 page 是否已排出无关）")

    stub = (
        "if (page == null) { text(state.status) } "
        "else { text(page.text, modifier = .onsizechanged { }) }"
    )
    stub_call = stub.find(".onsizechanged {")
    stub_branch = stub.find("page == null")
    if stub_branch < 0 or stub_call < stub_branch:
        fail("[8] 反向对照失效：把回传挂进 else 分支的死锁写法竟然通过检查")
    else:
        ok("[8] 反向对照：把回传挂进「已有页」分支的写法会被本检查判为失败")


def check_host_context_initializers(files: dict[Path, str]) -> None:
    print("[9] 宿主 Context 时序：Attribute 初始化器不得在构造期访问 applicationContext")
    activity = next((path for path in files if path.name == "MainActivity.kt"), None)
    if activity is None:
        skip("[9] 未找到 MainActivity.kt（时序检查无从执行）")
        return

    text = files[activity]
    hits = [m.group(0).strip() for m in INITIALIZER_USES_CONTEXT.finditer(text)]
    if hits:
        fail(
            "[9] MainActivity.kt 的属性初始化器在**构造期**访问 applicationContext（会 NPE 闪退）："
            + "; ".join(hit[:80] for hit in hits)
            + " —— 请改用 `by lazy` 或移入 onCreate"
        )
    else:
        ok("[9] 无构造期上下文访问（依赖上下文的成员均为 by lazy / onCreate 内赋值）")

    stub = "private val bad = ReaderSettingsPrefsStore(AndroidSafKeyValue(applicationContext))"
    lazy_stub = "private val good by lazy { ReaderSettingsPrefsStore(AndroidSafKeyValue(applicationContext)) }"
    if not INITIALIZER_USES_CONTEXT.search(stub) or INITIALIZER_USES_CONTEXT.search(lazy_stub):
        fail("[9] 反向对照失效：构造期写法与 by lazy 写法未被正确区分")
    else:
        ok("[9] 反向对照：构造期写法判为失败、by lazy 写法判为通过")


def check_reader_interaction(files: dict[Path, str]) -> None:
    print("[10] 阅读交互层：手势翻页、中部切换界面、章节目录入口必须真实存在")
    screen = next((path for path in files if path.name == "ReaderScreen.kt"), None)
    if screen is None:
        skip("[10] 未找到 ReaderScreen.kt（交互层无从校验）")
        return

    text = files[screen]
    required = (
        ("手势接收点 pointerInput", "pointerinput("),
        ("分带点击判定（复用纯函数）", "readerinteraction.tapzonefor("),
        ("横滑判定（复用纯函数）", "readerinteraction.swipepagefor("),
        ("中部点击切换界面", "readertapzone.toggle_chrome"),
        ("界面显隐状态（沉浸模式）", "chromevisible"),
        ("章节目录入口", "reader_chapter_list_button"),
        ("目录跳转回调", "onjumptochapter"),
    )
    missing = [label for label, token in required if token not in text.lower()]
    if missing:
        fail(
            "[10] ReaderScreen.kt 缺少阅读交互层要素："
            + "; ".join(missing)
            + "（规格 `:620` 要求 1/3 分带点击与中部切换界面，`:611` 要求目录）"
        )
    else:
        ok("[10] 手势（分带点击 + 横滑）+ 沉浸切换 + 章节目录入口均在位")

    stub = (
        "column { topbar { textbutton(onclick = onsaveposition) { text(\"保存\") } }; "
        "box(modifier = modifier.fillsize()) { text(page.text) } }; "
        "outlinedbutton(onclick = onnextpage) { text(\"下一页\") }"
    )
    if any(token in stub for token in (token for _, token in required)):
        fail("[10] 反向对照失效：纯按钮版阅读页竟然通过交互层检查")
    else:
        ok("[10] 反向对照：只有按钮、没有手势与目录的阅读页会被本检查判为失败")


def check_encoding_pipeline(files: dict[Path, str]) -> None:
    print("[11] 编码贯通：正文/章节必须按核心回报的编码解码，界面必须显示该编码")
    vm = next((path for path in files if path.name == "ReaderViewModel.kt"), None)
    screen = next((path for path in files if path.name == "ReaderScreen.kt"), None)
    if vm is None:
        skip("[11] 未找到 ReaderViewModel.kt（编码贯通无从校验）")
        return

    vm_text = files[vm].lower()
    issues: list[str] = []
    if "string(read.bytes, charsets.utf_8)" in vm_text:
        issues.append("正文窗口仍硬编码 UTF-8 解码（非 UTF-8 文档必乱码且页边界被拒）")
    if "contentencoding.charset" not in vm_text:
        issues.append("未使用核心回报的编码（ContentEncoding.charset）解码")
    if "encodedtextcodec.decode(read.bytes" not in vm_text:
        issues.append("未按源字节精确解码（页边界偏移必须来自 EncodedTextCodec 的映射）")
    if "prefixbyteends = decoded.prefixbyteends" not in vm_text:
        issues.append("切片未使用解码时记录的精确字节映射")
    if "byteoffsets.bytelengthofprefix" in vm_text:
        issues.append("仍在用「解码后重编码」推算字节长度（变长编码会算错）")
    if "nrchaptercodec.title(batch.bytes, entry), contentencoding.charset)" in vm_text:
        issues.append("章节标题被按内容编码解码（ABI §3 规定字符串一律 UTF-8）")
    if "nrchaptercodec.title(batch.bytes, entry), charsets.utf_8)" not in vm_text:
        issues.append("章节标题必须按 UTF-8 解码（ABI §3）")
    if screen is not None and "encodingname" not in files[screen].lower():
        issues.append("阅读页信息区未显示当前编码")

    if issues:
        fail("[11] 编码贯通缺失：" + "; ".join(issues))
    else:
        ok("[11] 正文/章节按核心编码解码、页边界按同编码计数、界面显示编码名")

    stub = (
        "val text = string(read.bytes, charsets.utf_8)\n"
        "val slice = pagelayout.slice(..., )\n"
        "title = string(nrchaptercodec.title(batch.bytes, entry), contentencoding.charset)\n"
        "subtitle = \"chapter\""
    ).lower()
    stub_bad = (
        "string(read.bytes, charsets.utf_8)" in stub
        or "encodedtextcodec.decode(" not in stub
        or "contentencoding.charset)" in stub
    )
    if not stub_bad:
        fail("[11] 反向对照失效：硬编码 UTF-8 的写法竟然通过检查")
    else:
        ok("[11] 反向对照：硬编码 UTF-8 解码的写法会被本检查判为失败")


def check_resume_and_chapter_list(files: dict[Path, str]) -> None:
    """[12] 退出即保存 + 目录自动定位到当前章。

    这两项都是真机缺陷的回归门禁：
    - 读到章中退出再入未继续原位置（桌面端已修，移动端此前只在翻页/切章时保存）；
    - 目录从第 1 项开始显示，章节上千时找不到当前位置。
    """
    print("[12] 退出即保存（返回/后台）与目录自动定位到当前章")
    vm = next((path for path in files if path.name == "ReaderViewModel.kt"), None)
    activity = next((path for path in files if path.name == "MainActivity.kt"), None)
    overlay = next((path for path in files if path.name == "ChapterListOverlay.kt"), None)
    interaction = next((path for path in files if path.name == "ReaderInteraction.kt"), None)
    if vm is None or overlay is None:
        skip("[12] 未找到 ReaderViewModel.kt / ChapterListOverlay.kt（无从校验）")
        return

    issues: list[str] = []
    vm_text = files[vm]
    vm_lower = vm_text.lower()
    if "fun onhostpaused(" not in vm_lower:
        issues.append("缺少「退到后台即保存」的入口 onHostPaused（退出后重开会退回上次保存点）")
    close_at = vm_lower.find("fun closebook(")
    if close_at < 0:
        issues.append("未找到 closeBook（返回书架路径无法校验）")
    elif "persistpositionlocked(" not in vm_lower[close_at:close_at + 900]:
        issues.append("closeBook 未在关闭前落盘当前位置（退出再入丢失章中位置）")
    if "announce: boolean = true" not in vm_lower:
        issues.append("persistPositionLocked 缺少 announce 开关（退出保存会覆盖界面状态文案）")

    if activity is None:
        issues.append("未找到 MainActivity.kt（无法确认宿主已接线）")
    else:
        act = files[activity].lower()
        if "override fun onstop()" not in act or "onhostpaused()" not in act:
            issues.append("MainActivity.onStop 未调用 onHostPaused（退到后台不落盘）")

    overlay_lower = files[overlay].lower()
    if "rememberlazyliststate(" not in overlay_lower:
        issues.append("目录浮层未持有 LazyListState（无法自动滚动）")
    if "scrolltoitem(" not in overlay_lower:
        issues.append("目录浮层未滚动到当前章")
    if "chapterlistinitialindex(" not in overlay_lower:
        issues.append("目录浮层未使用 chapterListInitialIndex 计算首个可见项")
    if "state = liststate" not in overlay_lower:
        issues.append("LazyColumn 未绑定 listState（滚动状态未生效）")
    if interaction is None or "fun chapterlistinitialindex(" not in files[interaction].lower():
        issues.append("缺少纯函数 chapterListInitialIndex（目录定位边界不可测）")

    if issues:
        fail("[12] 退出保存/目录定位缺失：" + "; ".join(issues))
    else:
        ok("[12] 关闭与后台均落盘当前位置；目录浮层定位并高亮当前章（纯函数可测）")


def check_keep_screen_on(files: dict[Path, str]) -> None:
    """[13] 阅读时保持屏幕常亮：路由驱动的窗口标志（真机反馈「阅读中很容易息屏」的回归门禁）。

    - 宿主 `MainActivity.kt` 必须持有 `FLAG_KEEP_SCREEN_ON` 开关**对**（`addFlags` + `clearFlags`），
      且由 `ui.route` 观测驱动 —— 不是 onCreate 里一次性的常开；
    - 决策必须落在纯 Kotlin 的 `ScreenOnController.kt`（仅 `ReaderRoute.READER` 保持常亮），
      宿主只做窗口写入，判定可被 JVM 单测覆盖（`ScreenOnControllerTest`）。
    """
    print("[13] 阅读时保持屏幕常亮：路由驱动窗口标志（仅阅读页生效，离开即恢复）")
    activity = next((path for path in files if path.name == MAIN_ACTIVITY_NAME), None)
    controller = next((path for path in files if path.name == SCREEN_ON_CONTROLLER_NAME), None)
    if activity is None:
        skip("[13] 未找到 MainActivity.kt（常亮接线无从校验）")
        return

    required = (
        ("窗口标志 FLAG_KEEP_SCREEN_ON", "flag_keep_screen_on"),
        ("置位调用 addFlags", "addflags("),
        ("清位调用 clearFlags（离开阅读页恢复系统息屏）", "clearflags("),
        ("按路由观测驱动（ui 的 route）", "it.route"),
        ("接上纯 Kotlin 决策 ScreenOnController", "screenoncontroller"),
        ("路由变化入口 onRouteChanged", "onroutechanged"),
    )
    issues: list[str] = []
    act = files[activity].lower()
    missing = [label for label, token in required if token not in act]
    if missing:
        issues.append("MainActivity.kt 常亮接线缺失：" + "; ".join(missing))

    if controller is None:
        issues.append(
            "缺少 ScreenOnController.kt：路由→常亮判定没有可单测的落点"
            "（判定内联在宿主机里时 JVM 套件覆盖不到）"
        )
    else:
        ctl = files[controller].lower()
        if "fun onroutechanged(" not in ctl:
            issues.append("ScreenOnController 缺少 onRouteChanged 入口")
        if "readerroute.reader" not in ctl:
            issues.append("ScreenOnController 未以「仅阅读页」判定（缺 readerroute.reader）")
        if "applyflag(" not in ctl:
            issues.append("ScreenOnController 未把判定交给窗口回调（缺 applyFlag 调用）")

    if issues:
        fail("[13] 阅读常亮缺失：" + "; ".join(issues))
    else:
        ok("[13] 窗口标志对 + 路由观测 + ScreenOnController（仅阅读页保持常亮）均在位")

    placeholder = (
        "class mainactivity : componentactivity() { override fun oncreate() { "
        "setcontent { text(\"novel reader v1\") } } }"
    )
    always_on = (
        "class mainactivity : componentactivity() { override fun oncreate() { "
        "super.oncreate(); window.addflags(windowmanager.layoutparams.flag_keep_screen_on) } }"
    )

    def missing_tokens(text: str) -> list[str]:
        lowered = text.lower()
        return [label for label, token in required if token not in lowered]

    if not missing_tokens(placeholder) or not missing_tokens(always_on):
        fail("[13] 反向对照失效：占位页或「无条件常开、从不恢复」的写法竟然通过检查")
    else:
        ok(
            "[13] 反向对照：占位页（无接线）与「只置位、不按路由、从不恢复」的写法"
            "均会被本检查判为失败"
        )


def check_shelf_drill(files: dict[Path, str]) -> None:
    """[14] 书架分类下钻（窄屏）与书籍移动（2026-10-09 文件管理器式改版）。

    - `ShelfScreen.kt`：自适应宽度判定 + 两分支（窄屏下钻 / 宽屏侧栏）+ 返回与移书接线；
    - `CategoryDrill.kt`：**目录先于书籍**下发（同一层级：目录在上、文件在下）、
      系统返回键逐级回退、「返回上一级」行、窄屏归一化（无「全部书籍」位置）；
    - 书籍移动链路：行「⋯」菜单 → 目标弹框 → `onMoveBookToCategory` 回调。
    """
    print("[14] 书架分类下钻（窄屏）与书籍移动（文件管理器式）")
    shelf = next((path for path in files if path.name == "ShelfScreen.kt"), None)
    drill = next((path for path in files if path.name == "CategoryDrill.kt"), None)
    if shelf is None:
        skip("[14] 未找到 ShelfScreen.kt（下钻接线无从校验）")
        return

    shelf_required = (
        ("自适应宽度判定（复用纯函数）", "shelflayout.iscompact("),
        ("窄屏分支：下钻列表", "categorydrill("),
        ("宽屏分支：保留侧栏", "categorysidebar("),
        ("逐级返回接线", "onnavigateup"),
        ("书籍移动接线（弹框确认 → 回调）", "onmovebooktocategory"),
    )
    drill_required = (
        ("目录/书籍同列（复用纯装配 levelChildren）", "categorytreeassembly.levelchildren("),
        ("系统返回键逐级回退（BackHandler）", "backhandler("),
        ("「返回上一级」行", "drill_back"),
        ("窄屏归一化（无「全部书籍」位置）", "launchedeffect(unit)"),
        ("「新建分类」建在当前目录", "drill_create"),
    )

    def judge_shelf(raw: str) -> list[str]:
        lowered = raw.lower()
        return [label for label, token in shelf_required if token not in lowered]

    def judge_drill(raw: str) -> list[str]:
        lowered = raw.lower()
        missing = [label for label, token in drill_required if token not in lowered]
        # 目录必须**先于**书籍下发（同一层级的目录在上方、文件在下方）。
        dirs_at = lowered.find("items(items = dirs")
        books_at = lowered.find("items(items = shelf.books")
        if dirs_at < 0 or books_at < 0 or dirs_at >= books_at:
            missing.append("目录未排在书籍之前（应：目录在上、文件在下）")
        return missing

    issues: list[str] = []
    missing = judge_shelf(files[shelf])
    if missing:
        issues.append("ShelfScreen.kt 缺少：" + "; ".join(missing))
    if drill is None:
        issues.append("缺少 CategoryDrill.kt：窄屏下钻视图不存在（会退回拥挤的左右布局）")
    else:
        missing = judge_drill(files[drill])
        if missing:
            issues.append("CategoryDrill.kt 缺少：" + "; ".join(missing))

    if issues:
        fail("[14] 书架下钻/移书缺失：" + "; ".join(issues))
    else:
        ok("[14] 下钻（目录先于书籍）+ 自适应分支 + 返回/归一化 + 书籍移动接线均在位")

    # 反向对照：两类坏形状必须被本检查判为失败。
    stub_sidebar_only = "class shelf { row { categorysidebar(...) } }"
    stub_books_first = (
        "launchedeffect(unit) {} backhandler(...) {} drill_back drill_create "
        "categorytreeassembly.levelchildren(...) "
        "lazycolumn { items(items = shelf.books) {}; items(items = dirs) {} }"
    )
    if not judge_shelf(stub_sidebar_only) or not judge_drill(stub_books_first):
        fail("[14] 反向对照失效：只有侧栏 / 书籍排在目录之前的形状竟然通过检查")
    else:
        ok("[14] 反向对照：缺下钻分支、以及「书籍排在目录之前」的写法均会被本检查判为失败")


def check_positive_control() -> None:
    print("[6] 正向对照：扫描规则在内置 fixture 上必须命中（证明非空检查）")
    fixture = {
        Path("core/Fixture.kt"): (
            "val rule = Regex(\"第[0-9]+章\")\n"
            "fun cacheKeyOf(id: Long) = id\n"
            "fun restorePosition() = Unit\n"
            "val bytes = file.readBytes()\n"
            "val total = totalBytes.toInt()\n"
            "import androidx.compose.runtime.Composable\n"
            "val probe = \"experiments/\" + \"ui-selection\"\n"
        ),
    }
    business = scan_tokens(fixture, FORBIDDEN_BUSINESS_TOKENS) + scan_regex(fixture, CHAPTER_PATTERN_RE)
    whole = scan_tokens(fixture, FORBIDDEN_WHOLE_FILE_TOKENS) + scan_regex(fixture, WHOLE_FILE_DOWNCAST_RE)
    platform = scan_tokens(fixture, FORBIDDEN_PLATFORM_STATE_TOKENS)
    probe = [line for line in fixture[Path("core/Fixture.kt")].splitlines() if PROBE_PATH_TOKEN in line]
    empty_callback = [
        line
        for line in {Path("com/novelreader/android/ui/Fixture.kt"): "onClick = {}"}[
            Path("com/novelreader/android/ui/Fixture.kt")
        ].splitlines()
        if EMPTY_LAMBDA_RE.search(line)
    ]
    if business and whole and platform and probe and empty_callback:
        ok(
            f"5 类规则在 fixture 上分别命中 "
            f"{len(business)}/{len(whole)}/{len(platform)}/{len(probe)}/{len(empty_callback)} 处"
            "（第 2/3/3b/4/5 项非空检查）"
        )
    else:
        fail(
            "扫描规则在 fixture 上未命中，说明检查为空检查："
            f"business={len(business)} whole_file={len(whole)} platform={len(platform)} "
            f"probe={len(probe)} empty_callback={len(empty_callback)}"
        )


def self_test() -> list[str]:
    """fixture 反向用例：每条规则都必须能报出违规。"""
    failures: list[str] = []
    print("[self-test] fixture 反向用例")

    cases = [
        ("章节业务原语", "fun detectChapters() = Unit", scan_tokens, FORBIDDEN_BUSINESS_TOKENS),
        ("位置校验原语", "fun restorePosition() = Unit", scan_tokens, FORBIDDEN_BUSINESS_TOKENS),
        ("分页原语", "fun computePage() = Unit", scan_tokens, FORBIDDEN_BUSINESS_TOKENS),
        ("缓存失效原语", "val cacheKey = 1", scan_tokens, FORBIDDEN_BUSINESS_TOKENS),
        ("书签业务原语", "val bookmarkRule = 1", scan_tokens, FORBIDDEN_BUSINESS_TOKENS),
        ("整文件读取", "val b = f.readBytes()", scan_tokens, FORBIDDEN_WHOLE_FILE_TOKENS),
        ("BufferedReader", "val r = BufferedReader(null)", scan_tokens, FORBIDDEN_WHOLE_FILE_TOKENS),
        ("平台状态", "import androidx.compose.runtime.Composable", scan_tokens, FORBIDDEN_PLATFORM_STATE_TOKENS),
    ]
    for label, line, expected in [
        ("空回调占位", "onClick = {}", True),
        ("真实回调", "onClick = { viewModel.open(it) }", False),
    ]:
        found = bool(EMPTY_LAMBDA_RE.search(line))
        if found == expected:
            print(f"  PASS  自测：{label} 判定正确（{'报出' if expected else '不误报'}）")
        else:
            failures.append(f"自测（{label}）判定错误")
            print(f"  FAIL  自测（{label}）判定错误")
    for label, line, scanner, tokens in cases:
        hits = scanner({Path("core/F.kt"): line}, tokens)
        if hits:
            print(f"  PASS  自测：{label} 被报出")
        else:
            failures.append(f"自测（{label}）未按预期报出")
            print(f"  FAIL  自测（{label}）未按预期报出")

    chapter_pattern = scan_regex({Path("core/F.kt"): 'val rule = Regex("第[0-9]+章")'}, CHAPTER_PATTERN_RE)
    if chapter_pattern:
        print("  PASS  自测：章节正则字面量被报出")
    else:
        failures.append("自测（章节正则字面量）未按预期报出")
        print("  FAIL  自测（章节正则字面量）未按预期报出")

    downcast = scan_regex({Path("core/F.kt"): "val n = totalBytes.toInt()"}, WHOLE_FILE_DOWNCAST_RE)
    if downcast:
        print("  PASS  自测：整书长度 32 位降级被报出")
    else:
        failures.append("自测（整书长度 32 位降级）未按预期报出")
        print("  FAIL  自测（整书长度 32 位降级）未按预期报出")

    # 注释不应误判：边界说明文档里的业务词汇不算违规。
    commented = strip_kotlin_comments("// 章节识别与分页算法属于 Rust 核心\nval x = 1\n")
    if not scan_tokens({Path("core/F.kt"): commented}, FORBIDDEN_BUSINESS_TOKENS):
        print("  PASS  自测：注释中的业务词汇不误报（注释已剥离）")
    else:
        failures.append("自测（注释中的业务词汇）误报")
        print("  FAIL  自测（注释中的业务词汇）误报")
    return failures


# ------------------------------------------------------------------ main


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Android 生产 Kotlin 的范围门禁")
    parser.add_argument("--kotlin", default=str(REPO_ROOT / "android" / "app" / "src" / "main" / "java"))
    parser.add_argument(
        "--check-no-whole-file-read",
        action="store_true",
        help="只执行「不得整本正文传递」门禁（其余检查打印显式 SKIP）",
    )
    parser.add_argument("--self-test", action="store_true", help="运行 fixture 反向用例")
    args = parser.parse_args(argv)

    print(f"verify-android-scope: repo={REPO_ROOT}")
    exit_code = 0

    if args.self_test:
        print("[self-test] 反向用例与注释剥离")
        failures = self_test()
        print("-" * 60)
        if failures:
            print(f"SELF-TEST RESULT: FAIL ({len(failures)} 项)")
            for item in failures:
                print(f"  - {item}")
            exit_code = 1
        else:
            print("SELF-TEST RESULT: PASS（违规均可被报出，注释不误报）")

    kotlin_dir = Path(args.kotlin)
    if not kotlin_dir.is_dir():
        print(f"RESULT: FAIL — Kotlin 目录不存在: {kotlin_dir}")
        return 2
    files = kotlin_files(kotlin_dir)
    if not files:
        print(f"RESULT: FAIL — Kotlin 目录内没有任何 .kt 文件: {kotlin_dir}")
        return 2
    core = core_files(files)

    if args.check_no_whole_file_read:
        print("模式：--check-no-whole-file-read（只执行整本正文读取门禁）")
        skip("[1] 生产 Kotlin 文件集（本模式跳过）")
        skip("[2] 业务复制扫描（本模式跳过）")
        check_no_whole_file_read(files)
        skip("[3b] 空回调占位（本模式跳过）")
        skip("[4] core/ 平台状态（本模式跳过）")
        skip("[5] 探针隔离（本模式跳过）")
        skip("[6] 正向对照（本模式跳过；由默认模式执行）")
        skip("[7] 宿主接线（本模式跳过；由默认模式执行）")
        skip("[8] 阅读页视口回传（本模式跳过；由默认模式执行）")
        skip("[9] 宿主 Context 时序（本模式跳过；由默认模式执行）")
        skip("[10] 阅读交互层（本模式跳过；由默认模式执行）")
        skip("[11] 编码贯通（本模式跳过；由默认模式执行）")
        skip("[13] 阅读时保持屏幕常亮（本模式跳过；由默认模式执行）")
        skip("[14] 书架下钻与书籍移动（本模式跳过；由默认模式执行）")
    else:
        check_fileset(files, core)
        check_business_not_copied(core)
        check_no_whole_file_read(files)
        check_no_empty_callback_placeholders(files)
        check_core_has_no_platform_state(core)
        check_probe_isolation(files)
        check_host_wiring(files)
        check_reader_viewport_reporting(files)
        check_host_context_initializers(files)
        check_reader_interaction(files)
        check_encoding_pipeline(files)
        check_resume_and_chapter_list(files)
        check_keep_screen_on(files)
        check_shelf_drill(files)
        check_positive_control()

    print("-" * 60)
    if FAILURES:
        print(f"RESULT: FAIL ({len(FAILURES)} 项违规)")
        for item in FAILURES:
            print(f"  - {item}")
        return 1
    if exit_code != 0:
        print("RESULT: FAIL — --self-test 未通过")
        return 1
    mode = "--check-no-whole-file-read" if args.check_no_whole_file_read else "全量"
    print(
        f"RESULT: PASS — {mode}：生产 Kotlin 无核心业务复制、无整本正文通道、"
        f"core/ 无平台状态、无探针引用、宿主接线完整；断言 {PASSES[0]} 项，跳过项 {len(SKIPS)} 个"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
