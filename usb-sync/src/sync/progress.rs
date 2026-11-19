//! 进度事件与实时速度计量

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 扫描实时探针：引擎侧原子更新，界面侧每 300ms 读取
#[derive(Debug, Default)]
pub struct ScanProbe {
    pub files: std::sync::atomic::AtomicU64,
    pub bytes: std::sync::atomic::AtomicU64,
    pub cur: std::sync::Mutex<String>,
    pub done: std::sync::atomic::AtomicBool,
}

impl ScanProbe {
    pub fn snapshot(&self) -> (u64, u64, String) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.files.load(Relaxed),
            self.bytes.load(Relaxed),
            self.cur.lock().map(|g| g.clone()).unwrap_or_default(),
        )
    }
}

/// 同步阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Scanning,
    Comparing,
    Transferring,
    Finalizing,
}

impl Phase {
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Scanning => "扫描中",
            Phase::Comparing => "比对中",
            Phase::Transferring => "同步中",
            Phase::Finalizing => "收尾中",
        }
    }
}

/// 单个操作的回报类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpReport {
    Copied,
    Deleted,
    MadeDir,
}

/// 引擎 → 界面的事件流
#[derive(Debug, Clone)]
pub enum Ev {
    Phase(Phase),
    PlanReady {
        files: usize,
        bytes: u64,
        ops: usize,
        unchanged: usize,
    },
    FileStart {
        rel: String,
    },
    FileDone {
        rel: String,
        bytes: u64,
        report: OpReport,
    },
    OpError {
        rel: String,
        msg: String,
    },
    /// 提示信息（非错误，如"若干不可读条目已跳过"）
    Notice(String),
    /// 预览模式：展示将执行的操作
    PreviewOp {
        rel: String,
        desc: String,
        bytes: u64,
    },
    /// 扫描实时进度（每 ~300ms 一次）
    ScanTick {
        files: u64,
        bytes: u64,
        dir: String,
    },
    Done,
}

/// 滑动窗口速度计量（近 2 秒字节数 / 时间）
#[derive(Debug, Default)]
pub struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
    window: Duration,
}

impl SpeedMeter {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            window: Duration::from_secs(2),
        }
    }

    pub fn mark(&mut self, total_bytes: u64) {
        let now = Instant::now();
        self.samples.push_back((now, total_bytes));
        while let Some((t, _)) = self.samples.front() {
            if now.duration_since(*t) > self.window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// 近窗口平均字节速率
    pub fn bytes_per_sec(&self) -> f64 {
        let Some((oldest_t, oldest_b)) = self.samples.front() else {
            return 0.0;
        };
        let Some((newest_t, newest_b)) = self.samples.back() else {
            return 0.0;
        };
        let dt = newest_t.duration_since(*oldest_t).as_secs_f64();
        if dt < 0.05 {
            return 0.0;
        }
        (newest_b - oldest_b) as f64 / dt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_zero_without_samples() {
        let m = SpeedMeter::new();
        assert_eq!(m.bytes_per_sec(), 0.0);
    }
}
