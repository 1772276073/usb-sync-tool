//! 方案预设与界面状态持久化（沿用旧版 %APPDATA%\USB-Sync-Tool\ 目录）
//!
//! profiles.json 与旧版共用：v1 存的中文模式文案可正常读取，v2 保存稳定英文标识。

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::sync::{parse_filter_lines, Mode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub local: String,
    pub usb: String,
    /// v2 稳定标识（sync/push/pull/preview）；v1 为中文文案，读取时自动映射
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<u32>,
    /// 非默认过滤规则（每行一条）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Vec<String>>,
}

impl Profile {
    pub fn mode(&self) -> Mode {
        Mode::parse(&self.mode).unwrap_or(Mode::Sync)
    }

    pub fn from_fields(name: &str, local: &str, usb: &str, mode: Mode, backup_dir: &str, threads: usize, filters: &str) -> Profile {
        Profile {
            name: name.to_string(),
            local: local.to_string(),
            usb: usb.to_string(),
            mode: serde_json::to_value(mode).and_then(serde_json::from_value).unwrap_or("sync".to_string()),
            backup_dir: (!backup_dir.trim().is_empty()).then(|| backup_dir.trim().to_string()),
            threads: Some(threads as u32),
            filters: {
                let rules = parse_filter_lines(filters);
                (!rules.is_empty()).then_some(rules)
            },
        }
    }
}

fn appdata_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("USB-Sync-Tool")
}

pub fn profiles_path() -> PathBuf {
    appdata_dir().join("profiles.json")
}

pub fn load_profiles() -> Vec<Profile> {
    let Ok(raw) = std::fs::read_to_string(profiles_path()) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("警告：profiles.json 解析失败（{}），忽略已有方案", e);
        Vec::new()
    })
}

pub fn save_profiles(profiles: &[Profile]) -> Result<()> {
    let dir = appdata_dir();
    std::fs::create_dir_all(&dir).context("创建配置目录失败")?;
    let json = serde_json::to_string_pretty(profiles)?;
    // 原子写：临时文件 + 改名，防止写入中断损坏配置
    let tmp = dir.join("profiles.json.tmp");
    std::fs::write(&tmp, json)?;
    let final_path = profiles_path();
    if final_path.exists() {
        std::fs::remove_file(&final_path)?;
    }
    std::fs::rename(&tmp, final_path)?;
    Ok(())
}

/// 界面记忆（上次使用的路径与设置，与 profiles 分离）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiState {
    #[serde(default)]
    pub local: String,
    #[serde(default)]
    pub usb: String,
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub backup_dir: String,
    /// 备份总量上限（GB），None = 未设置（默认 3），0 = 不限制
    #[serde(default)]
    pub backup_max_gb: Option<u64>,
    #[serde(default)]
    pub threads: usize,
    #[serde(default)]
    pub filters: String,
    #[serde(default)]
    pub selected_profile: Option<String>,
    /// 界面动效开关（None = 未设置，默认开启）
    #[serde(default)]
    pub animations: Option<bool>,
}

pub fn ui_state_path() -> PathBuf {
    appdata_dir().join("ui-state.json")
}

pub fn load_ui_state() -> Option<UiState> {
    let raw = std::fs::read_to_string(ui_state_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save_ui_state(state: &UiState) -> Result<()> {
    let dir = appdata_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(ui_state_path(), json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parse_v1_chinese_labels() {
        assert_eq!(Mode::parse("双向同步"), Some(Mode::Sync));
        assert_eq!(Mode::parse("推送到U盘"), Some(Mode::Push));
        assert_eq!(Mode::parse("从U盘拉取"), Some(Mode::Pull));
        assert_eq!(Mode::parse("预览变更"), Some(Mode::Preview));
        assert_eq!(Mode::parse("sync"), Some(Mode::Sync));
        assert_eq!(Mode::parse("bogus"), None);
    }

    #[test]
    fn profile_serializes_stable_mode() {
        let p = Profile::from_fields("工作", "F:\\A", "G:\\B", Mode::Push, "", 8, "# 无自定义");
        assert_eq!(p.mode, "push");
        assert_eq!(p.mode(), Mode::Push);
        assert!(p.backup_dir.is_none());
        assert!(p.filters.is_none());
    }

    #[test]
    fn v1_json_backcompat() {
        let v1 = r#"[{"name":"文档","local":"D:\\Docs","usb":"G:\\Docs","mode":"双向同步"}]"#;
        let list: Vec<Profile> = serde_json::from_str(v1).unwrap();
        assert_eq!(list[0].mode(), Mode::Sync);
        assert!(list[0].backup_dir.is_none());
    }
}
