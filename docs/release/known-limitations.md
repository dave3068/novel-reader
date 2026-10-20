# 已知限制与延期验收项登记（V1）

> 用途：发布说明的「已知限制」章节从本文件引用；`scripts/verify-v1-release.py` 会校验本文件已登记
> 以下必需条目（缺一项即门禁失败）。**本文件不产生任何通过结论**：所有未实测项一律为 `待验证`。

## 1. 延期验收项（未实测，不得写「通过」）

| # | 项目 | 当前状态 | 说明 / 闭环前置 |
|---|---|---|---|
| D1 | Android 真机（SAF 选择与持久授权、旋转、后台恢复、系统返回、JNI 取消、功耗） | 待验证 | 需 Android 12+ 真机；步骤见 `docs/android/device-runbook.md`，证据约定写入本地 `results/verify/android-device-verify.md`；就绪度由 `scripts/verify-android-device-readiness.py` 校验（真机执行前保持「待验证（文件未写入）」）。设备型号、验收者信息及 APK/证书指纹等本地记录不纳入版本库。所有未逐项复核的行为均保持待验证 |
| D2 | Windows 10/11 x86_64 构建、打包与启动 | ~~待验证~~ → **已通过（2026-10-02，MSVC 官方口径四步全绿）** | **官方口径验证完成**（`targets.json` 的 `windows-x86_64` = `verified`，证据 `results/desktop/windows-msvc.json`，`run_id=20261002T221500Z`，`result=PASS`）。工具链：rustc 1.98.0（与 `rust-toolchain.toml` 一致）+ `x86_64-pc-windows-msvc` target + MSVC 14.44.35207 + Windows SDK 10.0.22621.0 + pwsh 7.4.6。四步：①`cargo build --release --target x86_64-pc-windows-msvc` exit 0（11m38s，日志含 `accesskit_windows` 确证 MSVC 工具链）；②`package-windows.ps1 -NoBuild -Force` exit 0（7 文件 + `dlls\` + `docs\`，`MANIFEST.sha256` 自校验 **6/6**）；③`Get-FileHash` 产物 sha256 `9c8898f4…04315` 与构建产物**一致**；④`NR_*` 驱动启动（自行退出、`NR-STATE` 1 行、**stderr 空**、中文正常）。**期间修复一处真实缺陷**：`package-windows.ps1` 的 `$RepoRoot`/默认 `$OutDir` 上溯少一级（`..\..` → 应 `..\..\..`），此前从未被执行过，一实测即 exit 2。**仍未验证的细项**（见 `targets.json` 的 `unverified_items`）：DPI 100/125/150% 一致性、原生 picker 交互、字体族切换、exe 内嵌图标资源、干净环境安装 |
| D3 | Linux Wayland 构建产物启动与渲染/输入一致性 | 待验证 | 本机无 Wayland 合成器会话；见 `targets.json` 的 `linux-wayland` |
| D4 | 桌面完整发布目录在**干净目标发行版**（无 Slint 编译依赖、仅系统运行库）上的安装与启动 | 待验证 | 现有证据只在构建机 `/tmp` 组装并启动；口径：`bash desktop/packaging/linux/package-linux.sh --backend x11 --out dist` |
| D5 | 100%/125%/150% DPI、字体回退（无 `fonts-noto-cjk` 时的 tofu 表现） | 待验证（**2026-10-02 已排除三条无效路径**） | **实测排除**（详见 `results/verify/dpi-verification-attempts.md`）：①Linux `xrandr --scale` 只改帧缓冲，应用 `window=1200x800 viewport=720x606 lines=22 pitch=27` 三档一字不差（`scale_factor` 未变）；②Windows 注册表 `LogPixels`（96/120/144）对 DPI-aware 应用无效 —— 实测 `LogPixels=144` 时 `GetDeviceCaps(LOGPIXELSX)` 仍为 **96**；③FreeRDP `/scale-desktop` 是客户端拉伸，服务端 `LOGPIXELSX=96`、`BOUNDS=1024x768` 不变。**根因**：应用只读 `ui.window().scale_factor()`，而 DPI 由**服务端显示驱动的真实缩放**决定。**唯一可行路径**：在该机 GUI 改真实显示缩放（125%/150%）并**注销重登**后取证（人工，每档约 1 分钟；脚本 `scripts/win-dpi.cmd` 可复用）。**字体回退仍未覆盖**：需未装 CJK 字体的干净环境，Windows 有雅黑/宋体故测不出 |
| D6 | 最终 Linux 发行格式、Android 32 位 ABI、AAB/商店发布 | 待验证 | 规格「开放问题与延期决定」登记为延期，不阻塞 V1 桌面优先路径 |

## 2. 已知限制（当前交付的内在边界）

| # | 限制 | 影响 | 处理口径 |
|---|---|---|---|
| L1 | ~~Android Release APK **未签名**~~ → **已闭环（2026-10-02）** | ~~不能作为发布包分发~~ → **已可分发**：`dist/app-release-signed.apk`；APK 哈希与签名证书指纹仅记录在本机发布信息中。`apksigner verify` → **Verifies**，`Verified using v3 scheme: true` | 已闭环。密钥与口令**一律不入库**：keystore 在仓库外（`~/.config/novel-reader/novel-reader-release.jks`，PKCS12），口令在 `~/.config/novel-reader/signing.env`（600），Gradle 经 `android/local.properties`（`.gitignore` 已忽略）的 `nrSigning.*` → 脚本转环境变量 `NR_SIGNING_*` 注入；配置模板见 `docs/android/device-runbook.md` §7。**缺参数时构建仍产出未签名包（属预期，不得当发布包）**；重出包时产物名按实际签名状态自动取 `app-release-signed.apk` / `app-release-unsigned.apk` |
| L2 | ~~项目自身**发行许可证未定**~~ → **已定：GPL-3.0-only（2026-10-02 主人裁定）** | ~~无法对外声明整体许可~~ → **已可声明**：`LICENSE`（GPLv3 官方原文，35149 B，源自 gnu.org）；根 `Cargo.toml` 的 `[workspace.package]` 与 6 个 crate（reading-core/source/index/storage/ffi + desktop-app）均标注 `license = "GPL-3.0-only"` + `license-file`，`cargo metadata --no-deps` 实测全部解析正确 | **已闭环**。选择依据：Slint 1.17.1 为三选一许可（见 L7），本项目走 **GPLv3 开源路径** → 成本 0、无署名/披露义务；代价是发布物须以 GPLv3 分发（与 MIT/Apache-2.0 生态兼容）。**若日后改闭源，必须重走 L2 裁定并同步改 LICENSE 与各 crate 字段**
| L2b | **新发现的两个许可族（2026-10-02）** | 需人工复核，但**风险低于 Slint** | 全仓扫描本地 registry 发现除 Slint 外还有：**BSL-1.0**（`error-code` 3.4.0、`clipboard-win` 5.4.1 —— 经 `.d` 证据确认**均未编译进 Linux 发布物**，属 Windows 平台专用，仅影响 Windows 发布物）与 **Unlicense**（`ksni` 0.3.6 —— 经 `.d` 证据确认**已编译进 Linux 发布物**）。**BSL-1.0 原文已核实**（boost.org/LICENSE_1_0.txt，SPDX BSL-1.0）：属 **permissive 类**（授权使用/复制/修改/分发，条件是保留版权与许可声明），**无 copyleft 传染、无署名之外的义务**；Unlicense 为公有领域贡献。两族均**不影响 L2 的整体许可证选择**，但须在发布说明的第三方清单中登记 |
| L3 | Android **传递依赖**许可未逐项登记 | ~~许可清单只覆盖直接声明~~ → **已闭环（2026-10-02）**：执行 `./gradlew :app:dependencies --configuration releaseRuntimeClasspath`（exit 0），解析 **83 个模块 / FAILED 0**；逐个读 Gradle 缓存 POM 的 `<licenses>`：**83 个均为 Apache-2.0**。其中 `com.google.guava:listenablefuture:1.0` 的 POM/JAR 内确实无声明，已**核对上游权威登记**（NuGet `Xamarin.Google.Guava.ListenableFuture` 包元数据 + Maven 索引）认定为 Apache-2.0，并在清单 §2 留痕。全量清单见 `results/verify/android-transitive-licenses.md`，原始输出见 `results/verify/logs/android-releaseRuntimeClasspath-deps.txt` | **已闭环**。口径提醒：该条依据来自**上游登记**而非构建产物内声明，发布说明引用时不得写成「产物内已声明」 |
| L4 | 平台专用依赖（Windows/macOS/iOS/Android 专用 crate）的许可未在本地解析 | 许可清单覆盖 Linux 发布物，不覆盖其他平台 | 证据：这些包不在 `crates/desktop-app/target/release/deps/*.d` 的编译集合内；发布对应平台时重新解析 |
| L4b | 规格 §2.1 第 5 条「完整分页完成前显示『计算中』」 | **已按诚实口径实现（2026-10-02），与规格字面不同** | 本项目是**按需惰性分页**（`PageSession::next_page()` 逐页计算，`total_pages` 概念全仓不存在），读者读到第 3 页退出时「完整分页」永不发生 —— 若照字面显示「计算中」，它会**永远显示**，反而是更坏的误导；若为凑总页数而强制全量分页，则与「有限窗口读取、不得全文载入」的核心约束直接冲突（1GB 文件分钟级阻塞）。**故实现口径为**：中间页只显示真实页序号（「第 N 页」），**不给**总页数；**仅当翻到末页（`eof=true`）时**才显示「第 N 页（共 M 页）」——此时 M = `page_index + 1` 是**真实值**（`page_index_for_offset` 取自已登记边界的精确位置，非字节比例估算）。守卫测试 `page_label_never_fabricates_total_pages` +端到端 `last_page_label_total_matches_actual_page_count`，并经变异测试（加「计算中」/ 伪造总页数 → 均 exit 101）证明有牙 |
| L4d | 设置页分组说明的呈现方式 | ~~说明被拼进分组标题行（`label + "　" + note`）~~ → **2026-10-04 已改为独立说明块** | 旧做法把长说明与标题拼成一行，窄窗（最小 560）**必然被裁切**，且与设计稿 §263「顶部一条解释」不符。**现状**：分组标题只显示标题本身（`app.slint` 的 `kind == 5` 不再拼 note）；说明另起**独立说明块**（新增 `kind = 7` 行 + 专用轻量组件 `SectionNote`：左侧语义色条 + 正文 `wrap: word-wrap` 自动换行 + 88px 行高，窄窗换行后需约 60px 足够）。**未复用** `InlineNotice`：它带「关闭」按钮与 `dismissed` 回调，而分组说明是**常驻说明**（无状态可存、无关闭语义），复用会导致用户点 × 后整块消失且无处恢复。**方案 A（主人裁定）**：说明块**仅归属确有未生效项的分组**（`section = 1`），「排版」「版式 · 边距」等全生效分组不挂说明。分组命名一并修正：原先「版式」出现两次 → 现为「版式 · 边距」与「版式 · 段落」。守卫测试 `section_header_never_inlines_note_and_notes_become_blocks` 经**三项有效变异**证明有牙（note 塞回标题行 / 制造分组重名 / 行高压到 20 均 exit 101） |

| L5 | 未包含连续滚动、全文搜索、云同步、ZIP/EPUB 等能力 | 功能范围限制 | 规格「范围外」，非缺陷 |
| L6 | 桌面发布目录为「完整发布目录」口径，不是单可执行文件 | 移动目录后需按 README 修正 `.desktop` 的 Exec 绝对路径 | `desktop/packaging/targets.json` 的 `packaging_mode_forbidden` 明确禁止单文件口径 |
| L7 | Slint 1.17.1 及其同族 crate（**13 个 package**，已实测确认）许可为 `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | **已随 L2 裁定闭环**：本项目采用 **GPL-3.0-only** 路径（Slint 三选一中的第①条），Slint 依赖合规 | **2026-10-02 已核实官方条款**（slint-ui/slint v1.17.1 `LICENSE.md`，三条路径**任选其一**）：<br>①**GPLv3** —— 开源软件、桌面/移动/Web/嵌入式均可，**零成本**，自身文件可保持 MIT/Apache-2.0；<br>②**Royalty-free-2.0** —— **专有**桌面/移动/Web 应用零成本，**排除嵌入式**，**必须披露使用 Slint**（如 `AboutSlint` 徽章），不披露则须转商业许可；<br>③**Slint Software License 3.0（商业）** —— 专有**含嵌入式**均需付费购买席位（按用户数计seat），非商业/开源项目免royalty。<br>**本项目定性**：桌面阅读器、非嵌入式、非商业产品 → 三条路径均可行，**GPLv3 成本最低**；13 个同族包清单：`slint`、`slint-macros`、`i-slint-{common,core,core-macros,compiler,backend-selector,backend-winit,backend-linuxkms,backend-testing,renderer-femtovg,renderer-software,renderer-skia}` 均为 1.17.1 |
| L8 | `scripts/verify-android-apk.py`（AN6 交付的 APK 内容/签名校验脚本，被发布门禁调用）在 REL1 期间被证据工具**越界删除并重建**：重建版 sha256 与原版不同（**原始字节不可复原**），且首版重建缺 SDK 根自动探测——默认调用把 aapt2/apksigner 外部交叉校验记为跳过项（exit 仍 0），而发布门禁当时不消费校验器的跳过项，该降级在门禁输出中不可见 | APK 判定一度退回纯 Python 解析层（判定结论仍与 AN6/独立取证记录一致）；**AN6 原始字节属不可逆取证损失** | 已由修复卡 `t_481c627b` 按「**行为等价**」修复并实测（不声称恢复原始 sha256）：① 恢复 SDK 根探测链（显式 `--sdk-root` → PATH → `$ANDROID_HOME` → `$ANDROID_SDK_ROOT` → `android/local.properties` 的 `sdk.dir` → 本机默认根），默认口径重新输出 `[ok] aapt2 / apksigner 独立复核一致…`；② 找不到时才跳过，文案如实列出「已试位置」，并把 `facts.sdk_probe` 写入 `--json`；③ 发布门禁消费校验器 `skips`，单列 `[降级]` 并计入自身跳过项。事实与逐条对照见 `results/rel1-release-scripts.md` §4.1（事故）与 `results/f-rel1b-1-2-fix.md`（修复与验收，含两脚本新 size/sha256） |

### 需人工复核的许可族（脚本输出，发布前必须逐项裁定）

- Rust：`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`（13 个 package，
  Slint 同族；见 L7）。
- Android：直接声明依赖中 `junit:junit:4.13.2` 为 Eclipse Public License 1.0（EPL-1.0），
  scope 为 `testImplementation`（**不随包分发**）。其余直接声明依赖为 Apache-2.0。
- 运行时系统库（`libfontconfig`/`libfreetype`/`libc` 等）由目标发行版提供，不计入随包第三方许可清单。

## 3. 文档精度项（gate-1 裁定，不阻断放行，但**非「已验证修复」**）

来源：`results/gates/gate-1-core.md` §5（O1–O7）。以下项属文档/措辞精度类，仍登记为未验证修复项：

| # | 项目 | 登记口径 |
|---|---|---|
| O1/O2 | 存储层不校验章节一致性（由上层裁定）；负偏移辅助函数口径 | 由上层裁定维持现状，行为未变 |
| O3/O4/O5 | 头文件措辞精度（STALE vs NOT_FOUND）、`nr_engine_create` 失败未清零 `out_engine`、COMMIT 被拒返回码未冻结 | 归下次 NFR-C4 变更批次统一处理；文档精度项 |
| O6 | `Cargo.lock` 记账偏差（现哈希 ≠ RC6 记录值，mtime 落在 RC7 窗口内） | 发布证据中**必须重新登记发布时哈希**（`verify-v1-release.py` 每次运行输出该哈希 + package 数），不得沿用 RC6 旧值 |
| O7 | 入口声明顺序差异 | 按「集合 18/18 + 两侧自洽」判定 |

配套证据口径缺陷（`results/verify/vrc2-storage-driver.py` 的 `source_tree_digest_*` 把 `results/verify`
自身纳入指纹）：不得以该字段作为「产品源码未变」的判据，改用冻结产物哈希与 `gate-1-core.md` §4.3 的
产品哈希审计。

## 4. 阻塞项（**当前无未关闭阻塞项**）

> **2026-10-02 更新**：原 B1（AC-A2 Kotlin 侧锁违规）**已关闭** —— 实测
> `python3 scripts/verify-android-boundary.py --header include/novel_reader.h --kotlin android/app/src/main/java`
> → **exit 0**（`RESULT: PASS … 断言 32 项，跳过项 0 个`；`reader/` 的 2 个取消入口不再取状态串行化门）。
> 下方保留关闭记录以供审计，**不再作为阻塞项**。

| # | 阻塞项（**已关闭**） | 原取证 | 关闭证据（2026-10-02 实测） |
|---|---|---|---|
| B1 | ~~AC-A2 未关闭~~：`scripts/verify-android-boundary.py` 默认口径曾 **exit 1**（2 项 Kotlin 侧锁违规：`reader/ReaderViewModel.kt` 的 `Mutex` / `synchronized`） | 曾 exit 1（`RESULT: FAIL (2 项违规)`）；裁定见 `results/gates/gate-3-android.md` §4 | **exit 0**，32 断言 PASS，跳过项 0（含 `[6b]` 取消入口不得取串行化门） |

约束（由 `results/gates/gate-final.md` 裁定，B1 关闭前生效；现 B1 已关闭，但下列第 1 条仍成立）：

- ~~在 B1 关闭前，**不得**宣称 Android 侧验收完成或全平台发布~~ → **B1 已于 2026-10-02 关闭**；
  但因 D1/D2/D3 仍为「待验证」，**仍不得**宣称全平台发布（含发布说明、README、提交信息）。
- V1 结论仍只能是「**TXT 桌面可用 + Android 可构建**」——本限制**不因** B1 关闭而放宽。
- `scripts/verify-v1-release.py` 每次运行都会**真实执行**该边界门禁；边界门禁现 exit 0，故不再登记阻塞项
  （该脚本退出码语义 = 发布清单是否完整，与本项无关）。

## 5. 数据安全与回滚

- 原始 TXT **不会被修改或删除**；书架移除与缓存清理只影响应用自身数据（规格 NFR6 与父级 C13）。
- 桌面数据目录：`$XDG_DATA_HOME/novel-reader/library.db`（SQLite）。备份方式：应用退出后复制该
  数据库文件即可；回滚方式：退出应用 → 用备份文件覆盖 `library.db` → 重新启动；迁移只新增版本，
  不覆盖用户数据。
- Android 数据目录：应用私有存储（`library.db` + 不可随机访问 Provider 的私有缓存副本）。
  回滚/清理：系统设置中清除应用数据（不影响原始 TXT 与 SAF 授权记录以外内容）。
- 版本回滚：桌面直接部署上一发布目录（完整目录口径，无安装器副作用）；Android 安装上一版本 APK
  （`versionCode` 递增时才允许覆盖安装，降级需先卸载并接受本地数据丢失）。
- 发布物校验：重新下载后用发布说明中的 SHA-256 复核；发布目录内可用 `MANIFEST.sha256` 自校验
  （`sha256sum -c MANIFEST.sha256`）。
