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
$text = [System.Drawing.Color]::FromArgb(217, 217, 217)     # antd rgba(255,255,255,0.85)
$textDim = [System.Drawing.Color]::FromArgb(115, 115, 115)  # antd rgba(255,255,255,0.45)
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
    param([string]$Text, [string]$Align = "MiddleLeft", [System.Drawing.Color]$Color = $textDim)
    $l = New-Object System.Windows.Forms.Label
    $l.Text = $Text
    $l.Dock = "Fill"
    $l.TextAlign = $Align
    $l.ForeColor = $Color
    $l.Font = $baseFont
    return $l
}

function New-DarkButton {
    param([string]$Text, [System.Drawing.Color]$Back = $panel, [System.Drawing.Color]$Fore = $text, [bool]$Bold = $false)
    $b = New-Object System.Windows.Forms.Button
    $b.Text = $Text
    $b.BackColor = $Back
    $b.ForeColor = $Fore
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
$layout.RowCount = 9
$layout.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle("Absolute", 80)))
$layout.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle("Percent", 100)))
$layout.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle("Absolute", 78)))
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 0 标题
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 1 本地路径
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 2 U盘路径
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 42)))   # 3 同步模式
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 38)))   # 4 进度
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 30)))   # 5 状态
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 26)))   # 6 日志标签
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Percent", 100)))   # 7 日志框
$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle("Absolute", 48)))   # 8 按钮行
$form.Controls.Add($layout)

# 标题（跨 3 列）
$title = New-Object System.Windows.Forms.Label
$title.Text = "USB 同步"
$title.Dock = "Fill"
$title.TextAlign = "MiddleLeft"
$title.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 16, [System.Drawing.FontStyle]::Bold)
$title.ForeColor = $text
$layout.Controls.Add($title, 0, 0)
$layout.SetColumnSpan($title, 3)

# 本地路径行
$lblLocal = New-DarkLabel "本地路径"
$layout.Controls.Add($lblLocal, 0, 1)
$txtLocal = New-Object System.Windows.Forms.TextBox
$txtLocal.Dock = "Fill"
$txtLocal.Text = (Join-Path $PSScriptRoot "synced")
$txtLocal.BackColor = $input
$txtLocal.ForeColor = $text
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
$txtUsb.ForeColor = $text
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
$cmbMode.ForeColor = $text
$cmbMode.FlatStyle = "Flat"
$cmbMode.Items.AddRange(@("双向同步", "推送到U盘", "从U盘拉取", "预览变更"))
$cmbMode.SelectedIndex = 0
$layout.Controls.Add($cmbMode, 1, 3)

# 进度行（进度条跨 2 列）
$lblProgress = New-DarkLabel "进度"
$layout.Controls.Add($lblProgress, 0, 4)
$bar = New-Object System.Windows.Forms.ProgressBar
$bar.Dock = "Fill"
$bar.Style = "Marquee"
$bar.MarqueeAnimationSpeed = 0
$bar.BackColor = $panel
$layout.Controls.Add($bar, 1, 4)
$layout.SetColumnSpan($bar, 2)

# 状态行（跨 3 列）
$lblStatus = New-DarkLabel "就绪" "MiddleLeft" $success
$lblStatus.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 9, [System.Drawing.FontStyle]::Bold)
$layout.Controls.Add($lblStatus, 0, 5)
$layout.SetColumnSpan($lblStatus, 3)

# 日志标签（跨 3 列）
$lblLog = New-DarkLabel "日志"
$layout.Controls.Add($lblLog, 0, 6)
$layout.SetColumnSpan($lblLog, 3)

# 日志框（跨 3 列，占满剩余高度）
$txtLog = New-Object System.Windows.Forms.TextBox
$txtLog.Dock = "Fill"
$txtLog.Multiline = $true
$txtLog.ScrollBars = "Vertical"
$txtLog.ReadOnly = $true
$txtLog.BackColor = $input
$txtLog.ForeColor = $text
$txtLog.Font = New-Object System.Drawing.Font("Consolas", 9)
$txtLog.BorderStyle = "FixedSingle"
$layout.Controls.Add($txtLog, 0, 7)
$layout.SetColumnSpan($txtLog, 3)

# 按钮行：左（开始+取消）| 中（空）| 右（关闭）
$leftRow = New-Object System.Windows.Forms.FlowLayoutPanel
$leftRow.Dock = "Fill"
$leftRow.FlowDirection = "LeftToRight"
$leftRow.WrapContents = $false
$leftRow.BackColor = $bg
$btnSync = New-DarkButton "开始同步" $accent ([System.Drawing.Color]::White) $true
$btnSync.FlatAppearance.BorderSize = 0
$btnSync.Size = New-Object System.Drawing.Size(110, 36)
$leftRow.Controls.Add($btnSync)
$btnCancel = New-DarkButton "取消"
$btnCancel.Size = New-Object System.Drawing.Size(90, 36)
$btnCancel.Enabled = $false
$leftRow.Controls.Add($btnCancel)
$layout.Controls.Add($leftRow, 0, 8)
$layout.SetColumnSpan($leftRow, 2)

$btnClose = New-DarkButton "关闭"
$btnClose.Dock = "Fill"
$layout.Controls.Add($btnClose, 2, 8)

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

# Log function
function Write-Log {
    param([string]$Msg)
    $ts = Get-Date -Format "HH:mm:ss"
    $txtLog.AppendText("[$ts] $Msg`r`n")
    $txtLog.SelectionStart = $txtLog.Text.Length
    $txtLog.ScrollToCaret()
}

# Sync script
$syncScript = {
    param($LocalPath, $UsbPath, $Action, $BackupDir, $RclonePath, $Proc)
    if (-not (Test-Path $RclonePath)) { throw "找不到 rclone: $RclonePath" }
    if (-not (Test-Path $LocalPath)) { New-Item -ItemType Directory -Path $LocalPath -Force | Out-Null }
    if (-not (Test-Path $UsbPath)) { throw "U盘路径不存在: $UsbPath" }
    if (-not (Test-Path $BackupDir)) { New-Item -ItemType Directory -Path $BackupDir -Force | Out-Null }
    $flags = @("--backup-dir", $BackupDir, "--suffix", ".{time:2006-01-02_150405}", "--progress")
    switch ($Action) {
        "双向同步" { $cmd = @("bisync", $LocalPath, $UsbPath) + $flags }
        "推送到U盘" { $cmd = @("copy", $LocalPath, $UsbPath) + $flags }
        "从U盘拉取" { $cmd = @("copy", $UsbPath, $LocalPath) + $flags }
        "预览变更" { $cmd = @("bisync", $LocalPath, $UsbPath, "--dry-run") + $flags }
    }
    # 参数含空格时加引号，避免路径被拆分成多个参数
    $Proc.StartInfo.Arguments = (($cmd | ForEach-Object { if ($_ -match "\s") { '"' + $_ + '"' } else { $_ } }) -join " ")
    $Proc.Start() | Out-Null
    # 异步读取输出，避免大输出时管道阻塞导致死锁
    $outTask = $Proc.StandardOutput.ReadToEndAsync()
    $errTask = $Proc.StandardError.ReadToEndAsync()
    $Proc.WaitForExit()
    return @{ ExitCode = $Proc.ExitCode; Output = $outTask.Result; Error = $errTask.Result }
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
    $bar.MarqueeAnimationSpeed = 30
    $lblStatus.Text = "同步中..."
    $lblStatus.ForeColor = $accent
    $txtLog.Clear()
    Write-Log "开始同步: $mode"
    Write-Log "本地: $lp"
    Write-Log "U盘: $up"
    Write-Log "备份: $bd"
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
    while (-not $ar.IsCompleted) { Start-Sleep -Milliseconds 50; [System.Windows.Forms.Application]::DoEvents() }
    try {
        $r = $null
        $syncError = $null
        try { $r = $script:ps.EndInvoke($ar) } catch { $syncError = $_ }
        if ($script:cancelled) {
            $lblStatus.Text = "已取消"
            $lblStatus.ForeColor = $warning
            Write-Log "同步已取消"
        } elseif ($syncError) {
            $lblStatus.Text = "同步出错"
            $lblStatus.ForeColor = $errColor
            $msg = $syncError.Exception.InnerException.Message
            if (-not $msg) { $msg = $syncError.Exception.Message }
            Write-Log "错误: $msg"
        } elseif (-not $r) {
            $lblStatus.Text = "同步出错"
            $lblStatus.ForeColor = $errColor
            Write-Log "错误: 未收到同步结果"
        } elseif ($r.ExitCode -eq 0) {
            $lblStatus.Text = "同步完成"
            $lblStatus.ForeColor = $success
            Write-Log "同步成功完成"
        } elseif ($r.ExitCode -eq 1) {
            $lblStatus.Text = "需要重新同步"
            $lblStatus.ForeColor = $warning
            Write-Log "首次同步或需要 resync，请再次点击同步"
        } else {
            $lblStatus.Text = "同步失败"
            $lblStatus.ForeColor = $errColor
            Write-Log "同步失败 (退出码: $($r.ExitCode))"
        }
        if ($r) {
            if ($r.Output) { Write-Log ""; Write-Log "输出:"; Write-Log $r.Output }
            if ($r.Error) { Write-Log ""; Write-Log "错误:"; Write-Log $r.Error }
        }
    } finally {
        $script:ps.Dispose(); $script:rs.Close()
        $script:ps = $null; $script:rs = $null
        $script:syncProc = $null
        $btnSync.Enabled = $true
        $btnCancel.Enabled = $false
        $btnClose.Enabled = $true
        $bar.MarqueeAnimationSpeed = 0
    }
})

$btnCancel.Add_Click({
    if (-not $script:syncProc) { Write-Log "没有正在运行的同步"; return }
    $lblStatus.Text = "正在取消..."
    $lblStatus.ForeColor = $warning
    try {
        if (-not $script:syncProc.HasExited) {
            Write-Log "正在终止同步进程..."
            $script:syncProc.Kill()
            $script:syncProc.WaitForExit()
            $script:cancelled = $true
            Write-Log "同步进程已终止"
        } else {
            Write-Log "同步进程已结束"
        }
    } catch {
        Write-Log "取消失败: $($_.Exception.Message)"
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
    foreach ($c in @($txtLocal, $txtUsb, $cmbMode, $bar, $btnLocal, $btnUsb, $btnSync, $btnCancel, $btnClose)) {
        Set-RoundedCorners $c 6
    }
    $fadeTimer.Start()
})
$form.Show()
[System.Windows.Forms.Application]::DoEvents()
Start-Sleep -Milliseconds 300
[System.Windows.Forms.Application]::DoEvents()
"row col type             X    Y    W    H"
foreach ($ctl in $layout.Controls) {
    $pos = $layout.GetCellPosition($ctl)
    "{0}  {1}  {2,-16} {3,4} {4,4} {5,4} {6,4}" -f $pos.Row, $pos.Column, $ctl.GetType().Name, $ctl.Bounds.X, $ctl.Bounds.Y, $ctl.Bounds.Width, $ctl.Bounds.Height
}
"opacity after fade: $($form.Opacity)"
$form.Close()
