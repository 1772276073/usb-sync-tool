//! 同步引擎

pub mod exec;
pub mod filters;
pub mod plan;
pub mod progress;
pub mod scan;
pub mod state;

use std::path::PathBuf;

/// 同步模式（与旧版 4 种模式对应，profile 中存稳定英文标识）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// 双向同步
    Sync,
    /// 推送到U盘（本地 → U盘）
    Push,
    /// 从U盘拉取（U盘 → 本地）
    Pull,
    /// 预览变更（不落盘）
    Preview,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Sync, Mode::Push, Mode::Pull, Mode::Preview];

    pub fn label(&self) -> &'static str {
        match self {
            Mode::Sync => "双向同步",
            Mode::Push => "推送到U盘",
            Mode::Pull => "从U盘拉取",
            Mode::Preview => "预览变更",
        }
    }

    /// 兼容旧版 profiles.json（v1 存的是中文界面文案）
    pub fn parse(s: &str) -> Option<Mode> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
            .or(match s {
                "双向同步" => Some(Mode::Sync),
                "推送到U盘" => Some(Mode::Push),
                "从U盘拉取" => Some(Mode::Pull),
                "预览变更" => Some(Mode::Preview),
                _ => None,
            })
    }
}

/// 一次同步运行的配置
#[derive(Debug, Clone)]
pub struct Options {
    /// 本地路径（A 端）
    pub local: PathBuf,
    /// U盘路径（B 端）
    pub usb: PathBuf,
    pub mode: Mode,
    /// 备份目录覆盖；None → `<usb>\.sync-backup`
    pub backup_dir: Option<PathBuf>,
    /// 备份总量软上限（字节）；超过则从最旧的运行目录开始删除，0 = 不限制
    pub backup_max_bytes: u64,
    /// 并行复制线程数
    pub threads: usize,
    /// 排除规则（每行一条，`#` 开头为注释）
    pub filters: Vec<String>,
}

impl Options {
    /// 备份目录：显式指定或默认 `<usb>\.sync-backup`
    pub fn backup_root(&self) -> PathBuf {
        self.backup_dir
            .clone()
            .unwrap_or_else(|| self.usb.join(".sync-backup"))
    }
}

/// 同步结果汇总
#[derive(Debug, Clone, Default)]
pub struct SyncOutcome {
    /// 计划的操作数（复制+删除）
    pub planned_ops: usize,
    pub copied: usize,
    pub deleted: usize,
    pub backed_up: usize,
    pub bytes_copied: u64,
    /// 每条失败的 (相对路径, 错误信息)
    pub errors: Vec<(String, String)>,
    pub cancelled: bool,
    /// 预览模式下为 true：列出了操作但未执行
    pub dry_run: bool,
    /// 是否写入了新的状态快照（取消/出错时不写）
    pub state_saved: bool,
}

impl SyncOutcome {
    pub fn ok(&self) -> bool {
        self.errors.is_empty() && !self.cancelled
    }
}

/// 解析多行过滤文本 → 规则列表（跳过空行与注释）
pub fn parse_filter_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}
