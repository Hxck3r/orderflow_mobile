//! The stage 0 shell (plan §9): the PC version's whole window
//! (`app::TerminalApp`, compiled from the submodule as it is) on the phone,
//! with touch gestures turned into its mouse input, the system bars kept out
//! of its way and a diagnostics panel over it. The phone layout of stage 1
//! replaces the PC window here; the gestures and the diagnostics stay.
//!
//! What winit 0.30 does not carry on Android comes from [`Platform`]: the
//! system bars and the camera cutout (egui's root `Ui` ignores safe-area
//! insets anyway, so the terminal is drawn into a child `Ui` inside them), a
//! hovering S Pen, and the soft keyboard's text.

use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align2, Rect, RichText, UiBuilder, pos2};

use crate::app::TerminalApp;
use crate::diag::{Diag, GpuInfo, Screen};
use crate::gestures::{Gestures, Tuning};
use crate::ime::Mirror;
use crate::settings::{self, AppSettings};

/// A stylus hovering over the screen, physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub enum Hover {
    At(f32, f32),
    Gone,
}

/// What the platform knows that egui does not. Everything is read every
/// frame and must be cheap.
pub trait Platform {
    /// The window and its system-bar and cutout insets.
    fn screen(&self) -> Option<Screen> {
        None
    }
    /// The latest stylus hover since the last call.
    fn take_hover(&self) -> Option<Hover> {
        None
    }
    /// What the soft keyboard has typed since [`Platform::reset_keyboard`].
    fn keyboard_text(&self) -> Option<String> {
        None
    }
    fn reset_keyboard(&self) {}
    /// Lets the platform wake the interface (a hover or new insets arrive between frames).
    fn set_waker(&self, _ctx: &egui::Context) {}
}

/// While a text field has focus the keyboard's buffer is polled at this
/// pace: its changes wake nothing on their own.
const KEYBOARD_POLL: Duration = Duration::from_millis(40);

pub struct MobileApp {
    inner: TerminalApp,
    gestures: Gestures,
    diag: Diag,
    platform: Option<Box<dyn Platform>>,
    keyboard: Mirror,
    /// The shell's own buttons, last frame: touches there are plain taps.
    own: Vec<Rect>,
    /// A frame the input hook needs (a long press, the keyboard poll): asked
    /// for inside the pass, since egui drops requests made before it.
    repaint_in: Option<Duration>,
    /// The settings file as last read for the gestures' tuning.
    settings_seen: Option<SystemTime>,
    settings_checked: Instant,
}

impl MobileApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: AppSettings, platform: Option<Box<dyn Platform>>) -> Self {
        let tuning = Tuning { zoom_sensitivity: settings.chart.zoom_sensitivity, zoom_both_axes: settings.chart.zoom_both_axes, ..Tuning::default() };
        let gpu = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| {
                let info = rs.adapter.get_info();
                GpuInfo { name: info.name, backend: format!("{:?}", info.backend), driver: format!("{} {}", info.driver, info.driver_info) }
            })
            .unwrap_or_default();
        log::info!("GPU: {} ({}), driver {}", gpu.name, gpu.backend, gpu.driver);
        if let Some(p) = &platform {
            p.set_waker(&cc.egui_ctx);
        }
        // A finger's second tap takes longer than a mouse's double click.
        cc.egui_ctx.options_mut(|o| o.input_options.max_double_click_delay = 0.45);
        let inner = TerminalApp::new(cc, settings);
        MobileApp {
            inner,
            gestures: Gestures::new(tuning),
            diag: Diag::new(gpu),
            platform,
            keyboard: Mirror::Off,
            own: Vec::new(),
            repaint_in: None,
            settings_seen: modified(&settings_file()),
            settings_checked: Instant::now(),
        }
    }

    /// The terminal keeps its settings to itself; the gestures follow the
    /// zoom settings through the file it saves within a second of a change.
    fn follow_settings(&mut self) {
        if self.settings_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.settings_checked = Instant::now();
        let seen = modified(&settings_file());
        if seen != self.settings_seen {
            self.settings_seen = seen;
            let s = settings::load();
            self.gestures.tuning.zoom_sensitivity = s.chart.zoom_sensitivity;
            self.gestures.tuning.zoom_both_axes = s.chart.zoom_both_axes;
        }
    }

    fn repaint_within(&mut self, d: Duration) {
        self.repaint_in = Some(self.repaint_in.map_or(d, |r| r.min(d)));
    }

    /// Insets, stylus hover and keyboard text into this frame's input.
    fn platform_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(p) = &self.platform else { return };
        let ppp = ctx.pixels_per_point();
        if let Some(screen) = p.screen() {
            self.diag.screen = Some(screen);
            // Every frame: points change with the interface scale even when pixels do not.
            let [left, top, right, bottom] = screen.insets_px.map(|v| v.max(0) as f32 / ppp);
            raw.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { left, right, top, bottom }));
        }
        // First in the list: right after a touch it would read as the touch's emulation.
        match p.take_hover() {
            Some(Hover::At(x, y)) => raw.events.insert(0, egui::Event::PointerMoved(pos2(x / ppp, y / ppp))),
            Some(Hover::Gone) => raw.events.insert(0, egui::Event::PointerGone),
            None => {}
        }
        let now = raw.time.unwrap_or(0.0);
        let step = self.keyboard.step(ctx.egui_wants_keyboard_input(), now, || p.keyboard_text().unwrap_or_default());
        if step.reset {
            p.reset_keyboard();
        }
        raw.events.extend(step.events);
        if step.poll {
            self.repaint_within(KEYBOARD_POLL);
        }
    }

    /// The shell's buttons over the terminal: diagnostics, and while the
    /// crosshair is on, the chart's context menu and putting the crosshair away.
    fn overlay(&mut self, ctx: &egui::Context) {
        self.own.clear();
        let content = ctx.content_rect();
        let area = egui::Area::new(egui::Id::new("of-mobile-diag-button"))
            .order(egui::Order::Foreground)
            .anchor(Align2::LEFT_CENTER, [content.left() + 4.0, 0.0])
            .show(ctx, |ui| {
                let label = RichText::new(if self.diag.open { " × " } else { " i " }).size(18.0);
                if ui.add(egui::Button::new(label).min_size(egui::vec2(40.0, 40.0))).clicked() {
                    self.diag.open = !self.diag.open;
                }
            });
        self.own.push(area.response.rect);

        if self.gestures.crosshair().is_some() {
            let area = egui::Area::new(egui::Id::new("of-mobile-crosshair"))
                .order(egui::Order::Foreground)
                .anchor(Align2::CENTER_TOP, [0.0, content.top() + 44.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let size = egui::vec2(96.0, 40.0);
                        if ui.add(egui::Button::new("Меню").min_size(size)).clicked() {
                            self.gestures.context_menu_at_crosshair();
                        }
                        if ui.add(egui::Button::new("Убрать").min_size(size)).clicked() {
                            self.gestures.clear_crosshair();
                        }
                    });
                });
            self.own.push(area.response.rect);
        }
        if self.diag.open
            && let Some(rect) = ctx.memory(|m| m.area_rect(egui::Id::new("Диагностика этапа 0")))
        {
            self.own.push(rect);
        }
    }
}

impl eframe::App for MobileApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.diag.input(raw);
        self.platform_input(ctx, raw);
        if let Some(due) = self.gestures.process(raw) {
            self.repaint_within(Duration::from_secs_f64(due));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let started = Instant::now();
        self.diag.frame_started(started);
        if let Some(d) = self.repaint_in.take() {
            ui.ctx().request_repaint_after(d);
        }
        self.follow_settings();
        // egui's root Ui spans the whole window whatever the safe-area
        // insets say (egui 0.36 `Context::run_ui`): draw inside them.
        let content = ui.ctx().content_rect();
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| eframe::App::ui(&mut self.inner, ui, frame));
        let ctx = ui.ctx().clone();
        self.overlay(&ctx);
        self.diag.show(&ctx, &self.gestures.counters);
        // Gestures apply to the whole terminal: a tap is a click and a long
        // press a hover everywhere; only the shell's own buttons are left alone.
        self.gestures.set_areas(Some(ctx.content_rect()), self.own.clone());
        self.diag.frame_done(started.elapsed());
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        eframe::App::clear_color(&self.inner, visuals)
    }

    fn on_exit(&mut self) {
        eframe::App::on_exit(&mut self.inner);
    }
}

/// The settings, with the phone's defaults (plan §6) on the first launch.
pub fn load_settings() -> AppSettings {
    let first = !settings_file().exists();
    let mut s = settings::load();
    if first {
        mobile_profile(&mut s);
        log::info!("first launch: the phone's defaults are set");
    }
    // Always: the PC window's own frame has a close button, and a closed
    // event loop cannot be started again in the same Android process
    // (winit `EventLoop` recreation is refused).
    s.general.system_frame = true;
    if first && let Err(err) = settings::save(&s) {
        log::warn!("settings: {err}");
    }
    s
}

/// The PC version's `settings_path` (private there).
fn settings_file() -> std::path::PathBuf {
    crate::paths::config_dir().join("settings.json")
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The phone's defaults where they differ from the PC's (plan §6).
pub fn mobile_profile(s: &mut AppSettings) {
    // Trades of the current session only: memory and rebuild time.
    s.data.footprint_days = 1;
    // The book recording is ~590 MB a day; it only feeds the replay.
    s.data.record_book = false;
    s.data.keep_days = 7;
    s.performance.live_fps = 15;
    // GEX downloads the whole Deribit options tape; it comes in stage 6.
    s.gex.load = false;
    s.general.system_frame = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phone_profile_survives_the_pc_versions_normalize() {
        let mut s = AppSettings::default();
        mobile_profile(&mut s);
        s.normalize();
        assert_eq!(s.data.footprint_days, 1);
        assert!(!s.data.record_book);
        assert_eq!(s.data.keep_days, 7);
        assert_eq!(s.performance.live_fps, 15);
        assert!(!s.gex.load);
        assert!(s.general.system_frame);
    }
}
