# win-dpi.cmd — Windows DPI 三档取证（由 win-remote.py 调度，勿单独执行）
@echo off
REM 改 Windows 显示缩放并逐档跑 novel-reader 的 NR_* 驱动，取证 DPI 是否真正生效。
REM 缩放档位通过 HKCU\Control Panel\Desktop 的 LogPixels（96=100%、120=125%、144=150%）设置。
REM 注意：改LogPixels 后必须**发 WM_SETTINGCHANGE 通知**，否则已运行的进程不会感知。
REM 取证判据：LogPixels 值 + 应用侧 NR-PAGE 的 window/viewport/lines 是否随之变化。
setlocal EnableDelayedExpansion

set "EXE=%USERPROFILE%\nr-verify\dist\novel-reader-0.3.0-windows-x86_64\novel-reader.exe"
set "BOOK=%USERPROFILE%\nr-verify\book.txt"
set "OUT=%USERPROFILE%\nr-verify\dpilog"
if not exist "%OUT%" mkdir "%OUT%"

REM ---- 备份原LogPixels（跑完还原）----
set "ORIG_LP=unset"
for /f "tokens=3" %%v in ('reg query "HKCU\Control Panel\Desktop" /v LogPixels 2^>nul ^| findstr LogPixels') do set "ORIG_LP=%%v"
> "%OUT%\backup.txt" echo ORIG_LOGPIXELS=%ORIG_LP%
echo [dpi] 原始 LogPixels=%ORIG_LP%

REM ---- 关闭已运行实例，避免旧进程干扰 ----
taskkill /F /IM novel-reader.exe >nul 2>&1

call :run_case 96  100
call :run_case 120 125
call :run_case 144 150

REM ---- 还原 LogPixels ----
if "%ORIG_LP%"=="unset" (
  reg delete "HKCU\Control Panel\Desktop" /v LogPixels /f >nul 2>&1
) else (
  reg add "HKCU\Control Panel\Desktop" /v LogPixels /t REG_DWORD /d %ORIG_LP% /f >nul 2>&1
)
echo [dpi] 已还原 LogPixels=%ORIG_LP%
type "%OUT%\summary.txt" 2>nul
exit /b 0

:run_case
set "LP=%~1"
set "LBL=%~2"
echo.
echo ====dpi-%LBL% (LogPixels=%LP%)====
REM 设 DPI
reg add "HKCU\Control Panel\Desktop" /v LogPixels /t REG_DWORD /d %LP% /f >nul 2>&1
REM 广播 WM_SETTINGCHANGE，让新进程读取新值
powershell -NoProfile -NonInteractive -Command ^
  "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class W{[DllImport(\"user32.dll\",CharSet=CharSet.Auto)]public static extern IntPtr SendMessageTimeout(IntPtr h,uint m,UIntPtr w,IntPtr l,uint f,uint t,out UIntPtr r);}' ;" ^
  "$r=[UIntPtr]::Zero;[void][W]::SendMessageTimeout([IntPtr]0xffff,0x1A,[UIntPtr]::Zero,[IntPtr]0,2,3000,[ref]$r)" >nul 2>&1

if not exist "%BOOK%" echo fixture missing> "%OUT%\dpi-%LBL%.log"
set "WORK=%OUT%\w-%LBL%"
if exist "%WORK%" rmdir /S /Q "%WORK%"
mkdir "%WORK%"
copy /Y "%USERPROFILE%\nr-verify\smoke.txt" "%WORK%\book.txt" >nul 2>&1

REM跑应用：导入 -> 打开书 -> 进正文 -> 自行退出
set "NR_DB_PATH=%WORK%\library.db"
set "NR_STATE_LOG=1"
set "NR_IMPORT_PATH=%WORK%\book.txt"
set "NR_START_SCREEN=shelf"
set "NR_SHELF_OPEN=1"
set "NR_EXIT_AFTER_MS=9000"
"%EXE%" > "%OUT%\dpi-%LBL%.log" 2>&1

REM 读回应用侧的排版证据
set "NRPAGE="
for /f "delims=" %%L in ('findstr /C:"NR-PAGE" "%OUT%\dpi-%LBL%.log" 2^>nul') do if not defined NRPAGE set "NRPAGE=%%L"
set "WINSZ="
for /f "tokens=2 delims==" %%v in ('powershell -NoProfile -NonInteractive -Command "(Get-ItemProperty 'HKCU:\Control Panel\Desktop' -Name LogPixels).LogPixels"') do set "CUR_LP=%%v"
>> "%OUT%\summary.txt" echo dpi-%LBL% LogPixels=%CUR_LP% exit=%ERRORLEVEL%
>> "%OUT%\summary.txt" echo   NR-PAGE=%NRPAGE%
exit /b 0