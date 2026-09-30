# MIUI / 澎湃（HyperOS）真机安装与调试开关限制

适用卡片：AN6（`t_66d593c6`）；配合 `docs/android/device-runbook.md` 使用。
本文件只记录**平台限制与绕行手段**，不含真机结论：下方各「期望结果」在具备设备前一律保持 `待验证`。

## 1. 两个必须手动开启的开关

MIUI / 澎湃默认关闭以下开关，会同时阻断三件事，且 `adb shell settings put …` / `svc power` **无法绕过**
（adb shell 身份不具备 `WRITE_SETTINGS` / `WRITE_SECURE_SETTINGS`）：

| 能力 | 被阻断时的报错 | 需开启（开发者选项） |
|---|---|---|
| 静默安装 APK（`adb install`） | `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user` | **USB 安装** |
| `adb shell input tap/keyevent` 注入触控 | `SecurityException: requires INJECT_EVENTS permission` | **USB 调试（安全设置）** |
| 安装测试 APK（instrumentation / `adb shell am instrument`） | 同安装限制（`INSTALL_FAILED_USER_RESTRICTED`） | **USB 调试（安全设置）** |

开启路径：设置 → 我的设备 → 全部参数 → 连点 MIUI/HyperOS 版本号 7 次 → 返回设置 → 更多设置 →
开发者选项 → 打开「USB 调试」，再分别打开「USB 安装」与「USB 调试（安全设置）」。
部分版本需插入 SIM 卡并登录小米账号，开关才可点。

## 2. 无开关时的替代手段

| 目的 | 替代手段 | 期望结果 |
|---|---|---|
| 安装 APK | 把 `dist/app-debug.apk` 拷入手机存储，用文件管理器点击安装 | 安装成功（绕过 USB 安装限制） |
| 触发界面交互 | `adb shell monkey -p com.novelreader.android --throttle 150 -v 30` | 走 instrumentation 路径可注入触控，无 `INJECT_EVENTS` 报错 |
| 层级/边界断言 | `adb shell uiautomator dump` 后解析 XML 节点边界 | 拿到控件坐标与文本，无需特殊权限 |
| 崩溃排查 | `adb logcat -d` | 能读到本应用 `FATAL EXCEPTION`（若存在） |

## 3. 期望结果与判读（全部保持 待验证）

| 步骤 | 操作 | 期望结果 | 状态 |
|---|---|---|---|
| M1 | 开启「USB 安装」后 `adb install -r dist/app-debug.apk` | 输出 `Success`，无 `INSTALL_FAILED_USER_RESTRICTED` | 待验证 |
| M2 | 开启「USB 调试（安全设置）」后 `adb shell input tap x y` | 无 `SecurityException: requires INJECT_EVENTS permission` | 待验证 |
| M3 | 未开启时执行 `adb shell input tap x y` | 报 `INJECT_EVENTS` 权限错误（作为开关状态的判据） | 待验证 |
| M4 | 关闭「USB 安装」时 `adb install` | 报 `INSTALL_FAILED_USER_RESTRICTED`，据此确认是开关而非包体问题 | 待验证 |
| M5 | 拷贝 APK 到手机本地点击安装 | 安装成功（无 adb 依赖的兜底路径） | 待验证 |

## 4. 崩溃判读：shell 进程 ≠ 应用进程

`adb logcat | grep "FATAL EXCEPTION"` 会把 **shell 进程**的崩溃也扫进来，例如
`com.android.commands.svc.Svc`、`uiautomator`、`com.android.commands.monkey` 自身异常。
判定应用是否崩溃必须看堆栈首行的进程名/PID 是否属于 `com.novelreader.android`：

```bash
adb logcat -d > results/verify/android-device-logs/logcat.txt
adb logcat -d | grep -A5 "FATAL EXCEPTION"
# 期望：无 com.novelreader.android 进程的 FATAL EXCEPTION；若命中，须记录完整堆栈
```

以上三节的真机观察项，在具备设备并实际执行后由 V-AN2 卡写入
`results/verify/android-device-verify.md` 与附件目录 `results/verify/android-device-logs/`。
