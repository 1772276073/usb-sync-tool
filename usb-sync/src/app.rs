//! 图形界面：egui 实现，Ant Design 深色主题

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;
use egui::{vec2, Align2, Color32, ComboBox, FontId, RichText, ScrollArea, Sense, TextEdit};

use crate::anim::{self, Anim};
use crate::log::{Level, LogBuf};
use crate::profile::{self, Profile, UiState};
use crate::sync::exec;
use crate::sync::filters::DEFAULT_FILTERS;
use crate::sync::progress::{Ev, Phase, SpeedMeter};
use crate::sync::{parse_filter_lines, Mode, Options, SyncOutcome};
use crate::theme;

const VERSION: &str = "2.1";

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Status {
    Ready,
    Running(Phase),
    Success(String),
    Cancelled(String),
    Failed(String),
}

impl Status {
    fn running(&self) -> bool {
        matches!(self, Status::Running(_))
    }
    fn label(&self) -> String {
        match self {
            Status::Ready => "就绪".into(),
            Status::Running(p) => format!("{}…", p.label()),
            Status::Success(s) => s.clone(),
            Status::Cancelled(s) => s.clone(),
            Status::Failed(s) => s.clone(),
        }
    }
    fn color(&self) -> Color32 {
        match self {
            Status::Ready => theme::TEXT_DIM,
            Status::Running(_) => theme::TEAL,
            Status::Success(_) => theme::TEAL,
            Status::Cancelled(_) => theme::WARNING,
            Status::Failed(_) => theme::ERROR,
        }
    }
}

/// 一次正在进行的同步运行
struct Run {
    rx: Receiver<Ev>,
    cancel: Arc<AtomicBool>,
    /// Option：join() 按值消费，事件循环里用 take() 取出
    handle: Option<std::thread::JoinHandle<SyncOutcome>>,
    phase: Phase,
    plan_copies: usize,
    plan_bytes: u64,
    plan_ops: usize,
    /// 扫描实时计数（ScanTick 事件更新）
    scan_files: u64,
    scan_bytes: u64,
    scan_cur: String,
    done_files: usize,
    done_bytes: u64,
    last_done: String,
    speed: SpeedMeter,
    started: Instant,
}

impl Run {
    fn fraction_bytes(&self) -> f32 {
        if self.plan_bytes == 0 {
            return 0.0;
        }
        (self.done_bytes as f32 / self.plan_bytes as f32).clamp(0.0, 1.0)
    }
}

pub struct App {
    local: String,
    usb: String,
    mode: Mode,
    profiles: Vec<Profile>,
    selected: Option<usize>,
    /// 已把哪个方案应用到输入框（只在新选中时应用一次，避免覆盖手动编辑）
    applied: Option<usize>,
    save_dlg_open: bool,
    /// 窗口打开时刻（入场动画用）
    save_dlg_at: Instant,
    save_dlg_name: String,
    confirm_delete: Option<usize>,
    confirm_at: Instant,
    backup_dir: String,
    /// 备份总量软上限（GB），0 = 不限制
    backup_max_gb: u64,
    threads: usize,
    filters_text: String,
    /// 界面动效总开关（motion 指南的 reduced-motion 等价物）
    animations: bool,
    anim: Anim,
    log: LogBuf,
    status: Status,
    run: Option<Run>,
    session_log: Option<PathBuf>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.anim.enabled = self.animations;
        self.pump_events();
        let ctx = ui.ctx().clone();
        if self.status.running() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        // 内容不足一屏时也撑满高度，避免底部露出默认黑底
        ui.set_min_height(ui.available_height());
        // 两侧留白（用户反馈：太挤）
        egui::Frame::default()
            .fill(theme::BG)
            .inner_margin(egui::Margin { left: 18, right: 18, top: 8, bottom: 12 })
            .show(ui, |ui| {
                self.draw(ui);
            });
        self.draw_save_dialog(&ctx);
        self.draw_delete_confirm(&ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self) {
        // 界面记忆尽力保存
        let st = UiState {
            local: self.local.clone(),
            usb: self.usb.clone(),
            mode: Some(self.mode),
            backup_dir: self.backup_dir.clone(),
            threads: self.threads,
            filters: self.filters_text.clone(),
            selected_profile: self.selected.map(|i| self.profiles[i].name.clone()),
            backup_max_gb: Some(self.backup_max_gb),
            animations: Some(self.animations),
        };
        profile::save_ui_state(&st).ok();
    }
}

impl App {
    pub fn new() -> Self {
        let saved = profile::load_ui_state();
        let profiles = profile::load_profiles();
        let selected_name = saved.as_ref().and_then(|s| s.selected_profile.clone());
        let selected = selected_name
            .and_then(|n| profiles.iter().position(|p| p.name == n))
            .or(if profiles.is_empty() { None } else { Some(0) });

        let mut app = App {
            local: saved.as_ref().map(|s| s.local.clone()).unwrap_or_default(),
            usb: saved.as_ref().map(|s| s.usb.clone()).unwrap_or_default(),
            mode: saved.as_ref().and_then(|s| s.mode).unwrap_or(Mode::Sync),
            profiles,
            selected,
            applied: selected,
            save_dlg_open: false,
            save_dlg_at: Instant::now(),
            save_dlg_name: String::new(),
            confirm_delete: None,
            confirm_at: Instant::now(),
            backup_dir: saved.as_ref().map(|s| s.backup_dir.clone()).unwrap_or_default(),
            backup_max_gb: saved.as_ref().and_then(|s| s.backup_max_gb).unwrap_or(3),
            threads: saved.as_ref().map(|s| s.threads).filter(|t| (1..=32).contains(t)).unwrap_or(8),
            animations: saved.as_ref().and_then(|s| s.animations).unwrap_or(true),
            anim: Anim::new(saved.as_ref().and_then(|s| s.animations).unwrap_or(true)),
            filters_text: saved
                .as_ref()
                .map(|s| s.filters.clone())
                .unwrap_or_else(|| DEFAULT_FILTERS.join("\n")),
            log: LogBuf::default(),
            status: Status::Ready,
            run: None,
            session_log: None,
        };
        if app.local.is_empty() && app.usb.is_empty() {
            // 默认示例路径
            app.local = String::from("D:\\synced");
            app.usb = String::from("G:\\synced");
        }
        app
    }

    fn log(&mut self, level: Level, msg: impl Into<String>) {
        self.log.push(level, &msg.into());
    }

    // -----------------------------------------------------------------------
    // 事件泵
    // -----------------------------------------------------------------------

    fn pump_events(&mut self) {
        // borrow 策略：run 被可变借用期间不能调用 self.log —— 先把日志攒下来，循环结束后统一写入
        let mut pending_logs: Vec<(Level, String)> = Vec::new();
        let mut finished: Option<(SyncOutcome, Duration)> = None;
        if let Some(run) = &mut self.run {
            while let Ok(ev) = run.rx.try_recv() {
                match ev {
                    Ev::Phase(p) => {
                        run.phase = p;
                        if p == Phase::Transferring {
                            pending_logs.push((Level::Info, "开始传输…".into()));
                        }
                    }
                    Ev::PlanReady { files, bytes, ops, unchanged } => {
                        run.plan_copies = files;
                        run.plan_bytes = bytes;
                        run.plan_ops = ops;
                        if ops == 0 {
                            pending_logs.push((Level::Success, format!("两端一致，无需变更（比对 {unchanged} 项）")));
                        } else {
                            pending_logs.push((
                                Level::Info,
                                format!(
                                    "计划：复制 {files} 项（{}），删除/其他 {} 项，无需变更 {unchanged} 项",
                                    fmt_bytes(bytes),
                                    ops.saturating_sub(files)
                                ),
                            ));
                        }
                    }
                    Ev::FileStart { .. } => {}
                    Ev::FileDone { rel, bytes, .. } => {
                        run.done_files += 1;
                        run.done_bytes += bytes;
                        run.last_done = rel;
                        run.speed.mark(run.done_bytes);
                    }
                    Ev::OpError { rel, msg } => {
                        if rel.is_empty() {
                            pending_logs.push((Level::Error, msg));
                        } else {
                            pending_logs.push((Level::Error, format!("{rel}：{msg}")));
                        }
                    }
                    Ev::ScanTick { files, bytes, dir } => {
                        run.scan_files = files;
                        run.scan_bytes = bytes;
                        run.scan_cur = dir;
                    }
                    Ev::Notice(m) => pending_logs.push((Level::Warn, m)),
                    Ev::PreviewOp { desc, .. } => pending_logs.push((Level::Dim, desc)),
                    Ev::Done => {
                        let elapsed = run.started.elapsed();
                        let outcome = run
                            .handle
                            .take()
                            .map(|h| h.join().unwrap_or_default())
                            .unwrap_or_default();
                        finished = Some((outcome, elapsed));
                    }
                }
            }
        }
        for (level, msg) in pending_logs {
            self.log(level, msg);
        }
        if let Some((outcome, elapsed)) = finished {
            self.finish(outcome, elapsed);
        }
    }

    fn finish(&mut self, outcome: SyncOutcome, elapsed: Duration) {
        self.run = None;
        let t = fmt_mmss(elapsed);
        if outcome.dry_run {
            self.status = Status::Success(format!("预览完成（{} 项操作）", outcome.planned_ops));
            self.log(Level::Success, format!("预览完成：共 {} 项操作，未做任何更改（用时 {t}）", outcome.planned_ops));
        } else if outcome.cancelled && !outcome.errors.is_empty() {
            self.status = Status::Failed(format!("已取消，{} 项出错", outcome.errors.len()));
            self.log(Level::Warn, format!("已取消（用时 {t}），但有 {} 项操作失败", outcome.errors.len()));
        } else if outcome.cancelled {
            self.status = Status::Cancelled(format!("已取消（{t}）"));
            self.log(
                Level::Warn,
                format!("已取消：完成复制 {}、删除 {}（用时 {t}）", outcome.copied, outcome.deleted),
            );
        } else if !outcome.errors.is_empty() {
            self.status = Status::Failed(format!("完成，{} 项出错", outcome.errors.len()));
            self.log(
                Level::Error,
                format!(
                    "同步结束（用时 {t}）：复制 {}、删除 {}，其中 {} 项失败（详见上方日志）",
                    outcome.copied,
                    outcome.deleted,
                    outcome.errors.len()
                ),
            );
        } else {
            self.status = Status::Success(format!("同步完成（{t}）"));
            self.log(
                Level::Success,
                format!(
                    "同步完成：复制 {}（{}）、删除 {}、备份 {}，用时 {t}",
                    outcome.copied,
                    fmt_bytes(outcome.bytes_copied),
                    outcome.deleted,
                    outcome.backed_up
                ),
            );
        }
    }

    // -----------------------------------------------------------------------
    // 界面
    // -----------------------------------------------------------------------

    fn draw(&mut self, ui: &mut egui::Ui) {
        // 不透明底：防止窗口透出桌面内容
        let full = ui.max_rect();
        ui.painter().rect_filled(full, egui::CornerRadius::same(0), theme::BG);
        ui.add_space(6.0);

        // —— 路径卡片 ——
        let mut browse_local = false;
        let mut browse_usb = false;
        egui::Frame::default()
            .fill(theme::PANEL)
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(12))
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                browse_local = path_row(ui, "本地", &mut self.local, "D:\\synced");
                hint_line(ui, self.local_hint());
                ui.add_space(10.0);
                browse_usb = path_row(ui, "U盘", &mut self.usb, "G:\\synced");
                hint_line(ui, self.usb_hint());
            });
        if browse_local {
            if let Some(p) = pick_folder(&self.local) {
                self.local = p;
            }
        }
        if browse_usb {
            if let Some(p) = pick_folder(&self.usb) {
                self.usb = p;
            }
        }
        ui.add_space(10.0);

        // —— 同步模式（容器药丸 + 滑动指示块 0.15s）——
        ui.horizontal(|ui| {
            let n = Mode::ALL.len() as f32;
            let gap = 6.0;
            let pad = 3.0;
            let total = ui.available_width();
            let h = 34.0;
            let (row, _) = ui.allocate_exact_size(vec2(total, h), Sense::click());
            let painter = ui.painter();
            painter.rect_filled(row, egui::CornerRadius::same(10), theme::SURFACE_STRONG);
            let sel = self.mode as usize;
            let ctx = ui.ctx().clone();
            let w = (total - pad * 2.0 - gap * (n - 1.0)) / n;
            let ind_x = ctx.animate_value_with_time(egui::Id::unique("mode-seg-indicator"), sel as f32 * (w + gap), anim::FAST);
            // 指示块
            let r2 = egui::Rect::from_min_size(row.min + vec2(pad + ind_x, pad), vec2(w, h - pad * 2.0));
            painter.rect_filled(r2, egui::CornerRadius::same(8), theme::ACCENT);
            for (i, m) in Mode::ALL.iter().enumerate() {
                let r = egui::Rect::from_min_size(row.min + vec2(pad + i as f32 * (w + gap), pad), vec2(w, h - pad * 2.0));
                if i == sel {
                    painter.text(r.center(), Align2::CENTER_CENTER, m.label(), FontId::proportional(13.5), theme::ACCENT_INK);
                } else {
                    let hresp = ui.interact(r, egui::Id::unique(("mode-seg-h", i)), Sense::hover());
                    let h = ctx.animate_value_with_time(egui::Id::unique(("mode-seg-hv", i)), if hresp.hovered() { 1.0 } else { 0.0 }, anim::FAST);
                    let c = r.center();
                    if h > 0.01 {
                        painter.rect_filled(r, egui::CornerRadius::same(8), theme::PANEL_HOVER.gamma_multiply(h * 0.8));
                    }
                    painter.text(c, Align2::CENTER_CENTER, m.label(), FontId::proportional(13.5), anim::mix(theme::TEXT_DIM, theme::TEXT, h));
                    let cresp = ui.interact(r, egui::Id::unique(("mode-seg-c", i)), Sense::click());
                    if cresp.clicked() {
                        self.mode = *m;
                    }
                }
            }
        });
        ui.add_space(14.0);

        // —— 方案行 ——
        self.draw_profile_row(ui);
        ui.add_space(12.0);

        // 高级设置（CollapsingHeader 自带展开/收起动画）
        egui::CollapsingHeader::new(RichText::new("高级设置").size(13.0).color(theme::TEXT_DIM))
            .id_salt("advanced-settings")
            .show(ui, |ui| {
                self.draw_advanced(ui);
            });
        ui.add_space(6.0);

        // —— 进度卡片（仅运行中显示）——
        if self.run.is_some() {
            self.draw_progress(ui);
            ui.add_space(8.0);
        }

        // —— 日志卡片（占满剩余空间，给底栏预留 54px）——
        self.draw_log(ui, self.status.running());
        ui.add_space(8.0);

        // —— 底部按钮 ——
        self.draw_actions(ui);

        // 入场淡入：首帧到 0.45s 间盖一层渐隐的底色幕布
        let entrance = self.anim.entrance();
        if entrance < 1.0 {
            let max = ui.max_rect();
            ui.painter().rect_filled(max, egui::CornerRadius::same(0), theme::BG.gamma_multiply(1.0 - entrance));
        }
    }

    fn local_hint(&self) -> (String, Color32) {
        if self.local.trim().is_empty() {
            return ("本地路径未填写".into(), theme::TEXT_FAINT);
        }
        if Path::new(self.local.trim()).exists() {
            ("✓ 路径就绪".into(), theme::TEAL)
        } else {
            ("✗ 本地路径不存在".into(), theme::ERROR)
        }
    }

    fn usb_hint(&self) -> (String, Color32) {
        if self.usb.trim().is_empty() {
            return ("U盘路径未填写".into(), theme::TEXT_FAINT);
        }
        let (drive_ok, path_ok) = usb_status(self.usb.trim());
        match (drive_ok, path_ok) {
            (false, _) => ("✗ U盘未连接".into(), theme::ERROR),
            (true, false) => ("✗ U盘路径不存在".into(), theme::WARNING),
            (true, true) => ("✓ U盘已连接".into(), theme::TEAL),
        }
    }

    fn draw_profile_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (lrect, _) = ui.allocate_exact_size(vec2(62.0, 26.0), Sense::hover());
            ui.painter().text(
                lrect.left_center(),
                Align2::LEFT_CENTER,
                "方案",
                FontId::proportional(13.0),
                theme::TEXT_DIM,
            );
            let sel_name = self
                .selected
                .and_then(|i| self.profiles.get(i))
                .map(|p| format!("{}（{}）", p.name, p.mode().label()))
                .unwrap_or_else(|| "— 选择方案 —".into());
            ComboBox::from_id_salt("profile-select")
                .selected_text(sel_name)
                .height(30.0)
                .width(ui.available_width().min(240.0))
                .show_ui(ui, |ui| {
                    if self.profiles.is_empty() {
                        ui.label(RichText::new("（暂无方案）").color(theme::TEXT_DIM));
                    }
                    for (i, p) in self.profiles.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, Some(i), format!("{}（{}）", p.name, p.mode().label()));
                    }
                });
            if self.profiles.is_empty() {
                self.selected = None;
            }
            if std_button(ui, "保存方案", 82.0, 28.0, false, true).clicked() {
                self.save_dlg_open = true;
                self.save_dlg_at = Instant::now();
                self.save_dlg_name = self
                    .selected
                    .and_then(|i| self.profiles.get(i))
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
            }
            if std_button(ui, "删除", 52.0, 28.0, false, self.selected.is_some()).clicked() {
                self.confirm_delete = self.selected;
                self.confirm_at = Instant::now();
            }
            // 运行状态置于方案行最右侧（用户指定）
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let color = self.status.color();
                ui.label(RichText::new(self.status.label()).color(color).size(13.0));
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 12.0), Sense::hover());
                let c = rect.center();
                if self.status.running() {
                    // 运行中：柔和脉动光晕（≈0.9 次/秒，远低于 3 次/秒闪烁阈值）
                    let t = self.anim.pulse(ui);
                    ui.painter().circle_filled(c, 4.0 + 3.0 * t, color.gamma_multiply(0.30 + 0.25 * t));
                    ui.painter().circle_filled(c, 4.0, color);
                } else {
                    ui.painter().circle_filled(c, 4.5, color);
                }
            });
        });

        // 新选中的方案 → 一次性填入输入框（避免每帧覆盖手动编辑）
        if self.selected != self.applied {
            self.applied = self.selected;
            if let Some(p) = self.selected.and_then(|i| self.profiles.get(i)) {
                self.local = p.local.clone();
                self.usb = p.usb.clone();
                self.mode = p.mode();
                self.backup_dir = p.backup_dir.clone().unwrap_or_default();
                self.filters_text = p
                    .filters
                    .clone()
                    .unwrap_or_else(|| DEFAULT_FILTERS.iter().map(|s| s.to_string()).collect())
                    .join("\n");
            }
        }
    }

    fn draw_advanced(&mut self, ui: &mut egui::Ui) {
        egui::Frame::default()
            .fill(theme::PANEL)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("备份目录").color(theme::TEXT_DIM).size(13.0));
                    TextEdit::singleline(&mut self.backup_dir)
                        .desired_width(ui.available_width() * 0.62)
                        .hint_text("留空 = <U盘>\\.sync-backup")
                        .show(ui);
                    ui.add_space(12.0);
                    ui.label(RichText::new("线程").color(theme::TEXT_DIM).size(13.0));
                    ui.add(egui::Slider::new(&mut self.threads, 1..=16).show_value(true));
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("备份总量上限").color(theme::TEXT_DIM).size(13.0));
                    ui.add(egui::Slider::new(&mut self.backup_max_gb, 0..=50).text("GB"));
                    ui.label(RichText::new("超过后自动从最旧的备份开始清理；0 = 不限制").color(theme::TEXT_FAINT).size(12.0));
                });
                ui.add_space(6.0);
                ui.label(RichText::new("排除规则（每行一条；`#` 注释；留空则使用内置默认）").color(theme::TEXT_DIM).size(12.0));
                TextEdit::multiline(&mut self.filters_text)
                    .desired_width(ui.available_width())
                    .desired_rows(4)
                    .font(egui::TextStyle::Monospace)
                    .show(ui);
                ui.add_space(6.0);
                ui.checkbox(
                    &mut self.animations,
                    RichText::new("启用界面动画（关闭以减弱动效）").size(12.5).color(theme::TEXT_DIM),
                );
            });
    }

    fn draw_progress(&mut self, ui: &mut egui::Ui) {
        // 卡片入场：淡入 + 上滑（0.25s）
        let fade_t = self.run.as_ref().map(|r| self.dlg_t(r.started)).unwrap_or(1.0);
        egui::Frame::default()
            .fill(theme::PANEL)
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(12))
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_opacity(fade_t);
                ui.add_space((1.0 - fade_t) * 8.0);
                let Some(run) = &mut self.run else {
                    return;
                };
                // 第一行：[扫描/比对阶段 spinner] 阶段 + 计时
                ui.horizontal(|ui| {
                    if matches!(run.phase, Phase::Scanning | Phase::Comparing) {
                        let (srect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        let ang = self.anim.spin(ui, 1.4);
                        anim::draw_arc(
                            ui.painter(),
                            srect.center(),
                            5.0,
                            ang,
                            ang + 1.9,
                            egui::Stroke::new(2.0, theme::TEAL),
                        );
                        ui.add_space(2.0);
                    }
                    ui.label(RichText::new(run.phase.label()).color(theme::TEAL).size(13.5));
                    let elapsed = run.started.elapsed();
                    ui.label(RichText::new(fmt_mmss(elapsed)).color(theme::TEXT_DIM).size(13.0).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.0}%", run.fraction_bytes() * 100.0))
                                .color(theme::TEAL)
                                .size(13.0)
                                .monospace(),
                        );
                    });
                });
                ui.add_space(4.0);
                // 进度条：扫描/比对阶段为不定态滑块；传输阶段平滑逼近目标值（0.3s）
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
                ui.painter().rect_filled(rect, egui::CornerRadius::same(4), theme::SURFACE_STRONG);
                if matches!(run.phase, Phase::Scanning | Phase::Comparing) || run.plan_bytes == 0 {
                    let p = self.anim.indeterminate(ui, 0.7);
                    let seg_w = rect.width() * 0.3;
                    let x0 = rect.min.x + (rect.width() + seg_w) * p - seg_w;
                    let bar = egui::Rect::from_min_size(egui::pos2(x0, rect.min.y), vec2(seg_w, rect.height()));
                    ui.painter()
                        .with_clip_rect(rect)
                        .rect_filled(bar, egui::CornerRadius::same(4), theme::TEAL.gamma_multiply(0.85));
                } else {
                    let shown = ui
                        .ctx()
                        .animate_value_with_time(egui::Id::unique("prog-frac"), run.fraction_bytes(), anim::NORMAL);
                    if shown > 0.001 {
                        let bar = egui::Rect::from_min_size(rect.min, vec2(rect.width() * shown, rect.height()));
                        ui.painter().rect_filled(bar, egui::CornerRadius::same(4), theme::TEAL);
                    }
                }
                ui.add_space(4.0);
                // 扫描/比对阶段：实时发现计数 + 当前目录
                if matches!(run.phase, Phase::Scanning | Phase::Comparing) {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("已发现 {} 个文件 · {}", run.scan_files, fmt_bytes(run.scan_bytes)))
                                .color(theme::TEXT)
                                .size(12.5),
                        );
                    });
                    if !run.scan_cur.is_empty() {
                        ui.label(RichText::new(truncate_path(&run.scan_cur, 70)).color(theme::TEXT_FAINT).size(11.0));
                    }
                }
                ui.add_space(4.0);
                // 第二行：字节/文件/速度/当前文件
                let speed = run.speed.bytes_per_sec();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} / {}", fmt_bytes(run.done_bytes), fmt_bytes(run.plan_bytes)))
                            .color(theme::TEXT)
                            .size(12.5)
                            .monospace(),
                    );
                    ui.label(RichText::new(format!("文件 {}/{}", run.done_files, run.plan_copies)).color(theme::TEXT_DIM).size(12.5));
                    if speed > 1.0 {
                        ui.label(RichText::new(format!("{}/s", fmt_bytes(speed as u64))).color(theme::TEAL).size(12.5));
                    }
                });
                if !run.last_done.is_empty() {
                    ui.label(RichText::new(truncate_path(&run.last_done, 64)).color(theme::TEXT_FAINT).size(11.5));
                }
            });
    }

    fn draw_log(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("日志").color(theme::TEXT_DIM).size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("清空").color(theme::TEXT_DIM).size(12.0)).frame(true)).clicked() {
                    self.log.clear();
                }
                if let Some(p) = self.session_log.clone() {
                    if ui.add(egui::Button::new(RichText::new("打开日志目录").color(theme::TEXT_DIM).size(12.0)).frame(true)).clicked() {
                        let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or(p.clone());
                        open_in_explorer(&dir);
                    }
                }
            });
        });
        // 一次性收集行引用，避免 show_rows 每帧 O(n²) 遍历
        let lines: Vec<&crate::log::LogLine> = self.log.iter().collect();
        let total = lines.len();
        let row_h = 19.0;
        egui::Frame::default()
            .fill(theme::LOG_BG)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(12))
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .show(ui, |ui| {
                // 高度：日志始终填满剩余空间（为底栏预留 54px），底部不留空带
                let _ = running;
                let avail = (ui.available_height() - 54.0).max(90.0);
                ui.set_width(ui.available_width());
                ScrollArea::vertical()
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .max_height(avail)
                    .show_rows(ui, row_h, total, |ui, range| {
                        for i in range {
                            if let Some(line) = lines.get(i) {
                                let text = format!("[{}] {}{}", line.time, line.level.prefix(), line.msg);
                                ui.add(
                                    egui::Label::new(RichText::new(text).size(12.5).color(log_color(line.level)))
                                        .truncate(),
                                );
                            }
                        }
                    });
            });
    }

    fn draw_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let running = self.status.running();
            let avail = ui.available_width();
            let start_w = (avail - 98.0 - 8.0).max(220.0);
            let label = if self.mode == Mode::Preview { "预览变更" } else { "开始同步" };
            if primary_action_button(ui, label, start_w, 40.0, !running).clicked() {
                self.start();
            }
            ui.add_space(8.0);
            if std_button(ui, "取消", 90.0, 40.0, false, running).clicked() {
                if let Some(run) = &self.run {
                    run.cancel.store(true, Ordering::Relaxed);
                }
                self.log(Level::Warn, "已请求取消，正在停止…");
            }
        });
    }

    // -----------------------------------------------------------------------
    // 对话框
    // -----------------------------------------------------------------------

    /// 二级窗口入场进度 0→1（0.2s，减弱动效时直接到 1）
    fn dlg_t(&self, at: Instant) -> f32 {
        if !self.animations {
            return 1.0;
        }
        let t = (at.elapsed().as_secs_f32() / 0.2).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn draw_save_dialog(&mut self, ctx: &egui::Context) {
        if !self.save_dlg_open {
            return;
        }
        let t = self.dlg_t(self.save_dlg_at);
        let mut name = self.save_dlg_name.clone();
        let mut close = false;
        let mut do_save = false;
        egui::Window::new("保存方案")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // 入场：淡入 + 轻微上滑
                ui.set_opacity(t);
                ui.add_space((1.0 - t) * 10.0);
                ui.label(RichText::new("方案名称").color(theme::TEXT_DIM).size(12.5));
                let resp = TextEdit::singleline(&mut name)
                    .desired_width(280.0)
                    .hint_text("如：工作文件 / 文档备份")
                    .show(ui)
                    .response;
                if resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !name.trim().is_empty()
                {
                    do_save = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if std_button(ui, "保存", 90.0, 30.0, true, true).clicked() {
                        do_save = true;
                    }
                    ui.add_space(6.0);
                    if std_button(ui, "取消", 90.0, 30.0, false, true).clicked() {
                        close = true;
                    }
                });
            });
        self.save_dlg_name = name;
        if do_save {
            self.apply_save_profile();
            self.save_dlg_open = false;
        } else if close {
            self.save_dlg_open = false;
        }
    }

    fn apply_save_profile(&mut self) {
        let name = self.save_dlg_name.trim().to_string();
        if name.is_empty() {
            self.log(Level::Warn, "方案名称不能为空");
            return;
        }
        let p = Profile::from_fields(
            &name,
            self.local.trim(),
            self.usb.trim(),
            self.mode,
            self.backup_dir.trim(),
            self.threads,
            &self.filters_text,
        );
        if let Some(i) = self.profiles.iter().position(|p| p.name == name) {
            self.profiles[i] = p;
        } else {
            self.profiles.push(p);
            self.selected = Some(self.profiles.len() - 1);
        }
        match profile::save_profiles(&self.profiles) {
            Ok(()) => self.log(Level::Success, format!("方案「{name}」已保存")),
            Err(e) => self.log(Level::Error, format!("保存方案失败：{e}")),
        }
    }

    fn draw_delete_confirm(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.confirm_delete else { return };
        let Some(p) = self.profiles.get(idx) else {
            self.confirm_delete = None;
            return;
        };
        let t = self.dlg_t(self.confirm_at);
        let name = p.name.clone();
        let mut do_delete = false;
        let mut close = false;
        egui::Window::new("删除方案")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_opacity(t);
                ui.add_space((1.0 - t) * 10.0);
                ui.label(format!("确定删除方案「{name}」吗？"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if std_button(ui, "删除", 90.0, 30.0, false, true).clicked() {
                        do_delete = true;
                    }
                    ui.add_space(6.0);
                    if std_button(ui, "取消", 90.0, 30.0, false, true).clicked() {
                        close = true;
                    }
                });
            });
        if do_delete {
            self.profiles.remove(idx);
            self.selected = None;
            match profile::save_profiles(&self.profiles) {
                Ok(()) => self.log(Level::Success, format!("方案「{name}」已删除")),
                Err(e) => self.log(Level::Error, format!("保存方案列表失败：{e}")),
            }
            self.confirm_delete = None;
        } else if close {
            self.confirm_delete = None;
        }
    }

    // -----------------------------------------------------------------------
    // 启动同步
    // -----------------------------------------------------------------------

    fn start(&mut self) {
        let local = self.local.trim().to_string();
        let usb = self.usb.trim().to_string();
        if local.is_empty() || usb.is_empty() {
            self.log(Level::Error, "请填写本地路径与U盘路径");
            return;
        }
        let mode = self.mode;
        let lp = Path::new(&local);
        let up = Path::new(&usb);
        // 源端必须存在；目标端允许自动创建
        let need_local = !matches!(mode, Mode::Pull);
        let need_usb = !matches!(mode, Mode::Push);
        if need_local && !lp.exists() {
            self.log(Level::Error, format!("本地路径不存在：{local}"));
            return;
        }
        if need_usb {
            let (drive_ok, _) = usb_status(&usb);
            if !drive_ok {
                self.log(Level::Error, format!("U盘未连接（{usb}）"));
                return;
            }
            if !up.exists() {
                self.log(Level::Error, format!("U盘路径不存在：{usb}"));
                return;
            }
        }

        let opts = Options {
            local: local.clone().into(),
            usb: usb.clone().into(),
            mode,
            backup_dir: opt_trimmed(&self.backup_dir).map(PathBuf::from),
            backup_max_bytes: self.backup_max_gb.saturating_mul(1024 * 1024 * 1024),
            threads: self.threads.clamp(1, 16),
            filters: parse_filter_lines(&self.filters_text),
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = exec::spawn(opts, Arc::clone(&cancel), tx);
        self.run = Some(Run {
            rx,
            cancel,
            handle: Some(handle),
            phase: Phase::Scanning,
            plan_copies: 0,
            plan_bytes: 0,
            plan_ops: 0,
            scan_files: 0,
            scan_bytes: 0,
            scan_cur: String::new(),
            done_files: 0,
            done_bytes: 0,
            last_done: String::new(),
            speed: SpeedMeter::new(),
            started: Instant::now(),
        });
        self.status = Status::Running(Phase::Scanning);
        self.log(Level::Info, format!("开始同步：{}", mode.label()));
        self.log(Level::Dim, format!("本地：{local}　U盘：{usb}"));

        // 保存界面记忆（立即持久化，异常退出也不丢）
        let st = UiState {
            local,
            usb,
            mode: Some(mode),
            backup_dir: self.backup_dir.clone(),
            backup_max_gb: Some(self.backup_max_gb),
            threads: self.threads,
            filters: self.filters_text.clone(),
            selected_profile: self.selected.map(|i| self.profiles[i].name.clone()),
            animations: Some(self.animations),
        };
        profile::save_ui_state(&st).ok();
    }
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

/// 启动 GUI（lib.rs 的 run_gui 调用）
pub fn run(session_log: Option<PathBuf>) -> Result<(), String> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("USB 同步工具 v{VERSION}"))
            .with_inner_size([780.0, 600.0])
            .with_min_inner_size([680.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "usb-sync",
        native,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let mut app = App::new();
            app.session_log = session_log;
            if let Some(p) = &app.session_log {
                app.log(Level::Dim, format!("日志文件：{}", p.display()));
            }
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 自绘控件与工具函数
// ---------------------------------------------------------------------------

/// 路径提示行：与输入框左缘对齐的小号状态文字
fn hint_line(ui: &mut egui::Ui, hint: (String, Color32)) {
    ui.horizontal(|ui| {
        ui.add_space(40.0); // 与输入框左缘对齐（标签宽 40）
        ui.label(RichText::new(hint.0).color(hint.1).size(11.5));
    });
}

/// 路径输入行：标签 + 输入框 + 浏览按钮（自由函数：避免与调用方的字段借用冲突）
fn path_row(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) -> bool {
    let mut browse = false;
    ui.horizontal(|ui| {
        let (lrect, _) = ui.allocate_exact_size(vec2(40.0, 28.0), Sense::hover());
        ui.painter().text(
            lrect.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            theme::TEXT_DIM,
        );
        let w = ui.available_width() - 66.0;
        TextEdit::singleline(value).desired_width(w).hint_text(hint).show(ui);
        let (brect, resp) = ui.allocate_exact_size(vec2(56.0, 28.0), Sense::click());
        let ht = ui
            .ctx()
            .animate_value_with_time(resp.id, if resp.hovered() { 1.0 } else { 0.0 }, anim::FAST);
        let fill = anim::mix(theme::SURFACE_STRONG, theme::PANEL_HOVER, ht);
        ui.painter().rect_filled(brect, egui::CornerRadius::same(8), fill);
        ui.painter().rect_stroke(brect, egui::CornerRadius::same(8), egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
        ui.painter().text(brect.center(), Align2::CENTER_CENTER, "浏览", FontId::proportional(12.5), theme::TEXT);
        browse = resp.clicked();
    });
    browse
}

/// 主操作按钮：紫罗兰实心 + 墨字，左侧自绘线稿三角图标（与文字分别定位，像素级对齐）
fn primary_action_button(ui: &mut egui::Ui, label: &str, w: f32, h: f32, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let ht = ui
        .ctx()
        .animate_value_with_time(resp.id, if enabled && resp.hovered() { 1.0 } else { 0.0 }, anim::FAST);
    let fill = anim::mix(theme::ACCENT, theme::ACCENT_HOVER, ht);
    let fill = if enabled { fill } else { fill.gamma_multiply(0.45) };
    let txt = if enabled { theme::ACCENT_INK } else { theme::ACCENT_INK.gamma_multiply(0.55) };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(8), fill);

    // 线稿三角（右指）+ 文本，整体居中：图标宽 9 + 间距 8 + 文本
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(14.0), txt);
    let icon_w = 9.0;
    let gap = 8.0;
    let total = icon_w + gap + galley.size().x;
    let x0 = rect.center().x - total / 2.0;
    let yc = rect.center().y;
    let th = 10.0;
    let pts = vec![
        egui::pos2(x0, yc - th / 2.0),
        egui::pos2(x0, yc + th / 2.0),
        egui::pos2(x0 + icon_w, yc),
        egui::pos2(x0, yc - th / 2.0),
    ];
    ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.5, txt)));
    ui.painter().galley(egui::pos2(x0 + icon_w + gap, yc - galley.size().y / 2.0), galley, txt);
    resp
}

/// 通用按钮（Bisecto 风格：主色实心 + 墨字 / 次级深面 + 细边），hover 颜色平滑过渡（0.15s）
fn std_button(ui: &mut egui::Ui, text: &str, w: f32, h: f32, primary: bool, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let base = if primary { theme::ACCENT } else { theme::SURFACE_STRONG };
    let hover = if primary { theme::ACCENT_HOVER } else { theme::PANEL_HOVER };
    let ht = ui
        .ctx()
        .animate_value_with_time(resp.id, if enabled && resp.hovered() { 1.0 } else { 0.0 }, anim::FAST);
    let mut fill = anim::mix(base, hover, ht);
    let mut txt = if primary { theme::ACCENT_INK } else { theme::TEXT };
    if !enabled {
        fill = fill.gamma_multiply(0.4);
        txt = theme::TEXT_FAINT;
    }
    ui.painter().rect_filled(rect, egui::CornerRadius::same(8), fill);
    if !primary {
        ui.painter().rect_stroke(rect, egui::CornerRadius::same(8), egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(if primary { 14.0 } else { 13.5 }),
        txt,
    );
    // 禁用时点击由调用方按 enabled 过滤；这里只做视觉降级
    resp
}

/// rfd 文件夹选择（阻塞式原生对话框）
fn pick_folder(current: &str) -> Option<String> {
    let mut dlg = rfd::FileDialog::new().set_title("选择文件夹");
    let cur = Path::new(current);
    if cur.exists() {
        dlg = dlg.set_directory(cur);
    }
    dlg.pick_folder().and_then(|p| p.to_str().map(|s| s.to_string()))
}

fn open_in_explorer(dir: &Path) {
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

/// U盘状态：(盘符可用, 路径存在)
fn usb_status(path: &str) -> (bool, bool) {
    if path.is_empty() {
        return (false, false);
    }
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        let drive = format!("{}:", (b[0] as char).to_ascii_uppercase());
        let drive_ok = Path::new(&drive).exists();
        (drive_ok, drive_ok && Path::new(path).exists())
    } else {
        (true, Path::new(path).exists())
    }
}

fn opt_trimmed(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(t)
}

fn log_color(level: Level) -> Color32 {
    match level {
        Level::Info => theme::INFO,
        Level::Success => theme::SUCCESS,
        Level::Warn => theme::WARNING,
        Level::Error => theme::ERROR,
        Level::Dim => theme::TEXT_DIM,
    }
}

pub fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

fn fmt_mmss(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn truncate_path(p: &str, max_chars: usize) -> String {
    if p.chars().count() <= max_chars {
        p.to_string()
    } else {
        let head: String = p.chars().skip(p.chars().count() + 3 - max_chars).collect();
        format!("…{head}")
    }
}
