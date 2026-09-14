#Requires -Version 5.1
<#
.SYNOPSIS
    USB drive sync tool (powered by rclone)
.DESCRIPTION
    Bidirectional sync between local folder and USB drive (G:), with automatic version backup.
    - Incremental transfer (only changed parts)
    - Version history via --backup-dir
    - Conflict detection and resolution
.PARAMETER Action
    sync    - Bidirectional sync (default)
    push    - Local -> USB only
    pull    - USB -> Local only
    status  - Show sync status without changes
.PARAMETER LocalPath
    Local folder to sync (default: current script directory\synced)
.PARAMETER UsbPath
    USB drive folder (default: G:\synced)
.PARAMETER BackupDir
    Version backup directory (default: G:\.sync-backup)
.EXAMPLE
    .\sync-usb.ps1                    # Bidirectional sync
    .\sync-usb.ps1 -Action push       # Push local to USB
    .\sync-usb.ps1 -Action pull       # Pull USB to local
    .\sync-usb.ps1 -Action status     # Dry run, show what would change
#>

param(
    [ValidateSet("sync", "push", "pull", "status")]
    [string]$Action = "sync",

    [string]$LocalPath = (Join-Path $PSScriptRoot "synced"),

    [string]$UsbPath = "G:\synced",

    [string]$BackupDir = "G:\.sync-backup"
)

$ErrorActionPreference = "Stop"

# Locate rclone (config file -> PATH -> WinGet -> common locations)
. (Join-Path $PSScriptRoot "rclone-locate.ps1")
$rclone = Get-RclonePath
if (-not $rclone) {
    Write-Error "rclone not found. Install via: winget install rclone, or run the GUI once and pick rclone.exe manually (the choice is remembered)."
    exit 1
}

# Ensure directories exist
if (-not (Test-Path $LocalPath)) {
    New-Item -ItemType Directory -Path $LocalPath -Force | Out-Null
    Write-Host "Created local folder: $LocalPath"
}

if (-not (Test-Path $UsbPath)) {
    Write-Error "USB path not found: $UsbPath (Is G: drive inserted?)"
    exit 1
}

if (-not (Test-Path $BackupDir)) {
    New-Item -ItemType Directory -Path $BackupDir -Force | Out-Null
}

# Common rclone flags
$commonFlags = @(
    "--backup-dir", $BackupDir,
    "--suffix", ".{time:2006-01-02_150405}",
    "--check-access",
    "--filters-file", (Join-Path $PSScriptRoot "sync-filters.txt")
)

# Build command based on action
switch ($Action) {
    "sync" {
        Write-Host "Bidirectional sync: $LocalPath <-> $UsbPath" -ForegroundColor Cyan
        $cmd = @("bisync", $LocalPath, $UsbPath) + $commonFlags
    }
    "push" {
        Write-Host "Push: $LocalPath -> $UsbPath" -ForegroundColor Cyan
        $cmd = @("copy", $LocalPath, $UsbPath) + $commonFlags
    }
    "pull" {
        Write-Host "Pull: $UsbPath -> $LocalPath" -ForegroundColor Cyan
        $cmd = @("copy", $UsbPath, $LocalPath) + $commonFlags
    }
    "status" {
        Write-Host "Dry run (no changes):" -ForegroundColor Yellow
        $cmd = @("bisync", $LocalPath, $UsbPath, "--dry-run") + $commonFlags
    }
}

# Execute
Write-Host "Backup dir: $BackupDir" -ForegroundColor DarkGray
Write-Host ""

try {
    & $rclone @cmd
    $exitCode = $LASTEXITCODE

    if ($exitCode -eq 0) {
        Write-Host "`nSync completed successfully." -ForegroundColor Green
    } elseif ($exitCode -eq 1) {
        Write-Host "`nFirst-time sync or resync needed. Run with -Action sync again." -ForegroundColor Yellow
        exit 1
    } else {
        Write-Host "`nSync failed with exit code $exitCode" -ForegroundColor Red
        exit $exitCode
    }
} catch {
    Write-Error "Sync failed: $_"
    exit 1
}
