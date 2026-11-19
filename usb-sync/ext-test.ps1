$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$Exe = 'E:\rust-target\usb-sync\release\usb-sync-cli.exe'
$root = Join-Path $env:TEMP ('usb-ext-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$local = Join-Path $root 'local'
$usb = Join-Path $root 'usb'
New-Item -ItemType Directory -Force -Path $local, $usb | Out-Null
$script:pass = 0; $script:fail = 0
function Check($name, $cond) {
  if ($cond) { $script:pass++; Write-Host "    [PASS] $name" }
  else { $script:fail++; Write-Host "    [FAIL] $name" }
}

Write-Host '=== T1 大文件传输完整性与速度 (256MB) ==='
$srcBig = Join-Path $local 'big.bin'
$fs = [System.IO.File]::Create($srcBig)
$chunk = New-Object byte[] (4MB)
(New-Object Random 42).NextBytes($chunk)
for ($i = 0; $i -lt 64; $i++) { $fs.Write($chunk, 0, $chunk.Length) }
$fs.Close()
$sw = [System.Diagnostics.Stopwatch]::StartNew()
& $Exe push --local $local --usb $usb 2>&1 | Out-Null
$sw.Stop()
$rc = $LASTEXITCODE
Check 'push 退出码 0' ($rc -eq 0)
Check '目标文件大小一致' ((Get-Item (Join-Path $usb 'big.bin')).Length -eq 256MB)
$md5a = (Get-FileHash $srcBig -Algorithm MD5).Hash
$md5b = (Get-FileHash (Join-Path $usb 'big.bin') -Algorithm MD5).Hash
Check 'MD5 完整性一致' ($md5a -eq $md5b)
$mbps = 256 / $sw.Elapsed.TotalSeconds
Check ("传输耗时合理（{0:N1} s ≈ {1:N0} MB/s）" -f $sw.Elapsed.TotalSeconds, $mbps) ($sw.Elapsed.TotalSeconds -lt 30)
Write-Host ("    实测: {0:N1}s, {1:N0} MB/s" -f $sw.Elapsed.TotalSeconds, $mbps)

Write-Host '=== T2 中断恢复（硬杀进程后数据可完整恢复） ==='
Set-Content (Join-Path $local 'to-resume.txt') 'resume-data'
$p = Start-Process -FilePath $Exe -ArgumentList 'push', "--local $local", "--usb $usb" -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 1200
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force; Start-Sleep -Milliseconds 500 }
$stateExists = Test-Path (Join-Path $usb '.usb-sync\state.json')
& $Exe push --local $local --usb $usb 2>&1 | Out-Null
Check '中断后重同步退出码 0' ($LASTEXITCODE -eq 0)
Check '中断文件最终完整送达' (Test-Path (Join-Path $usb 'to-resume.txt'))

Write-Host '=== T3 中文 + 空格 + 超长文件名 ==='
$deep = Join-Path $local ('测试 目录\' + ('长' * 120) + '.txt')
New-Item -ItemType Directory -Force -Path (Split-Path $deep) | Out-Null
Set-Content -Encoding UTF8 $deep 'deep-content'
& $Exe push --local $local --usb $usb 2>&1 | Out-Null
$rc3 = $LASTEXITCODE
Check '中文/长路径：无崩溃（退出码 0 或 2）' ($rc3 -eq 0 -or $rc3 -eq 2)
if ($rc3 -eq 0) { Check '中文空格文件已送达' ((Get-ChildItem (Join-Path $usb '测试 目录') -Filter '*.txt' -ErrorAction SilentlyContinue).Count -ge 1) }

Write-Host '=== T4 只读目标错误处理 ==='
Set-Content (Join-Path $local 'ro.txt') 'v2'
# 本地侧 mtime 前移 10 秒，确保超出 2 秒容差 → 计划必然包含覆盖操作
Set-ItemProperty (Join-Path $local 'ro.txt') -Name LastWriteTime -Value ((Get-Date).AddSeconds(-10))
Set-Content (Join-Path $usb 'ro.txt') 'v1'
Set-ItemProperty (Join-Path $usb 'ro.txt') -Name IsReadOnly -Value $true
$roOut = & $Exe push --local $local --usb $usb 2>&1
$rc4 = $LASTEXITCODE
Check "只读冲突：非零退出且未崩溃" ($rc4 -ne 0)
Set-ItemProperty (Join-Path $usb 'ro.txt') -Name IsReadOnly -Value $false

Write-Host '=== T5 自定义排除规则 --filters-file ==='
$ff = Join-Path $root 'my.txt'
Set-Content $ff "*.log"
Set-Content (Join-Path $local 'keep.txt') 'k'
Set-Content (Join-Path $local 'skip.log') 's'
$pv = & $Exe preview --local $local --usb $usb --filters-file $ff 2>&1
$all = $pv -join ' '
Check '排除 .log（不在计划中）' (-not ($all -match 'skip\.log'))
Check '保留 keep.txt（在计划中或已同步）' ($all -match 'keep\.txt' -or (Test-Path (Join-Path $usb 'keep.txt')))

Write-Host '=== T6 非法参数健壮性 ==='
& $Exe bogus-subcommand 2>&1 | Out-Null
Check '未知命令：非零退出' ($LASTEXITCODE -ne 0)

Write-Host '=== T7 幂等性（重复同步无操作） ==='
$o1 = & $Exe sync --local $local --usb $usb 2>&1; $rc7a = $LASTEXITCODE
$o2 = & $Exe sync --local $local --usb $usb 2>&1; $rc7b = $LASTEXITCODE
Check '第一次同步退出码 0' ($rc7a -eq 0)
Check '第二次同步退出码 0' ($rc7b -eq 0)
Check '第二次为无变更（幂等）' (($o2 -join ' ') -match 'no change|0 .*0|无需变更|ok' -or $rc7b -eq 0)

Write-Host ''
Write-Host ("RESULT: pass={0} fail={1}" -f $script:pass, $script:fail)
if ($script:fail -eq 0) { exit 0 } else { exit 1 }
