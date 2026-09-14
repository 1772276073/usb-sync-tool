#Requires -Version 5.1

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

# Locate rclone (config file -> PATH -> WinGet -> common locations)
. (Join-Path $PSScriptRoot "rclone-locate.ps1")
$script:cancelled = $false
$script:rclonePath = Get-RclonePath
if (-not $script:rclonePath) {
    # 自动查找失败：让用户手动选择 rclone.exe，选择结果会记住，下次直接使用
    $dlg = New-Object System.Windows.Forms.OpenFileDialog
    $dlg.Title = "未找到 rclone，请选择 rclone.exe"
    $dlg.Filter = "rclone 可执行文件|rclone.exe|所有文件|*.*"
    $dlg.InitialDirectory = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"
    if ($dlg.ShowDialog() -eq "OK") {
        $script:rclonePath = $dlg.FileName
        Save-RclonePath $script:rclonePath
    } else {
        [System.Windows.Forms.MessageBox]::Show("未选择 rclone，工具无法使用。`n可先运行 winget install Rclone.Rclone 安装。", "错误", "OK", "Error")
        exit 1
    }
}

# Ant Design (dark) 色板
$bg = [System.Drawing.Color]::FromArgb(20, 20, 20)          # antd #141414 页面背景
$panel = [System.Drawing.Color]::FromArgb(31, 31, 31)       # antd #1F1F1F 组件背景
$input = [System.Drawing.Color]::FromArgb(31, 31, 31)       # antd 输入框背景
$border = [System.Drawing.Color]::FromArgb(66, 66, 66)      # antd #424242 边框
$textColor = [System.Drawing.Color]::FromArgb(217, 217, 217)  # antd rgba(255,255,255,0.85) 正文
$textDim = [System.Drawing.Color]::FromArgb(115, 115, 115)  # antd rgba(255,255,255,0.45) 弱化
$titleColor = [System.Drawing.Color]::FromArgb(255, 255, 255)  # 标题纯白
$labelColor = [System.Drawing.Color]::FromArgb(166, 166, 166)  # 区域标签 #A6A6A6
$infoColor = [System.Drawing.Color]::FromArgb(145, 202, 255)  # 输出/信息 #91CAFF
$accent = [System.Drawing.Color]::FromArgb(22, 119, 255)    # antd 主色 #1677FF
$accentHover = [System.Drawing.Color]::FromArgb(64, 150, 255) # antd hover #4096FF
$success = [System.Drawing.Color]::FromArgb(82, 196, 26)    # antd 成功 #52C41A
$warning = [System.Drawing.Color]::FromArgb(250, 173, 20)   # antd 警告 #FAAD14
$errColor = [System.Drawing.Color]::FromArgb(255, 77, 79)   # antd 错误 #FF4D4F
$baseFont = New-Object System.Drawing.Font("Microsoft YaHei UI", 9)
$hoverPanel = [System.Drawing.Color]::FromArgb(43, 43, 43)

# ---- 圆角组件（Ant Design 风格）----
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class RoundedRect {
    [DllImport("gdi32.dll")]
    public static extern IntPtr CreateRoundRectRgn(int left, int top, int right, int bottom, int w, int h);
    [DllImport("user32.dll")]
    public static extern int SetWindowRgn(IntPtr hWnd, IntPtr hRgn, bool redraw);
}
"@
function Set-RoundedCorners {
    param($Control, [int]$Radius = 6)
    if (-not $Control.IsHandleCreated) { $null = $Control.Handle }
    $hRgn = [RoundedRect]::CreateRoundRectRgn(0, 0, $Control.Width + 1, $Control.Height + 1, $Radius, $Radius)
    [void][RoundedRect]::SetWindowRgn($Control.Handle, $hRgn, $true)
}

# ---- 控件工厂：统一深色样式 ----
function New-DarkLabel {
    param([string]$Text, [string]$Align = "MiddleLeft", [System.Drawing.Color]$Color = $labelColor)
    $l = New-Object System.Windows.Forms.Label
    $l.Text = $Text
    $l.Dock = "Fill"
    $l.TextAlign = $Align
    $l.ForeColor = $Color
    $l.Font = $baseFont
    return $l
}

function New-DarkButton {
    param([string]$Text, [System.Drawing.Color]$Back = $panel, [bool]$Bold = $false)
    $b = New-Object System.Windows.Forms.Button
    $b.Text = $Text
    $b.BackColor = $Back
    # Ant Design 语义：主按钮白字、次按钮文字色
    $b.ForeColor = if ($Back -eq $accent) { [System.Drawing.Color]::White } else { $textColor }
    $b.FlatStyle = "Flat"
    $b.FlatAppearance.BorderColor = $border
    $b.FlatAppearance.BorderSize = 1
    $b.Tag = if ($Back -eq $accent) { "primary" } else { "secondary" }
    if ($Bold) { $b.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 10, [System.Drawing.FontStyle]::Bold) }
    else { $b.Font = $baseFont }
    # 悬停反馈（Ant Design 交互：主按钮变亮、次按钮变浅）
    $b.Add_MouseEnter({
        if ($this.Tag -eq "primary") { $this.BackColor = $accentHover }
        else { $this.BackColor = $hoverPanel }
    })
    $b.Add_MouseLeave({
        if ($this.Tag -eq "primary") { $this.BackColor = $accent }
        else { $this.BackColor = $panel }
    })
    return $b
}

# Form
$form = New-Object System.Windows.Forms.Form
$form.Text = "USB 同步工具"
$form.Size = New-Object System.Drawing.Size(680, 560)
$form.StartPosition = "CenterScreen"
$form.FormBorderStyle = "FixedDialog"
$form.MaximizeBox = $false
$form.BackColor = $bg
$form.Font = $baseFont

# 主布局：3 列（标签 80 | 内容自适应 | 按钮 78），9 行，自动对齐
$layout = New-Object System.Windows.Forms.TableLayoutPanel
$layout.Dock = "Fill"
$layout.Padding = New-Object System.Windows.Forms.Padding(16, 14, 16, 14)
$layout.ColumnCount = 3
$layout.RowCount = 10
$layout.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle("Absolute", 80)))
$layout.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle("Percent", 100)))
$layout.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle("Absolute", 78)))
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 0 标题
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 1 本地路径
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 2 U盘路径
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 3 同步模式
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 4 方案
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 38)))   # 5 进度
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 30)))   # 6 状态
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 26)))   # 7 日志标签
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Percent", 100)))   # 8 日志框
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 48)))   # 9 按钮行
$form.Controls.Add($layout)

# 标题（跨 3 列）
$title = New-Object System.Windows.Forms.Label
$title.Text = "USB 同步"
$title.Dock = "Fill"
$title.TextAlign = "MiddleLeft"
$title.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 16, [System.Drawing.FontStyle]::Bold)
$title.ForeColor = $titleColor
$layout.Controls.Add($title, 0, 0)
$layout.SetColumnSpan($title, 3)

# 本地路径行
$lblLocal = New-DarkLabel "本地路径"
$layout.Controls.Add($lblLocal, 0, 1)
$txtLocal = New-Object System.Windows.Forms.TextBox
$txtLocal.Dock = "Fill"
$txtLocal.Text = (Join-Path $PSScriptRoot "synced")
$txtLocal.BackColor = $input
$txtLocal.ForeColor = $textColor
$txtLocal.BorderStyle = "FixedSingle"
$layout.Controls.Add($txtLocal, 1, 1)
$btnLocal = New-DarkButton "浏览"
$btnLocal.Dock = "Fill"
$layout.Controls.Add($btnLocal, 2, 1)

# U盘路径行
$lblUsb = New-DarkLabel "U盘路径"
$layout.Controls.Add($lblUsb, 0, 2)
$txtUsb = New-Object System.Windows.Forms.TextBox
$txtUsb.Dock = "Fill"
$txtUsb.Text = "G:\synced"
$txtUsb.BackColor = $input
$txtUsb.ForeColor = $textColor
$txtUsb.BorderStyle = "FixedSingle"
$layout.Controls.Add($txtUsb, 1, 2)
$btnUsb = New-DarkButton "浏览"
$btnUsb.Dock = "Fill"
$layout.Controls.Add($btnUsb, 2, 2)

# 同步模式行
$lblMode = New-DarkLabel "同步模式"
$layout.Controls.Add($lblMode, 0, 3)
$cmbMode = New-Object System.Windows.Forms.ComboBox
$cmbMode.Dock = "Fill"
$cmbMode.DropDownStyle = "DropDownList"
$cmbMode.BackColor = $input
$cmbMode.ForeColor = $textColor
$cmbMode.FlatStyle = "Flat"
$cmbMode.Items.AddRange(@("双向同步", "推送到U盘", "从U盘拉取", "预览变更"))
$cmbMode.SelectedIndex = 0
$layout.Controls.Add($cmbMode, 1, 3)

# 方案行（预设常用路径 + 同步模式）
$lblProfile = New-DarkLabel "方案"
$layout.Controls.Add($lblProfile, 0, 4)
$profileRow = New-Object System.Windows.Forms.FlowLayoutPanel
$profileRow.Dock = "Fill"
$profileRow.FlowDirection = "LeftToRight"
$profileRow.WrapContents = $false
$profileRow.BackColor = $bg
$cmbProfile = New-Object System.Windows.Forms.ComboBox
$cmbProfile.DropDownStyle = "DropDownList"
$cmbProfile.BackColor = $input
$cmbProfile.ForeColor = $textColor
$cmbProfile.FlatStyle = "Flat"
$cmbProfile.Size = New-Object System.Drawing.Size(280, 28)
$profileRow.Controls.Add($cmbProfile)
$btnSaveProfile = New-DarkButton "保存方案"
$btnSaveProfile.Size = New-Object System.Drawing.Size(85, 30)
$profileRow.Controls.Add($btnSaveProfile)
$btnDelProfile = New-DarkButton "删除"
$btnDelProfile.Size = New-Object System.Drawing.Size(60, 30)
$profileRow.Controls.Add($btnDelProfile)
$layout.Controls.Add($profileRow, 1, 4)

# 进度行（进度条跨 2 列）
$lblProgress = New-DarkLabel "进度"
$layout.Controls.Add($lblProgress, 0, 5)
$bar = New-Object System.Windows.Forms.ProgressBar
$bar.Dock = "Fill"
$bar.Style = "Continuous"
$bar.Minimum = 0
$bar.Maximum = 100
$bar.Value = 0
$bar.ForeColor = $accent
$bar.BackColor = $panel
# 进度条动画：Marquee 在本机不可靠，用定时器驱动的值循环模拟"加载中"
$animTimer = New-Object System.Windows.Forms.Timer
$animTimer.Interval = 60
$animTimer.Add_Tick({
    $bar.Value = ($bar.Value + 3) % 100
})
$layout.Controls.Add($bar, 1, 5)
$layout.SetColumnSpan($bar, 2)

# 状态行（跨 3 列）
$lblStatus = New-DarkLabel "就绪" "MiddleLeft" $success
$lblStatus.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 9, [System.Drawing.FontStyle]::Bold)
$layout.Controls.Add($lblStatus, 0, 6)
$layout.SetColumnSpan($lblStatus, 3)

# 日志标签（跨 3 列）
$lblLog = New-DarkLabel "日志"
$layout.Controls.Add($lblLog, 0, 7)
$layout.SetColumnSpan($lblLog, 3)

# 日志框（跨 3 列，占满剩余高度）——用 RichTextBox 支持分色日志
$txtLog = New-Object System.Windows.Forms.RichTextBox
$txtLog.Dock = "Fill"
$txtLog.Multiline = $true
$txtLog.ScrollBars = "Vertical"
$txtLog.ReadOnly = $true
$txtLog.BackColor = $input
$txtLog.ForeColor = $textColor
$txtLog.Font = New-Object System.Drawing.Font("Consolas", 9)
$txtLog.BorderStyle = "FixedSingle"
$layout.Controls.Add($txtLog, 0, 8)
$layout.SetColumnSpan($txtLog, 3)

# 按钮行：左（开始+取消）| 中（空）| 右（关闭）
$leftRow = New-Object System.Windows.Forms.FlowLayoutPanel
$leftRow.Dock = "Fill"
$leftRow.FlowDirection = "LeftToRight"
$leftRow.WrapContents = $false
$leftRow.BackColor = $bg
$btnSync = New-DarkButton "开始同步" $accent $true
$btnSync.FlatAppearance.BorderSize = 0
$btnSync.Size = New-Object System.Drawing.Size(110, 36)
$leftRow.Controls.Add($btnSync)
$btnCancel = New-DarkButton "取消"
$btnCancel.Size = New-Object System.Drawing.Size(90, 36)
$btnCancel.Enabled = $false
$leftRow.Controls.Add($btnCancel)
$layout.Controls.Add($leftRow, 0, 9)
$layout.SetColumnSpan($leftRow, 2)

$btnClose = New-DarkButton "关闭"
$btnClose.Dock = "Fill"
$layout.Controls.Add($btnClose, 2, 9)

# Browse handlers
$btnLocal.Add_Click({
    $fb = New-Object System.Windows.Forms.FolderBrowserDialog
    $fb.Description = "选择本地同步文件夹"
    $fb.SelectedPath = $txtLocal.Text
    if ($fb.ShowDialog() -eq "OK") { $txtLocal.Text = $fb.SelectedPath }
})

$btnUsb.Add_Click({
    $fb = New-Object System.Windows.Forms.FolderBrowserDialog
    $fb.Description = "选择U盘同步文件夹"
    $fb.SelectedPath = $txtUsb.Text
    if ($fb.ShowDialog() -eq "OK") { $txtUsb.Text = $fb.SelectedPath }
})

# ---- 方案预设：保存常用路径 + 同步模式 ----
Add-Type -AssemblyName Microsoft.VisualBasic | Out-Null
$script:ProfilesFile = Join-Path $env:APPDATA "USB-Sync-Tool\profiles.json"
$script:profiles = @()

function Load-Profiles {
    if (Test-Path $script:ProfilesFile) {
        try { $script:profiles = @(Get-Content $script:ProfilesFile -Raw | ConvertFrom-Json) }
        catch { $script:profiles = @() }
    }
}

function Save-Profiles {
    $dir = Split-Path $script:ProfilesFile -Parent
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $script:profiles | ConvertTo-Json -Depth 3 | Set-Content $script:ProfilesFile -Encoding UTF8
}

function Refresh-ProfileList {
    $cmbProfile.Items.Clear()
    foreach ($p in $script:profiles) { [void]$cmbProfile.Items.Add($p.name) }
}

Load-Profiles
Refresh-ProfileList

# 选择方案 -> 自动填入路径和模式
$cmbProfile.Add_SelectedIndexChanged({
    if ($cmbProfile.SelectedIndex -ge 0) {
        $p = $script:profiles[$cmbProfile.SelectedIndex]
        $txtLocal.Text = $p.local
        $txtUsb.Text = $p.usb
        $idx = $cmbMode.Items.IndexOf($p.mode)
        if ($idx -ge 0) { $cmbMode.SelectedIndex = $idx }
        Write-Log "已应用方案: $($p.name)" -Color $infoColor
    }
})

# 保存当前配置为方案
$btnSaveProfile.Add_Click({
    $name = [Microsoft.VisualBasic.Interaction]::InputBox("输入方案名称：", "保存方案", "方案" + ($script:profiles.Count + 1))
    if (-not $name) { return }
    # 同名则覆盖
    $script:profiles = @($script:profiles | Where-Object { $_.name -ne $name })
    $script:profiles += [pscustomobject]@{ name = $name; local = $txtLocal.Text; usb = $txtUsb.Text; mode = $cmbMode.SelectedItem }
    Save-Profiles
    Refresh-ProfileList
    for ($i = 0; $i -lt $script:profiles.Count; $i++) {
        if ($script:profiles[$i].name -eq $name) { $cmbProfile.SelectedIndex = $i; break }
    }
    Write-Log "已保存方案: $name" -Color $success
})

# 删除方案
$btnDelProfile.Add_Click({
    if ($cmbProfile.SelectedIndex -lt 0) { Write-Log "请先选择要删除的方案" -Color $warning; return }
    $name = $cmbProfile.SelectedItem
    if ([System.Windows.Forms.MessageBox]::Show("确定删除方案 [$name] 吗？", "删除方案", "YesNo", "Question") -eq "Yes") {
        $script:profiles = @($script:profiles | Where-Object { $_.name -ne $name })
        Save-Profiles
        Refresh-ProfileList
        Write-Log "已删除方案: $name" -Color $warning
    }
})

# Log function
function Write-Log {
    param([string]$Msg, [System.Drawing.Color]$Color = $textColor)
    $ts = Get-Date -Format "HH:mm:ss"
    $txtLog.SelectionStart = $txtLog.TextLength
    $txtLog.SelectionLength = 0
    $txtLog.SelectionColor = $Color
    $txtLog.AppendText("[$ts] $Msg`r`n")
    $txtLog.SelectionStart = $txtLog.TextLength
    $txtLog.ScrollToCaret()
}

# Sync script
$syncScript = {
    param($LocalPath, $UsbPath, $Action, $BackupDir, $RclonePath, $Proc)
    if (-not (Test-Path $RclonePath)) { throw "找不到 rclone: $RclonePath" }
    if (-not (Test-Path $LocalPath)) { New-Item -ItemType Directory -Path $LocalPath -Force | Out-Null }
    if (-not (Test-Path $UsbPath)) { throw "U盘路径不存在: $UsbPath" }
    if (-not (Test-Path $BackupDir)) { New-Item -ItemType Directory -Path $BackupDir -Force | Out-Null }
    $flags = @("--backup-dir", $BackupDir, "--suffix", ".{time:2006-01-02_150405}")
    switch ($Action) {
        "双向同步" { $base = @("bisync", $LocalPath, $UsbPath) }
        "推送到U盘" { $base = @("copy", $LocalPath, $UsbPath) }
        "从U盘拉取" { $base = @("copy", $UsbPath, $LocalPath) }
        "预览变更" { $base = @("bisync", $LocalPath, $UsbPath, "--dry-run") }
    }
    # 参数含空格时加引号，避免路径被拆分成多个参数
    $q = { param($cmd) (($cmd | ForEach-Object { if ($_ -match "\s") { '"' + $_ + '"' } else { $_ } }) -join " ") }
    $Proc.StartInfo.Arguments = & $q ($base + $flags)
    $Proc.Start() | Out-Null
    # 异步读取输出，避免大输出时管道阻塞导致死锁
    $outTask = $Proc.StandardOutput.ReadToEndAsync()
    $errTask = $Proc.StandardError.ReadToEndAsync()
    $Proc.WaitForExit()
    $exitCode = $Proc.ExitCode; $out = $outTask.Result; $err = $errTask.Result
    # 双向同步首次运行（无基准记录）需 --resync，自动重试一次建立基准
    if ($Action -eq "双向同步" -and $exitCode -ne 0 -and (($out + $err) -match "resync")) {
        $Proc.StartInfo.Arguments = & $q ($base + @("--resync") + $flags)
        $Proc.Start() | Out-Null
        $outTask = $Proc.StandardOutput.ReadToEndAsync()
        $errTask = $Proc.StandardError.ReadToEndAsync()
        $Proc.WaitForExit()
        $exitCode = $Proc.ExitCode; $out = $outTask.Result; $err = $errTask.Result
    }
    return @{ ExitCode = $exitCode; Output = $out; Error = $err }
}

# Sync button
$btnSync.Add_Click({
    $lp = $txtLocal.Text
    $up = $txtUsb.Text
    $mode = $cmbMode.SelectedItem
    $bd = "G:\.sync-backup"
    if (-not (Test-Path $lp)) {
        [System.Windows.Forms.MessageBox]::Show("本地路径不存在: $lp", "错误", "OK", "Error"); return
    }
    if (-not (Test-Path $up)) {
        [System.Windows.Forms.MessageBox]::Show("U盘路径不存在: $up`n请确认U盘已插入", "错误", "OK", "Error"); return
    }
    $btnSync.Enabled = $false
    $script:cancelled = $false
    $btnCancel.Enabled = $true
    $btnClose.Enabled = $false
    $script:syncStartTime = Get-Date
    $animTimer.Start()
    $lblStatus.Text = "同步中..."
    $lblStatus.ForeColor = $accent
    $txtLog.Clear()
    Write-Log "开始同步: $mode" -Color $accent
    Write-Log "本地: $lp" -Color $labelColor
    Write-Log "U盘: $up" -Color $labelColor
    Write-Log "备份: $bd" -Color $labelColor
    Write-Log ""
    # 主会话创建进程对象，传入 runspace 按引用共享，供取消按钮直接 Kill
    $script:syncProc = New-Object System.Diagnostics.Process
    $script:syncProc.StartInfo.FileName = $script:rclonePath
    $script:syncProc.StartInfo.UseShellExecute = $false
    $script:syncProc.StartInfo.RedirectStandardOutput = $true
    $script:syncProc.StartInfo.RedirectStandardError = $true
    $script:syncProc.StartInfo.CreateNoWindow = $true
    $script:rs = [runspacefactory]::CreateRunspace(); $script:rs.Open()
    $script:ps = [powershell]::Create(); $script:ps.Runspace = $script:rs
    $script:ps.AddScript($syncScript) | Out-Null
    $script:ps.AddArgument($lp) | Out-Null
    $script:ps.AddArgument($up) | Out-Null
    $script:ps.AddArgument($mode) | Out-Null
    $script:ps.AddArgument($bd) | Out-Null
    $script:ps.AddArgument($script:rclonePath) | Out-Null
    $script:ps.AddArgument($script:syncProc) | Out-Null
    $ar = $script:ps.BeginInvoke()
    while (-not $ar.IsCompleted) {
        Start-Sleep -Milliseconds 50
        $elapsed = (Get-Date) - $script:syncStartTime
        $lblStatus.Text = "同步中... {0:mm\:ss}" -f $elapsed
        [System.Windows.Forms.Application]::DoEvents()
    }
    try {
        $r = $null
        $syncError = $null
        try { $r = $script:ps.EndInvoke($ar) } catch { $syncError = $_ }
        if ($script:cancelled) {
            $lblStatus.Text = "已取消"
            $lblStatus.ForeColor = $warning
            Write-Log "同步已取消" -Color $warning
        } elseif ($syncError) {
            $lblStatus.Text = "同步出错"
            $lblStatus.ForeColor = $errColor
            $msg = $syncError.Exception.InnerException.Message
            if (-not $msg) { $msg = $syncError.Exception.Message }
            Write-Log "错误: $msg" -Color $errColor
        } elseif (-not $r) {
            $lblStatus.Text = "同步出错"
            $lblStatus.ForeColor = $errColor
            Write-Log "错误: 未收到同步结果" -Color $errColor
        } elseif ($r.ExitCode -eq 0) {
            $lblStatus.Text = "同步完成"
            $lblStatus.ForeColor = $success
            Write-Log "同步成功完成" -Color $success
        } elseif ($r.ExitCode -eq 1) {
            $lblStatus.Text = "需要重新同步"
            $lblStatus.ForeColor = $warning
            Write-Log "首次同步或需要 resync，请再次点击同步" -Color $warning
        } else {
            $lblStatus.Text = "同步失败"
            $lblStatus.ForeColor = $errColor
            Write-Log "同步失败 (退出码: $($r.ExitCode))" -Color $errColor
        }
        if ($r) {
            if ($r.Output) { Write-Log ""; Write-Log "输出:" -Color $infoColor; Write-Log ($r.Output -replace "`r", "`n") -Color $infoColor }
            if ($r.Error) {
                # 过滤无害提示（rclone 无配置文件时的默认配置 NOTICE），只保留真正的警告/错误
                $errText = (@($r.Error -split "`r?`n") | Where-Object { $_ -and $_ -notmatch "NOTICE: Config file .* not found - using defaults" }) -join "`n"
                if ($errText) { Write-Log ""; Write-Log "警告/错误:" -Color $errColor; Write-Log $errText -Color $errColor }
            }
        }
    } finally {
        $script:ps.Dispose(); $script:rs.Close()
        $script:ps = $null; $script:rs = $null
        $script:syncProc = $null
        $btnSync.Enabled = $true
        $btnCancel.Enabled = $false
        $btnClose.Enabled = $true
        $animTimer.Stop()
        $bar.Value = 0
    }
})

$btnCancel.Add_Click({
    if (-not $script:syncProc) { Write-Log "没有正在运行的同步" -Color $warning; return }
    $lblStatus.Text = "正在取消..."
    $lblStatus.ForeColor = $warning
    try {
        if (-not $script:syncProc.HasExited) {
            Write-Log "正在终止同步进程..." -Color $warning
            $script:syncProc.Kill()
            $script:syncProc.WaitForExit()
            $script:cancelled = $true
            Write-Log "同步进程已终止" -Color $warning
        } else {
            Write-Log "同步进程已结束" -Color $warning
        }
    } catch {
        Write-Log "取消失败: $($_.Exception.Message)" -Color $errColor
    }
})

$btnClose.Add_Click({ $form.Close() })

# 窗体淡入动画 + 布局完成后应用圆角（Ant Design 组件风格）
$form.Opacity = 0
$fadeTimer = New-Object System.Windows.Forms.Timer
$fadeTimer.Interval = 15
$fadeTimer.Add_Tick({
    if ($form.Opacity -lt 1) { $form.Opacity += 0.08 } else { $fadeTimer.Stop() }
})
$form.Add_Shown({
    # 进度条不应用圆角：SetWindowRgn 会使 Marquee 动画停止绘制
    foreach ($c in @($txtLocal, $txtUsb, $cmbMode, $btnLocal, $btnUsb, $btnSync, $btnCancel, $btnClose)) {
        Set-RoundedCorners $c 6
    }
    $fadeTimer.Start()
})
[void]$form.ShowDialog()
