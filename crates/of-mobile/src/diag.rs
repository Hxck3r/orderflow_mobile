//! Stage 0 diagnostics (plan §9): what the spike has to measure on the phone,
//! on one panel the owner can screenshot — screen size and density, the GPU,
//! frame times, gestures seen, insets, lifecycle, the log tail — plus a text
//! field to try the soft keyboard and the interface scale.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, RichText, Ui};

use crate::gestures::Counters;

/// Frames kept for the frame-time figures (about four seconds at 60 fps).
const FRAMES: usize = 240;

#[derive(Default)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub driver: String,
}

/// Screen facts the platform knows and egui does not.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Screen {
    /// The window in physical pixels.
    pub window_px: [u32; 2],
    /// System bars and the camera cutout over the window's edges, physical
    /// pixels (left, top, right, bottom).
    pub insets_px: [i32; 4],
}

pub struct Diag {
    pub open: bool,
    started: Instant,
    gpu: GpuInfo,
    /// CPU time of each frame's UI pass, and the time between frames.
    work: VecDeque<Duration>,
    gaps: VecDeque<Duration>,
    last_frame: Option<Instant>,
    frames: u64,
    focus_changes: u32,
    focused: Option<bool>,
    pub screen: Option<Screen>,
    keyboard_test: String,
}

impl Diag {
    pub fn new(gpu: GpuInfo) -> Self {
        Diag {
            open: false,
            started: Instant::now(),
            gpu,
            work: VecDeque::with_capacity(FRAMES),
            gaps: VecDeque::with_capacity(FRAMES),
            last_frame: None,
            frames: 0,
            focus_changes: 0,
            focused: None,
            screen: None,
            keyboard_test: String::new(),
        }
    }

    /// Called from the input hook: counts focus changes (the app going to the background and back).
    pub fn input(&mut self, raw: &egui::RawInput) {
        if self.focused != Some(raw.focused) {
            if self.focused.is_some() {
                self.focus_changes += 1;
            }
            self.focused = Some(raw.focused);
        }
    }

    pub fn frame_started(&mut self, now: Instant) {
        if let Some(last) = self.last_frame {
            push(&mut self.gaps, now - last);
        }
        self.last_frame = Some(now);
        self.frames += 1;
    }

    pub fn frame_done(&mut self, work: Duration) {
        push(&mut self.work, work);
    }

    pub fn show(&mut self, ctx: &egui::Context, counters: &Counters) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Диагностика этапа 0")
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.75).show(ui, |ui| self.contents(ui, counters));
            });
        self.open = open;
    }

    fn contents(&mut self, ui: &mut Ui, counters: &Counters) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let native = ctx.native_pixels_per_point().unwrap_or(ppp);
        let viewport = ctx.viewport_rect();
        let content = ctx.content_rect();
        heading(ui, "Экран");
        row(ui, "Окно, точек", format!("{:.0} × {:.0}", viewport.width(), viewport.height()));
        row(ui, "Без системных полос, точек", format!("{:.0} × {:.0} (сдвиг {:.0}, {:.0})", content.width(), content.height(), content.left(), content.top()));
        row(ui, "Плотность (пикселей на точку)", format!("{native:.3} системная, {ppp:.3} с масштабом"));
        if let Some(s) = self.screen {
            row(ui, "Окно, пикселей", format!("{} × {}", s.window_px[0], s.window_px[1]));
            let [l, t, r, b] = s.insets_px;
            row(ui, "Отступы панелей и выреза, пикселей", format!("слева {l}, сверху {t}, справа {r}, снизу {b}"));
        }
        ui.horizontal(|ui| {
            ui.label("Масштаб интерфейса");
            let zoom = ctx.zoom_factor();
            if ui.button(" − ").clicked() {
                ctx.set_zoom_factor((zoom - 0.05).max(0.5));
            }
            ui.label(format!("{:.0} %", zoom * 100.0));
            if ui.button(" + ").clicked() {
                ctx.set_zoom_factor((zoom + 0.05).min(2.0));
            }
        });

        heading(ui, "Графика");
        row(ui, "GPU", self.gpu.name.clone());
        row(ui, "API", self.gpu.backend.clone());
        row(ui, "Драйвер", self.gpu.driver.clone());

        heading(ui, "Кадры");
        let (work_avg, work_max) = stats(&self.work);
        let (gap_avg, gap_max) = stats(&self.gaps);
        row(ui, "Работа интерфейса за кадр", format!("{:.2} мс в среднем, {:.2} мс максимум", ms(work_avg), ms(work_max)));
        let fps = if gap_avg > Duration::ZERO { 1.0 / gap_avg.as_secs_f64() } else { 0.0 };
        row(ui, "Между кадрами", format!("{:.1} мс в среднем ({fps:.0} в секунду), {:.1} мс максимум", ms(gap_avg), ms(gap_max)));
        row(ui, "Кадров с запуска", self.frames.to_string());

        heading(ui, "Жесты");
        row(ui, "Касания / перетаскивания", format!("{} / {}", counters.taps, counters.drags));
        row(ui, "Щипки / перекрестия", format!("{} / {}", counters.pinches, counters.crosshairs));
        row(ui, "Наведение без касания (S Pen, мышь)", counters.hovers.to_string());

        heading(ui, "Жизненный цикл");
        row(ui, "Работает", format_uptime(self.started.elapsed()));
        row(ui, "Уходов в фон и возвратов", self.focus_changes.to_string());

        heading(ui, "Клавиатура");
        ui.label("Коснитесь поля — должна появиться экранная клавиатура:");
        ui.add(egui::TextEdit::singleline(&mut self.keyboard_test).hint_text("проверка ввода").desired_width(f32::INFINITY));

        heading(ui, "Журнал");
        crate::logger::with_lines(|lines| {
            for line in lines.iter().rev().take(30) {
                let color = match line.level {
                    log::Level::Error => Color32::from_rgb(230, 90, 90),
                    log::Level::Warn => Color32::from_rgb(220, 170, 60),
                    _ => ui.visuals().weak_text_color(),
                };
                ui.label(RichText::new(format!("{} {}", line.target, line.text)).small().color(color));
            }
        });
    }
}

fn push(q: &mut VecDeque<Duration>, d: Duration) {
    if q.len() == FRAMES {
        q.pop_front();
    }
    q.push_back(d);
}

fn stats(q: &VecDeque<Duration>) -> (Duration, Duration) {
    if q.is_empty() {
        return (Duration::ZERO, Duration::ZERO);
    }
    let sum: Duration = q.iter().sum();
    (sum / q.len() as u32, q.iter().copied().max().unwrap_or_default())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn format_uptime(d: Duration) -> String {
    let s = d.as_secs();
    format!("{} ч {:02} мин {:02} с", s / 3600, s / 60 % 60, s % 60)
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).strong());
}

fn row(ui: &mut Ui, label: &str, value: String) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(label).weak());
        ui.label(value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_figures_average_and_peak_over_the_window() {
        let mut q = VecDeque::new();
        for ms in [2, 4, 6] {
            push(&mut q, Duration::from_millis(ms));
        }
        assert_eq!(stats(&q), (Duration::from_millis(4), Duration::from_millis(6)));
        for _ in 0..FRAMES {
            push(&mut q, Duration::from_millis(1));
        }
        assert_eq!(q.len(), FRAMES);
        assert_eq!(stats(&q), (Duration::from_millis(1), Duration::from_millis(1)));
    }

    #[test]
    fn uptime_reads_as_hours_minutes_seconds() {
        assert_eq!(format_uptime(Duration::from_secs(3 * 3600 + 7 * 60 + 5)), "3 ч 07 мин 05 с");
    }
}
