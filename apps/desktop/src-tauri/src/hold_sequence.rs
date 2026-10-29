// SPDX-License-Identifier: MPL-2.0
//! Pure modifier hold decisions. No native input is injected by these tests.
#[derive(Default, Debug, PartialEq)]
pub struct Decision {
    pub consume: bool,
    pub pressed: bool,
    pub released: bool,
    pub clear_os_modifier: Option<u32>,
    pub replay: Vec<(u32, bool)>,
}
#[derive(Default)]
pub struct Sequence {
    pub held: Option<(u32, u64)>,
    pub forwarded: bool,
    captured: bool,
    other: std::collections::HashSet<u32>,
    suppressed_wins: std::collections::HashSet<u32>,
}
impl Sequence {
    /// A suppressed physical Win is still ours after the capture closes. It is
    /// retired by its physical release, never inferred from our OS-state reset.
    pub fn owned_modifier(&self) -> Option<u32> {
        self.held.filter(|_| !self.forwarded).map(|(key, _)| key)
    }
    pub fn owns_modifier(&self, key: u32) -> bool {
        self.owned_modifier() == Some(key) || self.suppressed_wins.contains(&key)
    }
    pub fn has_owned_modifiers(&self) -> bool {
        self.owned_modifier().is_some() || !self.suppressed_wins.is_empty()
    }
    /// An editor/gesture cancellation must not forget a physically held Win.
    /// Its OS state has already been reset, so reseeding from GetAsyncKeyState
    /// would lose it and turn a subsequent repeat into a second tap.
    pub fn cancel_owned(&mut self) {
        if self.owned_modifier().is_some() {
            self.captured = true;
        }
    }
    /// A click/scroll before the hold threshold is an ordinary Win+mouse chord.
    /// After claiming the hold, keep Windows neutral while Mewu receives input.
    pub fn mouse(&mut self, now: u64, threshold: u32) -> Decision {
        let mut decision = Decision::default();
        if let Some((key, began)) = self.held.filter(|_| !self.forwarded) {
            if self.captured || now.saturating_sub(began) >= u64::from(threshold) {
                self.captured = true;
                decision.clear_os_modifier = Some(key);
            } else {
                self.forwarded = true;
                decision.released = true;
                decision.replay.push((key, false));
            }
        }
        decision
    }
    /// Only physical keyboard keys have matching keyboard-hook releases.
    /// Mouse buttons and aggregate Shift/Ctrl/Alt aliases must not be seeded.
    pub fn physical_keyboard(key: u32) -> bool {
        (7..=254).contains(&key) && !matches!(key, 0x10..=0x12)
    }
    pub fn seed_down(&mut self, key: u32, now: u64) {
        if !Self::physical_keyboard(key) {
            return;
        }
        if matches!(key, 0x5b | 0x5c) && self.held.is_none() {
            self.held = Some((key, now));
            self.forwarded = true;
        } else {
            self.other.insert(key);
        }
    }
    /// A foreign synthetic release is not proof that our physical key was
    /// released. Keep an owned gesture through such events and auto-repeat;
    /// otherwise the next repeat can start a second tap and reopen the shell.
    pub fn observed_release(&mut self, key: u32) -> bool {
        if self.suppressed_wins.contains(&key)
            || (!self.forwarded && self.held.is_some_and(|(held, _)| held == key))
        {
            return false;
        }
        self.other.remove(&key);
        if self.held.is_some_and(|(held, _)| held == key) {
            self.held = None;
            self.forwarded = false;
            self.captured = false;
            return true;
        }
        false
    }
    /// Called by the message-loop timer, never inside LowLevelKeyboardProc.
    /// Reconcile missed releases without consulting the deliberately suppressed
    /// OS state of our own held Win, or manufacturing a new key activation.
    pub fn reconcile_releases(&mut self, is_down: impl Fn(u32) -> bool) -> bool {
        self.other
            .retain(|key| self.suppressed_wins.contains(key) || is_down(*key));
        if self.forwarded && self.held.is_some_and(|(key, _)| !is_down(key)) {
            self.held = None;
            self.forwarded = false;
            self.captured = false;
            return true;
        }
        false
    }
    pub fn key(&mut self, key: u32, down: bool, now: u64, threshold: u32) -> Decision {
        let mut decision = Decision::default();
        if !self.forwarded
            && self
                .held
                .is_some_and(|(_, began)| now.saturating_sub(began) >= u64::from(threshold))
        {
            self.captured = true;
        }
        let win = matches!(key, 0x5b | 0x5c);
        if win {
            if down {
                if self.suppressed_wins.contains(&key) {
                    decision.consume = true;
                    decision.clear_os_modifier = Some(key);
                    return decision;
                }
                if let Some((held, _)) = self.held {
                    if held == key {
                        decision.consume = !self.forwarded;
                        // Physical auto-repeat can reassert the OS modifier
                        // state even when the keyboard message is swallowed.
                        decision.clear_os_modifier = decision.consume.then_some(key);
                        return decision;
                    }
                    self.other.insert(key);
                    if self.captured && !self.forwarded {
                        self.suppressed_wins.insert(key);
                        decision.consume = true;
                        decision.clear_os_modifier = Some(key);
                        return decision;
                    }
                    if !self.forwarded {
                        decision.replay.push((held, false));
                        decision.released = true;
                        self.forwarded = true;
                    }
                    return decision;
                }
                self.forwarded = self
                    .other
                    .iter()
                    .any(|key| !self.suppressed_wins.contains(key));
                self.captured = false;
                self.held = Some((key, now));
                decision.consume = !self.forwarded;
                decision.pressed = !self.forwarded;
                decision.clear_os_modifier = decision.pressed.then_some(key);
                return decision;
            }
            if let Some((held, began)) = self.held {
                if held == key {
                    self.held = None;
                    decision.released = true;
                    decision.consume = !self.forwarded;
                    decision.clear_os_modifier = decision.consume.then_some(key);
                    if !self.forwarded
                        && !self.captured
                        && now.saturating_sub(began) < u64::from(threshold)
                    {
                        decision.replay = vec![(key, false), (key, true)];
                    }
                    self.forwarded = false;
                    self.captured = false;
                    return decision;
                }
            }
            self.other.remove(&key);
            if self.suppressed_wins.remove(&key) {
                decision.consume = true;
                decision.clear_os_modifier = Some(key);
            }
            return decision;
        }
        if down {
            self.other.insert(key);
        } else {
            self.other.remove(&key);
        }
        if down && !self.forwarded && !self.captured {
            if let Some((held, _)) = self.held {
                self.forwarded = true;
                decision.released = true;
                decision.replay.push((held, false));
            }
        }
        decision
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deferred_state_clear_cannot_intercept_a_chord_or_an_already_released_key() {
        let mut keys = Sequence::default();
        keys.key(0x5b, true, 0, 400);
        assert_eq!(keys.owned_modifier(), Some(0x5b));
        keys.key(0x43, true, 100, 400);
        assert_eq!(keys.owned_modifier(), None);
        keys.key(0x43, false, 120, 400);
        keys.key(0x5b, false, 150, 400);
        keys.key(0x5c, true, 200, 400);
        keys.key(0x5c, false, 900, 400);
        assert_eq!(keys.owned_modifier(), None);
        keys.key(0x5b, true, 1000, 400);
        keys.key(0x1b, true, 1500, 400);
        assert_eq!(keys.owned_modifier(), Some(0x5b));
    }
    #[test]
    fn keys_in_an_owned_long_hold_never_replay_win_down_to_the_system() {
        for win in [0x5b, 0x5c] {
            for ordinary in [0x1b, 0x0d, 0x08, 0xa0, 0xa2, 0x43, 0x5a, 0x25, 0x70] {
                let mut sequence = Sequence::default();
                assert!(sequence.key(win, true, 100, 400).consume);
                let ordinary_down = sequence.key(ordinary, true, 650, 400);
                assert!(
                    ordinary_down.replay.is_empty(),
                    "Win {win:x} plus key {ordinary:x} manufactured a system modifier"
                );
                assert!(!ordinary_down.consume && !ordinary_down.released);
                assert!(sequence.key(ordinary, false, 700, 400).replay.is_empty());
                let up = sequence.key(win, false, 850, 400);
                assert!(up.consume && up.released && up.replay.is_empty());
                assert!(sequence.held.is_none());
                assert!(sequence.key(win, true, 1000, 400).pressed);
            }
        }
    }
    #[test]
    fn foreign_key_up_cannot_split_an_owned_physical_hold_into_another_tap() {
        let mut sequence = Sequence::default();
        sequence.key(0x5b, true, 0, 400);
        assert!(!sequence.observed_release(0x5b));
        let repeated = sequence.key(0x5b, true, 600, 400);
        assert!(repeated.consume && !repeated.pressed && repeated.replay.is_empty());
        assert!(sequence.key(0x1b, true, 700, 400).replay.is_empty());
        sequence.key(0x1b, false, 710, 400);
        let physical_up = sequence.key(0x5b, false, 750, 400);
        assert!(physical_up.consume && physical_up.released && physical_up.replay.is_empty());
        assert!(sequence.held.is_none());
        assert!(sequence.key(0x5b, true, 800, 400).pressed);
    }
    #[test]
    fn both_win_keys_remain_balanced_when_the_second_arrives_after_the_hold() {
        for (first, second) in [(0x5b, 0x5c), (0x5c, 0x5b)] {
            let mut sequence = Sequence::default();
            sequence.key(first, true, 0, 400);
            let other = sequence.key(second, true, 600, 400);
            assert!(other.consume && other.replay.is_empty());
            assert_eq!(other.clear_os_modifier, Some(second));
            assert!(sequence.key(first, false, 800, 400).consume);
            assert!(sequence.key(second, true, 850, 400).consume);
            let second_up = sequence.key(second, false, 900, 400);
            assert!(second_up.consume && second_up.replay.is_empty());
            assert_eq!(second_up.clear_os_modifier, Some(second));
            assert!(sequence.key(first, true, 1000, 400).pressed);
        }
    }
    #[test]
    fn a_second_owned_win_survives_primary_release_until_its_own_physical_release() {
        let mut s = Sequence::default();
        s.key(0x5b, true, 0, 400);
        s.key(0x5c, true, 500, 400);
        s.key(0x5b, false, 600, 400);
        s.cancel_owned();
        assert!(s.has_owned_modifiers() && s.owns_modifier(0x5c) && !s.owns_modifier(0x5b));
        assert!(s.mouse(620, 400).replay.is_empty());
        let repeat = s.key(0x5c, true, 640, 400);
        assert_eq!(repeat.clear_os_modifier, Some(0x5c));
        assert!(repeat.consume && !repeat.pressed);
        assert!(s.key(0x5c, false, 700, 400).consume);
        assert!(!s.has_owned_modifiers());
    }
    #[test]
    fn a_claimed_hold_cannot_become_a_short_tap_if_configuration_changes() {
        let mut sequence = Sequence::default();
        sequence.key(0x5b, true, 0, 400);
        assert!(sequence.key(0x1b, true, 500, 400).replay.is_empty());
        sequence.key(0x1b, false, 550, 400);
        assert!(sequence.key(0x5b, false, 700, 1000).replay.is_empty());
    }
    #[test]
    fn startup_mouse_buttons_and_generic_modifier_aliases_cannot_disable_holds_forever() {
        for key in [0x01, 0x02, 0x04, 0x05, 0x06, 0x10, 0x11, 0x12] {
            let mut s = Sequence::default();
            s.seed_down(key, 0);
            let down = s.key(0x5b, true, 1000, 400);
            assert!(
                down.consume && down.pressed,
                "stale non-physical keyboard seed {key:x}"
            );
        }
    }
    #[test]
    fn missed_and_injected_releases_retire_only_os_visible_state() {
        let mut s = Sequence::default();
        s.seed_down(0xa2, 0);
        s.reconcile_releases(|_| false);
        assert!(s.key(0x5b, true, 100, 400).pressed);
        assert!(!s.reconcile_releases(|_| false));
        assert!(!s.key(0x5b, true, 450, 400).pressed);
        assert!(s.key(0x5b, false, 500, 400).replay.is_empty());
        let mut held = Sequence::default();
        held.seed_down(0xa2, 0);
        held.reconcile_releases(|key| key == 0xa2);
        assert!(!held.key(0x5b, true, 100, 400).consume);
        let mut stale_win = Sequence::default();
        stale_win.seed_down(0x5b, 0);
        assert!(stale_win.observed_release(0x5b));
        assert!(stale_win.key(0x5b, true, 100, 400).pressed);
        let mut canceled = Sequence::default();
        canceled.key(0x5b, true, 0, 400);
        canceled.key(0x41, true, 10, 400);
        assert!(canceled.reconcile_releases(|_| false));
        assert!(canceled.key(0x5b, true, 500, 400).pressed);
    }
    #[test]
    fn short_tap_is_replayed_once_and_long_hold_never_opens_start() {
        for key in [0x5b, 0x5c] {
            let mut s = Sequence::default();
            let first = s.key(key, true, 100, 400);
            assert!(first.consume && first.pressed);
            assert_eq!(first.clear_os_modifier, Some(key));
            assert_eq!(
                s.key(key, true, 150, 400),
                Decision {
                    consume: true,
                    clear_os_modifier: Some(key),
                    ..Decision::default()
                }
            );
            let tap = s.key(key, false, 250, 400);
            assert!(tap.consume && tap.released);
            assert_eq!(tap.replay, vec![(key, false), (key, true)]);
            assert_eq!(s.key(key, false, 251, 400), Decision::default());
            s.key(key, true, 1000, 400);
            let long = s.key(key, false, 1400, 400);
            assert!(long.consume && long.released);
            assert!(long.replay.is_empty());
        }
    }
    #[test]
    fn owned_win_state_is_cleared_after_every_physical_down_and_at_final_up() {
        let mut keys = Sequence::default();
        assert_eq!(keys.key(0x5b, true, 0, 400).clear_os_modifier, Some(0x5b));
        assert_eq!(keys.key(0x5b, true, 100, 400).clear_os_modifier, Some(0x5b));
        let chord = keys.key(0x43, true, 200, 400);
        assert_eq!(chord.clear_os_modifier, None);
        assert_eq!(chord.replay, vec![(0x5b, false)]);
        assert_eq!(keys.key(0x5b, false, 500, 400).clear_os_modifier, None);
        let mut already_down = Sequence::default();
        already_down.seed_down(0x5b, 0);
        assert_eq!(
            already_down.key(0x5b, true, 100, 400).clear_os_modifier,
            None
        );
        let mut captured = Sequence::default();
        captured.key(0x5b, true, 0, 400);
        assert_eq!(
            captured.key(0x5b, false, 600, 400).clear_os_modifier,
            Some(0x5b)
        );
    }
    #[test]
    fn existing_and_new_windows_chords_preserve_modifier_order_and_cancel_hold() {
        let mut s = Sequence::default();
        s.key(0x5b, true, 0, 400);
        let chord = s.key(0x53, true, 30, 400);
        assert!(!chord.consume && chord.released);
        assert_eq!(chord.replay, vec![(0x5b, false)]);
        assert!(!s.key(0x53, false, 50, 400).consume);
        assert!(!s.key(0x5b, false, 60, 400).consume);
        for held in [0x10, 0x11, 0x12, 0x41] {
            let mut s = Sequence::default();
            s.key(held, true, 0, 400);
            let win = s.key(0x5b, true, 10, 400);
            assert!(!win.consume && !win.pressed);
            assert!(!s.key(0x5b, false, 500, 400).consume);
        }
    }
    #[test]
    fn two_windows_keys_do_not_leave_a_suppressed_modifier_or_trigger_a_second_hold() {
        let mut s = Sequence::default();
        s.key(0x5b, true, 0, 400);
        let second = s.key(0x5c, true, 50, 400);
        assert!(second.released && !second.consume);
        assert_eq!(second.replay, vec![(0x5b, false)]);
        assert!(!s.key(0x5b, false, 600, 400).consume);
        assert!(!s.key(0x5c, false, 601, 400).consume);
        assert!(s.held.is_none());
        assert!(s.key(0x5b, true, 1000, 400).pressed);
    }
    #[test]
    fn modifier_already_down_when_transport_starts_is_never_replayed_or_suppressed() {
        let mut s = Sequence::default();
        s.seed_down(0x5b, 0);
        assert_eq!(s.key(0x5b, false, 1000, 400).replay, vec![]);
        let mut s = Sequence::default();
        s.seed_down(0xa0, 0);
        assert!(!s.key(0x5b, true, 1, 400).consume);
        s.key(0x5b, false, 2, 400);
        s.key(0xa0, false, 3, 400);
        assert!(s.key(0x5b, true, 1000, 400).pressed);
    }
    #[test]
    fn every_repeat_is_reset_without_dispatching_another_hold_or_tap() {
        for win in [0x5b, 0x5c] {
            let mut s = Sequence::default();
            s.key(win, true, 0, 400);
            for time in [30, 200, 399, 400, 450, 900, 1100] {
                let repeat = s.key(win, true, time, 400);
                assert!(repeat.consume && !repeat.pressed && !repeat.released);
                assert_eq!(repeat.clear_os_modifier, Some(win));
                assert!(repeat.replay.is_empty());
                assert!(s.owns_modifier(win));
            }
            let click = s.mouse(1200, 400);
            assert_eq!(click.clear_os_modifier, Some(win));
            assert!(click.replay.is_empty() && !click.released);
            let up = s.key(win, false, 1250, 400);
            assert!(up.consume && up.replay.is_empty());
            assert!(!s.owns_modifier(win) && !s.has_owned_modifiers());
        }
    }
    #[test]
    fn short_win_mouse_chords_restore_the_modifier_once_and_cancel_the_hold() {
        for win in [0x5b, 0x5c] {
            for delay in [0, 10, 200, 399] {
                let mut s = Sequence::default();
                s.key(win, true, 0, 400);
                let click = s.mouse(delay, 400);
                assert_eq!(click.replay, vec![(win, false)]);
                assert!(click.released && !click.consume && click.clear_os_modifier.is_none());
                assert!(
                    !s.owns_modifier(win),
                    "a pending clear must not break this chord"
                );
                assert_eq!(s.mouse(delay + 1, 400), Decision::default());
                assert!(!s.key(win, false, delay + 20, 400).consume);
                assert!(!s.has_owned_modifiers());
            }
        }
    }
    #[test]
    fn ordinary_windows_chords_have_one_original_modifier_and_no_synthetic_release() {
        // Win+E/R/D/V/Tab/arrows and Win+Shift+S, from both modifier orders.
        for win in [0x5b, 0x5c] {
            for key in [
                0x45, 0x52, 0x44, 0x56, 0x09, 0x25, 0x26, 0x27, 0x28, 0xa0, 0xa2, 0xa4,
            ] {
                let mut s = Sequence::default();
                s.key(win, true, 0, 400);
                let chord = s.key(key, true, 80, 400);
                assert_eq!(chord.replay, vec![(win, false)]);
                assert!(chord.released && !chord.consume && chord.clear_os_modifier.is_none());
                assert!(!s.owns_modifier(win));
                assert_eq!(s.key(win, true, 900, 400), Decision::default());
                assert!(!s.key(key, false, 1000, 400).consume);
                assert!(!s.key(win, false, 1010, 400).consume);
                let mut reverse = Sequence::default();
                reverse.key(key, true, 0, 400);
                assert_eq!(reverse.key(win, true, 10, 400), Decision::default());
                assert!(reverse.key(key, false, 20, 400).replay.is_empty());
                assert!(reverse.key(win, false, 30, 400).replay.is_empty());
            }
        }
    }
    #[test]
    fn editor_cancellation_keeps_ownership_through_repeat_escape_and_release() {
        for win in [0x5b, 0x5c] {
            let mut s = Sequence::default();
            s.key(win, true, 0, 400);
            s.cancel_owned();
            assert!(s.has_owned_modifiers());
            let repeat = s.key(win, true, 50, 400);
            assert!(repeat.consume && !repeat.pressed && repeat.replay.is_empty());
            assert_eq!(repeat.clear_os_modifier, Some(win));
            assert_eq!(s.key(0x1b, true, 60, 400), Decision::default());
            s.key(0x1b, false, 70, 400);
            assert!(s.key(win, false, 80, 400).replay.is_empty());
            assert!(!s.has_owned_modifiers());
            assert_eq!(s.key(0x41, true, 90, 400), Decision::default());
            assert!(s.key(win, true, 100, 400).replay.is_empty());
        }
    }
}
