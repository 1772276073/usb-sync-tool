using System;
using System.Diagnostics;
using System.IO;
using System.Windows.Forms;

// 便携启动器：双击运行同目录的 sync-gui.ps1（深色主题 GUI）
// 编译: csc.exe /nologo /target:winexe /out:USB-Sync.exe /r:System.Windows.Forms.dll Launcher.cs
class USB_Sync_Launcher {
    [STAThread]
    static void Main() {
        string dir = AppDomain.CurrentDomain.BaseDirectory;
        string ps1 = Path.Combine(dir, "sync-gui.ps1");
        if (!File.Exists(ps1)) {
            MessageBox.Show("缺少 sync-gui.ps1，请确认它与 USB-Sync.exe 在同一目录。", "USB 同步工具",
                MessageBoxButtons.OK, MessageBoxIcon.Error);
            return;
        }
        try {
            Process p = new Process();
            p.StartInfo.FileName = "powershell.exe";
            p.StartInfo.Arguments = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"" + ps1 + "\"";
            p.StartInfo.WorkingDirectory = dir;
            p.Start();
        } catch (Exception ex) {
            MessageBox.Show("启动失败: " + ex.Message, "USB 同步工具",
                MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
    }
}
