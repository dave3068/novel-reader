#!/usr/bin/env python3
"""Windows 远程验证助手 —— 通过 SSH 在 Windows 主机上做程序化取证。

用途：novel-reader 桌面端的 Windows 验证（启动、结构化状态、产物指纹）。
口径：
  * 认证口令**只出现在进程内存里**：本脚本从环境变量读取，**不写入任何文件**，
    也不接受命令行传入（避免进入 shell 历史 / ps 输出）。
  * 本机通常**没有 sshpass / pexpect**（实测 Deepin 20.9），故用标准库 `pty.fork()`
    直接驱动 `ssh` / `scp` 并在收到 password 提示时写入口令。
  * Windows PowerShell 5.1 的 `ConvertTo-Json` 会混进 CLIXML 进度流，
    因此远端一律「写 JSON 文件 → 再读回」，并在命令前加 `$ProgressPreference='SilentlyContinue'`。

环境变量（必填）：
  NR_WIN_HOST / NR_WIN_USER / NR_WIN_PASS
可选：
  NR_WIN_SSH_OPTS   额外 ssh 选项（空格分隔）
  NR_WIN_TIMEOUT    单条命令超时秒数（默认 180）

命令行：
  python3 scripts/win-remote.py probe                     # 环境探查
  python3 scripts/win-remote.py ps <文件路径>              # 远端执行 .ps1（-File 方式）
  python3 scripts/win-remote.py psb64 <ps1源码>            # 远端执行 ps1 源码（Base64 传入）
  python3 scripts/win-remote.py run <exe相对路径> [环境变量赋值...]  # 启动验证（NR_* 驱动）
  python3 scripts/win-remote.py gethash <远端文件路径>
"""
from __future__ import annotations

import base64
import json
import os
import pty
import re
import select
import subprocess
import sys
import time

HOST = os.environ.get("NR_WIN_HOST", "")
USER = os.environ.get("NR_WIN_USER", "")
PASS = os.environ.get("NR_WIN_PASS", "")
TIMEOUT = int(os.environ.get("NR_WIN_TIMEOUT", "180"))
EXTRA = os.environ.get("NR_WIN_SSH_OPTS", "").split()

BASE = ["-o", "StrictHostKeyChecking=accept-new", "-o", "ConnectTimeout=15",
        "-o", "NumberOfPasswordPrompts=1",
        "-o", "PreferredAuthentications=password,keyboard-interactive"]


def _drive(argv: list[str], timeout: int, feeder=None):
    """用 pty 驱动一个交互式程序，自动应答 password 提示。返回 (rc, 输出)。"""
    pid, fd = pty.fork()
    if pid == 0:
        os.execvp(argv[0], argv)
    out, sent, rc = b"", False, None
    end = time.time() + timeout
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 1.0)
        if fd in r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            if not sent and b"assword" in out[-300:]:
                os.write(fd, (PASS + "\n").encode())
                sent = True
            if feeder:
                feeder(chunk, sent)
        done, st = os.waitpid(pid, os.WNOHANG)
        if done:
            rc = os.waitstatus_to_exitcode(st)
            # 排空剩余输出
            while True:
                r2, _, _ = select.select([fd], [], [], 0.4)
                if fd not in r2:
                    break
                try:
                    c = os.read(fd, 65536)
                except OSError:
                    break
                if not c:
                    break
                out += c
            break
    if rc is None:
        try:
            os.kill(pid, 9)
        except Exception:
            pass
        rc = -1
    text = out.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "")
    text = re.sub(r"^%s@.*password:.*$" % re.escape(USER), "", text, flags=re.M)
    return rc, text.strip()


def ssh(cmd: str, timeout: int | None = None) -> tuple[int, str]:
    argv = ["ssh", *BASE, *EXTRA, f"{USER}@{HOST}", cmd]
    return _drive(argv, timeout or TIMEOUT)


def scp(local: str, remote: str, timeout: int | None = None) -> tuple[int, str]:
    argv = ["scp", *BASE, *EXTRA, local, f"{USER}@{HOST}:{remote}"]
    return _drive(argv, timeout or TIMEOUT)


def ps_encoded(script: str, timeout: int | None = None) -> tuple[int, str]:
    """把 ps1 源码以 UTF-16LE Base64 传入（规避 cmd 引号地狱与编码问题）。"""
    enc = base64.b64encode(script.encode("utf-16-le")).decode()
    return ssh(f"powershell -NoProfile -NonInteractive -EncodedCommand {enc}", timeout)


def ps_json(script: str, out_marker: str, timeout: int | None = None) -> dict:
    """执行 ps1 并取回 JSON（远端写文件→读回，避免 CLIXML 污染）。"""
    wrapped = (
        "$ProgressPreference='SilentlyContinue';"
        "$ErrorActionPreference='Continue';"
        + script
        + f";$j|Out-File -Encoding utf8 '{out_marker}';Get-Content '{out_marker}'"
    )
    rc, out = ps_encoded(wrapped, timeout)
    m = re.search(r"\{.*\}", out, re.S)
    if not m:
        raise RuntimeError(f"远端未返回 JSON（rc={rc}）：\n{out[-1500:]}")
    return json.loads(m.group(0).replace("﻿", ""))


def _require_env() -> None:
    missing = [k for k in ("NR_WIN_HOST", "NR_WIN_USER", "NR_WIN_PASS") if not os.environ.get(k)]
    if missing:
        sys.exit("缺少环境变量：" + ", ".join(missing) + "（口令只经环境变量传入，不写文件）")


def cmd_probe(_args: list[str]) -> int:
    d = r"C:\Users\%s\nr-verify" % USER
    script = (
        f"$d='{d}';New-Item -ItemType Directory -Force -Path $d|Out-Null;"
        "$r=[ordered]@{"
        "os=(Get-CimInstance Win32_OperatingSystem).Caption;"
        "osver=[string](Get-CimInstance Win32_OperatingSystem).Version;"
        "build=(Get-CimInstance Win32_OperatingSystem).BuildNumber;"
        "arch=$env:PROCESSOR_ARCHITECTURE;"
        "isAdmin=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent())"
        ".IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);"
        "cargo=(Get-Command cargo -EA SilentlyContinue).Source;"
        "rustc=(Get-Command rustc -EA SilentlyContinue).Source;"
        "cl=(Get-Command cl -EA SilentlyContinue).Source;"
        "link=(Get-Command link -EA SilentlyContinue).Source;"
        "pwsh=(Get-Command pwsh -EA SilentlyContinue).Source;"
        "git=(Get-Command git -EA SilentlyContinue).Source;"
        "vswhere=(Test-Path \"${env:ProgramFiles(x86)}\\Microsoft Visual Studio\\Installer\\vswhere.exe\");"
        "vs=(Get-ChildItem 'C:\\Program Files\\Microsoft Visual Studio' -EA SilentlyContinue|Select-Object -Expand Name);"
        "psv=$PSVersionTable.PSVersion.ToString()};"
        "$j=$r|ConvertTo-Json -Compress"
    )
    print(json.dumps(ps_json(script, d + r"\probe.json"), ensure_ascii=False, indent=1))
    return 0


def cmd_gethash(args: list[str]) -> int:
    if not args:
        sys.exit("用法：gethash <远端文件路径>")
    path = args[0]
    d = r"C:\Users\%s\nr-verify" % USER
    # 注意：不得用 f-string —— 脚本里含 PowerShell 的 hash 字面量 @{…} 与 $(if…{…})，
    # 其中的 { } 会被 Python 当作 f-string 的替换域而报语法错。
    script = (
        "$p='" + path + "';$r=[ordered]@{exists=(Test-Path $p);"
        "bytes=$(if(Test-Path $p){(Get-Item $p).Length}else{0});"
        "sha256=$(if(Test-Path $p){(Get-FileHash $p -Algorithm SHA256).Hash.ToLower()}else{$null})};"
        "$j=$r|ConvertTo-Json -Compress"
    )
    print(json.dumps(ps_json(script, d + r"\hash.json"), ensure_ascii=False))
    return 0


def cmd_run(args: list[str]) -> int:
    """启动验证：用桌面端内置的 NR_* 驱动（与 Linux 侧 smoke-desktop.sh 同一套判据）。

    发布目录由 `NR_WIN_RELDIR` 指定 —— 此前硬编码 `novel-reader-0.2.0-windows-x86_64`，
    换版本后**静默指向旧目录**（实测踩中：0.5.0 的启动验证会跑到 0.2.0 的产物上）。
    """
    exe = args[0] if args else "novel-reader.exe"
    d = os.environ.get("NR_WIN_RELDIR", "")
    if not d:
        sys.exit("缺少 NR_WIN_RELDIR（远端发布目录绝对路径），例："
                 r"C:\Users\<user>\nr-verify\dist\novel-reader-0.5.0-windows-x86_64")
    envs = args[1:]
    sets = "".join(f"$env:{e.split('=',1)[0]}='{e.split('=',1)[1]}';" for e in envs)
    script = (
        "$ProgressPreference='SilentlyContinue';"
        f"$d='{d}';"
        f"{sets}"
        "$env:NR_STATE_LOG='1';"
        "if(-not $env:NR_START_SCREEN){$env:NR_START_SCREEN='shelf'};"
        "if(-not $env:NR_EXIT_AFTER_MS){$env:NR_EXIT_AFTER_MS='5000'};"
        "$out=\"$d\\..\\launch.log\";$err=\"$d\\..\\launch.err\";"
        "Remove-Item $out,$err -EA SilentlyContinue;"
        f"$p=Start-Process -FilePath \"$d\\{exe}\" -WorkingDirectory $d -PassThru -NoNewWindow "
        "-RedirectStandardOutput $out -RedirectStandardError $err;"
        "$exited=$p.WaitForExit(60000);"
        "if(-not $exited){$p.Kill();$timedOut=$true}else{$timedOut=$false};"
        "$log=@(Get-Content $out -EA SilentlyContinue);"
        "$st=@($log|Where-Object{$_ -match 'NR-STATE'});"
        "$r=[ordered]@{exited=$exited;timedOut=$timedOut;"
        "exitCode=$(if($exited){$p.ExitCode}else{$null});"
        "nrStateLines=$st.Count;stderr=@(Get-Content $err -EA SilentlyContinue|Select-Object -First 10)};"
        "$j=$r|ConvertTo-Json -Compress"
    )
    print(json.dumps(ps_json(script, d + r"\..\run.json", timeout=180), ensure_ascii=False, indent=1))
    return 0


def cmd_psb64(args: list[str]) -> int:
    if not args:
        sys.exit("用法：psb64 <ps1 源码（单行）>")
    rc, out = ps_encoded(args[0])
    print(out)
    return 0


def cmd_ps(args: list[str]) -> int:
    if not args:
        sys.exit("用法：ps <远端 ps1 路径>")
    rc, out = ssh(f'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{args[0]}"')
    print(out)
    return rc


def cmd_push(args: list[str]) -> int:
    """把本地文件推到远端（复用 pty 认证；口令只从环境读取，不落盘）。"""
    if len(args) < 2:
        sys.exit("用法：push <本地路径> <远端路径>")
    rc, out = scp(args[0], args[1])
    print(out or f"rc={rc}")
    return rc


def cmd_pull(args: list[str]) -> int:
    """把远端文件/目录拉回本地。"""
    if len(args) < 2:
        sys.exit("用法：pull <远端路径> <本地路径>")
    argv = ["scp", *BASE, *EXTRA, "-r", f"{USER}@{HOST}:{args[0]}", args[1]]
    rc, out = _drive(argv, TIMEOUT)
    print(out or f"rc={rc}")
    return rc


COMMANDS = {
    "probe": cmd_probe,
    "gethash": cmd_gethash,
    "run": cmd_run,
    "psb64": cmd_psb64,
    "ps": cmd_ps,
    "push": cmd_push,
    "pull": cmd_pull,
}


def main(argv: list[str]) -> int:
    _require_env()
    if not argv or argv[0] not in COMMANDS:
        sys.exit("用法：" + " | ".join(COMMANDS))
    return COMMANDS[argv[0]](argv[1:])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))