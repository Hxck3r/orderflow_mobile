//! Soft keyboard text into egui. GameActivity keeps what the keyboard typed in
//! its own buffer (GameTextInput); winit 0.30 shows and hides the keyboard but
//! never forwards that text (it drops android-activity's `TextEvent`). The
//! shell reads the buffer every frame while a text field has focus and turns
//! each change into the key presses and text egui expects: the common start
//! stays, what was after it is erased with Backspace, the new end is typed.
//! A keyboard that corrects a word replaces it the same way.

use eframe::egui::{Event, Key, Modifiers};

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
    fn nothing_changed_gives_nothing() {
        assert!(diff("одно", "одно").is_empty());
    }
}
