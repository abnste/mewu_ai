// SPDX-License-Identifier: MPL-2.0
//! Pure modifier hold decisions. No native input is injected by these tests.
#[derive(Default, Debug, PartialEq)]
pub struct Decision {
    pub consume: bool,
    pub pressed: bool,
    pub released: bool,
    pub replay: Vec<(u32, bool)>,
}
#[derive(Default)]
pub struct Sequence {
    pub held: Option<(u32, u64)>,
    pub forwarded: bool,
    other: std::collections::HashSet<u32>,
}
impl Sequence {
    pub fn seed_down(&mut self, key: u32, now: u64) {
        if matches!(key, 0x5b | 0x5c) && self.held.is_none() {
            self.held = Some((key, now));
            self.forwarded = true;
        } else {
            self.other.insert(key);
        }
    }
    pub fn key(&mut self, key: u32, down: bool, now: u64, threshold: u32) -> Decision {
        let mut decision = Decision::default();
        let win = matches!(key, 0x5b | 0x5c);
        if win {
            if down {
                if let Some((held, _)) = self.held {
                    if held == key {
                        decision.consume = !self.forwarded;
                        return decision;
                    }
                    self.other.insert(key);
                    if !self.forwarded {
                        decision.replay.push((held, false));
                        decision.released = true;
                        self.forwarded = true;
                    }
                    return decision;
                }
                self.forwarded = !self.other.is_empty();
                self.held = Some((key, now));
                decision.consume = !self.forwarded;
                decision.pressed = !self.forwarded;
                return decision;
            }
            if let Some((held, began)) = self.held {
                if held == key {
                    self.held = None;
                    decision.released = true;
                    decision.consume = !self.forwarded;
                    if !self.forwarded && now.saturating_sub(began) < u64::from(threshold) {
                        decision.replay = vec![(key, false), (key, true)];
                    }
                    self.forwarded = false;
                    return decision;
                }
            }
            self.other.remove(&key);
            return decision;
        }
        if down {
            self.other.insert(key);
        } else {
            self.other.remove(&key);
        }
        if down && !self.forwarded {
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
    fn short_tap_is_replayed_once_and_long_hold_never_opens_start() {
        for key in [0x5b, 0x5c] {
            let mut s = Sequence::default();
            let first = s.key(key, true, 100, 400);
            assert!(first.consume && first.pressed);
            assert_eq!(
                s.key(key, true, 150, 400),
                Decision {
                    consume: true,
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
        s.seed_down(0x10, 0);
        assert!(!s.key(0x5b, true, 1, 400).consume);
        s.key(0x5b, false, 2, 400);
        s.key(0x10, false, 3, 400);
        assert!(s.key(0x5b, true, 1000, 400).pressed);
    }
}
