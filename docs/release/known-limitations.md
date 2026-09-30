# 已知限制与延期验收项登记（V1）

> 用途：发布说明的「已知限制」章节从本文件引用；`scripts/verify-v1-release.py` 会校验本文件已登记
> 以下必需条目（缺一项即门禁失败）。**本文件不产生任何通过结论**：所有未实测项一律为 `待验证`。

## 1. 延期验收项（未实测，不得写「通过」）

| # | 项目 | 当前状态 | 说明 / 闭环前置 |
|---|---|---|---|
| D1 | Android 真机（SAF 选择与持久授权、旋转、后台恢复、系统返回、JNI 取消、功耗） | 待验证 | 需 Android 12+ 真机；步骤见 `docs/android/device-runbook.md`，证据约定写入 `results/verify/android-device-verify.md`；就绪度由 `scripts/verify-android-device-readiness.py` 校验（真机执行前保持「待验证（文件未写入）」） |
| D2 | Windows 10/11 x86_64 构建、打包与启动 | 待验证 | 本机为 Deepin 20.9/Linux，无 pwsh、无 MSVC 工具链；状态与未验证项见 `desktop/packaging/targets.json` 的 `windows-x86_64` |
| D3 | Linux Wayland 构建产物启动与渲染/输入一致性 | 待验证 | 本机无 Wayland 合成器会话；见 `targets.json` 的 `linux-wayland` |
| D4 | 桌面完整发布目录在**干净目标发行版**（无 Slint 编译依赖、仅系统运行库）上的安装与启动 | 待验证 | 现有证据只在构建机 `/tmp` 组装并启动；口径：`bash desktop/packaging/linux/package-linux.sh --backend x11 --out dist` |
| D5 | 100%/125%/150% DPI、字体回退（无 `fonts-noto-cjk` 时的 tofu 表现） | 待验证 | 需人工与目标环境实测；程序化回退见桌面 checklist C4/C5/C6 |
| D6 | 最终 Linux 发行格式、Android 32 位 ABI、AAB/商店发布 | 待验证 | 规格「开放问题与延期决定」登记为延期，不阻塞 V1 桌面优先路径 |

## 2. 已知限制（当前交付的内在边界）

| # | 限制 | 影响 | 处理口径 |
|---|---|---|---|
| L1 | Android Release APK **未签名**（无正式签名配置） | 不能作为发布包分发 | 未签名属当前预期（`dist/BUILD-INFO.md` 已标注）；按规格 §15.2，发布前必须配置正式签名并重新出包，签名密钥不入版本库 |
| L2 | 项目自身**发行许可证未定** | 无法对外声明整体许可 | 规格登记为延期项；发布前必须确定并写入发布说明 |
| L3 | Android **传递依赖**许可未逐项登记 | 许可清单只覆盖直接声明 | 闭环命令：`cd android && ./gradlew :app:dependencies --configuration releaseRuntimeClasspath`，产物并入发布说明 |
| L4 | 平台专用依赖（Windows/macOS/iOS/Android 专用 crate）的许可未在本地解析 | 许可清单覆盖 Linux 发布物，不覆盖其他平台 | 证据：这些包不在 `crates/desktop-app/target/release/deps/*.d` 的编译集合内；发布对应平台时重新解析 |
| L5 | 未包含连续滚动、全文搜索、云同步、ZIP/EPUB 等能力 | 功能范围限制 | 规格「范围外」，非缺陷 |
| L6 | 桌面发布目录为「完整发布目录」口径，不是单可执行文件 | 移动目录后需按 README 修正 `.desktop` 的 Exec 绝对路径 | `desktop/packaging/targets.json` 的 `packaging_mode_forbidden` 明确禁止单文件口径 |
| L7 | Slint 1.17.1 及其同族 crate（13 个 package）许可为 `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`，属**需人工复核**许可族 | 项目整体可分发条件依赖该项目自身许可证的选择；宽松许可不成立时需采用 Slint 的免版税/商业条款 | 依赖许可清单由 `verify-v1-release.py` 输出（含「需人工复核」标记）；选择项目许可证时必须与 Slint 条款一并裁定并登记到 L2 |
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

## 4. 阻塞项（未关闭，**不得**计为通过，也不得并入「已知限制」）

| # | 阻塞项 | 取证（真实命令 / 退出码） | 关闭条件 |
|---|---|---|---|
| B1 | **AC-A2 未关闭**：`scripts/verify-android-boundary.py` 默认口径 **exit 1**（2 项 Kotlin 侧锁违规：`reader/ReaderViewModel.kt` 的 `Mutex` / `synchronized`） | `python3 scripts/verify-android-boundary.py --header include/novel_reader.h --kotlin android/app/src/main/java` → exit 1（`RESULT: FAIL (2 项违规)`）；裁定与作用域核证见 `results/gates/gate-3-android.md` §4 | 修复卡 `t_15bbd0e6` 闭环：默认口径 exit 0，且全量单测不低于基线 186 tests / 0 failures / 0 errors |

约束（由 `results/gates/gate-final.md` 裁定）：

- V1 结论只能是「**TXT 桌面可用 + Android 可构建**」；Android 侧只能声明「可构建 / 可测试 / 可校验」。
- 在 B1 关闭前，**不得**宣称 Android 侧验收完成或全平台发布（含发布说明、README、提交信息）。
- `scripts/verify-v1-release.py` 每次运行都会**真实执行**该边界门禁，并把未关闭状态登记为「阻塞项」
  （既不写入「未验证项」延期口径，也不计为「已通过」，且不改变该脚本的退出码语义）。

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
