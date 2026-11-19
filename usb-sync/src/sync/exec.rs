//! 计划执行：多线程复制/备份/删除 + 进度事件 + 优雅取消
//!
//! 取消语义：AtomicBool 协作式停止——正在复制中的文件会完成，之后的操作不再执行；
//! 取消或出错时不写状态快照（快照必须如实反映两端一致的已同步状态）。
//! 备份：被覆盖/删除的旧文件复制到 `<备份目录>\<运行时间戳>\<相对路径>`，结构可还原。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{bounded, unbounded, Sender};

use super::filters::Filters;
use super::plan::{Op, OpKind, Plan};
use super::progress::{Ev, OpReport, Phase, ScanProbe};
use super::scan::{self, ScanResult};
use super::state::StateFile;
use super::{Mode, Options, SyncOutcome};

/// 两端根路径
#[derive(Debug, Clone)]
pub struct Roots {
    pub local: PathBuf,
    pub usb: PathBuf,
}

impl From<&Options> for Roots {
    fn from(o: &Options) -> Self {
        Roots { local: o.local.clone(), usb: o.usb.clone() }
    }
}

/// 单个操作执行结果
struct OpResult {
    report: OpReport,
    bytes: u64,
    backed_up: bool,
}

/// 启动引擎线程，立即返回。事件经 `tx` 推送，最终结果为线程返回值。
pub fn spawn(opts: Options, cancel: Arc<AtomicBool>, tx: Sender<Ev>) -> std::thread::JoinHandle<SyncOutcome> {
    std::thread::Builder::new()
        .name("usb-sync-engine".into())
        .spawn(move || run(&opts, &cancel, &tx))
        .expect("启动同步引擎线程失败")
}

fn run(opts: &Options, cancel: &Arc<AtomicBool>, tx: &Sender<Ev>) -> SyncOutcome {
    let mut outcome = SyncOutcome { dry_run: opts.mode == Mode::Preview, ..Default::default() };
    let filters = Filters::with_defaults(&opts.filters);
    let roots = Roots::from(opts);

    // ---- 扫描 ----
    let _ = tx.send(Ev::Phase(Phase::Scanning));
    // 实时进度：ticker 线程每 300ms 读取探针并向界面发一次 ScanTick
    let probe = Arc::new(ScanProbe::default());
    let ticker = {
        let probe = Arc::clone(&probe);
        let tx = tx.clone();
        std::thread::spawn(move || {
            while !probe.done.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(300));
                if probe.done.load(Ordering::Relaxed) {
                    break;
                }
                let (files, bytes, dir) = probe.snapshot();
                let _ = tx.send(Ev::ScanTick { files, bytes, dir });
            }
        })
    };
    let scan_res = scan_both(opts, &filters, cancel, Some(&probe));
    probe.done.store(true, Ordering::Relaxed);
    let (a, b, skipped) = match scan_res {
        Ok(v) => v,
        Err(e) if e.downcast_ref::<super::scan::ScanCancelled>().is_some() => {
            // 扫描期间用户取消：非错误，按取消收尾
            outcome.cancelled = true;
            let _ = ticker.join();
            let _ = tx.send(Ev::Done);
            return outcome;
        }
        Err(e) => {
            let _ = ticker.join();
            outcome.errors.push((String::new(), e.to_string()));
            let _ = tx.send(Ev::OpError { rel: String::new(), msg: e.to_string() });
            let _ = tx.send(Ev::Done);
            return outcome;
        }
    };
    let _ = ticker.join();
    if skipped > 0 {
        let _ = tx.send(Ev::Notice(format!("{} 个不可读或非 UTF-8 路径的条目已跳过", skipped)));
    }

    // ---- 比对 ----
    let _ = tx.send(Ev::Phase(Phase::Comparing));
    let snap = match opts.mode {
        Mode::Sync | Mode::Preview => StateFile::load(&opts.usb),
        _ => None,
    };
    let plan = super::plan::build(&a, &b, snap.as_ref(), opts.mode).sorted();
    outcome.planned_ops = plan.ops.len();
    let _ = tx.send(Ev::PlanReady {
        files: plan.copies(),
        bytes: plan.total_bytes,
        ops: plan.ops.len(),
        unchanged: plan.unchanged,
    });

    // ---- 空计划 ----
    if plan.ops.is_empty() {
        // 即使本次无变更，也检查备份累积（预览模式不清理）
        if !outcome.dry_run {
            enforce_retention(opts, cancel, tx, "");
        }
        let _ = tx.send(Ev::Done);
        return outcome;
    }

    // ---- 预览：只列出操作 ----
    if outcome.dry_run {
        for op in &plan.ops {
            let _ = tx.send(Ev::PreviewOp { rel: op.rel.clone(), desc: op.kind.describe(&op.rel), bytes: op.bytes });
        }
        let _ = tx.send(Ev::Done);
        return outcome;
    }

    // ---- 执行 ----
    let _ = tx.send(Ev::Phase(Phase::Transferring));
    let run_stamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_root = opts.backup_root().join(&run_stamp);
    let stats = execute_plan(&plan, &roots, &backup_root, opts.threads, cancel, tx);
    outcome.copied = stats.copied;
    outcome.deleted = stats.deleted;
    outcome.backed_up = stats.backed_up;
    outcome.bytes_copied = stats.bytes;
    outcome.errors.extend(stats.errors);
    outcome.cancelled = cancel.load(Ordering::Relaxed);

    // ---- 收尾：全部成功的双向同步写入新快照 ----
    let _ = tx.send(Ev::Phase(Phase::Finalizing));
    if outcome.ok() && opts.mode == Mode::Sync {
        match finish_sync(opts, &filters, cancel) {
            Ok((a2, b2)) => {
                let mut st = StateFile::build(&a2, &b2);
                st.local_root = opts.local.display().to_string();
                st.usb_root = opts.usb.display().to_string();
                match st.save(&opts.usb) {
                    Ok(()) => outcome.state_saved = true,
                    Err(e) => {
                        let msg = format!("同步完成，但状态快照写入失败：{}（下次将按首次合并处理）", e);
                        outcome.errors.push((String::new(), msg.clone()));
                        let _ = tx.send(Ev::OpError { rel: String::new(), msg });
                    }
                }
            }
            Err(e) => {
                let msg = format!("同步完成，但复核扫描失败：{}（下次将按首次合并处理）", e);
                outcome.errors.push((String::new(), msg.clone()));
                let _ = tx.send(Ev::OpError { rel: String::new(), msg });
            }
        }
    }
    // ---- 备份保留策略：总量超上限时从最旧开始清理（保护本次运行目录）----
    if !cancel.load(Ordering::Relaxed) {
        enforce_retention(opts, cancel, tx, &run_stamp);
    }
    let _ = tx.send(Ev::Done);
    outcome
}

/// 备份保留策略入口：超过软上限时从最旧的运行目录开始删除。
/// `keep` 为本次运行目录名（永远不删）；0 上限 = 不限制；取消/预览不执行。
fn enforce_retention(opts: &Options, cancel: &Arc<AtomicBool>, tx: &Sender<Ev>, keep: &str) {
    if opts.backup_max_bytes == 0 || cancel.load(Ordering::Relaxed) {
        return;
    }
    let root = opts.backup_root();
    if !root.is_dir() {
        return;
    }
    match enforce_backup_retention(&root, opts.backup_max_bytes, keep) {
        Ok((n, freed)) if n > 0 => {
            let _ = tx.send(Ev::Notice(format!(
                "备份清理：删除 {n} 个旧备份目录，释放 {:.0} MiB",
                freed as f64 / 1048576.0
            )));
        }
        Ok(_) => {}
        Err(e) => {
            let _ = tx.send(Ev::Notice(format!("备份清理失败：{e}")));
        }
    }
}

/// 计算目录总大小（递归）
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = fs::read_dir(path) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(md) = e.metadata() {
                total += md.len();
            }
        }
    }
    total
}

/// 保留策略核心：目录名匹配 `YYYYMMDD_HHMMSS` 的运行目录按时间排序，
/// 总量超过 max_bytes 时从最旧开始删除；`keep`（本次运行目录）与最近一次备份永不删除。
fn enforce_backup_retention(backup_root: &Path, max_bytes: u64, keep: &str) -> Result<(usize, u64)> {
    if max_bytes == 0 || !backup_root.is_dir() {
        return Ok((0, 0));
    }
    let mut runs: Vec<(String, u64)> = Vec::new();
    for entry in fs::read_dir(backup_root)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_run = name.len() == 15
            && name
                .as_bytes()
                .iter()
                .enumerate()
                .all(|(i, b)| if i == 8 { *b == b'_' } else { b.is_ascii_digit() });
        if !is_run {
            continue; // 非运行目录（用户自放文件）一律不动
        }
        runs.push((name, dir_size(&entry.path())));
    }
    runs.sort_by(|a, b| a.0.cmp(&b.0));
    let total: u64 = runs.iter().map(|(_, s)| *s).sum();
    if total <= max_bytes {
        return Ok((0, 0));
    }
    let mut deleted = 0usize;
    let mut freed = 0u64;
    let mut remaining = total;
    for (idx, (name, sz)) in runs.iter().enumerate() {
        if remaining <= max_bytes {
            break;
        }
        if *name == keep {
            continue; // 本次运行目录不删
        }
        if idx + 1 == runs.len() {
            break; // 始终保留最近一次备份
        }
        fs::remove_dir_all(backup_root.join(name))
            .with_context(|| format!("删除备份目录失败：{}", backup_root.join(name).display()))?;
        deleted += 1;
        freed += *sz;
        remaining -= *sz;
    }
    Ok((deleted, freed))
}

/// 扫描两端。双向模式两端都必须存在；单向模式目标端自动创建。
fn scan_both(
    opts: &Options,
    filters: &Filters,
    cancel: &AtomicBool,
    probe: Option<&ScanProbe>,
) -> Result<(ScanResult, ScanResult, usize)> {
    let require = |p: &Path, side: &str| -> Result<()> {
        if !p.exists() {
            return Err(anyhow!("{}路径不存在：{}（请确认U盘已插入、路径正确）", side, p.display()));
        }
        Ok(())
    };
    match opts.mode {
        Mode::Sync | Mode::Preview => {
            require(&opts.local, "本地")?;
            require(&opts.usb, "U盘")?;
        }
        Mode::Push => {
            require(&opts.local, "本地")?;
            fs::create_dir_all(&opts.usb).with_context(|| format!("创建U盘目标目录失败：{}", opts.usb.display()))?;
        }
        Mode::Pull => {
            require(&opts.usb, "U盘")?;
            fs::create_dir_all(&opts.local).with_context(|| format!("创建本地目标目录失败：{}", opts.local.display()))?;
        }
    }
    if let Some(p) = probe {
        if let Ok(mut cur) = p.cur.lock() {
            *cur = "本地…".into();
        }
    }
    let (a, sa) = scan::scan(&opts.local, filters, cancel, probe)?;
    if let Some(p) = probe {
        if let Ok(mut cur) = p.cur.lock() {
            *cur = "U盘…".into();
        }
    }
    let (b, sb) = scan::scan(&opts.usb, filters, cancel, probe)?;
    Ok((a, b, sa + sb))
}

struct RunStats {
    copied: usize,
    deleted: usize,
    backed_up: usize,
    bytes: u64,
    errors: Vec<(String, String)>,
}

/// 工作线程 → 引擎线程消息
enum Wk {
    Done { rel: String, res: OpResult },
    Err { rel: String, msg: String },
}

fn execute_plan(
    plan: &Plan,
    roots: &Roots,
    backup_root: &Path,
    threads: usize,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<Ev>,
) -> RunStats {
    let mut stats = RunStats { copied: 0, deleted: 0, backed_up: 0, bytes: 0, errors: Vec::new() };
    let (ops_tx, ops_rx) = unbounded::<Op>();
    let (msg_tx, msg_rx) = bounded::<Wk>(128);
    let n_workers = threads.clamp(1, 32).min(plan.ops.len().max(1));

    let mut handles = Vec::with_capacity(n_workers);
    for _ in 0..n_workers {
        let ops_rx = ops_rx.clone();
        let msg_tx = msg_tx.clone();
        let cancel = Arc::clone(cancel);
        let roots = roots.clone();
        let backup_root = backup_root.to_path_buf();
        handles.push(std::thread::spawn(move || {
            for op in ops_rx {
                if cancel.load(Ordering::Relaxed) {
                    break; // 之后的操作不再执行（当前文件若已开始会先完成）
                }
                match perform(&op, &roots, &backup_root) {
                    Ok(res) => {
                        if msg_tx.send(Wk::Done { rel: op.rel.clone(), res }).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let msg = format!("{}：{}", op.kind.describe(""), e);
                        if msg_tx.send(Wk::Err { rel: op.rel.clone(), msg }).is_err() {
                            break;
                        }
                    }
                }
            }
        }));
    }
    drop(msg_tx);
    drop(ops_rx);

    for op in &plan.ops {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let _ = tx.send(Ev::FileStart { rel: op.rel.clone() });
        if ops_tx.send(op.clone()).is_err() {
            break;
        }
    }
    drop(ops_tx);

    for msg in msg_rx {
        match msg {
            Wk::Done { rel, res } => {
                match res.report {
                    OpReport::Copied => {
                        stats.copied += 1;
                        stats.bytes += res.bytes;
                    }
                    OpReport::Deleted => stats.deleted += 1,
                    OpReport::MadeDir => {}
                }
                if res.backed_up {
                    stats.backed_up += 1;
                }
                let _ = tx.send(Ev::FileDone { rel, bytes: res.bytes, report: res.report });
            }
            Wk::Err { rel, msg } => {
                stats.errors.push((rel.clone(), msg.clone()));
                let _ = tx.send(Ev::OpError { rel, msg });
            }
        }
    }
    for h in handles {
        let _ = h.join();
    }
    stats
}

fn perform(op: &Op, roots: &Roots, backup_root: &Path) -> Result<OpResult> {
    match op.kind {
        OpKind::CopyAToB { backup } => copy_op(&roots.local, &roots.usb, &op.rel, backup, backup_root),
        OpKind::CopyBToA { backup } => copy_op(&roots.usb, &roots.local, &op.rel, backup, backup_root),
        OpKind::DeleteA { backup } => delete_op(&roots.local, &op.rel, backup, backup_root),
        OpKind::DeleteB { backup } => delete_op(&roots.usb, &op.rel, backup, backup_root),
        OpKind::MakeDirA => {
            fs::create_dir_all(roots.local.join(&op.rel)).context("创建目录失败")?;
            Ok(OpResult { report: OpReport::MadeDir, bytes: 0, backed_up: false })
        }
        OpKind::MakeDirB => {
            fs::create_dir_all(roots.usb.join(&op.rel)).context("创建目录失败")?;
            Ok(OpResult { report: OpReport::MadeDir, bytes: 0, backed_up: false })
        }
    }
}

fn copy_op(src_root: &Path, dst_root: &Path, rel: &str, backup: bool, backup_root: &Path) -> Result<OpResult> {
    let src = src_root.join(rel);
    let dst = dst_root.join(rel);
    // 目标为只读文件 → 拒绝覆盖（与 rclone 一致；用户设只读通常是有意保护）
    if dst.exists() {
        let md = fs::metadata(&dst)?;
        if md.permissions().readonly() {
            return Err(anyhow!("目标文件为只读，拒绝覆盖：{}", dst.display()));
        }
    }
    if backup && dst.exists() {
        // 覆盖前把旧文件备份（备份失败则中止该操作，绝不无备份覆盖）
        backup_file(dst_root, rel, backup_root).context("覆盖前备份失败")?;
        return Ok(OpResult { report: OpReport::Copied, bytes: copy_preserving_mtime(&src, &dst)?, backed_up: true });
    }
    Ok(OpResult { report: OpReport::Copied, bytes: copy_preserving_mtime(&src, &dst)?, backed_up: false })
}

fn delete_op(root: &Path, rel: &str, backup: bool, backup_root: &Path) -> Result<OpResult> {
    let p = root.join(rel);
    if !p.exists() {
        return Ok(OpResult { report: OpReport::Deleted, bytes: 0, backed_up: false }); // 已不存在，视为完成
    }
    if backup {
        backup_file(root, rel, backup_root).context("删除前备份失败")?;
    }
    fs::remove_file(&p).with_context(|| format!("删除失败：{}", p.display()))?;
    if let Some(parent) = Path::new(rel).parent() {
        prune_empty_dirs(root, parent);
    }
    Ok(OpResult { report: OpReport::Deleted, bytes: 0, backed_up: backup })
}

/// 复制文件并保留源端修改时间（修改时间是增量比对依据，必须保留）。
/// 先写 `.usb-partial` 临时文件，成功后原子改名——中断/崩溃不会在目标位置留下残缺文件。
pub fn copy_preserving_mtime(src: &Path, dst: &Path) -> Result<u64> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建目录失败：{}", parent.display()))?;
    }
    let file_name = dst
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow!("无效的目标路径：{}", dst.display()))?;
    let tmp = dst.with_file_name(format!("{file_name}.usb-partial"));
    let n = fs::copy(src, &tmp).with_context(|| format!("复制 {} → {}", src.display(), dst.display()))?;
    let mtime = fs::metadata(src)
        .and_then(|m| m.modified())
        .map_err(|e| anyhow!("读取修改时间失败：{}", e))?;
    fs::set_times(&tmp, fs::FileTimes::new().set_accessed(mtime).set_modified(mtime))
        .with_context(|| format!("恢复修改时间失败：{}", tmp.display()))?;
    fs::rename(&tmp, dst).with_context(|| format!("落盘失败：{} → {}", tmp.display(), dst.display()))?;
    Ok(n)
}

/// 备份单个文件到 `<backup_root>\<rel>`（保留目录结构，可整体拷回还原）
fn backup_file(root: &Path, rel: &str, backup_root: &Path) -> Result<()> {
    let src = root.join(rel);
    if !src.exists() {
        return Ok(());
    }
    let dst = backup_root.join(rel);
    copy_preserving_mtime(&src, &dst)?;
    Ok(())
}

/// 删除文件后自底向上清理空目录（不越过根）
fn prune_empty_dirs(root: &Path, rel: &Path) {
    let mut cur = rel.to_path_buf();
    loop {
        if cur.as_os_str().is_empty() {
            break;
        }
        let dir = root.join(&cur);
        if !dir.is_dir() {
            break;
        }
        let is_empty = fs::read_dir(&dir).map(|mut it| it.next().is_none()).unwrap_or(false);
        if !is_empty || fs::remove_dir(&dir).is_err() {
            break;
        }
        cur.pop();
    }
}

/// 全部成功后的复核扫描（用于生成准确的新快照）
fn finish_sync(opts: &Options, filters: &Filters, cancel: &AtomicBool) -> Result<(ScanResult, ScanResult)> {
    let dummy = ScanProbe::default();
    let a = scan::scan(&opts.local, filters, cancel, Some(&dummy))?.0;
    let b = scan::scan(&opts.usb, filters, cancel, Some(&dummy))?.0;
    Ok((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_refuses_readonly_target() {
        let tmp = std::env::temp_dir().join(format!("usb-sync-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("a")).unwrap();
        std::fs::create_dir_all(tmp.join("b")).unwrap();
        std::fs::write(tmp.join("a").join("f.txt"), "new").unwrap();
        std::fs::write(tmp.join("b").join("f.txt"), "old").unwrap();
        let md = std::fs::metadata(tmp.join("b").join("f.txt")).unwrap();
        let mut perm = md.permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(tmp.join("b").join("f.txt"), perm).unwrap();

        let roots = Roots { local: tmp.join("a"), usb: tmp.join("b") };
        let op = Op {
            kind: OpKind::CopyAToB { backup: true },
            rel: "f.txt".into(),
            bytes: 3,
            reason: String::new(),
        };
        let backup_root = tmp.join("bk");
        let res = perform(&op, &roots, &backup_root);
        assert!(res.is_err(), "覆盖只读目标必须报错");
        let msg = format!("{}", res.err().expect("应返回错误"));
        assert!(msg.contains("只读"), "错误信息应说明只读：{msg}");
        // 恢复权限供清理
        let mut perm2 = std::fs::metadata(tmp.join("b").join("f.txt")).unwrap().permissions();
        perm2.set_readonly(false);
        std::fs::set_permissions(tmp.join("b").join("f.txt"), perm2).unwrap();
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;

    #[test]
    fn copy_is_atomic_no_partial_left() {
        let tmpd = std::env::temp_dir().join(format!("usb-sync-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmpd);
        std::fs::create_dir_all(tmpd.join("s")).unwrap();
        std::fs::write(tmpd.join("s").join("f.txt"), "content").unwrap();
        let dst = tmpd.join("d").join("f.txt");

        let n = copy_preserving_mtime(&tmpd.join("s").join("f.txt"), &dst).unwrap();
        assert_eq!(n, 7);
        assert_eq!(std::fs::read(&dst).unwrap(), b"content");

        // 目标目录不残留 .usb-partial 临时文件（原子改名完成）
        let leftovers: Vec<_> = std::fs::read_dir(dst.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("usb-partial"))
            .collect();
        assert!(leftovers.is_empty(), "不应残留临时文件");

        // mtime 保留（增量比对依据）
        let sm = std::fs::metadata(tmpd.join("s").join("f.txt")).unwrap().modified().unwrap();
        let dm = std::fs::metadata(&dst).unwrap().modified().unwrap();
        assert_eq!(sm, dm);
        std::fs::remove_dir_all(&tmpd).ok();
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;

    #[test]
    fn retention_deletes_oldest_beyond_cap() {
        let tmp = std::env::temp_dir().join(format!("usb-sync-ret-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("bk");
        let mk = |name: &str, files: &[usize]| {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            for (i, sz) in files.iter().enumerate() {
                std::fs::write(d.join(format!("f{i}.bin")), vec![0u8; *sz]).unwrap();
            }
        };
        mk("20260101_010101", &[600, 600]); // 1200B
        mk("20260102_010101", &[800]); // 800B
        mk("20260103_010101", &[700]); // 700B
        std::fs::write(root.join("notes.txt"), "user file").unwrap();

        // 总量 2700 > 上限 1500 → 删最旧（1200）→ 剩 1500 ≤ 上限
        let (deleted, freed) = enforce_backup_retention(&root, 1500, "").unwrap();
        assert_eq!((deleted, freed), (1, 1200));
        assert!(!root.join("20260101_010101").exists());
        assert!(root.join("20260102_010101").exists());
        assert!(root.join("20260103_010101").exists());
        // 用户自放文件不动
        assert!(root.join("notes.txt").exists());

        // 上限降到 500：剩 1500 仍超 → 删 20260102（800）→ 剩 700 > 500 但最新目录受保护
        let (deleted2, _) = enforce_backup_retention(&root, 500, "").unwrap();
        assert_eq!(deleted2, 1);
        assert!(!root.join("20260102_010101").exists());
        assert!(root.join("20260103_010101").exists(), "最近一次备份永不删除");

        // keep 保护：keep 指向最旧时跳过它
        let (d3, _) = enforce_backup_retention(&root, 10, "20260103_010101").unwrap();
        assert_eq!(d3, 0, "唯一剩余目录即为 keep，不应删除");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn retention_noop_cases() {
        let tmp = std::env::temp_dir().join(format!("usb-sync-ret2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        // 0 上限 = 不限制
        let root = tmp.join("bk");
        std::fs::create_dir_all(root.join("20260101_010101")).unwrap();
        assert_eq!(enforce_backup_retention(&root, 0, "").unwrap(), (0, 0));
        // 根目录不存在
        assert_eq!(enforce_backup_retention(&tmp.join("nope"), 1500, "").unwrap(), (0, 0));
        // 非时间戳目录不清理
        std::fs::create_dir_all(root.join("my-notes")).unwrap();
        std::fs::write(root.join("my-notes").join("a.txt"), b"x").unwrap();
        enforce_backup_retention(&root, 10, "").unwrap();
        assert!(root.join("my-notes").exists(), "非时间戳目录不应被清理");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
