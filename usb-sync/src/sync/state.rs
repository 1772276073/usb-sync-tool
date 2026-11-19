//! 状态快照：记录上次双向同步后两端每文件的大小与修改时间。
//!
//! 存放于 `<usb>\.usb-sync\state.json`（随 U 盘走，换电脑可继续正确同步）。
//! 仅在"双向同步"完整成功后写入；取消或出错时不写（快照必须如实反映已同步状态）。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::scan::ScanResult;

pub const STATE_DIR: &str = ".usb-sync";
pub const STATE_FILE: &str = "state.json";

/// 单文件的"上次同步后"印记
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Stamp {
    pub size: u64,
    pub mtime_a: i64,
    pub mtime_b: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFile {
    pub version: u32,
    /// RFC3339
    pub saved_at: String,
    pub local_root: String,
    pub usb_root: String,
    /// rel → Stamp
    pub files: BTreeMap<String, Stamp>,
}

impl StateFile {
    pub fn state_path(usb: &Path) -> std::path::PathBuf {
        usb.join(STATE_DIR).join(STATE_FILE)
    }

    /// 读取快照；不存在 → None；损坏 → 也按 None 处理（退化为首_run合并语义，数据安全）
    pub fn load(usb: &Path) -> Option<StateFile> {
        let path = Self::state_path(usb);
        let raw = std::fs::read_to_string(path).ok()?;
        match serde_json::from_str(&raw) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("警告：状态文件损坏（{}），将按首次同步合并处理", e);
                None
            }
        }
    }

    /// 从两端实时扫描结果构建快照
    pub fn build(local: &ScanResult, usb_sc: &ScanResult) -> StateFile {
        let mut files = BTreeMap::new();
        for (rel, a) in &local.files {
            let b = usb_sc.files.get(rel);
            files.insert(
                rel.clone(),
                Stamp {
                    size: b.map(|e| e.size).unwrap_or(a.size),
                    mtime_a: a.mtime_ms,
                    mtime_b: b.map(|e| e.mtime_ms).unwrap_or(-1),
                },
            );
        }
        // 仅存在于 B 端的文件
        for (rel, b) in &usb_sc.files {
            if !files.contains_key(rel) {
                files.insert(
                    rel.clone(),
                    Stamp {
                        size: b.size,
                        mtime_a: -1,
                        mtime_b: b.mtime_ms,
                    },
                );
            }
        }
        StateFile {
            version: 1,
            saved_at: chrono::Utc::now().to_rfc3339(),
            local_root: String::new(),
            usb_root: String::new(),
            files,
        }
    }

    /// 原子写入：先写临时文件再改名覆盖
    pub fn save(&self, usb: &Path) -> Result<()> {
        let dir = usb.join(STATE_DIR);
        std::fs::create_dir_all(&dir).context("创建 .usb-sync 目录失败")?;
        let final_path = dir.join(STATE_FILE);
        let tmp_path = dir.join(format!("state.json.tmp-{}", std::process::id()));
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp_path, json).context("写入临时状态文件失败")?;
        // Windows 上 rename 到已存在目标会失败，先移除旧文件
        if final_path.exists() {
            std::fs::remove_file(&final_path).context("移除旧状态文件失败")?;
        }
        std::fs::rename(&tmp_path, &final_path).context("状态文件落盘失败")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::scan::Entry;
    use std::fs;

    fn put(map: &mut ScanResult, rel: &str, size: u64, mtime: i64) {
        map.files.insert(rel.to_string(), Entry { rel: rel.to_string(), size, mtime_ms: mtime });
    }

    #[test]
    fn build_roundtrip() {
        let mut a = ScanResult::default();
        put(&mut a, "x.txt", 10, 1000);
        let mut b = ScanResult::default();
        put(&mut b, "x.txt", 10, 998); // FAT 2s 粒度内的差异
        put(&mut b, "only-b.txt", 5, 2000);

        let st = StateFile::build(&a, &b);
        assert_eq!(st.files["x.txt"].mtime_a, 1000);
        assert_eq!(st.files["x.txt"].mtime_b, 998);
        assert_eq!(st.files["only-b.txt"].mtime_a, -1);

        let dir = std::env::temp_dir().join(format!("usb-sync-test-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        st.save(&dir).unwrap();
        let loaded = StateFile::load(&dir).unwrap();
        assert_eq!(loaded.files, st.files);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_returns_none() {
        assert!(StateFile::load(Path::new("Z:/not/exist")).is_none());
    }
}
