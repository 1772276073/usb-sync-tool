# =====================================================================
# USB-Sync v2 场景验证脚本（对 CLI 全链路验证；GUI 手测）
# 用法：.\test-scenario.ps1 [-Exe <路径>]   默认用 debug 构建产物
# =====================================================================
param(
    [string]$Exe = "E:\rust-target\usb-sync\debug\usb-sync-cli.exe"
)
$ErrorActionPreference = "Stop"
# 让 PowerShell 按 UTF-8 解码子进程（exe）输出，避免中文比对失败
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
if (-not (Test-Path $Exe)) { Write-Host "找不到 $Exe，请先构建" -ForegroundColor Red; exit 1 }

$root = Join-Path $env:TEMP ("usbsync-scenario-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
$local = Join-Path $root "local"
$usb   = Join-Path $root "usb"
$backupDefault = Join-Path $usb ".sync-backup"
New-Item -ItemType Directory -Force -Path $local, $usb | Out-Null
$script:pass = 0; $script:fail = 0

function Check($name, $cond) {
    if ($cond) { $script:pass++;  Write-Host "    [通过] $name" -ForegroundColor Green }
    else       { $script:fail++;  Write-Host "    [失败] $name" -ForegroundColor Red }
}
function RunSync { & $Exe sync --local $local --usb $usb 2>&1 | Out-Host; return $LASTEXITCODE }
function SetMtime($path, [datetime]$t) { (Get-Item $path).LastWriteTime = $t }

Write-Host "`n=== 场景1：首次双向同步（合并：缺的补、新者胜） ===" -ForegroundColor Cyan
# 本地独有
New-Item -ItemType Directory -Force -Path (Join-Path $local "文档") | Out-Null
Set-Content -Encoding UTF8 (Join-Path $local "文档\a.txt") "local-only"
# U盘独有
New-Item -ItemType Directory -Force -Path (Join-Path $usb "文档") | Out-Null
Set-Content -Encoding UTF8 (Join-Path $usb "文档\b.txt") "usb-only"
# 两端都有、本地较新（相差5小时，远超2s窗口）
Set-Content -Encoding UTF8 (Join-Path $local "both.txt") "local-v2"
Set-Content -Encoding UTF8 (Join-Path $usb "both.txt") "usb-v1"
SetMtime (Join-Path $local "both.txt") (Get-Date).AddHours(-1)
SetMtime (Join-Path $usb  "both.txt") (Get-Date).AddHours(-6)
# 两端一致
Set-Content -Encoding UTF8 (Join-Path $local "same.txt") "same"
Copy-Item (Join-Path $local "same.txt") (Join-Path $usb "same.txt")

$rc = RunSync
Check "退出码 0" ($rc -eq 0)
Check "a.txt 已复制到U盘"   (Test-Path (Join-Path $usb "文档\a.txt"))
Check "b.txt 已复制到本地"  (Test-Path (Join-Path $local "文档\b.txt"))
Check "both.txt 本地较新胜出（U盘侧已是 local-v2）" ((Get-Content (Join-Path $usb "both.txt") -Raw).Trim() -eq "local-v2")
Check "both.txt 败者已备份" ((Get-ChildItem $backupDefault -Recurse -File | Where-Object Name -like "both.txt*").Count -ge 1)
Check "状态快照已写入"      (Test-Path (Join-Path $usb ".usb-sync\state.json"))

Write-Host "`n=== 场景2：二次同步应为空操作 ==="
$out2 = & $Exe sync --local $local --usb $usb 2>&1
Check "报告两端一致"  (($out2 -join " ") -match "无需变更")
$rc2 = $LASTEXITCODE
Check "退出码 0" ($rc2 -eq 0)

Write-Host "`n=== 场景3：单端修改 → 覆盖传播 + 旧版备份 ==="
Set-Content -Encoding UTF8 (Join-Path $local "文档\a.txt") "local-v2"
$rc3 = RunSync
Check "退出码 0" ($rc3 -eq 0)
Check "U盘 a.txt 已更新"    ((Get-Content (Join-Path $usb "文档\a.txt") -Raw).Trim() -eq "local-v2")
Check "旧版 a.txt 已备份"   ((Get-ChildItem $backupDefault -Recurse -File | Where-Object Name -like "a.txt*").Count -ge 1)
Check "备份保留相对路径结构" ((Get-ChildItem $backupDefault -Recurse -File -Filter "a.txt*" | Where-Object { $_.FullName -match "文档" }).Count -ge 1)

Write-Host "`n=== 场景4：单端删除 → 传播删除 + 备份 ==="
Remove-Item (Join-Path $local "文档\b.txt")
$rc4 = RunSync
Check "退出码 0" ($rc4 -eq 0)
Check "U盘 b.txt 已删除"    (-not (Test-Path (Join-Path $usb "文档\b.txt")))
Check "被删 b.txt 已备份"   ((Get-ChildItem $backupDefault -Recurse -File | Where-Object Name -like "b.txt*").Count -ge 1)

Write-Host "`n=== 场景5：两端同时修改（冲突）→ 新者胜 ==="
Set-Content -Encoding UTF8 (Join-Path $local "same.txt") "conflict-local"
Set-Content -Encoding UTF8 (Join-Path $usb  "same.txt") "conflict-usb"
SetMtime (Join-Path $local "same.txt") (Get-Date).AddMinutes(-5)
SetMtime (Join-Path $usb  "same.txt") (Get-Date).AddMinutes(-1)
$rc5 = RunSync
Check "退出码 0" ($rc5 -eq 0)
Check "两端均为 U盘（较新）版本" (
    ((Get-Content (Join-Path $local "same.txt") -Raw).Trim() -eq "conflict-usb") -and
    ((Get-Content (Join-Path $usb  "same.txt") -Raw).Trim() -eq "conflict-usb"))
Check "败者（本地旧版）已备份" ((Get-ChildItem $backupDefault -Recurse -File | Where-Object Name -like "same.txt*").Count -ge 1)

Write-Host "`n=== 场景6：排除规则生效（默认 + 自定义规则文件） ==="
Set-Content (Join-Path $local "junk.tmp") "junk"
Set-Content (Join-Path $local ".DS_Store") "mac"
Set-Content (Join-Path $local "custom-junk.log") "custom"
New-Item -ItemType Directory -Force -Path (Join-Path $local "node_modules\pkg") | Out-Null
Set-Content (Join-Path $local "node_modules\pkg\c.js") "nm"
# 自定义规则文件：额外排除 node_modules 与 *.log（默认规则不含）
$filtersFile = Join-Path $root "my-filters.txt"
Set-Content $filtersFile "node_modules`n*.log"
$rc6 = & $Exe preview --local $local --usb $usb --filters-file $filtersFile 2>&1
$all6 = $rc6 -join " "
Check "preview 不含 junk.tmp（默认规则）"      (-not ($all6 -match "junk\.tmp"))
Check "preview 不含 .DS_Store（默认规则）"     (-not ($all6 -match "DS_Store"))
Check "preview 不含 node_modules（自定义）"    (-not ($all6 -match "node_modules"))
Check "preview 不含 custom-junk.log（自定义）" (-not ($all6 -match "custom-junk\.log"))
Check "退出码 0" ($LASTEXITCODE -eq 0)

Write-Host "`n=== 场景7：推送/拉取（单向，多余文件保留） ==="
Set-Content -Encoding UTF8 (Join-Path $usb "extra-usb.txt") "extra"
Set-Content -Encoding UTF8 (Join-Path $local "push-new.txt") "new"
$rc7 = & $Exe push --local $local --usb $usb 2>&1; $rc7c = $LASTEXITCODE
Check "push 退出码 0" ($rc7c -eq 0)
Check "push：新文件已到U盘"    (Test-Path (Join-Path $usb "push-new.txt"))
Check "push：U盘多余文件保留"  (Test-Path (Join-Path $usb "extra-usb.txt"))
Remove-Item (Join-Path $local "push-new.txt")
$rc8 = & $Exe pull --local $local --usb $usb 2>&1; $rc8c = $LASTEXITCODE
Check "pull 退出码 0" ($rc8c -eq 0)
Check "pull：文件已回到本地"   (Test-Path (Join-Path $local "push-new.txt"))

Write-Host "`n=== 场景8：预览不落盘 ==="
Set-Content -Encoding UTF8 (Join-Path $local "preview-only.txt") "never"
$out9 = & $Exe preview --local $local --usb $usb 2>&1
Check "preview 列出新文件"  (($out9 -join " ") -match "preview-only\.txt")
Check "preview 未创建文件"  (-not (Test-Path (Join-Path $usb "preview-only.txt")))

Write-Host ""
if ($script:fail -eq 0) {
    Write-Host "全部通过：$($script:pass) 项 ✅" -ForegroundColor Green
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    exit 0
} else {
    Write-Host "通过 $($script:pass) / 失败 $($script:fail) ❌（测试目录：$root）" -ForegroundColor Red
    exit 1
}
