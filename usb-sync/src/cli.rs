//! 命令行版（usb-sync-cli）：定时任务/脚本化场景，与 GUI 共用同一引擎

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use clap::{Parser, Subcommand};

use crate::log::enable_session_log;
use crate::sync::exec;
use crate::sync::progress::{Ev, Phase};
use crate::sync::{parse_filter_lines, Mode, Options};

#[derive(Parser)]
#[command(
    name = "usb-sync-cli",
    version,
    about = "USB 同步工具 v2 命令行版（与图形版共用同步引擎）",
    after_help = "示例：\n  usb-sync-cli sync --local D:\\Docs --usb G:\\Docs\n  usb-sync-cli push --local D:\\Docs --usb G:\\Docs --threads 4"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// 备份目录（默认 <usb>\\.sync-backup）
    #[arg(long, global = true)]
    backup_dir: Option<String>,

    /// 备份总量上限（GB，超过后从最旧开始清理；0 = 不限制）
    #[arg(long, global = true, default_value_t = 3.0)]
    max_backup_gb: f64,

    /// 并行复制线程数
    #[arg(long, global = true, default_value_t = 8)]
    threads: usize,

    /// 排除规则文件（每行一条；缺省用内置规则）
    #[arg(long, global = true)]
    filters_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 双向同步（快照式，首次运行自动合并）
    Sync {
        /// 本地路径
        #[arg(long)]
        local: String,
        /// U盘路径
        #[arg(long)]
        usb: String,
    },
    /// 推送到U盘（本地 → U盘，单向增量）
    Push {
        #[arg(long)]
        local: String,
        #[arg(long)]
        usb: String,
    },
    /// 从U盘拉取（U盘 → 本地，单向增量）
    Pull {
        #[arg(long)]
        local: String,
        #[arg(long)]
        usb: String,
    },
    /// 预览变更（不执行任何更改）
    Preview {
        #[arg(long)]
        local: String,
        #[arg(long)]
        usb: String,
    },
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let _ = enable_session_log();

    let (mode, local, usb) = match &cli.cmd {
        Cmd::Sync { local, usb } => (Mode::Sync, local, usb),
        Cmd::Push { local, usb } => (Mode::Push, local, usb),
        Cmd::Pull { local, usb } => (Mode::Pull, local, usb),
        Cmd::Preview { local, usb } => (Mode::Preview, local, usb),
    };

    let filters = match &cli.filters_file {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(s) => parse_filter_lines(&s),
            Err(e) => {
                eprintln!("[ERR] 读取过滤规则失败 {}: {}", p.display(), e);
                return 1;
            }
        },
        None => parse_filter_lines(&crate::sync::filters::DEFAULT_FILTERS.join("\n")),
    };

    let opts = Options {
        local: local.into(),
        usb: usb.into(),
        mode,
        backup_dir: cli.backup_dir.as_ref().map(PathBuf::from),
        backup_max_bytes: (cli.max_backup_gb * 1073741824.0) as u64,
        threads: cli.threads.clamp(1, 32),
        filters,
    };

    println!("[SYNC] {}  本地：{}  U盘：{}", mode.label(), local, usb);
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    if std::env::var("USBSYNC_NO_CTRLC").is_err() {
        install_ctrlc(Arc::clone(&cancel));
    }

    let handle = exec::spawn(opts, Arc::clone(&cancel), tx);
    let mut phase = Phase::Scanning;
    while let Ok(ev) = rx.recv() {
        match ev {
            Ev::Phase(p) => {
                if p == Phase::Transferring {
                    println!("[INFO] 开始传输…");
                }
                phase = p;
            }
            Ev::PlanReady { files, bytes, ops, unchanged } => {
                if ops == 0 {
                    println!("[OK]   两端一致，无需变更");
                } else {
                    println!(
                        "[INFO] 计划：复制 {files} 项（{}），删除/其他 {} 项，无需变更 {unchanged} 项",
                        fmt_bytes(bytes),
                        ops.saturating_sub(files)
                    );
                }
            }
            Ev::FileStart { .. } => {}
            Ev::FileDone { rel, bytes, report } => {
                let _ = phase;
                match report {
                    crate::sync::progress::OpReport::Copied => {
                        println!("[COPY] {} ({})", rel, fmt_bytes(bytes));
                    }
                    crate::sync::progress::OpReport::Deleted => println!("[DEL ] {}", rel),
                    crate::sync::progress::OpReport::MadeDir => println!("[MKDIR] {}", rel),
                }
            }
            Ev::OpError { rel, msg } => {
                if rel.is_empty() {
                    eprintln!("[ERR] {}", msg);
                } else {
                    eprintln!("[ERR] {}: {}", rel, msg);
                }
            }
            Ev::ScanTick { .. } => {}
            Ev::Notice(m) => eprintln!("[WARN] {}", m),
            Ev::PreviewOp { desc, .. } => println!("[PLAN] {}", desc),
            Ev::Done => break,
        }
    }

    let outcome = handle.join().unwrap_or_default();
    let verb = if outcome.dry_run { "预览" } else { "同步" };
    if outcome.cancelled {
        println!(
            "[WARN] {}已取消：复制 {}、删除 {}（部分完成，未写状态快照）",
            verb, outcome.copied, outcome.deleted
        );
        return 1;
    }
    if !outcome.errors.is_empty() {
        println!(
            "[ERR] {}结束：{} 项失败（共复制 {}、删除 {}）",
            verb,
            outcome.errors.len(),
            outcome.copied,
            outcome.deleted
        );
        return 2;
    }
    if outcome.dry_run {
        println!("[OK] 预览完成：共 {} 项操作，未做任何更改", outcome.planned_ops);
    } else {
        println!(
            "[OK] 同步完成：复制 {}（{}）、删除 {}、备份 {}{}",
            outcome.copied,
            fmt_bytes(outcome.bytes_copied),
            outcome.deleted,
            outcome.backed_up,
            if outcome.state_saved { "，快照已更新" } else { "" }
        );
    }
    0
}

/// Ctrl+C → 优雅取消（Windows 控制台）。处理器置位取消标志并拦截默认终止。
fn install_ctrlc(cancel: Arc<AtomicBool>) {
    static PENDING: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(Some(cancel)));

    #[cfg(windows)]
    unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
        let _ = ctrl_type;
        if let Some(cell) = PENDING.get() {
            if let Ok(mut guard) = cell.lock() {
                if let Some(c) = guard.take() {
                    c.store(true, Ordering::Relaxed);
                    eprintln!("\n[WARN] 收到中断信号，正在优雅停止…");
                    return 1; // 已处理，交给引擎自然退出
                }
            }
        }
        0 // 二次按下等极端情况：走默认终止
    }

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}
