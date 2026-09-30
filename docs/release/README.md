# 发布目录说明（V1 / REL1）

本目录存放 **V1 发布说明模板与已知限制登记**。产物与验收证据由脚本生成，本目录只做口径与模板，
不保存任何「已发布/已通过」的结论。

## 文件

| 文件 | 作用 |
|---|---|
| `README.md` | 本文件：发布门禁命令、清单口径与诚实规则 |
| `release-notes-template.md` | 版本发布说明模板（版本、构建标识、发布物、SHA-256、已知限制、回滚） |
| `known-limitations.md` | 已知限制与**延期验收项**登记（真机 / Windows / Wayland / 未签名 / 许可证 / 文档精度项） |

## 发布门禁命令

```bash
cd ${NR_PROJECT_DIR}
python3 scripts/verify-v1-structure.py                 # 正式工程结构（父级 checklist C1）
python3 scripts/verify-v1-release.py --desktop --android   # 发布物/版本/构建标识/许可/未验证项（C11）
```

两条命令都必须 `exit 0`。`verify-v1-release.py` 另支持把发布清单写成机器可读 JSON：

```bash
python3 scripts/verify-v1-release.py --desktop --android --json results/rel1-release-manifest.json
```

## 清单口径

- **发布物**：桌面 Release 二进制 `crates/desktop-app/target/release/desktop-app`（含 ELF 目标、
  符号剥离、`DT_NEEDED`/`ldd` 运行库、SHA-256）；Android `dist/*.apk`（含大小、SHA-256、签名状态、
  版本、构建标记、包内 JNI `.so` 与源码树预编译 `.so` 同源）。
- **版本标识**：Rust workspace 版本 ↔ 桌面 crate 版本（必须一致）；Android `versionName/versionCode`
  自 `app/build.gradle.kts` 读取并与 APK 内清单逐项核对。
- **构建标识**：Android `BUILD_MARKER` 三方一致（`app/build.gradle.kts` ↔ `dist/BUILD-INFO.md` ↔ APK 内
  dex 字符串）；桌面二进制以 ELF `.note.gnu.build-id` 作为构建标识。
- **依赖许可**：Rust 覆盖两个 `Cargo.lock` 全量 package（来源：本地 registry / path crate 的
  `Cargo.toml` 的 `license` 字段），并用 `target/release/deps/*.d` 的**构建证据**核对「实际编译进发布物
  的依赖」全部有许可条目；Android 覆盖 `build.gradle.kts` 直接声明依赖（来源：Gradle 缓存 POM），
  传递依赖缺口登记为未验证项。
- **SHA-256**：桌面二进制与每个 APK 均由脚本现场重算，并与 `dist/BUILD-INFO.md` 记录逐项比对。
- **未验证项**：真机、Windows/Wayland、未签名 Release、平台专用依赖许可、项目自身发行许可证、
  干净发行版上的发布目录 —— 一律以 `待验证` 输出。

## 诚实规则（本目录的硬约束）

1. 未实测平台/设备不得写「通过」；延期项只能写 `待验证`，并给出闭环命令或前置条件。
2. 发布说明中的每个版本号、SHA-256、构建标记必须来自脚本输出或真实产物，不得手填估算值。
3. `release-notes-template.md` 必须保留占位符（`{{...}}`）；把模板填成结论会被
   `verify-v1-release.py` 判为失败。
4. 不得出现未实测的真机/跨平台通过宣称（例如把真机项写成已验证、把三平台发布写成已完成）；
   所有未实测项只能出现在 `known-limitations.md` 的延期表中，状态列为 `待验证`。
5. 发布物被重新构建后必须重新生成 `dist/BUILD-INFO.md` 与发布说明中的 SHA-256（记录过期即视为失败）。
