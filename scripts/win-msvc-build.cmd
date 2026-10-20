@echo off
REM MSVC 官方口径构建：载入 vcvars64 后执行 targets.json 的 build_command。
REM 由 scripts/win-remote.py 调度；日志与结果 JSON 落在 %OUT%。
setlocal

set "SRC=%USERPROFILE%\nr-src"
set "OUT=%USERPROFILE%\nr-verify"
set "LOG=%OUT%\build.log"
set "VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set "CARGO=%USERPROFILE%\.cargo\bin"

if not exist "%VCVARS%" (
  echo VCVARS_MISSING> "%OUT%\b1.json"
  exit /b 2
)

call "%VCVARS%" > "%OUT%\vcvars.log" 2>&1
if errorlevel 1 (
  echo VCVARS_FAILED> "%OUT%\b1.json"
  exit /b 3
)

set "PATH=%CARGO%;%PATH%"
cd /d "%SRC%"

REM targets.json 官方 build_command（原文）:
REM   cargo build --manifest-path crates/desktop-app/Cargo.toml --release --target x86_64-pc-windows-msvc
cargo build --manifest-path crates/desktop-app/Cargo.toml --release --target x86_64-pc-windows-msvc > "%LOG%" 2>&1
set "RC=%ERRORLEVEL%"

set "EXE=%SRC%\crates\desktop-app\target\x86_64-pc-windows-msvc\release\desktop-app.exe"
if exist "%EXE%" (set "EXE_EXISTS=true") else (set "EXE_EXISTS=false")
for %%F in ("%EXE%") do set "EXE_BYTES=%%~zF"

> "%OUT%\b1.json" echo {"exitCode":%RC%,"exeExists":"%EXE_EXISTS%","exeBytes":"%EXE_BYTES%","log":"%LOG%"}
exit /b %RC%
