# 应用图标资源（编译期内嵌）

本目录的文件由 **设计源（`docs/design/icon/`，唯一事实来源）** 复制而来，
供 `desktop/ui/app.slint` 的 `AppWindow.icon` 在编译期内嵌（Slint `@image-url`）：

| 文件 | 来源 | 用途 |
|---|---|---|
| `novel-reader-256.png` | `docs/design/icon/png/novel-reader-icon-256.png` | 窗口图标（标题栏 / 任务栏），由窗口管理器缩放 |
| `novel-reader-128.png` | `docs/design/icon/png/novel-reader-icon-128.png` | 备用（较小尺寸内嵌需求） |

**刷新方式**（设计改动后同步；复制后 `sha256sum` 应一致）：

```bash
cp -f docs/design/icon/png/novel-reader-icon-256.png desktop/assets/icons/novel-reader-256.png
cp -f docs/design/icon/png/novel-reader-icon-128.png desktop/assets/icons/novel-reader-128.png
```

Linux 打包（`desktop/packaging/linux/package-linux.sh`）安装的 hicolor 图标同样取自
`docs/design/icon/png/`，因此**不在此处重复维护**。
