# novel-reader 应用图标（开卷）

> 设计与资产说明。**本目录只含设计资产，尚未接入应用资源（未改动任何代码/打包脚本）。**
> 预览：`preview.png`（明/暗背景全尺寸一览）

![预览](preview.png)

---

## 1. 设计概念

**「开卷」** —— 蓝色书封上的一本打开的书：两页白色书页，外侧为竖直书口，上下缘向中线抬升，
两页在**下端点（书脊根部）相交**，中缝形成楔形负空间。

| 项 | 取值 | 说明 |
|---|---|---|
| 主色 | `#1890ff` | 与桌面端设计系统的 Primary 一致 |
| 墨色 | `#ffffff` | 单墨色：只有一个图形色，不叠细节 |
| 网格 | 32 × 32 单位（`viewBox 0 0 32 32`） | 与 Carbon 2x 网格习惯一致，便于 2 的幂缩放 |
| 圆角 | `rx = 7.2`（22.5%） | 桌面与移动通用；直角版本见 `-square` |
| 安全边距 | 左右 4.6 / 上 6.6 / 下 5.8（单位） | 内容居中，留白充分 |

**精确几何**（32 网格，右侧为镜像）：

```
左页：M4.6 11.8 L14.5 6.6 L16 21 L4.6 26.2 Z
右页：M27.4 11.8 L17.5 6.6 L16 21 L27.4 26.2 Z
```

### 为什么是这个形状（按实测收敛）

在下用 8 组候选在 128/64/48/32/16px 上逐轮实测筛掉了下列做法（都保留在这份记录里，便于后续回看）：

| 做法 | 实测结果 |
|---|---|
| 纯字母 N | 16px 最锐利，但**不表达阅读功能**，观感像通用占位 |
| N + 书脊细缝 / 书签丝带 / 斜笔折角 | 细节在 32px 以下变成**毛刺或噪点**，像画错 |
| N 倾斜、笔画倒角 | 小尺寸被读作「挤压/破损」 |
| 右页浅蓝、中缝浅蓝书脊 | 大尺寸有层次，**小尺寸变脏点**，对比度下降 |
| 封面镂空 N、合上的书 + 折角 | 16px 下 N 不可辨、折角像缺口 |
| **两页下端相交的开卷（本方案）** | 16/32px 仍可辨认，中缝干净不糊（与「下半部方形收口」对比后胜出） |

结论落在三条硬约束上：**单一墨色、无细碎细节、轮廓本身承担语义**。

### 小尺寸专用：像素对齐

16px 与 32px 直接用主矢量缩放会出现半像素灰边、左右页发虚。因此另出两份**像素对齐矢量**：

- `svg/novel-reader-icon-small-16.svg` —— 全部坐标为偶整数（16px = 0.5 倍，边缘正好落在整像素），中缝加宽到 4 单位（2px）；
- `svg/novel-reader-icon-small-32.svg` —— 全部坐标为整数，按 32px 网格重排。

`png/novel-reader-icon-16.png` 与 `-32.png` 即由这两份生成（实测比对：对齐版边缘更硬、左右更对称）。

---

## 2. 文件清单

### 源矢量（`svg/`）

| 文件 | 用途 |
|---|---|
| `novel-reader-icon.svg` | **主图标**：圆角方（桌面 + 移动通用） |
| `novel-reader-icon-square.svg` | 直角方：Carbon/IBM 口径，用于 Linux/Windows 打包 |
| `novel-reader-icon-small-16.svg` / `-small-32.svg` | 小尺寸像素对齐专用（见上） |
| `novel-reader-icon-mono.svg` | 单色（透明底，`currentColor`，可直接内联着色） |
| `novel-reader-icon-mono-white.svg` | 单色白（透明底，深色外壳/托盘） |
| `novel-reader-icon-adaptive-foreground.svg` | Android 自适应图标前景层（108dp 画布，内容收在 66dp 安全圈内） |
| `novel-reader-icon-alt-n.svg` | **备选**：字母 N + 书脊（品牌化场景，见 §5） |

### 位图（`png/`）

| 名称 | 尺寸 | 用途 |
|---|---|---|
| `novel-reader-icon-{16,32,48,64,128,256,512}.png` | 主图标全尺寸 | 16/32 为像素对齐版 |
| `novel-reader-icon-square-{16,32,48,64,128,256,512}.png` | 直角方全尺寸 | Linux/Windows 打包 |
| `novel-reader-icon-mono-{32,48,128}.png` | 单色黑（透明底） | 明色外壳、印刷 |
| `novel-reader-icon-mono-white-{32,48,128}.png` | 单色白（透明底） | 深色外壳、托盘 |
| `novel-reader-icon-alt-n-{16,32,48,64,128}.png` | 备选字母版 | 应用内品牌场景 |
| `novel-reader-icon-android-{48,72,96,144,192}.png` | Android 传统启动图标 | mdpi / hdpi / xhdpi / xxhdpi / xxxhdpi |
| `novel-reader-icon-adaptive-foreground-{108,162,216,324,432}.png` | Android 自适应前景 | 同上五档密度（108dp 换算） |

### 其他

| 文件 | 说明 |
|---|---|
| `novel-reader.ico` | Windows 图标，含 16/32/48/64/128/256 六层；**每层都用 cairo 单独渲染**（非二次缩放），小尺寸更利落 |
| `preview.png` | 明/暗背景全尺寸一览 |

---

## 3. 尺寸对照

| 尺寸 | 典型落点 |
|---|---|
| 16 | 窗口标题栏、系统托盘、列表小图标（用 `small-16` 对齐版） |
| 32 | 任务栏/程序坞、GNOME 应用菜单（用 `small-32` 对齐版） |
| 48 | Windows 中图标、Android mdpi 启动图标 |
| 64 | 中等图标视图、Android hdpi |
| 128 | 桌面图标视图、Android xxhdpi、商店图 |
| 256 / 512 | 打包与商店（Linux `hicolor` 大尺寸、Windows ICO 最大层、应用商店） |

---

## 4. 各平台落点（接入时照此放置）

### Linux（当前交付平台）

```ini
# share/applications/novel-reader.desktop
Icon=novel-reader
```

```sh
# 图标安装口径（hicolor 主题）
share/icons/hicolor/16x16/apps/novel-reader.png      # = png/…-16.png
share/icons/hicolor/32x32/apps/novel-reader.png
share/icons/hicolor/48x48/apps/novel-reader.png
share/icons/hicolor/64x64/apps/novel-reader.png
share/icons/hicolor/128x128/apps/novel-reader.png
share/icons/hicolor/256x256/apps/novel-reader.png
share/icons/hicolor/scalable/apps/novel-reader.svg   # = svg/novel-reader-icon.svg
```

现有 `desktop/packaging/linux/package-linux.sh` 目前**未安装任何图标**（`.desktop` 也没有 `Icon=` 行）——
接入属于打包脚本改动，需另行授权。

### Windows

现有脚本（`package-windows.ps1` / `package-windows-gnu.sh`）未引用图标。接入点：

- 可执行文件资源：用 `novel-reader.ico` 作为应用图标（MSVC 路径可加 `.rc` + `WINDRES`；gnu 路径可用 `windres`）；
- 安装/快捷方式：同一 `.ico`。

### Android

```text
app/src/main/res/mipmap-mdpi/ic_launcher.png      ← png/novel-reader-icon-android-48.png
app/src/main/res/mipmap-hdpi/ic_launcher.png      ← …-72.png
app/src/main/res/mipmap-xhdpi/ic_launcher.png     ← …-96.png
app/src/main/res/mipmap-xxhdpi/ic_launcher.png    ← …-144.png
app/src/main/res/mipmap-xxxhdpi/ic_launcher.png   ← …-192.png

# Android 8.0+ 自适应图标
app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml
    <adaptive-icon>
      <background android:drawable="@color/ic_launcher_background"/>   <!-- #1890ff 纯色 -->
      <foreground android:drawable="@mipmap/ic_launcher_foreground"/>  <!-- 前景 PNG -->
      <monochrome android:drawable="@mipmap/ic_launcher_monochrome"/>  <!-- 13+ 主题图标：单色版 -->
    </adaptive-icon>
app/src/main/res/mipmap-{m,h,xh,xxh,xxxh}dpi/ic_launcher_foreground.png
    ← png/novel-reader-icon-adaptive-foreground-{108,162,216,324,432}.png
```

`AndroidManifest.xml` 目前没有 `android:icon`（用系统默认图标），接入时补 `android:icon="@mipmap/ic_launcher"`。
自适应前景已按 66dp 安全圈缩放（缩放系数 2.45），圆形/方形/圆角方形遮罩下都不会被裁到主体。

### 应用窗口图标（Slint）

Slint 窗口图标由后端从可执行文件/桌面项读取；若需显式设置，可用 `32/48/128` 的 PNG 或 `novel-reader.ico`。

---

## 5. 备选方案：字母 N（书脊）

`novel-reader-icon-alt-n.svg` 是**品牌字母版**：白色粗体 N + 左侧亮蓝书脊带。

- 优点：16px 下最锐利（纯几何笔画），品牌识别强；
- 缺点：**不表达「阅读」语义**，第一眼像通用 monogram，需要品牌认知支撑。

在下建议：**应用图标用「开卷」**（语义直接、小尺寸可用），字母版仅用于应用内的品牌场景
（启动页、关于页、侧栏 logo），如需改用字母版做主图标请指示，在下会把全部尺寸与 ICO 一并重出。

---

## 6. 复现与校验

```bash
# 矢量 → 位图（要求 rsvg-convert；逐尺寸直接渲染，不做二次缩放）
rsvg-convert -w 128 -h 128 svg/novel-reader-icon.svg -o png/novel-reader-icon-128.png

# 16/32px 请改用像素对齐源
rsvg-convert -w 16 -h 16 svg/novel-reader-icon-small-16.svg -o png/novel-reader-icon-16.png

# 校验位图尺寸与透明通道
```

已校验：8 个 SVG 均为合法 XML（`ElementTree` 解析通过）；35 个 PNG 尺寸与文件名一致，
单色/前景 PNG 带透明通道；`novel-reader.ico` 可被解析且含 6 个尺寸层。
