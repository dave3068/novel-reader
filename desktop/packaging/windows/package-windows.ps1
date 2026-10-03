# package-windows.ps1 — DT6 Windows 10/11（x86_64 MSVC）构建与**完整发布目录**打包脚本。
#
# 口径（卡片 `t_e83b8ad0` / desktop checklist C7 / desktop/tasks.md D-T6）：
#   1) **完整发布目录**，不使用「单可执行文件」口径：产出
#        novel-reader-<版本>-windows-x86_64/
#          novel-reader.exe
#          start-novel-reader.cmd      启动包装（把 dlls\ 加入 PATH 后启动）
#          dlls\                       随包 DLL（未 -StaticCrt 且探测到 MSVC 运行库时）
#          fonts\                      仅 -BundleFont 时（Windows 不走 fontconfig，需系统安装）
#          docs\{README.txt,platform-requirements.md}
#          VERSION / MANIFEST.sha256
#   2) 目标清单的单一事实来源是 `desktop/packaging/targets.json`；本脚本只产出发布目录。
#   3) **不伪造平台结论**：本脚本在 Windows 上运行才有意义；本次交付的开发机为 DeepLinux，
#      没有 pwsh / MSVC，**该脚本从未在任何 Windows 主机上执行过**（targets.json 中该目标为「待验证」）。
#
# 用法（在 Windows 10/11 + MSVC 工具链 + Rust 1.98 下）：
#   pwsh -NoProfile -File desktop\packaging\windows\package-windows.ps1 -OutDir dist
#   pwsh -NoProfile -File desktop\packaging\windows\package-windows.ps1 -DryRun
#   pwsh -NoProfile -File desktop\packaging\windows\package-windows.ps1 -StaticCrt -BundleFont C:\fonts\NotoSansCJKsc-Regular.otf -Zip
#
# 退出码：0 = 成功（或 dry-run 通过）；1 = 打包/自检失败；2 = 前提缺失（工具/构件/参数）。

[CmdletBinding()]
param(
    [string]$OutDir = (Join-Path $PSScriptRoot '..\..\..\dist'),
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$Version = '',
    [string[]]$BundleFont = @(),
    [switch]$NoBuild,
    [switch]$StaticCrt,
    [switch]$Zip,
    [switch]$Force,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# 仓库根：脚本位于 desktop/packaging/windows/，故需上溯**三级**（..\..\..）。
# 此前误写 ..\..（只到 desktop/），导致 $AppManifest 解析成 desktop\crates\... 而 exit 2
# （2026-10-02 在真实 Windows 主机实测暴露；Linux 侧 package-linux.sh 用 ../../.. 是正确的）。
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$AppManifest = Join-Path $RepoRoot 'crates\desktop-app\Cargo.toml'
$DepsDoc = Join-Path $RepoRoot 'docs\desktop\platform-requirements.md'
# 应用图标（设计事实来源：docs\design\icon\novel-reader.ico）；随包分发，
# exe 内嵌图标需 rc.exe 资源编译，属未实测项（见 targets.json 的 windows-x86_64）。
$IconSrc = Join-Path $RepoRoot 'docs\design\icon\novel-reader.ico'

# 目标三元组 → 发布目录名里的架构段
$ArchSegment = switch ($Target) {
    'x86_64-pc-windows-msvc' { 'x86_64' }
    'aarch64-pc-windows-msvc' { 'aarch64' }
    default { throw "不支持的 -Target：$Target" }
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    if (-not (Test-Path -LiteralPath $AppManifest)) {
        Write-Error "PRECONDITION-MISSING: 缺少 $AppManifest" -ErrorAction Continue
        exit 2
    }
    $line = Select-String -LiteralPath $AppManifest -Pattern '^version\s*=\s*"([^"]+)"' |
        Select-Object -First 1
    if ($null -eq $line) {
        Write-Error "PRECONDITION-MISSING: 无法从 $AppManifest 解析 package.version" -ErrorAction Continue
        exit 2
    }
    $Version = $line.Matches[0].Groups[1].Value
}

$ReleaseDirName = "novel-reader-$Version-windows-$ArchSegment"
$ReleaseDir = Join-Path $OutDir $ReleaseDirName
$BuildDir = Join-Path $RepoRoot "crates\desktop-app\target\$Target\release"
$BuiltExe = Join-Path $BuildDir 'desktop-app.exe'

# 发布目录必含项（required=true，与 desktop/packaging/targets.json 一致）
$RequiredFiles = @(
    'novel-reader.exe',
    'start-novel-reader.cmd',
    'docs\README.txt',
    'docs\platform-requirements.md',
    'VERSION',
    'MANIFEST.sha256'
)
$RequiredDirs = @('dlls', 'docs')

Write-Host "package-windows: repo=$RepoRoot"
Write-Host "NR-PKG target=$Target version=$Version static_crt=$([bool]$StaticCrt)"
Write-Host "NR-PKG release_dir=$ReleaseDir"

if ($DryRun) {
    Write-Host '[1] 发布目录布局（完整发布目录口径，非单可执行文件）'
    foreach ($item in $RequiredFiles) { Write-Host ("    file  {0}" -f $item) }
    foreach ($item in $RequiredDirs) { Write-Host ("    dir   {0}\" -f $item) }
    if ($BundleFont.Count -gt 0) {
        Write-Host ("    dir   fonts\        （-BundleFont 指定 {0} 个字体）" -f $BundleFont.Count)
    } else {
        Write-Host '    可选  fonts\        （未使用 -BundleFont，依赖系统已安装中文字体）'
    }
    Write-Host '[2] 依赖与系统要求文档：docs\desktop\platform-requirements.md'
    if (Test-Path -LiteralPath $DepsDoc) {
        Write-Host ("    OK  文档存在（{0} 字节）" -f (Get-Item -LiteralPath $DepsDoc).Length)
    } else {
        Write-Host '    WARN 文档缺失（依赖说明将不随包分发）'
    }
    Write-Host ("[3] 构建命令：cargo build --manifest-path crates\desktop-app\Cargo.toml --release --target $Target")
    Write-Host "    构件：$BuiltExe"
    if (Test-Path -LiteralPath $BuiltExe) {
        Write-Host ("    OK  已存在（{0} 字节）" -f (Get-Item -LiteralPath $BuiltExe).Length)
    } else {
        Write-Host '    待构建（dry-run 不构建）'
    }
    if ($StaticCrt) {
        Write-Host '[4] -StaticCrt：RUSTFLAGS=-C target-feature=+crt-static（不依赖 vcruntime140.dll）'
    } else {
        Write-Host '[4] 默认动态 CRT：需要 MSVC 运行库（未随包分发时要求目标机已安装 VC++ 可再发行组件）'
    }
    Write-Host '[5] 未实测声明：本脚本未在 Windows 主机执行过；windows-x86_64 目标在 targets.json 中为「待验证」'
    Write-Host '[6] 目标清单：desktop/packaging/targets.json（校验：python3 scripts/verify-desktop-targets.py --dry-run）'
    Write-Host 'RESULT: dry-run OK（未构建、未落盘）'
    exit 0
}

# ---------------------------------------------------------------------------
# [1] 前提与构建
# ---------------------------------------------------------------------------
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($null -eq $cargo) {
    Write-Error 'PRECONDITION-MISSING: 缺少 cargo（请安装 Rust 1.98 并选择 MSVC 工具链）' -ErrorAction Continue
    exit 2
}
if (-not $NoBuild) {
    Write-Host '[1] Release 构建'
    if ($StaticCrt) {
        $env:RUSTFLAGS = '-C target-feature=+crt-static'
        Write-Host '    RUSTFLAGS=-C target-feature=+crt-static'
    }
    & cargo build --manifest-path $AppManifest --release --target $Target
    if ($LASTEXITCODE -ne 0) {
        Write-Error "FAILURE: cargo build 退出码 $LASTEXITCODE" -ErrorAction Continue
        exit 1
    }
} else {
    Write-Host '[1] 跳过构建（-NoBuild）'
}
if (-not (Test-Path -LiteralPath $BuiltExe)) {
    Write-Error "PRECONDITION-MISSING: 缺少构件 $BuiltExe" -ErrorAction Continue
    exit 2
}

# ---------------------------------------------------------------------------
# [2] 输出目录
# ---------------------------------------------------------------------------
if (Test-Path -LiteralPath $ReleaseDir) {
    if (-not $Force) {
        Write-Error "FAILURE: 输出目录已存在：$ReleaseDir（加 -Force 覆盖）" -ErrorAction Continue
        exit 1
    }
    if (-not $ReleaseDirName.StartsWith('novel-reader-')) {
        Write-Error "FAILURE: 拒绝删除非发布目录路径：$ReleaseDir" -ErrorAction Continue
        exit 1
    }
    Remove-Item -LiteralPath $ReleaseDir -Recurse -Force
}
Write-Host "[2] 组装发布目录：$ReleaseDir"
$null = New-Item -ItemType Directory -Force -Path (Join-Path $ReleaseDir 'dlls'),
    (Join-Path $ReleaseDir 'docs') | Out-Null

Copy-Item -LiteralPath $BuiltExe -Destination (Join-Path $ReleaseDir 'novel-reader.exe') -Force

if (Test-Path -LiteralPath $IconSrc) {
    Copy-Item -LiteralPath $IconSrc -Destination (Join-Path $ReleaseDir 'novel-reader.ico') -Force
    Write-Host "[2b] 应用图标：novel-reader.ico 已随包分发"
} else {
    Write-Warning "缺少 $IconSrc，未随包分发图标"
}

# ---------------------------------------------------------------------------
# [3] 启动包装（把 dlls\ 加入 PATH，原样转发参数）
# ---------------------------------------------------------------------------
$launcher = @'
@echo off
rem novel-reader 启动包装：把随包 DLL 目录加入 PATH 后启动，原样转发参数与环境。
setlocal
set "APPDIR=%~dp0"
if exist "%APPDIR%dlls" set "PATH=%APPDIR%dlls;%PATH%"
start "" "%APPDIR%novel-reader.exe" %*
endlocal
'@
Set-Content -LiteralPath (Join-Path $ReleaseDir 'start-novel-reader.cmd') -Value $launcher -Encoding ascii

# ---------------------------------------------------------------------------
# [4] 随包字体（可选）
# ---------------------------------------------------------------------------
if ($BundleFont.Count -gt 0) {
    $fontDir = Join-Path $ReleaseDir 'fonts'
    $null = New-Item -ItemType Directory -Force -Path $fontDir | Out-Null
    foreach ($font in $BundleFont) {
        if (-not (Test-Path -LiteralPath $font)) {
            Write-Error "FAILURE: 字体文件不存在：$font" -ErrorAction Continue
            exit 1
        }
        Copy-Item -LiteralPath $font -Destination $fontDir -Force
    }
    Write-Host ("[4] 随包字体：{0} 个 → fonts\（Windows 不走 fontconfig，仍需目标机安装或手工安装该字体）" -f $BundleFont.Count)
}

# ---------------------------------------------------------------------------
# [5] 随包文档 + VERSION
# ---------------------------------------------------------------------------
if (Test-Path -LiteralPath $DepsDoc) {
    Copy-Item -LiteralPath $DepsDoc -Destination (Join-Path $ReleaseDir 'docs\platform-requirements.md') -Force
} else {
    Write-Warning "缺少 $DepsDoc，未随包分发依赖文档（发布物不完整）"
}

$readme = @"
novel-reader $Version — Windows $ArchSegment（$Target）

本目录是**完整发布目录**（非单可执行文件）：可执行文件、启动包装、随包文档与校验清单在同一目录内。

启动
----
  1) 双击 start-novel-reader.cmd，或
  2) 命令行执行：start-novel-reader.cmd

* 数据目录：%LOCALAPPDATA%\novel-reader\library.db（可用 NR_DB_PATH 环境变量覆盖）；
* 随包 DLL 位于 dlls\，启动包装会自动加入 PATH；
* 应用图标：novel-reader.ico（随包分发，可用于快捷方式；exe 内嵌图标本轮未做）；
* 依赖与系统要求见 docs\platform-requirements.md。

完整性校验（PowerShell）
------------------------
  Get-Content MANIFEST.sha256 | ForEach-Object {
    `$hash, `$path = `$_ -split '\s+', 2
    if ((Get-FileHash -Algorithm SHA256 -Path `$path).Hash.ToLower() -ne `$hash) { Write-Error "校验失败：`$path" }
  }

验证状态
--------
* 本发布目录的打包脚本从未在 Windows 主机上执行过（开发机为 Linux）；
  windows-x86_64 目标在 desktop/packaging/targets.json 中为「待验证」，未实测前不得视为已发布。
"@
Set-Content -LiteralPath (Join-Path $ReleaseDir 'docs\README.txt') -Value $readme -Encoding utf8

$rustc = Get-Command rustc -ErrorAction SilentlyContinue
$rustcVersion = if ($null -ne $rustc) { (& rustc --version) } else { 'rustc-unavailable' }
$statusNote = 'pending-待验证（无 Windows 实测证据）'
$versionLines = @(
    'name=novel-reader',
    "version=$Version",
    "target=$Target",
    'session=win32',
    "release_directory=$ReleaseDirName",
    "built_at=$((Get-Date).ToString('o'))",
    "rustc=$rustcVersion",
    'packaging_script=desktop/packaging/windows/package-windows.ps1',
    'packaging_mode=release-directory',
    "static_crt=$([bool]$StaticCrt)",
    "verification_status=$statusNote"
)
Set-Content -LiteralPath (Join-Path $ReleaseDir 'VERSION') -Value $versionLines -Encoding utf8

# ---------------------------------------------------------------------------
# [6] 清单与自校验（Get-FileHash）
# ---------------------------------------------------------------------------
Write-Host '[6] 生成 MANIFEST.sha256 并自校验'
$manifestPath = Join-Path $ReleaseDir 'MANIFEST.sha256'
$files = Get-ChildItem -LiteralPath $ReleaseDir -Recurse -File |
    Where-Object { $_.Name -ne 'MANIFEST.sha256' } |
    Sort-Object FullName
$manifestLines = foreach ($file in $files) {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLower()
    $rel = $file.FullName.Substring($ReleaseDir.Length).TrimStart('\', '/').Replace('\', '/')
    "$hash  ./$rel"
}
Set-Content -LiteralPath $manifestPath -Value $manifestLines -Encoding ascii

$bad = 0
foreach ($line in Get-Content -LiteralPath $manifestPath) {
    $hash, $rel = $line -split '\s+', 2
    $abs = Join-Path $ReleaseDir ($rel -replace '^\./', '' -replace '/', '\')
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $abs).Hash.ToLower()
    if ($actual -ne $hash) {
        Write-Host ("    FAIL {0}" -f $rel)
        $bad++
    }
}
if ($bad -gt 0) {
    Write-Error "FAILURE: MANIFEST.sha256 自校验有 $bad 项不一致" -ErrorAction Continue
    exit 1
}

# ---------------------------------------------------------------------------
# [7] 布局自检
# ---------------------------------------------------------------------------
Write-Host '[7] 发布目录布局自检'
$missing = 0
foreach ($item in $RequiredFiles) {
    if (Test-Path -LiteralPath (Join-Path $ReleaseDir $item)) {
        Write-Host ("    OK  file {0}" -f $item)
    } else {
        Write-Host ("    FAIL file {0}" -f $item)
        $missing++
    }
}
foreach ($item in $RequiredDirs) {
    if (Test-Path -LiteralPath (Join-Path $ReleaseDir $item)) {
        Write-Host ("    OK  dir  {0}\" -f $item)
    } else {
        Write-Host ("    FAIL dir  {0}\" -f $item)
        $missing++
    }
}
if ($missing -gt 0) {
    Write-Error "FAILURE: 发布目录缺少 $missing 个必含项" -ErrorAction Continue
    exit 1
}

$fileCount = (Get-ChildItem -LiteralPath $ReleaseDir -Recurse -File).Count
$exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $ReleaseDir 'novel-reader.exe')).Hash.ToLower()
Write-Host "NR-PKG files=$fileCount binary_sha256=$exeHash"
Write-Host "NR-PKG manifest=$manifestPath"

# ---------------------------------------------------------------------------
# [8] 可选 zip
# ---------------------------------------------------------------------------
if ($Zip) {
    $zipPath = Join-Path $OutDir "$ReleaseDirName.zip"
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
    Compress-Archive -Path (Join-Path $ReleaseDir '*') -DestinationPath $zipPath -CompressionLevel Optimal
    Write-Host ("[8] 压缩包：{0}（{1} 字节）" -f $zipPath, (Get-Item -LiteralPath $zipPath).Length)
}

Write-Host "RESULT: OK — $ReleaseDir（$fileCount 个文件，binary_sha256=$exeHash）"
Write-Host 'NR-PKG 注意：本脚本未在任何 Windows 主机执行过，packaging/targets.json 中该目标为「待验证」'
