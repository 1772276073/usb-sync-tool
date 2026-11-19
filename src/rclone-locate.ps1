<#
.SYNOPSIS
    Locate the rclone executable automatically.
.DESCRIPTION
    Search order:
      0. Saved path from config file (%APPDATA%\USB-Sync-Tool\rclone-path.txt)
      1. rclone on PATH (Get-Command)
      2. WinGet package directory under %LOCALAPPDATA%
      3. Common install locations
      4. Last known WinGet install path (fallback)
    Returns the full path to rclone.exe, or $null if not found.
    Dot-source this file before calling Get-RclonePath / Save-RclonePath:
      . (Join-Path $PSScriptRoot "rclone-locate.ps1")
#>

$configRoot = $env:APPDATA
if (-not $configRoot) { $configRoot = $env:TEMP }
$script:RcloneConfigFile = Join-Path $configRoot "USB-Sync-Tool\rclone-path.txt"

# locate 文件自身所在目录（$PSCommandPath 在点源时指向被点源文件；$PSScriptRoot 在某些模式为空）
$script:RcloneLocalDir = Split-Path -Parent $PSCommandPath
if (-not $script:RcloneLocalDir) { $script:RcloneLocalDir = $PSScriptRoot }
if (-not $script:RcloneLocalDir) { $script:RcloneLocalDir = (Get-Location).Path }

function Get-RclonePath {
    # 0. 便携版：与脚本同目录的 rclone.exe（无需安装，换电脑拷贝即用）
    $localExe = Join-Path $script:RcloneLocalDir "rclone.exe"
    if ($localExe -and (Test-Path $localExe)) { return $localExe }

    # 1. Config file: path the user picked manually last time
    $saved = Get-SavedRclonePath
    if ($saved) { return $saved }

    # 1. rclone on PATH (covers winget shims, manual installs, etc.)
    $cmd = Get-Command rclone -ErrorAction SilentlyContinue
    if ($cmd -and $cmd.Source -and (Test-Path $cmd.Source)) { return $cmd.Source }

    # 2. WinGet package directory: ...\WinGet\Packages\Rclone.Rclone_*\rclone.exe
    $pkgRoot = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"
    $exe = Get-ChildItem -Path (Join-Path $pkgRoot "Rclone.Rclone_*") -Filter rclone.exe -Recurse -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($exe) { return $exe.FullName }

    # 3. Common install locations
    $candidates = @(
        (Join-Path $env:ProgramFiles "rclone\rclone.exe"),
        (Join-Path ${env:ProgramFiles(x86)} "rclone\rclone.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\rclone\rclone.exe"),
        "C:\rclone\rclone.exe",
        (Join-Path $env:USERPROFILE "rclone\rclone.exe")
    )
    foreach ($c in $candidates) {
        if ($c -and (Test-Path $c)) { return $c }
    }

    # 4. Fallback: last known WinGet install path
    $legacy = "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Rclone.Rclone_Microsoft.Winget.Source_8wekyb3d8bbwe\rclone-v1.75.0-windows-amd64\rclone.exe"
    if (Test-Path $legacy) { return $legacy }

    return $null
}

function Get-SavedRclonePath {
    if (Test-Path $script:RcloneConfigFile) {
        $p = (Get-Content $script:RcloneConfigFile -Raw -ErrorAction SilentlyContinue).Trim()
        if ($p -and (Test-Path $p)) { return $p }
    }
    return $null
}

function Save-RclonePath {
    param([string]$Path)
    $dir = Split-Path $script:RcloneConfigFile -Parent
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    Set-Content -Path $script:RcloneConfigFile -Value $Path -Encoding UTF8
}
