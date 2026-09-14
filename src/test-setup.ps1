# 测试 rclone 安装
. (Join-Path $PSScriptRoot "rclone-locate.ps1")

Write-Host "Testing rclone installation..." -ForegroundColor Cyan

$rclone = Get-RclonePath
if (-not $rclone) {
    Write-Error "rclone not found. Install with: winget install Rclone.Rclone"
    exit 1
}
Write-Host "✓ rclone found at: $rclone" -ForegroundColor Green
& $rclone version

Write-Host "`nTesting G: drive..." -ForegroundColor Cyan
if (Test-Path "G:\") {
    Write-Host "✓ G: drive is accessible" -ForegroundColor Green
    Get-PSDrive G | Select-Object Name, Used, Free, @{N='Total';E={$_.Used+$_.Free}}
} else {
    Write-Error "G: drive not found"
    exit 1
}

Write-Host "`nSetup complete! You can now use:" -ForegroundColor Green
Write-Host "  .\sync-usb.ps1              # Bidirectional sync"
Write-Host "  .\sync-usb.ps1 -Action push # Push to USB"
Write-Host "  .\sync-usb.ps1 -Action pull # Pull from USB"
