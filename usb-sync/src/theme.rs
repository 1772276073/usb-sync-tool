//! Bisecto 风格深色主题：深海军蓝底 + 紫罗兰强调（墨字压色）+ 青绿副色，大圆角细线
//!
//! 配色取自 bisecto.com 暗色主题变量：
//! bg #101522 / surface #19202f / surface-strong #222a3b / accent #9184ff
//! accent-ink #111522 / accent-2 #57d2c8 / danger #ff5c5c / text #f2f4f8 / line #ffffff1f

use std::fs;

use egui::{Color32, Context, FontData, FontDefinitions, FontFamily, Style};

// ---- Bisecto 调色板（dark-mode 变量）----
pub const BG: Color32 = Color32::from_rgb(0x10, 0x15, 0x22); // 页面背景 #101522
pub const PANEL: Color32 = Color32::from_rgb(0x19, 0x20, 0x2F); // 卡片 #19202f
pub const PANEL_HOVER: Color32 = Color32::from_rgb(0x26, 0x30, 0x4A); // 悬停面
pub const SURFACE_STRONG: Color32 = Color32::from_rgb(0x22, 0x2A, 0x3B); // 内层/输入 #222a3b
pub const INPUT_BG: Color32 = SURFACE_STRONG;
pub const LOG_BG: Color32 = Color32::from_rgb(0x0D, 0x12, 0x20); // 日志内层（更深）
/// 分隔线：等效 12% 白叠在深底上的不透明色（原值 #ffffff1f 非常量构造，无法用于 const）
pub const BORDER: Color32 = Color32::from_rgb(0x2B, 0x31, 0x44);
pub const TEXT: Color32 = Color32::from_rgb(0xF2, 0xF4, 0xF8); // #f2f4f8
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9A, 0xA3, 0xB5);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x6B, 0x74, 0x88);
pub const TITLE: Color32 = Color32::from_rgb(0xF2, 0xF4, 0xF8);

pub const ACCENT: Color32 = Color32::from_rgb(0x91, 0x84, 0xFF); // #9184ff 紫罗兰
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xA7, 0x9B, 0xFF);
/// 压在强调色上的墨色文字（bisecto 的标志性做法）
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x11, 0x15, 0x22);
/// 青绿副色：进度/成功/运行态
pub const TEAL: Color32 = Color32::from_rgb(0x57, 0xD2, 0xC8); // #57d2c8

pub const SUCCESS: Color32 = TEAL;
pub const WARNING: Color32 = Color32::from_rgb(0xFF, 0xD1, 0x66);
pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x5C); // #ff5c5c
pub const INFO: Color32 = Color32::from_rgb(0x8F, 0xB3, 0xFF);

/// Windows 系统中文字体候选（按优先级）
const CJK_FONT_CANDIDATES: &[&str] = &[
    "C:/Windows/Fonts/msyh.ttc",   // 微软雅黑
    "C:/Windows/Fonts/msyhl.ttc",  // 微软雅黑 Light
    "C:/Windows/Fonts/simhei.ttf", // 黑体
    "C:/Windows/Fonts/simsun.ttc", // 宋体
];

/// 应用主题与字体。必须在首次绘制前调用（App 创建时）。
pub fn apply(ctx: &Context) {
    apply_fonts(ctx);
    let s = style();
    ctx.all_styles_mut(|st| *st = s.clone());
}

fn style() -> Style {
    let mut s = Style::default();
    let v = &mut s.visuals;
    v.dark_mode = true;
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = SURFACE_STRONG; // 输入框背景
    v.faint_bg_color = PANEL;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);

    let widget = &mut v.widgets;
    widget.noninteractive.bg_fill = PANEL;
    widget.noninteractive.weak_bg_fill = PANEL;
    widget.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    widget.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    widget.inactive.bg_fill = SURFACE_STRONG;
    widget.inactive.weak_bg_fill = SURFACE_STRONG;
    widget.inactive.bg_stroke = egui::Stroke::NONE;
    widget.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    widget.hovered.bg_fill = PANEL_HOVER;
    widget.hovered.weak_bg_fill = PANEL_HOVER;
    widget.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.8));
    widget.hovered.fg_stroke = egui::Stroke::new(1.0, TITLE);
    widget.active.bg_fill = ACCENT.gamma_multiply(0.55);
    widget.active.weak_bg_fill = ACCENT.gamma_multiply(0.55);
    widget.active.fg_stroke = egui::Stroke::new(1.0, TITLE);

    v.window_stroke = egui::Stroke::new(1.0, BORDER);
    v.window_corner_radius = egui::CornerRadius::same(12);
    v.menu_corner_radius = egui::CornerRadius::same(10);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        w.corner_radius = egui::CornerRadius::same(8);
    }

    s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(19.0));
    s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    s.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::proportional(13.0));
    s
}

fn apply_fonts(ctx: &Context) {
    for path in CJK_FONT_CANDIDATES {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert("cjk".into(), FontData::from_owned(bytes).into());
        // CJK 字体放最前：中英文统一由雅黑渲染，视觉一致
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.get_mut(&family).unwrap().insert(0, "cjk".into());
        }
        ctx.set_fonts(fonts);
        return;
    }
    // 找不到系统中文字体（理论上 Win10/11 不会）：保留默认字体，中文会显示为方块
    eprintln!("警告：未找到系统中文字体，中文可能无法显示");
}
