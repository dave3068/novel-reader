@echo off
REM 计划任务包装：让 MSVC 构建**完全脱离 SSH 会话进程树**。
REM 起因（2026-10-03 实测两次）：用 `Start-Process ... -WindowStyle Hidden` 投放后，
REM SSH 会话一断，子进程树被回收 → 日志停在 manifest 阶段、exe 未产出、无error。
REM 根治：改用 `schtasks /Create /SC ONCE`，任务由 Task Scheduler 服务拉起，
REM 与 SSH 会话**无父子关系**，会话断开也不受影响。
REM
REM 用法（远端）：
REM   schtasks /Create /TN nr-build /TR "C:\...\build.cmd" /SC ONCE /ST <HH:mm> /F
REM   schtasks /Run /TN nr-build
REM   （轮询 build.log / b1.json 判断完成）
setlocal EnableDelayedExpansion

set "O=%USERPROFILE%\nr-verify"
set "LOG=%O%\build.log"

REM 清掉上一轮残留，确保本轮从头编译
del /q "%O%\b1.json" >nul 2>&1
if exist "%LOG%" del /q "%LOG%"

echo [%DATE% %TIME%] scheduled build start >> "%O%\task.log"
call "%O%\build.cmd" >> "%O%\task.log" 2>&1
set "RC=%ERRORLEVEL%"
echo [%DATE% %TIME%] scheduled build exit=%RC% >> "%O%\task.log"
exit /b %RC%