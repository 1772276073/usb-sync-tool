//! USB 同步工具 v2 核心库
//!
//! 纯 Rust 原生同步引擎：并行扫描 → 快照比对 → 多线程执行 → 原子状态落盘。
//! 图形界面（`usb-sync`）与命令行（`usb-sync-cli`）共用此库。

pub mod anim;
pub mod app;
pub mod cli;
pub mod log;
pub mod profile;
pub mod sync;
pub mod theme;

pub use sync::{Mode, Options, SyncOutcome};

/// 图形界面入口（`usb-sync` bin 调用）
pub fn run_gui() -> Result<(), String> {
    let log_path = log::enable_session_log();
    app::run(log_path)
}

/// 命令行入口（`usb-sync-cli` bin 调用）；返回进程退出码
pub fn run_cli() -> i32 {
    cli::run()
}
