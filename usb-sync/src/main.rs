//! USB 同步工具 v2 图形界面入口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = usb_sync::run_gui() {
        eprintln!("GUI 启动失败：{e}");
        std::process::exit(1);
    }
}

