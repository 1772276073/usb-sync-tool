//! 界面日志缓冲 + 会话日志文件（追加到 %APPDATA%\USB-Sync-Tool\logs\）

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 常规输出（淡蓝）
    Info,
    /// 成功（绿）
    Success,
    /// 警告（橙）
    Warn,
    /// 错误（红）
    Error,
    /// 次要信息（灰）
    Dim,
}

impl Level {
    pub fn prefix(&self) -> &'static str {
        match self {
            Level::Info => "",
            Level::Success => "✓ ",
            Level::Warn => "⚠ ",
            Level::Error => "✗ ",
            Level::Dim => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub msg: String,
}

/// 环形缓冲日志（界面用）
#[derive(Debug, Clone)]
pub struct LogBuf {
    lines: VecDeque<LogLine>,
    cap: usize,
}

impl LogBuf {
    pub fn new(cap: usize) -> Self {
        Self { lines: VecDeque::with_capacity(cap.min(64)), cap }
    }

    pub fn push(&mut self, level: Level, msg: &str) {
        self.push_line(LogLine {
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            level,
            msg: msg.to_string(),
        });
    }

    pub fn push_line(&mut self, line: LogLine) {
        log_to_file(line.level, &line.msg);
        if self.lines.len() >= self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn iter(&self) -> impl Iterator<Item = &LogLine> {
        self.lines.iter()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

impl Default for LogBuf {
    fn default() -> Self {
        Self::new(3000)
    }
}

static LOG_FILE: OnceLock<Mutex<Option<std::path::PathBuf>>> = OnceLock::new();

/// 启用会话日志文件；返回日志文件路径（便于"打开日志目录"）
pub fn enable_session_log() -> Option<std::path::PathBuf> {
    let dir = std::env::var("APPDATA")
        .map(|a| std::path::PathBuf::from(a).join("USB-Sync-Tool").join("logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("USB-Sync-Tool").join("logs"));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("usb-sync-{}.log", chrono::Local::now().format("%Y%m%d")));
    LOG_FILE.set(Mutex::new(Some(path.clone()))).ok()?;
    Some(path)
}

fn log_to_file(level: Level, msg: &str) {
    let cell = match LOG_FILE.get() {
        Some(c) => c,
        None => return,
    };
    let guard = cell.lock();
    let Ok(g) = guard else { return };
    let Some(path) = g.as_ref() else { return };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let tag = match level {
            Level::Info => "INFO",
            Level::Success => "OK",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Dim => "TRACE",
        };
        let _ = writeln!(f, "[{}] [{}] {}", stamp, tag, msg);
    }
}
