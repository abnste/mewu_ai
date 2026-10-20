// SPDX-License-Identifier: MPL-2.0
//! Optional, bounded live text for the authenticated CLI. Never a memory store.
use serde::Serialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const MAX_RUNS: usize = 32;
const MAX_TEXT: usize = 32 * 1024;
const TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StreamView {
    pub text: String,
    pub truncated: bool,
}
struct Entry {
    began: Instant,
    view: StreamView,
}
#[derive(Default)]
pub(crate) struct Streams {
    rows: HashMap<(String, String), Entry>,
}
impl Streams {
    pub fn begin(&mut self, scene: &str, run: &str, now: Instant) {
        self.rows
            .retain(|(s, _), e| s != scene && now.saturating_duration_since(e.began) < TTL);
        if self.rows.len() < MAX_RUNS {
            self.rows.insert(
                (scene.into(), run.into()),
                Entry {
                    began: now,
                    view: StreamView {
                        text: String::new(),
                        truncated: false,
                    },
                },
            );
        }
    }
    pub fn push(&mut self, scene: &str, run: &str, text: &str, now: Instant) {
        // Only an admitted run start may allocate. Late deltas never recreate an
        // evicted/revoked entry and cannot masquerade as a complete prefix.
        let Some(entry) = self.rows.get_mut(&(scene.into(), run.into())) else {
            return;
        };
        if now.saturating_duration_since(entry.began) >= TTL || entry.view.truncated {
            return;
        }
        let remaining = MAX_TEXT.saturating_sub(entry.view.text.len());
        let mut end = text.len().min(remaining);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        entry.view.text.push_str(&text[..end]);
        entry.view.truncated = end < text.len();
    }
    pub fn view(
        &mut self,
        scene: &str,
        run: &str,
        running: bool,
        now: Instant,
    ) -> Option<StreamView> {
        let key = (scene.into(), run.into());
        if !running
            || self
                .rows
                .get(&key)
                .is_some_and(|e| now.saturating_duration_since(e.began) >= TTL)
        {
            self.rows.remove(&key);
            return None;
        }
        self.rows.get(&key).map(|e| e.view.clone())
    }
    pub fn clear(&mut self) {
        self.rows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_prefix_is_bound_to_the_admitted_scene_and_run() {
        let t = Instant::now();
        let mut s = Streams::default();
        s.push("s", "r", "late", t);
        assert!(s.view("s", "r", true, t).is_none());
        s.begin("s", "r", t);
        s.push("s", "other", "wrong", t);
        s.push("other", "r", "wrong", t);
        s.push("s", "r", "你", t);
        s.push("s", "r", "好", t);
        assert_eq!(s.view("s", "r", true, t).unwrap().text, "你好");
        s.begin("s", "next", t);
        s.push("s", "r", "late", t);
        assert!(s.view("s", "r", true, t).is_none());
        assert_eq!(s.view("s", "next", true, t).unwrap().text, "");
    }
    #[test]
    fn unicode_bounds_are_explicit_and_final_messages_remain_authoritative() {
        let t = Instant::now();
        let mut s = Streams::default();
        s.begin("s", "r", t);
        s.push("s", "r", &"好".repeat(MAX_TEXT), t);
        let v = s.view("s", "r", true, t).unwrap();
        assert!(v.truncated);
        assert!(v.text.len() <= MAX_TEXT);
        let prefix = v.text;
        s.push("s", "r", "extra", t);
        assert_eq!(s.view("s", "r", true, t).unwrap().text, prefix);
        assert!(s.view("s", "r", false, t).is_none());
        s.push("s", "r", "late", t);
        assert!(s.view("s", "r", true, t).is_none());
    }
    #[test]
    fn pool_expiry_and_revocation_never_restart_a_partial_prefix() {
        let t = Instant::now();
        let mut s = Streams::default();
        for n in 0..MAX_RUNS {
            s.begin(&format!("s{n}"), "r", t);
        }
        s.begin("overflow", "r", t);
        assert_eq!(s.rows.len(), MAX_RUNS);
        s.push("overflow", "r", "lost-prefix", t);
        assert!(s.view("overflow", "r", true, t).is_none());
        assert!(s.view("s0", "r", true, t + TTL).is_none());
        s.push("s0", "r", "late", t + TTL);
        assert!(s.view("s0", "r", true, t + TTL).is_none());
        s.clear();
        s.push("s1", "r", "late", t);
        assert!(s.rows.is_empty());
    }
}
