$ErrorActionPreference = 'Stop'
$exe = 'E:\rust-target\usb-sync\release\usb-sync.exe'
$out = 'E:\rust-target\usb-sync\ui-preview.png'
Remove-Item $out -Force -ErrorAction SilentlyContinue

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class Win {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public struct RECT { public int L; public int T; public int R; public int B; }
  public static IntPtr MainByPid(uint targetPid) {
    IntPtr best = IntPtr.Zero; long bestArea = 0;
    EnumWindows(delegate(IntPtr h, IntPtr l) {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid == targetPid && IsWindowVisible(h)) {
        RECT r; GetWindowRect(h, out r);
        long area = (long)(r.R - r.L) * (r.B - r.T);
        if (area > bestArea) { bestArea = area; best = h; }
      }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
"@
Add-Type -AssemblyName System.Drawing

$p = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 4
$h = [Win]::MainByPid([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; throw 'main window not found' }
Start-Sleep -Milliseconds 2000   # 等入场淡入完成

$r = New-Object 'Win+RECT'
[Win]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $hh = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap($w, $hh)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# PW_RENDERFULLCONTENT(0x2)：抓取 GPU 渲染窗口的实际表面，无需窗口在前台
$ok = [Win]::PrintWindow($h, $hdc, 2)
$g.ReleaseHdc($hdc)
$g.Dispose()
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
"printwindow ok=$ok  size=${w}x${hh}  saved=$out"
