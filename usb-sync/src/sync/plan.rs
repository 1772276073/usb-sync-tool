//! 快照比对 → 操作计划（同步引擎的核心判定逻辑）

use super::scan::{Entry, ScanResult};
use super::state::StateFile;
use super::Mode;

/// FAT/exFAT 时间戳 2 秒粒度容差
pub const MTIME_WINDOW_MS: i64 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    CopyAToB { backup: bool },
    CopyBToA { backup: bool },
    DeleteA { backup: bool },
    DeleteB { backup: bool },
    /// 空目录传播
    MakeDirA,
    MakeDirB,
}

impl OpKind {
    /// 人读描述（预览与日志用）
    pub fn describe(&self, rel: &str) -> String {
        match self {
            OpKind::CopyAToB { backup } => format!("本地→U盘  {}{}", rel, if *backup { "（覆盖前备份）" } else { "" }),
            OpKind::CopyBToA { backup } => format!("U盘→本地  {}{}", rel, if *backup { "（覆盖前备份）" } else { "" }),
            OpKind::DeleteA { backup } => format!("删除本地  {}{}", rel, if *backup { "（已备份）" } else { "" }),
            OpKind::DeleteB { backup } => format!("删除U盘  {}{}", rel, if *backup { "（已备份）" } else { "" }),
            OpKind::MakeDirA => format!("建目录  本地/{}", rel),
            OpKind::MakeDirB => format!("建目录  U盘/{}", rel),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Op {
    pub kind: OpKind,
    pub rel: String,
    pub bytes: u64,
    /// 变更原因（如"新文件（本地）"、"冲突：本地较新"）
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub ops: Vec<Op>,
    /// 需传输的总字节
    pub total_bytes: u64,
    pub unchanged: usize,
}

impl Plan {
    pub fn copies(&self) -> usize {
        self.ops.iter().filter(|o| matches!(o.kind, OpKind::CopyAToB { .. } | OpKind::CopyBToA { .. })).count()
    }
    pub fn deletions(&self) -> usize {
        self.ops.iter().filter(|o| matches!(o.kind, OpKind::DeleteA { .. } | OpKind::DeleteB { .. })).count()
    }
    /// 执行顺序：空目录 → 复制（大文件优先，减少并行尾延迟）→ 删除（最后，安全）
    pub fn sorted(mut self) -> Plan {
        self.ops.sort_by(|x, y| {
            let rank = |o: &Op| match o.kind {
                OpKind::MakeDirA | OpKind::MakeDirB => 0,
                OpKind::CopyAToB { .. } | OpKind::CopyBToA { .. } => 1,
                OpKind::DeleteA { .. } | OpKind::DeleteB { .. } => 2,
            };
            rank(x).cmp(&rank(y)).then(y.bytes.cmp(&x.bytes)).then(x.rel.cmp(&y.rel))
        });
        self
    }
}

/// 两端实时条目是否一致（大小 + mtime 窗口）
fn same_live(x: &Entry, y: &Entry) -> bool {
    x.size == y.size && (x.mtime_ms - y.mtime_ms).abs() <= MTIME_WINDOW_MS
}

/// 快照印记是否与实时条目一致
fn matches_stamp(e: &Entry, size: u64, mtime: i64) -> bool {
    e.size == size && (e.mtime_ms - mtime).abs() <= MTIME_WINDOW_MS
}

/// 构建同步计划。
///
/// 双向同步使用快照判定"新增 vs 删除"；首次运行（无快照）按合并处理：
/// 缺的补齐，两端都有的按新者胜（败者备份）。
/// 推送/拉取为单向增量复制（对端多余文件保留，不删除——沿用旧版 rclone copy 语义）。
pub fn build(a: &ScanResult, b: &ScanResult, snap: Option<&StateFile>, mode: Mode) -> Plan {
    match mode {
        Mode::Sync | Mode::Preview => build_sync(a, b, snap),
        // a = 本地，b = U盘；单向模式第一参数是源端
        Mode::Push => build_oneway(a, b, Mode::Push),
        Mode::Pull => build_oneway(b, a, Mode::Pull),
    }
}

fn build_sync(a: &ScanResult, b: &ScanResult, snap: Option<&StateFile>) -> Plan {
    let mut plan = Plan::default();
    let mut rels: Vec<&String> = a.files.keys().chain(b.files.keys()).collect();
    rels.sort();
    rels.dedup();

    for rel in rels {
        let ea = a.files.get(rel);
        let eb = b.files.get(rel);
        let st = snap.and_then(|s| s.files.get(rel));

        match (ea, eb) {
            (Some(x), Some(y)) => {
                if same_live(x, y) {
                    plan.unchanged += 1;
                    continue;
                }
                match st {
                    None => {
                        // 快照无记录（首次同步/后来单端出现）：新者胜，败者备份
                        if x.mtime_ms >= y.mtime_ms {
                            push_copy(&mut plan, OpKind::CopyAToB { backup: true }, rel, x.size, "首次合并：本地较新");
                        } else {
                            push_copy(&mut plan, OpKind::CopyBToA { backup: true }, rel, y.size, "首次合并：U盘较新");
                        }
                    }
                    Some(s) => {
                        let ca = !matches_stamp(x, s.size, s.mtime_a);
                        let cb = !matches_stamp(y, s.size, s.mtime_b);
                        match (ca, cb) {
                            (false, false) => plan.unchanged += 1, // 两端与快照一致（大小同但 mtime 微移等）
                            (true, false) => {
                                push_copy(&mut plan, OpKind::CopyAToB { backup: true }, rel, x.size, "本地已修改")
                            }
                            (false, true) => {
                                push_copy(&mut plan, OpKind::CopyBToA { backup: true }, rel, y.size, "U盘已修改")
                            }
                            (true, true) => {
                                // 冲突：两端都改。mtime 相等时偏向本地（推送后 mtime 一致场景取本地无实质损失）
                                if x.mtime_ms >= y.mtime_ms {
                                    push_copy(&mut plan, OpKind::CopyAToB { backup: true }, rel, x.size, "冲突：本地较新");
                                } else {
                                    push_copy(&mut plan, OpKind::CopyBToA { backup: true }, rel, y.size, "冲突：U盘较新");
                                }
                            }
                        }
                    }
                }
            }
            (Some(x), None) => match st {
                // 快照里有 → U盘端删除了它 → 传播删除（先备份本地这份）
                Some(_) => {
                    plan.ops.push(Op {
                        kind: OpKind::DeleteA { backup: true },
                        rel: rel.clone(),
                        bytes: 0,
                        reason: "U盘已删除".into(),
                    });
                }
                None => push_copy(&mut plan, OpKind::CopyAToB { backup: false }, rel, x.size, "新文件（本地）"),
            },
            (None, Some(y)) => match st {
                Some(_) => {
                    plan.ops.push(Op {
                        kind: OpKind::DeleteB { backup: true },
                        rel: rel.clone(),
                        bytes: 0,
                        reason: "本地已删除".into(),
                    });
                }
                None => push_copy(&mut plan, OpKind::CopyBToA { backup: false }, rel, y.size, "新文件（U盘）"),
            },
            (None, None) => unreachable!(),
        }
    }

    // 空目录传播（仅双向模式）
    if snap.is_some() {
        for d in &a.dirs {
            if !b.dirs.contains(d) && !plan.ops.iter().any(|o| o.rel == *d) {
                plan.ops.push(Op { kind: OpKind::MakeDirB, rel: d.clone(), bytes: 0, reason: "新目录（本地）".into() });
            }
        }
        for d in &b.dirs {
            if !a.dirs.contains(d) && !plan.ops.iter().any(|o| o.rel == *d) {
                plan.ops.push(Op { kind: OpKind::MakeDirA, rel: d.clone(), bytes: 0, reason: "新目录（U盘）".into() });
            }
        }
    }

    plan.total_bytes = plan.ops.iter().filter(|o| matches!(o.kind, OpKind::CopyAToB { .. } | OpKind::CopyBToA { .. })).map(|o| o.bytes).sum();
    plan
}

fn build_oneway(src: &ScanResult, dst: &ScanResult, mode: Mode) -> Plan {
    let mut plan = Plan::default();
    for (rel, x) in &src.files {
        match dst.files.get(rel) {
            None => {
                let (kind, reason) = if mode == Mode::Push {
                    (OpKind::CopyAToB { backup: false }, "新文件（本地）")
                } else {
                    (OpKind::CopyBToA { backup: false }, "新文件（U盘）")
                };
                push_copy(&mut plan, kind, rel, x.size, reason);
            }
            Some(y) => {
                if !same_live(x, y) {
                    let (kind, reason) = if mode == Mode::Push {
                        (OpKind::CopyAToB { backup: true }, "已修改（本地较新或不同）")
                    } else {
                        (OpKind::CopyBToA { backup: true }, "已修改（U盘较新或不同）")
                    };
                    push_copy(&mut plan, kind, rel, x.size, reason);
                } else {
                    plan.unchanged += 1;
                }
            }
        }
    }
    plan.total_bytes = plan.ops.iter().map(|o| o.bytes).sum();
    plan
}

fn push_copy(plan: &mut Plan, kind: OpKind, rel: &str, bytes: u64, reason: &str) {
    plan.ops.push(Op {
        kind,
        rel: rel.to_string(),
        bytes,
        reason: reason.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::scan::ScanResult;
    use crate::sync::state::{Stamp, StateFile};

    fn ins(map: &mut ScanResult, rel: &str, size: u64, mtime: i64) {
        map.files.insert(
            rel.to_string(),
            Entry { rel: rel.to_string(), size, mtime_ms: mtime },
        );
    }

    fn snap(pairs: &[(&str, u64, i64, i64)]) -> StateFile {
        let mut st = StateFile {
            version: 1,
            saved_at: String::new(),
            local_root: String::new(),
            usb_root: String::new(),
            files: Default::default(),
        };
        for (rel, size, ma, mb) in pairs {
            st.files.insert(
                rel.to_string(),
                Stamp { size: *size, mtime_a: *ma, mtime_b: *mb },
            );
        }
        st
    }

    fn kinds(plan: &Plan) -> Vec<(OpKind, &str)> {
        plan.ops.iter().map(|o| (o.kind, o.rel.as_str())).collect()
    }

    #[test]
    fn first_run_merges_newer_wins() {
        let mut a = ScanResult::default();
        let mut b = ScanResult::default();
        ins(&mut a, "new.txt", 1, 100); // 仅本地有
        ins(&mut b, "b.txt", 2, 100); // 仅U盘有
        ins(&mut a, "both.txt", 3, 200); // 两端都有，本地较新
        ins(&mut b, "both.txt", 9, 100);
        ins(&mut a, "same.txt", 4, 100); // 两端一致
        ins(&mut b, "same.txt", 4, 101); // 1ms 差 → 视为一致

        let plan = build(&a, &b, None, Mode::Sync);
        let ops = kinds(&plan);
        assert!(ops.contains(&(OpKind::CopyAToB { backup: false }, "new.txt")));
        assert!(ops.contains(&(OpKind::CopyBToA { backup: false }, "b.txt")));
        assert!(ops.contains(&(OpKind::CopyAToB { backup: true }, "both.txt"))); // 本地较新胜
        assert_eq!(plan.unchanged, 1);
        assert_eq!(plan.ops.len(), 3);
    }

    #[test]
    fn snapshot_disambiguates_new_vs_deleted() {
        let mut a = ScanResult::default();
        let mut b = ScanResult::default();
        // 快照里有 keep.txt：A 上还在（未改），B 上没了 → B 删除传播到 A
        ins(&mut a, "keep.txt", 10, 100);
        // 快照里有 gone-b.txt：A 上没了，B 上还在 → A 删除传播到 B
        ins(&mut b, "gone-b.txt", 5, 100);
        // 快照里没有的新文件
        ins(&mut a, "fresh.txt", 1, 300);
        // 快照里 A 单端修改（时间差 > 2s 窗口）
        ins(&mut a, "mod-a.txt", 7, 9999);
        ins(&mut b, "mod-a.txt", 7, 100);

        let st = snap(&[
            ("keep.txt", 10, 100, 100),
            ("gone-b.txt", 5, 100, 100),
            ("mod-a.txt", 7, 100, 100),
        ]);
        let plan = build(&a, &b, Some(&st), Mode::Sync);
        let ops = kinds(&plan);
        assert!(ops.contains(&(OpKind::DeleteA { backup: true }, "keep.txt")));
        assert!(ops.contains(&(OpKind::DeleteB { backup: true }, "gone-b.txt")));
        assert!(ops.contains(&(OpKind::CopyAToB { backup: false }, "fresh.txt")));
        assert!(ops.contains(&(OpKind::CopyAToB { backup: true }, "mod-a.txt")));
        assert_eq!(plan.ops.len(), 4);
    }

    #[test]
    fn conflict_newer_wins() {
        let mut a = ScanResult::default();
        let mut b = ScanResult::default();
        ins(&mut a, "c.txt", 1, 5000); // 本地新
        ins(&mut b, "c.txt", 2, 4000);
        ins(&mut a, "d.txt", 1, 100); // U盘新
        ins(&mut b, "d.txt", 2, 8000);
        let st = snap(&[("c.txt", 1, 100, 100), ("d.txt", 1, 100, 100)]);
        let plan = build(&a, &b, Some(&st), Mode::Sync);
        let ops = kinds(&plan);
        assert!(ops.contains(&(OpKind::CopyAToB { backup: true }, "c.txt")));
        assert!(ops.contains(&(OpKind::CopyBToA { backup: true }, "d.txt")));
    }

    #[test]
    fn push_one_way_no_delete() {
        let mut a = ScanResult::default();
        let mut b = ScanResult::default();
        ins(&mut a, "s.txt", 1, 100);
        ins(&mut a, "mod.txt", 2, 5000);
        ins(&mut b, "mod.txt", 2, 100); // 时间差 > 2s 窗口 → 推送覆盖
        ins(&mut b, "extra-b.txt", 9, 100); // B 多余文件保留（不删）

        let plan = build(&a, &b, None, Mode::Push);
        let ops = kinds(&plan);
        assert!(ops.contains(&(OpKind::CopyAToB { backup: false }, "s.txt")));
        assert!(ops.contains(&(OpKind::CopyAToB { backup: true }, "mod.txt")));
        assert!(!ops.iter().any(|(k, _)| matches!(k, OpKind::DeleteB { .. })));
        assert!(!ops.iter().any(|(_, r)| *r == "extra-b.txt"));
    }

    #[test]
    fn pull_copies_from_usb_to_local() {
        // 回归测试：pull 的源端必须是 U盘（曾把方向传反导致永远拉不到文件）
        let mut a = ScanResult::default();
        let mut b = ScanResult::default();
        ins(&mut b, "from-usb.txt", 5, 100); // 仅U盘有 → 拉取应复制到本地

        let plan = build(&a, &b, None, Mode::Pull);
        let ops = kinds(&plan);
        assert!(ops.contains(&(OpKind::CopyBToA { backup: false }, "from-usb.txt")));
        assert_eq!(plan.ops.len(), 1);
    }

    #[test]
    fn fat_time_window_tolerated() {
        let mut a = ScanResult::default();
        let mut b = ScanResult::default();
        ins(&mut a, "f.txt", 10, 100_000);
        ins(&mut b, "f.txt", 10, 101_999); // <2s → 一致
        ins(&mut a, "g.txt", 10, 100_000);
        ins(&mut b, "g.txt", 10, 102_001); // >2s → 首次合并按不同处理
        let plan = build(&a, &b, None, Mode::Push);
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(plan.ops[0].rel, "g.txt");
    }

    #[test]
    fn sorting_puts_deletions_last() {
        let mut a = ScanResult::default();
        let b = ScanResult::default();
        ins(&mut a, "z-del.txt", 1, 100);
        ins(&mut a, "a-copy.txt", 1, 100);
        let st = snap(&[("z-del.txt", 1, 100, 100)]);
        let plan = build(&a, &b, Some(&st), Mode::Sync).sorted();
        assert!(matches!(plan.ops.last().unwrap().kind, OpKind::DeleteA { .. }));
        assert!(matches!(plan.ops.first().unwrap().kind, OpKind::CopyAToB { .. }));
    }
}
