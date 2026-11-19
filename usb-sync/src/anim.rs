//! 界面动效助手：统一时长、缓动与"减弱动效"降级开关
//!
//! 原则（源自 modern-web-guidance: motion/progress/spinner）：
//! - 只动便宜的属性（颜色/位移/透明度），不动布局
//! - 减弱动效时：放慢或禁用位移类动画、保留淡入淡出，且动画结束后必须停在最终状态
//! - 闪烁频率不超过 3 次/秒

use std::f32::consts::TAU;
use std::time::Instant;

use egui::{Color32, Ui};

/// 动画时长档位（秒）
pub const FAST: f32 = 0.15;
pub const NORMAL: f32 = 0.3;

/// 会话级动效上下文（窗口启动时间 + 全局开关）
pub struct Anim {
    pub enabled: bool,
    started: Instant,
}

impl Anim {
    pub fn new(enabled: bool) -> Anim {
        Anim { enabled, started: Instant::now() }
    }

    pub fn time(&self, ui: &Ui) -> f32 {
        ui.input(|i| i.time as f32)
    }

    /// 入场淡入系数 0→1（smoothstep），关闭动效时恒为 1
    pub fn entrance(&self) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        let t = (self.started.elapsed().as_secs_f32() / 0.45).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// 运行状态点的柔和脉动 0→1（约 0.9 次/秒，远低于闪烁阈值）
    pub fn pulse(&self, ui: &Ui) -> f32 {
        if !self.enabled {
            return 0.6;
        }
        0.5 + 0.5 * (self.time(ui) * TAU * 0.9).sin()
    }

    /// 不定进度滑块位置 0→1（减弱动效 → 放慢 4 倍而非停止，保持"仍在工作"语义）
    pub fn indeterminate(&self, ui: &Ui, loops_per_sec: f32) -> f32 {
        let s = if self.enabled { loops_per_sec } else { loops_per_sec * 0.25 };
        (self.time(ui) * s) % 1.0
    }

    /// 旋转指示器角度（弧度）；减弱动效 → 显著放慢（spinner 指南：放慢而非停止）
    pub fn spin(&self, ui: &Ui, period_secs: f32) -> f32 {
        let p = if self.enabled { period_secs } else { period_secs * 4.0 };
        (self.time(ui) / p) % 1.0 * TAU
    }
}

/// 两色插值（gamma 空间足够 UI 微交互）
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()), ch(a.a(), b.a()))
}

/// 圆弧描边（等价 spinner 的 conic-gradient 轨迹），采样折线实现以兼容各 egui 版本
pub fn draw_arc(painter: &egui::Painter, center: egui::Pos2, radius: f32, start: f32, end: f32, stroke: egui::Stroke) {
    let n = 28;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let a = start + (end - start) * (i as f32 / n as f32);
        pts.push(center + egui::vec2(radius * a.cos(), radius * a.sin()));
    }
    painter.add(egui::Shape::line(pts, stroke));
}
