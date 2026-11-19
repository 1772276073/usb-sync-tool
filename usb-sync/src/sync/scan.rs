//! 并行目录扫描（jwalk 多线程遍历 + 过滤剪枝 + 实时进度探针 + 可取消）

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use anyhow::{bail, Result};

use super::filters::Filters;
use super::progress::ScanProbe;

/// 扫描被用户取消
#[derive(Debug, thiserror::Error)]
#[error("扫描已取消")]
pub struct ScanCancelled;

/// 单个文件的扫描记录
#[derive(Debug, Clone)]
pub struct Entry {
    /// 相对路径，`/` 分隔
    pub rel: String,
    pub size: u64,
    /// 毫秒级时间戳（用于比对；FAT 2s 粒度由比对窗口吸收）
    pub mtime_ms: i64,
}

/// 一端目录树的扫描结果
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub files: BTreeMap<String, Entry>,
    /// 相对路径的目录集合（不含根；用于空目录传播）
    pub dirs: BTreeSet<String>,
}

pub fn mtime_to_ms(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 扫描 `root`（不递归跟随符号链接），返回文件与目录集合。
///
/// 过滤规则命中的目录在遍历时直接剪枝；不可读条目跳过并计数。
/// `cancel` 置位时提前返回 `ScanCancelled`；`probe` 用于向界面汇报实时扫描进度。
/// 返回 `(结果, 跳过的不可读条目数)`。
pub fn scan(
    root: &Path,
    filters: &Filters,
    cancel: &AtomicBool,
    probe: Option<&ScanProbe>,
) -> Result<(ScanResult, usize)> {
    if !root.exists() {
        bail!("路径不存在：{}", root.display());
    }
    let mut out = ScanResult::default();
    let mut skipped = 0usize;
    let mut checked_since_cancel = 0usize;

    let walker = jwalk::WalkDir::new(root)
        .follow_links(false)
        .skip_hidden(false)
        .process_read_dir({
            let root = root.to_path_buf();
            let filters = filters.clone(); // 闭包要求 'static
            move |_depth, _path, _state, children| {
                // 目录命中排除规则 → 从结果中移除，jwalk 不会进入其子树
                children.retain(|entry| match entry {
                    Ok(e) => match e.path().strip_prefix(&root).ok().and_then(|p| p.to_str()) {
                        Some(rel) => {
                            let rel = rel.replace('\\', "/");
                            let name = e.file_name.to_string_lossy();
                            !if e.file_type.is_dir() {
                                filters.excluded_dir(&rel, &name)
                            } else {
                                filters.excluded_file(&rel)
                            }
                        }
                        None => false, // 非合法 UTF-8 路径：不同步
                    },
                    Err(_) => false,
                });
            }
        });

    for entry in walker {
        // 取消检查（每 64 个条目一次，负载可忽略）
        checked_since_cancel += 1;
        if checked_since_cancel >= 64 {
            checked_since_cancel = 0;
            if cancel.load(Ordering::Relaxed) {
                bail!(ScanCancelled);
            }
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let full = entry.path();
        let rel_os = match full.strip_prefix(root) {
            Ok(p) => p,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let Some(rel) = rel_os.to_str() else {
            skipped += 1;
            continue;
        };
        let rel = rel.replace('\\', "/");
        if rel.is_empty() {
            continue; // 根自身
        }
        let ft = entry.file_type;
        if ft.is_dir() {
            out.dirs.insert(rel);
            continue;
        }
        if ft.is_symlink() {
            skipped += 1;
            continue; // 符号链接不同步（避免逃逸/循环）
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        // 实时进度探针
        if let Some(p) = probe {
            p.files.fetch_add(1, Ordering::Relaxed);
            p.bytes.fetch_add(meta.len(), Ordering::Relaxed);
            if let Ok(mut cur) = p.cur.lock() {
                *cur = rel.clone();
            }
        }
        out.files.insert(
            rel.clone(),
            Entry {
                rel,
                size: meta.len(),
                mtime_ms: meta.modified().map(mtime_to_ms).unwrap_or(0),
            },
        );
    }

    Ok((out, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("usb-sync-test-scan-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn scan_collects_files_and_dirs_and_prunes() {
        let d = tmpdir("basic");
        fs::write(d.join("a.txt"), "hello").unwrap();
        fs::create_dir_all(d.join("sub/deep")).unwrap();
        fs::write(d.join("sub/deep/b.txt"), "x").unwrap();
        fs::create_dir_all(d.join("node_modules/pkg")).unwrap();
        fs::write(d.join("node_modules/pkg/c.js"), "y").unwrap();
        fs::write(d.join("junk.tmp"), "z").unwrap();

        let flt = Filters::with_defaults(&["node_modules".to_string()]);
        let cancel = AtomicBool::new(false);
        let (res, skipped) = scan(&d, &flt, &cancel, None).unwrap();
        assert_eq!(skipped, 0);
        assert!(res.files.contains_key("a.txt"));
        assert!(res.files.contains_key("sub/deep/b.txt"));
        assert!(!res.files.contains_key("junk.tmp"));
        assert!(!res.files.keys().any(|k| k.contains("node_modules")));
        assert!(res.dirs.contains("sub/deep"));
        assert!(!res.dirs.contains("node_modules"));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn missing_root_errors() {
        let flt = Filters::with_defaults(&[]);
        let cancel = AtomicBool::new(false);
        let r = scan(Path::new("Z:/definitely/not/here"), &flt, &cancel, None);
        assert!(r.is_err());
    }
}
