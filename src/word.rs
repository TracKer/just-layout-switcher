//! Remembers the keys of the word being typed, so it can be typed again in
//! another layout.
//!
//! Keys, not characters, are kept: the same physical keys pressed in the
//! other layout give the intended text. The buffer holds the last word and
//! the spaces typed after it. It is forgotten when the caret may have moved
//! or the text was changed some other way: navigation and editing keys,
//! shortcuts with Ctrl, Alt or Win, mouse clicks, another window.

use crate::keys;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

/// More than any real word; past this the oldest keys are dropped.
const MAX_KEYS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub vk: u32,
    pub scan: u32,
    pub extended: bool,
    pub shift: bool,
}

#[derive(Default)]
pub struct Word {
    keys: Vec<Key>,
    /// The window the keys were typed in.
    window: isize,
}

impl Word {
    /// Takes a key press. `window` is the foreground window; `shortcut` is
    /// true if Ctrl, Alt or Win is held.
    pub fn key_down(&mut self, key: Key, window: isize, shortcut: bool) {
        if keys::is_modifier(keys::normalize(key.vk)) || key.vk == VK_CAPITAL as u32 {
            return;
        }
        if window != self.window {
            self.keys.clear();
            self.window = window;
        }
        if shortcut {
            self.keys.clear();
        } else if key.vk == VK_BACK as u32 {
            self.keys.pop();
        } else if is_text(key.vk) {
            // A letter after a space starts a new word.
            if key.vk != VK_SPACE as u32 && self.keys.last().is_some_and(|k| k.vk == VK_SPACE as u32) {
                self.keys.clear();
            }
            if self.keys.len() == MAX_KEYS {
                self.keys.remove(0);
            }
            self.keys.push(key);
        } else {
            self.keys.clear();
        }
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    /// The keys of the last word and the spaces after it, if they were
    /// typed in `window` and there is a word at all.
    pub fn keys(&self, window: isize) -> Option<&[Key]> {
        (window == self.window && self.keys.iter().any(|k| k.vk != VK_SPACE as u32)).then_some(&self.keys)
    }
}

/// Keys that type a character in some layout: letters, digits, punctuation
/// (a letter in another layout) and the numeric keypad.
fn is_text(vk: u32) -> bool {
    matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: isize = 1;

    fn key(vk: u8) -> Key {
        Key { vk: vk as u32, scan: 0, extended: false, shift: false }
    }

    fn typed(word: &mut Word, text: &str) {
        for c in text.bytes() {
            word.key_down(key(c), WINDOW, false);
        }
    }

    fn text(word: &Word) -> String {
        word.keys(WINDOW).map_or(String::new(), |keys| keys.iter().map(|k| k.vk as u8 as char).collect())
    }

    #[test]
    fn keeps_the_last_word_with_the_spaces_after_it() {
        let mut word = Word::default();
        typed(&mut word, "AB CD  ");
        assert_eq!(text(&word), "CD  ");
        typed(&mut word, "E");
        assert_eq!(text(&word), "E");
    }

    #[test]
    fn backspace_removes_the_last_key() {
        let mut word = Word::default();
        typed(&mut word, "ABC");
        word.key_down(key(VK_BACK as u8), WINDOW, false);
        assert_eq!(text(&word), "AB");
    }

    #[test]
    fn other_keys_shortcuts_and_windows_forget_the_word() {
        let mut word = Word::default();
        typed(&mut word, "AB");
        word.key_down(key(VK_LEFT as u8), WINDOW, false);
        assert_eq!(text(&word), "");

        typed(&mut word, "AB");
        word.key_down(key(b'C'), WINDOW, true);
        assert_eq!(text(&word), "");

        typed(&mut word, "AB");
        word.key_down(key(b'C'), 2, false);
        assert_eq!(word.keys(WINDOW), None);
        assert_eq!(word.keys(2).map(<[Key]>::len), Some(1));
    }

    #[test]
    fn modifiers_alone_change_nothing() {
        let mut word = Word::default();
        typed(&mut word, "AB");
        word.key_down(key(VK_LSHIFT as u8), WINDOW, false);
        word.key_down(key(VK_LCONTROL as u8), WINDOW, true);
        assert_eq!(text(&word), "AB");
    }

    #[test]
    fn only_spaces_are_no_word() {
        let mut word = Word::default();
        typed(&mut word, "  ");
        assert_eq!(word.keys(WINDOW), None);
    }
}
