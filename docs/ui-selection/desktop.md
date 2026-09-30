# 桌面端候选（hybrid-native · Rust + Slint）构建 · 运行 · 实测记录

> 工作目录：`experiments/ui-selection/hybrid-native-probe/desktop-slint`
> 渲染：Slint 1.17.1（Release）· 后端 X11 · 渲染器 femtovg
> 排版：**不使用 Slint 的文本排版 API**（其门面未公开），改由共享库
> `rust-reading-core` 的公共分页 API（`layout_page`）产出页边界，Slint 只负责渲染（H-R1）。

## 1. 构建与运行

```bash
source /opt/dev-tools/slint-libs/slint-env.sh          # 必须（fontconfig/freetype 链接环境）
cd experiments/ui-selection/hybrid-native-probe/desktop-slint
cargo build --release --offline
./target/release/desktop-slint-probe                   # 正常启动（窗口 1080x720）
```

### 1.1 验证用启动开关（探针用途）

| 环境变量 | 作用 |
|---|---|
| `NR_START_PAGE=1` | 直接进入「阅读」页（否则停在书架页） |
| `NR_THEME_DARK=1` | 以夜间模式启动 |
| `NR_BACKGROUND_MODE=0\|1\|2` | 启动时的阅读背景（白 / 米黄 / 绿） |
| `NR_EXIT_AFTER_READY_MS=N` | 写出 ready.json 后 N 毫秒自动退出（冒烟/截图用） |
| `NR_READY_PAGES=N` | 自检遍历页数（默认 1000） |

> 在 Hermes 会话里运行需去掉注入的预载：`env -u LD_PRELOAD ...`；您自己在终端跑不必加。

### 1.2 一键门禁

```bash
cd experiments/ui-selection
bash scripts/build-linux.sh --release hybrid-native
bash scripts/test-pagination.sh hybrid-native --platform desktop --pages 1000
bash scripts/smoke-linux.sh --backend x11 --timeout 30 hybrid-native
python3 scripts/verify-ui-contract.py --candidate hybrid-native --platform desktop
```

## 2. 实测记录（人工复测 + 无头截图）

### 2.1 第一轮：用户人工复测发现 2 个问题

| # | 症状 | 根因 | 修复 |
|---|---|---|---|
| D1 | 切**夜间模式**时其他界面元素都变色，唯独**阅读区仍是白底黑字** | `reader-bg` / `reader-fg` 只按 `background-mode`（白/米黄/绿）取色，**完全没有考虑 `theme-dark`** | 两个属性改为 `theme-dark ? ... : ...` 外层判断，夜间给出深底浅字（`ReaderScreen` 同源） |
| D2 | 阅读区**似乎没有分页**（正文只占上部、下方大片空白） | 排版配置固定用 `LayoutConfig::desktop_default()` 的 **800×560**，而 Slint 真实正文区约 **1032×560** → 库按 752px 宽排页、界面按 1032px 渲染 | 新增 `set_viewport()`：把「真实正文区域 + 页边距」喂给排版配置；**窗口映射后**对齐 |

**D2 的两个陷阱（都已修）**：

1. `window.size()` 在**事件循环启动前返回 0×0**（日志实锤：`viewport: 窗口 0x0 → 正文区 -48x-160`）
   → 对齐必须在窗口映射后执行（就绪探针定时器内），修好后日志为
   `viewport: 窗口 1080x720 → 正文区 1032x560 重排=true`。
2. Slint 的 `Text` **不叠加段间距、也不做首行缩进**，而 `desktop_default()` 带
   `paragraph_spacing=8` / `first_line_indent=36` → 库按 `行高(25.2)+段间距(8)=33.2px/行`
   计算，界面只按行高渲染 → **每页少排 6 行**（实测 16 行，应有 22 行）→ 底部留白。
   修复：桌面端把这两项置 0（行距倍数 1.4 与渲染实测行高 ≈25px 一致，保持不变）。

### 2.2 第二轮：修复后核验

- **用户人工核验：夜间模式阅读页已正常** ✔
- 无头截图（`ffmpeg x11grab` 仅截窗口）+ 图像核验：阅读页显示中文正文、
  顶部 `第1页 · 16行`、底部 `上一页/下一页/保存位置`，排版与配色均正常。
- 自检补充修复：`reflow_ok` 原用「宽度 −96px」作重排刺激，对**一句一行**的语料无效
  （行在 `\n` 处断开，宽度变化不改变断行）→ 同时改变字号，使刺激必然影响断行。

### 2.3 无头截图方法（本机无 import/scrot/xwd）

```bash
export DISPLAY=:0
WID=$(xdotool search --name "novel-reader" | tail -1)
xdotool windowactivate --sync $WID; xdotool windowraise $WID   # 必须先置顶，否则被其他窗口遮挡
eval "$(xdotool getwindowgeometry --shell $WID)"
ffmpeg -loglevel error -f x11grab -video_size ${WIDTH}x${HEIGHT} -i :0.0+${X},${Y} \
       -frames:v 1 -y /tmp/win.png
```

## 3. 版式常量（Rust ↔ Slint 必须同步）

`src/lib.rs` 中的 `ROOT_TOP_BAR=52` / `READER_HEAD_BAR=44` / `READER_FOOT_BAR=52` /
`READER_TEXT_PAD_TOP=12` / `READER_MENU=116` 与 `ui/app.slint` 中对应字面量**必须一致**，
否则「真实正文区域」推算错误、页边界再次与可见高度不符。
