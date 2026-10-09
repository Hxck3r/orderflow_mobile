//! Soft keyboard text into egui. GameActivity keeps what the keyboard typed in
//! its own buffer (GameTextInput); winit 0.30 shows and hides the keyboard but
//! never forwards that text (it drops android-activity's `TextEvent`). The
//! shell reads the buffer every frame while a text field has focus and turns
//! each change into the key presses and text egui expects: the common start
//! stays, what was after it is erased with Backspace, the new end is typed.
//! A keyboard that corrects a word replaces it the same way.

use eframe::egui::{Event, Key, Modifiers};

/// How long a reset of the keyboard's buffer may take to show before the
/// buffer is taken as it is.
const RESET_WAIT_S: f64 = 0.5;

/// The keyboard's buffer as mirrored into egui, across frames.
///
/// Resetting the buffer goes through the Java side and lands a frame or more
/// later: a read straight after a reset still returns the previous field's
/// text, which must not be typed into the new one. So a reset is followed by
/// reads that are ignored until the buffer reads back empty.
#[derive(Debug, Default, PartialEq)]
pub enum Mirror {
    #[default]
    Off,
    Resetting {
        since: f64,
    },
    On(String),
}

/// What a frame must do with the keyboard.
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    /// Empty the keyboard's buffer.
    pub reset: bool,
    /// Events for egui.
    pub events: Vec<Event>,
    /// Whether the buffer must be read again soon (a field has focus).
    pub poll: bool,
}

impl Mirror {
    /// One frame: `focused` says whether a text field has the keyboard,
    /// `read` returns the buffer (called only when needed).
    pub fn step(&mut self, focused: bool, now: f64, read: impl FnOnce() -> String) -> Step {
        if !focused {
            // The field lost focus: its text must not outlive it in the buffer.
            let reset = *self != Mirror::Off;
            *self = Mirror::Off;
            return Step { reset, ..Step::default() };
        }
        match std::mem::take(self) {
            Mirror::Off => {
                *self = Mirror::Resetting { since: now };
                Step { reset: true, poll: true, ..Step::default() }
            }
            Mirror::Resetting { since } => {
                let text = read();
                *self = if text.is_empty() {
                    Mirror::On(text)
                } else if now - since >= RESET_WAIT_S {
                    // The reset never showed: take the buffer as the start.
                    Mirror::On(text)
                } else {
                    Mirror::Resetting { since }
                };
                Step { poll: true, ..Step::default() }
            }
            Mirror::On(before) => {
                let after = read();
                let events = diff(&before, &after);
                *self = Mirror::On(after);
                Step { events, poll: true, ..Step::default() }
            }
        }
    }
}

/// Events that turn `before` into `after`.
pub fn diff(before: &str, after: &str) -> Vec<Event> {
    let common = before.chars().zip(after.chars()).take_while(|(a, b)| a == b).count();
    let erase = before.chars().count() - common;
    let mut events = Vec::new();
    for _ in 0..erase {
        press(&mut events, Key::Backspace);
    }
    let mut typed = String::new();
    for c in after.chars().skip(common) {
        if c == '\n' {
            if !typed.is_empty() {
                events.push(Event::Text(std::mem::take(&mut typed)));
            }
            press(&mut events, Key::Enter);
        } else {
            typed.push(c);
        }
    }
    if !typed.is_empty() {
        events.push(Event::Text(typed));
    }
    events
}

fn press(events: &mut Vec<Event>, key: Key) {
    for pressed in [true, false] {
        events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed: true, .. } => Some(format!("{key:?}")),
                Event::Key { .. } => None,
                Event::Text(t) => Some(format!("'{t}'")),
                other => Some(format!("{other:?}")),
            })
            .collect()
    }

    #[test]
    fn typing_appends_the_new_end() {
        assert_eq!(keys(&diff("", "п")), ["'п'"]);
        assert_eq!(keys(&diff("пр", "при")), ["'и'"]);
    }

    #[test]
    fn erasing_presses_backspace_per_character() {
        assert_eq!(keys(&diff("цена", "цен")), ["Backspace"]);
        assert_eq!(keys(&diff("ab", "")), ["Backspace", "Backspace"]);
    }

    #[test]
    fn a_corrected_word_is_erased_back_to_where_it_differs_and_retyped() {
        assert_eq!(keys(&diff("обьём", "объём")), ["Backspace", "Backspace", "Backspace", "'ъём'"]);
    }

    #[test]
    fn a_new_line_is_the_enter_key() {
        assert_eq!(keys(&diff("76", "76012\n")), ["'012'", "Enter"]);
        assert_eq!(keys(&diff("", "\n")), ["Enter"]);
    }

    #[test]
    fn a_new_field_ignores_the_previous_fields_text_until_the_reset_lands() {
        let mut m = Mirror::default();
        let step = m.step(true, 0.0, || panic!("not read on focus"));
        assert!(step.reset && step.poll && step.events.is_empty());
        // The reset has not landed: the old text is still there and is not typed.
        let step = m.step(true, 0.04, || "BTC".into());
        assert!(step.events.is_empty() && !step.reset);
        let step = m.step(true, 0.08, String::new);
        assert!(step.events.is_empty());
        assert_eq!(m, Mirror::On(String::new()));
        assert_eq!(keys(&m.step(true, 0.12, || "7".into()).events), ["'7'"]);
    }

    #[test]
    fn a_reset_that_never_lands_takes_the_buffer_as_it_is() {
        let mut m = Mirror::default();
        m.step(true, 0.0, String::new);
        m.step(true, 0.6, || "x".into());
        assert_eq!(m, Mirror::On("x".into()));
        assert_eq!(keys(&m.step(true, 0.64, || "xy".into()).events), ["'y'"]);
    }

    #[test]
    fn losing_focus_resets_the_buffer_once() {
        let mut m = Mirror::On("abc".into());
        let step = m.step(false, 1.0, || panic!("not read without focus"));
        assert!(step.reset && !step.poll);
        assert!(!m.step(false, 1.04, || panic!()).reset);
    }

    #[test]
    fn nothing_changed_gives_nothing() {
        assert!(diff("одно", "одно").is_empty());
    }
}
