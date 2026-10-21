//! Recognizes the switch hotkeys in the stream of keyboard events.
//!
//! A combination of modifiers only (e.g. "Alt + Shift") fires when one of its
//! keys is released, provided that exactly these keys were held and nothing
//! else was pressed (or clicked) in between, so Ctrl+Shift+Left or
//! Shift+click don't switch the layout. A combination with an ordinary key
//! (e.g. "Ctrl + Alt + L") fires on that key's press; tapping the key again
//! while the modifiers are still held fires again. Auto-repeat never fires.
//!
//! The matcher works on raw virtual keys as the hook sees them and compares
//! their normalized set (left/right variants merged) with the hotkey.

use crate::keys;

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Index of the hotkey that fired.
    pub fire: Option<usize>,
    /// Don't pass the event on.
    pub swallow: bool,
    /// Inject a dummy key so that the held Alt/Win (or the system language
    /// hotkey) doesn't react to the modifiers being released on their own.
    pub mask: bool,
    /// The swallowed key-up has to be sent again after the mask key, so the
    /// system doesn't consider the key still held.
    pub replay: bool,
    /// The hotkey fired again while its modifiers stayed held since the
    /// previous time (Ctrl+Alt held, L tapped again).
    pub continues: bool,
}

#[derive(Default)]
pub struct Matcher {
    /// Normalized keys of each hotkey, sorted.
    hotkeys: Vec<Vec<u32>>,
    block: bool,
    /// Raw keys currently held.
    held: Vec<u32>,
    /// Raw keys whose key-down was swallowed; their repeats and key-up are
    /// swallowed too.
    swallowed: Vec<u32>,
    /// A modifier-only hotkey is held with nothing else pressed since.
    armed: Option<usize>,
    /// A hotkey with an ordinary key fired and its modifiers are still held.
    series: bool,
}

impl Matcher {
    /// Sets the hotkeys (normalized keys, see `keys::parse`) and whether a
    /// triggered hotkey is kept from the system and the active application.
    pub fn set(&mut self, hotkeys: Vec<Vec<u32>>, block: bool) {
        self.hotkeys = hotkeys;
        self.block = block;
        self.armed = None;
        self.series = false;
    }

    /// True if some hotkey fires on release, so mouse clicks matter.
    pub fn fires_on_release(&self) -> bool {
        (0..self.hotkeys.len()).any(|i| self.on_release(i))
    }

    /// Forgets the keyboard state, e.g. when events were not seen.
    pub fn reset(&mut self) {
        self.held.clear();
        self.swallowed.clear();
        self.armed = None;
        self.series = false;
    }

    /// Drops keys that are no longer down although their key-up was missed
    /// (another hook swallowed it, the hook was removed for a while...).
    pub fn forget_released(&mut self, is_down: impl Fn(u32) -> bool) {
        let swallowed = &self.swallowed;
        self.held.retain(|&vk| swallowed.contains(&vk) || is_down(vk));
    }

    /// Raw keys currently held, as the hook saw them.
    pub fn held(&self) -> &[u32] {
        &self.held
    }

    pub fn mouse_down(&mut self) {
        self.armed = None;
    }

    pub fn key_down(&mut self, vk: u32) -> Outcome {
        if self.swallowed.contains(&vk) {
            return Outcome { swallow: true, ..Default::default() };
        }
        let repeat = self.held.contains(&vk);
        if !repeat {
            self.held.push(vk);
        }

        let held = self.held_set();
        let Some(index) = self.hotkeys.iter().position(|h| *h == held) else {
            self.armed = None;
            return Outcome::default();
        };
        if self.on_release(index) {
            // Auto-repeat keeps the hotkey armed but doesn't arm it again
            // after it was cancelled.
            self.armed = (self.armed == Some(index) || !repeat).then_some(index);
            return Outcome::default();
        }
        self.armed = None;
        if repeat || keys::is_modifier(keys::normalize(vk)) {
            return Outcome::default();
        }
        let continues = std::mem::replace(&mut self.series, true);
        if !self.block {
            return Outcome { fire: Some(index), continues, ..Default::default() };
        }
        self.swallowed.push(vk);
        let mask = held.iter().any(|&k| k == keys::VK_ALT || k == keys::VK_WIN);
        Outcome { fire: Some(index), swallow: true, mask, replay: false, continues }
    }

    pub fn key_up(&mut self, vk: u32) -> Outcome {
        self.held.retain(|&k| k != vk);
        let normalized = keys::normalize(vk);
        if keys::is_modifier(normalized) && !self.held_set().contains(&normalized) {
            self.series = false;
        }
        if let Some(pos) = self.swallowed.iter().position(|&k| k == vk) {
            self.swallowed.remove(pos);
            return Outcome { swallow: true, ..Default::default() };
        }
        let Some(index) = self.armed.take() else {
            return Outcome::default();
        };
        if !self.hotkeys[index].contains(&normalized) {
            return Outcome::default();
        }
        if !self.block {
            return Outcome { fire: Some(index), ..Default::default() };
        }
        Outcome { fire: Some(index), swallow: true, mask: true, replay: true, continues: false }
    }

    fn on_release(&self, index: usize) -> bool {
        self.hotkeys[index].iter().all(|&vk| keys::is_modifier(vk))
    }

    fn held_set(&self) -> Vec<u32> {
        let mut set: Vec<_> = self.held.iter().map(|&vk| keys::normalize(vk)).collect();
        set.sort_unstable();
        set.dedup();
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

    const LCTRL: u32 = VK_LCONTROL as u32;
    const LALT: u32 = VK_LMENU as u32;
    const LSHIFT: u32 = VK_LSHIFT as u32;
    const RSHIFT: u32 = VK_RSHIFT as u32;
    const L: u32 = b'L' as u32;
    const A: u32 = b'A' as u32;

    fn matcher(hotkey: &str, block: bool) -> Matcher {
        let mut m = Matcher::default();
        m.set(keys::parse(hotkey).into_iter().collect(), block);
        m
    }

    fn fired(o: Outcome) -> bool {
        o.fire.is_some()
    }

    #[test]
    fn modifier_hotkey_fires_on_release() {
        let mut m = matcher("Alt + Shift", false);
        assert!(!fired(m.key_down(LALT)));
        assert!(!fired(m.key_down(LSHIFT)));
        assert!(!fired(m.key_down(LSHIFT))); // auto-repeat
        assert_eq!(m.key_up(LSHIFT), Outcome { fire: Some(0), ..Default::default() });
        assert!(!fired(m.key_up(LALT)));
    }

    #[test]
    fn modifier_hotkey_is_cancelled_by_another_key_or_click() {
        let mut m = matcher("Ctrl + Shift", false);
        m.key_down(LCTRL);
        m.key_down(LSHIFT);
        m.key_down(VK_LEFT as u32);
        m.key_up(VK_LEFT as u32);
        assert!(!fired(m.key_up(LSHIFT)));
        m.key_up(LCTRL);

        m.key_down(LCTRL);
        m.key_down(LSHIFT);
        m.mouse_down();
        assert!(!fired(m.key_up(LCTRL)));
    }

    #[test]
    fn modifier_hotkey_needs_exactly_its_keys() {
        let mut m = matcher("Ctrl + Shift", false);
        m.key_down(LCTRL);
        m.key_down(LALT);
        m.key_down(LSHIFT);
        assert!(!fired(m.key_up(LSHIFT)));
    }

    #[test]
    fn left_and_right_modifiers_are_the_same() {
        let mut m = matcher("Alt + Shift", false);
        m.key_down(LALT);
        m.key_down(RSHIFT);
        assert!(fired(m.key_up(LALT)));
    }

    #[test]
    fn blocked_modifier_hotkey_masks_and_replays_the_release() {
        let mut m = matcher("Alt + Shift", true);
        m.key_down(LALT);
        m.key_down(LSHIFT);
        assert_eq!(m.key_up(LSHIFT), Outcome { fire: Some(0), swallow: true, mask: true, replay: true, continues: false });
        assert_eq!(m.key_up(LALT), Outcome::default());
    }

    #[test]
    fn key_hotkey_fires_on_each_tap_while_modifiers_are_held() {
        let mut m = matcher("Ctrl + Alt + L", false);
        m.key_down(LCTRL);
        m.key_down(LALT);
        assert!(fired(m.key_down(L)));
        assert!(!fired(m.key_down(L))); // auto-repeat
        assert!(!fired(m.key_up(L)));
        assert!(fired(m.key_down(L)));
        m.key_up(L);
        assert!(!fired(m.key_up(LALT)));
    }

    #[test]
    fn key_hotkey_continues_while_modifiers_are_held() {
        let mut m = matcher("Ctrl + Alt + L", false);
        m.key_down(LCTRL);
        m.key_down(LALT);
        assert!(!m.key_down(L).continues);
        m.key_up(L);
        assert!(m.key_down(L).continues);
        m.key_up(L);
        m.key_up(LALT);
        m.key_down(LALT);
        let o = m.key_down(L);
        assert!(o.fire.is_some() && !o.continues);
    }

    #[test]
    fn modifier_hotkey_never_continues() {
        let mut m = matcher("Alt + Shift", false);
        for _ in 0..2 {
            m.key_down(LALT);
            m.key_down(LSHIFT);
            assert!(!m.key_up(LSHIFT).continues);
            m.key_up(LALT);
        }
    }

    #[test]
    fn key_hotkey_needs_exactly_its_keys() {
        let mut m = matcher("Ctrl + L", false);
        m.key_down(LCTRL);
        m.key_down(LSHIFT);
        assert!(!fired(m.key_down(L)));
        m.key_up(L);
        m.key_up(LSHIFT);
        assert!(!fired(m.key_down(A)));
    }

    #[test]
    fn blocked_key_hotkey_swallows_the_key_and_masks_alt() {
        let mut m = matcher("Ctrl + Alt + L", true);
        m.key_down(LCTRL);
        m.key_down(LALT);
        assert_eq!(m.key_down(L), Outcome { fire: Some(0), swallow: true, mask: true, replay: false, continues: false });
        assert_eq!(m.key_down(L), Outcome { swallow: true, ..Default::default() });
        assert_eq!(m.key_up(L), Outcome { swallow: true, ..Default::default() });
        assert_eq!(m.key_up(LALT), Outcome::default());
    }

    #[test]
    fn blocked_key_hotkey_without_alt_or_win_needs_no_mask() {
        let mut m = matcher("Ctrl + L", true);
        m.key_down(LCTRL);
        assert_eq!(m.key_down(L), Outcome { fire: Some(0), swallow: true, mask: false, replay: false, continues: false });
    }

    #[test]
    fn single_key_hotkey() {
        let mut m = matcher("CapsLock", true);
        let caps = VK_CAPITAL as u32;
        assert!(m.key_down(caps).swallow);
        assert!(m.key_up(caps).swallow);
        // Not while a modifier is held.
        m.key_down(LSHIFT);
        assert!(!fired(m.key_down(caps)));
    }

    #[test]
    fn missed_key_ups_are_forgotten() {
        let mut m = matcher("Ctrl + L", false);
        m.key_down(LSHIFT); // its key-up is never seen
        m.forget_released(|_| false);
        m.key_down(LCTRL);
        assert!(fired(m.key_down(L)));
    }

    #[test]
    fn several_hotkeys_report_which_one_fired() {
        let mut m = Matcher::default();
        let hotkeys = ["Ctrl + Shift", "Ctrl + Shift + 1", "Ctrl + 2"];
        m.set(hotkeys.iter().filter_map(|h| keys::parse(h)).collect(), false);
        let one = b'1' as u32;
        let two = b'2' as u32;

        m.key_down(LCTRL);
        m.key_down(LSHIFT);
        assert_eq!(m.key_down(one).fire, Some(1));
        m.key_up(one);
        // Ctrl+Shift was cancelled by the 1 in between.
        assert_eq!(m.key_up(LSHIFT).fire, None);
        assert_eq!(m.key_down(two).fire, Some(2));
        m.key_up(two);
        m.key_up(LCTRL);

        m.key_down(LCTRL);
        m.key_down(LSHIFT);
        assert_eq!(m.key_up(LCTRL).fire, Some(0));
    }

    #[test]
    fn no_hotkey_never_fires() {
        let mut m = matcher("", true);
        assert_eq!(m.key_down(LCTRL), Outcome::default());
        assert_eq!(m.key_up(LCTRL), Outcome::default());
    }
}
