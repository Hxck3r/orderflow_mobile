//! Touch gestures turned into the mouse input the PC version's chart already
//! understands, so its code stays untouched (plan §3.6).
//!
//! egui-winit reports every finger as an `Event::Touch` and, for the first
//! finger, follows it with emulated `PointerMoved`/`PointerButton`/
//! `PointerGone` events (egui-winit 0.36.2 `on_touch`). For a touch that
//! starts in the translated area those emulated events are dropped and
//! replaced:
//!
//! | gesture                         | the PC code gets                                  |
//! |---------------------------------|---------------------------------------------------|
//! | one finger moved past the slop  | press where it landed, then moves: a drag         |
//! | one finger lifted in place      | press + release: a click; a second tap near the   |
//! |                                 | first is put on the first's spot: a double click  |
//! | one finger held in place        | crosshair: a hover without a press, lifted above  |
//! |                                 | the finger, kept after the finger is up           |
//! | two fingers apart / together    | `MouseWheel` (time) and Ctrl+`MouseWheel` (price),|
//! |                                 | at most one per axis a frame, past a dead zone    |
//!
//! Touches on the shell's own buttons keep egui-winit's emulation. A stylus
//! hovering over the screen arrives as plain pointer events without touches
//! and is left alone: it hovers like a mouse.

use eframe::egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, TouchId, TouchPhase, Vec2, pos2};

/// How the chart turns wheel points into zoom factors (`chart/mod.rs`, the
/// mouse wheel block): time `exp(dy · 0.0022 · sensitivity)`, price
/// `exp(dy · 0.002 · sensitivity)` with Ctrl.
const WHEEL_TIME: f32 = 0.0022;
const WHEEL_PRICE: f32 = 0.002;

#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Points a finger may wander before a press becomes a drag.
    pub slop: f32,
    /// Seconds a finger must stay put to become the crosshair.
    pub long_press: f64,
    /// Points the crosshair sits above the finger, so the finger does not hide the tooltip.
    pub lift: f32,
    /// The chart's `zoom_sensitivity` setting, to undo its scaling of wheel points.
    pub zoom_sensitivity: f32,
    /// The chart's `zoom_both_axes` setting: its plain wheel zooms price too.
    pub zoom_both_axes: bool,
    /// Below this span (points) a pinch axis is too short to measure a ratio on.
    pub min_span: f32,
    /// A pinch is sent once it has zoomed an axis by this much (ln of the
    /// ratio): finger jitter must not move a stepped scale (the `Стакан`
    /// tab changes its price step one notch per wheel event).
    pub pinch_dead_zone: f32,
    /// A second tap this soon and this close to the first is a double click.
    pub double_tap_s: f64,
    pub double_tap_dist: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            slop: 10.0,
            long_press: 0.45,
            lift: 64.0,
            zoom_sensitivity: 1.0,
            zoom_both_axes: false,
            min_span: 48.0,
            pinch_dead_zone: 0.05,
            double_tap_s: 0.4,
            double_tap_dist: 40.0,
        }
    }
}

/// Which axes a pinch zooms, fixed when the second finger lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinchAxes {
    Time,
    Price,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Idle,
    /// One finger down, not yet a drag, a tap or a long press.
    Pending {
        id: TouchId,
        start: Pos2,
        last: Pos2,
        t0: f64,
    },
    /// The emulated left button is down.
    Dragging {
        id: TouchId,
        last: Pos2,
    },
    /// `zoom`: ln of the zoom per axis the fingers made and the wheel has not sent yet.
    Pinch {
        a: (TouchId, Pos2),
        b: (TouchId, Pos2),
        axes: PinchAxes,
        zoom: Vec2,
    },
    /// The crosshair is on; `finger` moves it, or it rests where it was left.
    Crosshair {
        at: Pos2,
        finger: Option<CrossFinger>,
    },
    /// A gesture ended with fingers still down: ignore them until all are up.
    Settling,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CrossFinger {
    id: TouchId,
    start: Pos2,
    last: Pos2,
    moved: bool,
}

/// What happened, for the diagnostics panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub taps: u64,
    pub double_taps: u64,
    pub drags: u64,
    pub pinches: u64,
    pub crosshairs: u64,
    /// Pointer moves that came without any touch: a mouse or a hovering S Pen.
    pub hovers: u64,
}

#[derive(Debug)]
pub struct Gestures {
    pub tuning: Tuning,
    mode: Mode,
    /// Fingers down that this translator owns.
    down: Vec<TouchId>,
    /// Fingers down that egui-winit's emulation keeps.
    passthrough: Vec<TouchId>,
    /// Where gestures are translated and, inside it, the shell's own widgets.
    area: Option<Rect>,
    exempt: Vec<Rect>,
    /// Events for the next frame.
    queued: Vec<Event>,
    /// Where and when the last tap was sent, for a double tap.
    last_tap: Option<(Pos2, f64)>,
    /// The keyboard's modifiers as the last `ModifiersChanged` left them
    /// (a keyboard in DeX); synthesized pointer events carry them.
    modifiers: Modifiers,
    pub counters: Counters,
}

impl Default for Gestures {
    fn default() -> Self {
        Gestures::new(Tuning::default())
    }
}

impl Gestures {
    pub fn new(tuning: Tuning) -> Self {
        Gestures {
            tuning,
            mode: Mode::Idle,
            down: Vec::new(),
            passthrough: Vec::new(),
            area: None,
            exempt: Vec::new(),
            queued: Vec::new(),
            last_tap: None,
            modifiers: Modifiers::NONE,
            counters: Counters::default(),
        }
    }

    /// The translated area and the shell's widgets over it, from the last frame.
    pub fn set_areas(&mut self, area: Option<Rect>, exempt: Vec<Rect>) {
        self.area = area;
        self.exempt = exempt;
    }

    pub fn crosshair(&self) -> Option<Pos2> {
        match self.mode {
            Mode::Crosshair { at, .. } => Some(at),
            _ => None,
        }
    }

    /// A right click at the crosshair on the next frame: the chart's context
    /// menu. The crosshair goes, so the next tap reaches the menu.
    pub fn context_menu_at_crosshair(&mut self) {
        if let Some(at) = self.crosshair() {
            let modifiers = self.modifiers;
            self.queued.extend([
                Event::PointerMoved(at),
                Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers },
                Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers },
            ]);
            self.mode = if self.down.is_empty() { Mode::Idle } else { Mode::Settling };
        }
    }

    pub fn clear_crosshair(&mut self) {
        if matches!(self.mode, Mode::Crosshair { .. }) {
            self.mode = if self.down.is_empty() { Mode::Idle } else { Mode::Settling };
            self.queued.push(Event::PointerGone);
        }
    }

    /// Rewrites this frame's input. Returns how many seconds from now the
    /// next frame is needed (a pending long press, or events queued for it).
    pub fn process(&mut self, raw: &mut RawInput) -> Option<f64> {
        let now = raw.time.unwrap_or(0.0);
        let mut out = std::mem::take(&mut self.queued);
        // A finger that rested past the long press is a crosshair before
        // whatever it does next: its lifting must not read as a tap.
        self.tick(now, &mut out);
        // The touch whose emulated pointer events follow, and whether we own it.
        let mut emulated_ours: Option<bool> = None;
        for event in std::mem::take(&mut raw.events) {
            match event {
                Event::Touch { id, phase, pos, .. } => {
                    let ours = self.touch(id, phase, pos, now, &mut out);
                    emulated_ours = Some(ours);
                    out.push(event);
                }
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone if emulated_ours.is_some() => {
                    if emulated_ours == Some(false) {
                        out.push(event);
                    }
                }
                other => {
                    emulated_ours = None;
                    if let Event::ModifiersChanged(m) = other {
                        self.modifiers = m;
                    }
                    if matches!(other, Event::PointerMoved(_)) && self.down.is_empty() && self.passthrough.is_empty() {
                        self.counters.hovers += 1;
                    }
                    out.push(other);
                }
            }
        }
        self.tick(now, &mut out);
        self.send_zoom(&mut out);
        raw.events = out;
        if !self.queued.is_empty() {
            return Some(0.0);
        }
        match self.mode {
            Mode::Pending { t0, .. } => Some((t0 + self.tuning.long_press - now).max(0.0)),
            _ => None,
        }
    }

    /// A long press is due even when no event arrives while the finger rests.
    fn tick(&mut self, now: f64, out: &mut Vec<Event>) {
        if let Mode::Pending { id, last, t0, .. } = self.mode
            && now - t0 >= self.tuning.long_press
        {
            let at = self.lifted(last);
            self.mode = Mode::Crosshair { at, finger: Some(CrossFinger { id, start: last, last, moved: true }) };
            self.counters.crosshairs += 1;
            out.push(Event::PointerMoved(at));
        }
    }

    fn lifted(&self, finger: Pos2) -> Pos2 {
        let y = finger.y - self.tuning.lift;
        let top = self.area.map_or(0.0, |r| r.top());
        pos2(finger.x, y.max(top + 1.0))
    }

    fn translated(&self, pos: Pos2) -> bool {
        self.area.is_some_and(|r| r.contains(pos)) && !self.exempt.iter().any(|r| r.contains(pos))
    }

    /// Handles one touch event; returns whether this translator owns the finger.
    fn touch(&mut self, id: TouchId, phase: TouchPhase, pos: Pos2, now: f64, out: &mut Vec<Event>) -> bool {
        let modifiers = self.modifiers;
        match phase {
            TouchPhase::Start => {
                // A finger joins whoever owns the fingers already down; a lone
                // finger is ours when it lands in the translated area.
                let ours = if !self.down.is_empty() {
                    true
                } else if !self.passthrough.is_empty() {
                    false
                } else {
                    self.translated(pos)
                };
                if !ours {
                    self.passthrough.push(id);
                    return false;
                }
                self.down.push(id);
                self.start(id, pos, now, modifiers, out);
                true
            }
            TouchPhase::Move => {
                if !self.down.contains(&id) {
                    return false;
                }
                self.moved(id, pos, modifiers, out);
                true
            }
            TouchPhase::End | TouchPhase::Cancel => {
                if let Some(i) = self.passthrough.iter().position(|&t| t == id) {
                    self.passthrough.swap_remove(i);
                    // egui-winit ends a cancelled touch with `PointerGone` alone,
                    // and egui keeps the button down: release it.
                    if phase == TouchPhase::Cancel {
                        out.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers });
                    }
                    return false;
                }
                let Some(i) = self.down.iter().position(|&t| t == id) else { return false };
                self.down.swap_remove(i);
                self.ended(id, pos, now, phase == TouchPhase::Cancel, modifiers, out);
                true
            }
        }
    }

    fn start(&mut self, id: TouchId, pos: Pos2, now: f64, modifiers: Modifiers, out: &mut Vec<Event>) {
        self.mode = match self.mode {
            Mode::Idle => Mode::Pending { id, start: pos, last: pos, t0: now },
            Mode::Pending { id: first, last, .. } => {
                self.counters.pinches += 1;
                pinch((first, last), (id, pos))
            }
            Mode::Dragging { id: first, last } => {
                // The move queued for the drag must not land after its release.
                self.queued.clear();
                out.push(Event::PointerButton { pos: last, button: PointerButton::Primary, pressed: false, modifiers });
                self.counters.pinches += 1;
                pinch((first, last), (id, pos))
            }
            Mode::Crosshair { at, finger: None } => Mode::Crosshair { at, finger: Some(CrossFinger { id, start: pos, last: pos, moved: false }) },
            // A third finger, or a second one while the crosshair is held.
            other => other,
        };
    }

    fn moved(&mut self, id: TouchId, pos: Pos2, modifiers: Modifiers, out: &mut Vec<Event>) {
        match self.mode {
            Mode::Pending { id: p, start, t0, .. } if p == id => {
                if start.distance(pos) > self.tuning.slop {
                    self.counters.drags += 1;
                    // The press goes where the finger landed, in a frame of its
                    // own: egui hit-tests once a frame, at the last position,
                    // and a thin handle (a pane splitter) must get the drag.
                    out.push(Event::PointerMoved(start));
                    out.push(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers });
                    self.queued.push(Event::PointerMoved(pos));
                    self.mode = Mode::Dragging { id, last: pos };
                } else {
                    self.mode = Mode::Pending { id, start, last: pos, t0 };
                }
            }
            Mode::Dragging { id: d, .. } if d == id => {
                if self.queued.is_empty() {
                    out.push(Event::PointerMoved(pos));
                } else {
                    // Still the frame of the press: move the queued step on instead.
                    self.queued = vec![Event::PointerMoved(pos)];
                }
                self.mode = Mode::Dragging { id, last: pos };
            }
            Mode::Pinch { a, b, axes, zoom } => {
                let (na, nb) = if a.0 == id {
                    ((id, pos), b)
                } else if b.0 == id {
                    (a, (id, pos))
                } else {
                    return;
                };
                let zoom = zoom + self.span_zoom(a.1, b.1, na.1, nb.1);
                self.mode = Mode::Pinch { a: na, b: nb, axes, zoom };
            }
            Mode::Crosshair { at, finger: Some(f) } if f.id == id => {
                // The crosshair follows the finger by its moves, so taking it
                // up again does not make it jump under the finger.
                let moved = f.moved || f.start.distance(pos) > self.tuning.slop;
                let at = if moved { self.clamp(at + (pos - f.last)) } else { at };
                if moved {
                    out.push(Event::PointerMoved(at));
                }
                self.mode = Mode::Crosshair { at, finger: Some(CrossFinger { last: pos, moved, ..f }) };
            }
            _ => {}
        }
    }

    fn ended(&mut self, id: TouchId, pos: Pos2, now: f64, cancelled: bool, modifiers: Modifiers, out: &mut Vec<Event>) {
        let all_up = self.down.is_empty();
        self.mode = match self.mode {
            Mode::Pending { id: p, start, .. } if p == id => {
                if !cancelled {
                    // egui counts a double click only within a few points of the
                    // first; a finger's second tap lands further away.
                    let at = match self.last_tap {
                        Some((first, t)) if now - t <= self.tuning.double_tap_s && first.distance(start) <= self.tuning.double_tap_dist => {
                            self.counters.double_taps += 1;
                            self.last_tap = None;
                            first
                        }
                        _ => {
                            self.last_tap = Some((start, now));
                            start
                        }
                    };
                    self.counters.taps += 1;
                    out.push(Event::PointerMoved(at));
                    out.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
                    out.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
                }
                out.push(Event::PointerGone);
                Mode::Idle
            }
            Mode::Dragging { id: d, .. } if d == id => {
                // A step still queued goes before the release.
                out.append(&mut self.queued);
                out.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers });
                out.push(Event::PointerGone);
                Mode::Idle
            }
            // What the dead zone still holds (under 5 %) is dropped: on a
            // stepped scale it would be one more notch the fingers never made.
            Mode::Pinch { .. } => {
                out.push(Event::PointerGone);
                if all_up { Mode::Idle } else { Mode::Settling }
            }
            // A tap without moving puts the crosshair away; a drag leaves it where it is.
            Mode::Crosshair { at, finger: Some(f) } if f.id == id => {
                if f.moved && !cancelled {
                    Mode::Crosshair { at, finger: None }
                } else {
                    out.push(Event::PointerGone);
                    if all_up { Mode::Idle } else { Mode::Settling }
                }
            }
            Mode::Settling if all_up => Mode::Idle,
            other => other,
        };
    }

    fn clamp(&self, p: Pos2) -> Pos2 {
        match self.area {
            Some(r) => pos2(p.x.clamp(r.left() + 1.0, r.right() - 1.0), p.y.clamp(r.top() + 1.0, r.bottom() - 1.0)),
            None => p,
        }
    }

    /// ln of the zoom per axis between two finger positions.
    fn span_zoom(&self, a0: Pos2, b0: Pos2, a1: Pos2, b1: Pos2) -> Vec2 {
        let min = self.tuning.min_span;
        let (s0, s1) = ((b0 - a0).abs(), (b1 - a1).abs());
        Vec2::new((s1.x.max(min) / s0.x.max(min)).ln(), (s1.y.max(min) / s0.y.max(min)).ln())
    }

    /// Wheel events for the pinch's unsent zoom, an axis once it passes the dead zone.
    fn send_zoom(&mut self, out: &mut Vec<Event>) {
        let Mode::Pinch { a, b, axes, mut zoom } = self.mode else { return };
        let dead_zone = self.tuning.pinch_dead_zone;
        let sens = self.tuning.zoom_sensitivity.max(0.05);
        let time = axes != PinchAxes::Price;
        // With `zoom_both_axes` the plain wheel zooms price too: never add Ctrl to it.
        let price = axes == PinchAxes::Price || (axes == PinchAxes::Both && !self.tuning.zoom_both_axes);
        let mut events = Vec::with_capacity(2);
        if time && zoom.x.abs() > dead_zone {
            events.push(wheel(Vec2::new(0.0, zoom.x / (WHEEL_TIME * sens)), Modifiers::NONE));
            zoom.x = 0.0;
        }
        if price && zoom.y.abs() > dead_zone {
            events.push(wheel(Vec2::new(0.0, zoom.y / (WHEEL_PRICE * sens)), Modifiers::CTRL));
            zoom.y = 0.0;
        }
        if !events.is_empty() {
            // The chart zooms around the pointer: put it between the fingers.
            out.push(Event::PointerMoved(a.1 + (b.1 - a.1) * 0.5));
            out.extend(events);
        }
        self.mode = Mode::Pinch { a, b, axes, zoom };
    }
}

fn pinch(a: (TouchId, Pos2), b: (TouchId, Pos2)) -> Mode {
    let d = (b.1 - a.1).abs();
    let axes = if d.x > 2.0 * d.y {
        PinchAxes::Time
    } else if d.y > 2.0 * d.x {
        PinchAxes::Price
    } else {
        PinchAxes::Both
    };
    Mode::Pinch { a, b, axes, zoom: Vec2::ZERO }
}

fn wheel(delta: Vec2, modifiers: Modifiers) -> Event {
    Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, modifiers }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::TouchDeviceId;

    fn area() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 400.0))
    }

    fn gestures() -> Gestures {
        let mut g = Gestures::default();
        g.set_areas(Some(area()), vec![Rect::from_min_max(pos2(760.0, 0.0), pos2(800.0, 40.0))]);
        g
    }

    /// What egui-winit sends for one touch event: the touch, then for the
    /// first finger the emulated pointer events.
    fn winit(id: u64, phase: TouchPhase, pos: Pos2, first: bool) -> Vec<Event> {
        let mut v = vec![Event::Touch { device_id: TouchDeviceId(0), id: TouchId(id), phase, pos, force: None }];
        if first {
            let m = Modifiers::NONE;
            match phase {
                TouchPhase::Start => {
                    v.extend([Event::PointerMoved(pos), Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: m }])
                }
                TouchPhase::Move => v.push(Event::PointerMoved(pos)),
                TouchPhase::End => v.extend([Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: m }, Event::PointerGone]),
                TouchPhase::Cancel => v.push(Event::PointerGone),
            }
        }
        v
    }

    fn frame(g: &mut Gestures, time: f64, events: Vec<Event>) -> (Vec<Event>, Option<f64>) {
        let mut raw = RawInput { time: Some(time), events, ..Default::default() };
        let due = g.process(&mut raw);
        (raw.events.into_iter().filter(|e| !matches!(e, Event::Touch { .. })).collect(), due)
    }

    fn presses(events: &[Event]) -> Vec<(PointerButton, bool)> {
        events.iter().filter_map(|e| if let Event::PointerButton { button, pressed, .. } = e { Some((*button, *pressed)) } else { None }).collect()
    }

    fn wheels(events: &[Event]) -> Vec<(f32, Modifiers)> {
        events.iter().filter_map(|e| if let Event::MouseWheel { delta, modifiers, .. } = e { Some((delta.y, *modifiers)) } else { None }).collect()
    }

    #[test]
    fn a_quick_tap_is_a_click_at_the_touch_point() {
        let mut g = gestures();
        let p = pos2(100.0, 100.0);
        let (out, due) = frame(&mut g, 0.0, winit(1, TouchPhase::Start, p, true));
        assert!(out.is_empty(), "nothing before the gesture is known: {out:?}");
        assert!(due.is_some());
        let (out, due) = frame(&mut g, 0.1, winit(1, TouchPhase::End, p, true));
        assert_eq!(presses(&out), [(PointerButton::Primary, true), (PointerButton::Primary, false)]);
        assert!(matches!(out.last(), Some(Event::PointerGone)));
        assert_eq!(due, None);
        assert_eq!(g.counters.taps, 1);
    }

    #[test]
    fn a_second_tap_nearby_lands_on_the_first_for_a_double_click() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(100.0, 100.0), true));
        frame(&mut g, 0.08, winit(1, TouchPhase::End, pos2(100.0, 100.0), true));
        frame(&mut g, 0.2, winit(2, TouchPhase::Start, pos2(118.0, 109.0), true));
        let (out, _) = frame(&mut g, 0.28, winit(2, TouchPhase::End, pos2(118.0, 109.0), true));
        assert!(out.iter().all(|e| !matches!(e, Event::PointerButton { pos, .. } if *pos != pos2(100.0, 100.0))), "{out:?}");
        assert_eq!(g.counters.double_taps, 1);
        // A third tap starts a new pair, and a far or late one is a plain tap.
        frame(&mut g, 0.5, winit(3, TouchPhase::Start, pos2(300.0, 100.0), true));
        let (out, _) = frame(&mut g, 0.55, winit(3, TouchPhase::End, pos2(300.0, 100.0), true));
        assert!(matches!(out[1], Event::PointerButton { pos, .. } if pos == pos2(300.0, 100.0)));
        frame(&mut g, 1.5, winit(4, TouchPhase::Start, pos2(305.0, 100.0), true));
        frame(&mut g, 1.55, winit(4, TouchPhase::End, pos2(305.0, 100.0), true));
        assert_eq!(g.counters.double_taps, 1);
    }

    #[test]
    fn a_moved_finger_presses_where_it_landed_and_moves_a_frame_later() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(100.0, 100.0), true));
        let (out, _) = frame(&mut g, 0.05, winit(1, TouchPhase::Move, pos2(104.0, 100.0), true));
        assert!(out.is_empty(), "inside the slop: {out:?}");
        let (out, due) = frame(&mut g, 0.1, winit(1, TouchPhase::Move, pos2(150.0, 100.0), true));
        assert_eq!(
            out,
            vec![
                Event::PointerMoved(pos2(100.0, 100.0)),
                Event::PointerButton { pos: pos2(100.0, 100.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            ]
        );
        assert_eq!(due, Some(0.0), "the queued move needs a frame");
        let (out, _) = frame(&mut g, 0.11, vec![]);
        assert_eq!(out, vec![Event::PointerMoved(pos2(150.0, 100.0))]);
        // Held still long after: still a drag, no crosshair.
        let (out, due) = frame(&mut g, 2.0, vec![]);
        assert!(out.is_empty());
        assert_eq!(due, None);
        let (out, _) = frame(&mut g, 2.1, winit(1, TouchPhase::End, pos2(150.0, 100.0), true));
        assert_eq!(presses(&out), [(PointerButton::Primary, false)]);
        assert_eq!(g.crosshair(), None);
    }

    #[test]
    fn a_drag_released_in_the_frame_of_its_press_still_moves_first() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(100.0, 100.0), true));
        frame(&mut g, 0.05, winit(1, TouchPhase::Move, pos2(150.0, 100.0), true));
        let (out, _) = frame(&mut g, 0.06, winit(1, TouchPhase::End, pos2(150.0, 100.0), true));
        assert_eq!(out[0], Event::PointerMoved(pos2(150.0, 100.0)));
        assert_eq!(presses(&out), [(PointerButton::Primary, false)]);
    }

    #[test]
    fn a_held_finger_becomes_a_crosshair_above_it_that_stays_after_lifting() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(200.0, 200.0), true));
        // No event arrives while the finger rests; the frame the timer asked for does it.
        let (out, _) = frame(&mut g, 0.5, vec![]);
        let at = pos2(200.0, 200.0 - g.tuning.lift);
        assert_eq!(out, vec![Event::PointerMoved(at)]);
        assert_eq!(g.crosshair(), Some(at));
        let (out, _) = frame(&mut g, 0.6, winit(1, TouchPhase::Move, pos2(230.0, 210.0), true));
        assert_eq!(out, vec![Event::PointerMoved(at + Vec2::new(30.0, 10.0))]);
        let (out, _) = frame(&mut g, 0.7, winit(1, TouchPhase::End, pos2(230.0, 210.0), true));
        assert!(out.is_empty(), "lifting keeps the hover: {out:?}");
        assert_eq!(g.crosshair(), Some(at + Vec2::new(30.0, 10.0)));
        // A later tap puts it away without clicking through.
        frame(&mut g, 2.0, winit(2, TouchPhase::Start, pos2(500.0, 300.0), true));
        let (out, _) = frame(&mut g, 2.1, winit(2, TouchPhase::End, pos2(500.0, 300.0), true));
        assert_eq!(out, vec![Event::PointerGone]);
        assert_eq!(g.crosshair(), None);
    }

    #[test]
    fn a_long_hold_lifted_before_any_frame_is_a_crosshair_not_a_click() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(200.0, 200.0), true));
        // The timer's frame never came: the next one brings the finger's lifting.
        let (out, _) = frame(&mut g, 1.0, winit(1, TouchPhase::End, pos2(200.0, 200.0), true));
        assert!(presses(&out).is_empty(), "{out:?}");
        assert!(g.crosshair().is_some());
        assert_eq!(g.counters.taps, 0);
    }

    #[test]
    fn the_crosshair_never_leaves_the_area_upwards() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(50.0, 20.0), true));
        frame(&mut g, 0.5, vec![]);
        assert_eq!(g.crosshair(), Some(pos2(50.0, 1.0)));
    }

    #[test]
    fn spreading_fingers_sideways_zooms_time_by_their_ratio() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(300.0, 200.0), true));
        frame(&mut g, 0.02, winit(2, TouchPhase::Start, pos2(400.0, 205.0), false));
        let (out, _) = frame(&mut g, 0.05, winit(2, TouchPhase::Move, pos2(500.0, 205.0), false));
        assert!(presses(&out).is_empty(), "a pinch never presses: {out:?}");
        assert_eq!(out[0], Event::PointerMoved(pos2(400.0, 202.5)));
        let w = wheels(&out);
        assert_eq!(w.len(), 1, "time only: {out:?}");
        assert_eq!(w[0].1, Modifiers::NONE);
        // The chart's own formula gives back the fingers' ratio (100 → 200).
        assert!(((w[0].0 * WHEEL_TIME).exp() - 2.0).abs() < 1e-3, "{w:?}");
        assert_eq!(g.counters.pinches, 1);
    }

    #[test]
    fn spreading_fingers_vertically_zooms_price_with_ctrl() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(300.0, 100.0), true));
        frame(&mut g, 0.02, winit(2, TouchPhase::Start, pos2(305.0, 200.0), false));
        let (out, _) = frame(&mut g, 0.05, winit(1, TouchPhase::Move, pos2(300.0, 50.0), true));
        let w = wheels(&out);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].1, Modifiers::CTRL);
        assert!(((w[0].0 * WHEEL_PRICE).exp() - 1.5).abs() < 1e-3, "{w:?}");
    }

    #[test]
    fn jittering_fingers_send_nothing_and_many_moves_send_one_wheel_a_frame() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(300.0, 100.0), true));
        frame(&mut g, 0.02, winit(2, TouchPhase::Start, pos2(305.0, 300.0), false));
        // A pixel of span jitter, back and forth, many times: inside the dead zone.
        for i in 0..20 {
            let y = if i % 2 == 0 { 301.0 } else { 300.0 };
            let (out, _) = frame(&mut g, 0.03 + i as f64 * 0.008, winit(2, TouchPhase::Move, pos2(305.0, y), false));
            assert!(wheels(&out).is_empty(), "{out:?}");
        }
        // Several real moves in one frame: one Ctrl wheel with all of them.
        let mut events = Vec::new();
        for y in [320.0, 340.0, 360.0, 400.0] {
            events.extend(winit(2, TouchPhase::Move, pos2(305.0, y), false));
        }
        let (out, _) = frame(&mut g, 0.3, events);
        let w = wheels(&out);
        assert_eq!(w.len(), 1, "{out:?}");
        assert!(((w[0].0 * WHEEL_PRICE).exp() - 1.5).abs() < 1e-3, "{w:?}");
    }

    #[test]
    fn with_zoom_both_axes_a_pinch_never_adds_a_ctrl_wheel_to_the_plain_one() {
        let mut g = gestures();
        g.tuning.zoom_both_axes = true;
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(300.0, 100.0), true));
        frame(&mut g, 0.02, winit(2, TouchPhase::Start, pos2(400.0, 200.0), false));
        let (out, _) = frame(&mut g, 0.05, winit(2, TouchPhase::Move, pos2(500.0, 300.0), false));
        let w = wheels(&out);
        assert_eq!(w.len(), 1, "{out:?}");
        assert_eq!(w[0].1, Modifiers::NONE);
    }

    #[test]
    fn a_drag_that_gets_a_second_finger_releases_the_button_and_pinches() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(100.0, 100.0), true));
        frame(&mut g, 0.05, winit(1, TouchPhase::Move, pos2(150.0, 100.0), true));
        let (out, _) = frame(&mut g, 0.1, winit(2, TouchPhase::Start, pos2(300.0, 100.0), false));
        assert_eq!(presses(&out), [(PointerButton::Primary, false)]);
        // One finger up: the other is ignored until it is up too.
        frame(&mut g, 0.2, winit(2, TouchPhase::End, pos2(300.0, 100.0), false));
        let (out, _) = frame(&mut g, 0.3, winit(1, TouchPhase::Move, pos2(200.0, 100.0), true));
        assert!(out.is_empty(), "{out:?}");
        let (out, _) = frame(&mut g, 0.4, winit(1, TouchPhase::End, pos2(200.0, 100.0), true));
        assert!(out.is_empty(), "{out:?}");
        // And the next touch starts afresh.
        frame(&mut g, 1.0, winit(3, TouchPhase::Start, pos2(100.0, 100.0), true));
        let (out, _) = frame(&mut g, 1.1, winit(3, TouchPhase::End, pos2(100.0, 100.0), true));
        assert_eq!(g.counters.taps, 1, "{out:?}");
    }

    #[test]
    fn touches_on_shell_buttons_or_outside_keep_egui_emulation() {
        let mut g = gestures();
        for p in [pos2(100.0, 450.0), pos2(780.0, 20.0)] {
            let start = winit(1, TouchPhase::Start, p, true);
            let (out, _) = frame(&mut g, 0.0, start.clone());
            assert_eq!(out, start[1..].to_vec());
            let end = winit(1, TouchPhase::End, p, true);
            let (out, _) = frame(&mut g, 0.1, end.clone());
            assert_eq!(out, end[1..].to_vec());
        }
        assert_eq!(g.counters, Counters::default());
    }

    #[test]
    fn a_cancelled_touch_on_a_shell_button_releases_the_button() {
        let mut g = gestures();
        let p = pos2(100.0, 450.0);
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, p, true));
        let (out, _) = frame(&mut g, 0.1, winit(1, TouchPhase::Cancel, p, true));
        assert_eq!(presses(&out), [(PointerButton::Primary, false)]);
    }

    #[test]
    fn a_hovering_stylus_passes_through_and_is_counted() {
        let mut g = gestures();
        let (out, _) = frame(&mut g, 0.0, vec![Event::PointerMoved(pos2(10.0, 10.0))]);
        assert_eq!(out, vec![Event::PointerMoved(pos2(10.0, 10.0))]);
        assert_eq!(g.counters.hovers, 1);
    }

    #[test]
    fn the_context_menu_is_a_right_click_at_the_crosshair_and_frees_the_next_tap() {
        let mut g = gestures();
        frame(&mut g, 0.0, winit(1, TouchPhase::Start, pos2(200.0, 200.0), true));
        frame(&mut g, 0.5, vec![]);
        frame(&mut g, 0.6, winit(1, TouchPhase::End, pos2(200.0, 200.0), true));
        // Lifting the finger of the long press keeps the crosshair.
        assert_eq!(g.crosshair(), Some(pos2(200.0, 200.0 - g.tuning.lift)));
        g.context_menu_at_crosshair();
        let (out, _) = frame(&mut g, 0.65, vec![]);
        assert_eq!(presses(&out), [(PointerButton::Secondary, true), (PointerButton::Secondary, false)]);
        // The next tap is a click (on the menu), not "put the crosshair away".
        frame(&mut g, 1.0, winit(2, TouchPhase::Start, pos2(220.0, 150.0), true));
        let (out, _) = frame(&mut g, 1.05, winit(2, TouchPhase::End, pos2(220.0, 150.0), true));
        assert_eq!(presses(&out), [(PointerButton::Primary, true), (PointerButton::Primary, false)]);
    }
}
