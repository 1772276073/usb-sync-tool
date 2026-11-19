$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$Exe = 'E:\rust-target\usb-sync\release\usb-sync-cli.exe'
$root = Join-Path $env:TEMP ('usb-ret-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$bk = Join-Path $root 'bk'
$local = Join-Path $root 'local'
$usb = Join-Path $root 'usb'
New-Item -ItemType Directory -Force -Path $bk, $local, $usb | Out-Null
# 预置两个旧备份目录（模拟历史运行）
New-Item -ItemType Directory -Force -Path (Join-Path $bk '20260101_010101') | Out-Null
Set-Content (Join-Path $bk '20260101_010101\old.bin') ('x' * 800)
New-Item -ItemType Directory -Force -Path (Join-Path $bk '20260102_010101') | Out-Null
Set-Content (Join-Path $bk '20260102_010101\old2.bin') ('y' * 600)
# 本地文件（较新），U盘侧放旧版并置只读 → 触发覆盖+备份
Set-Content (Join-Path $local 'doc.txt') 'v2'
Set-Content (Join-Path $usb 'doc.txt') 'v1'
Set-ItemProperty (Join-Path $usb 'doc.txt') -Name IsReadOnly -Value $true

Write-Host '--- push（上限 0.000001 GB ≈ 1KB，必然触发清理）---'
& $Exe push --local $local --usb $usb --backup-dir $bk --max-backup-gb 0.000001 2>&1
Write-Host ("exit=" + $LASTEXITCODE)
$kept = (Get-ChildItem $bk -Directory | Select-Object -ExpandProperty Name) -join ','
Write-Host ("备份目录: " + $kept)
Write-Host ("20260101 已清理: " + (-not (Test-Path (Join-Path $bk '20260101_010101'))))
Write-Host ("20260102 保留: " + (Test-Path (Join-Path $bk '20260102_010101')))
Write-Host '--- 0 = 不限制 ---'
Set-Content (Join-Path $local 'doc2.txt') 'more'
& $Exe push --local $local --usb $usb --backup-dir $bk --max-backup-gb 0 2>&1 | Out-Null
Write-Host ("exit=" + $LASTEXITCODE + "（0 上限不清理即为通过）")
