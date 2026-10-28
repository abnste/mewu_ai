// SPDX-License-Identifier: MPL-2.0
//! Bounded display-only batching. The complete command log is written separately.
use std::{collections::VecDeque, time::Duration};

const INTERVAL: Duration = Duration::from_millis(40);
const MAX_PARTS: usize = 16;

#[derive(Default)]
pub(crate) struct Preview {
    parts: VecDeque<(&'static str, String)>,
    bytes: usize,
    last_sent: Duration,
}
impl Preview {
    pub fn push(&mut self, stream: &'static str, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.parts.back_mut() {
            Some((last, value)) if *last == stream => value.push_str(text),
            _ => self.parts.push_back((stream, text.into())),
        }
        self.bytes += text.len();
        while self.parts.len() > MAX_PARTS {
            self.bytes -= self.parts.pop_front().unwrap().1.len();
        }
        while self.bytes > mewu_agent_tools::MAX_OUTPUT {
            let (_, front) = self.parts.front_mut().unwrap();
            let mut remove = (self.bytes - mewu_agent_tools::MAX_OUTPUT).min(front.len());
            while !front.is_char_boundary(remove) {
                remove += 1;
            }
            front.drain(..remove);
            self.bytes -= remove;
            if front.is_empty() {
                self.parts.pop_front();
            }
        }
    }
    pub fn due(&mut self, now: Duration, emit: impl FnMut(&str, &str)) {
        if now.saturating_sub(self.last_sent) >= INTERVAL {
            self.finish(now, emit);
        }
    }
    pub fn finish(&mut self, now: Duration, mut emit: impl FnMut(&str, &str)) {
        for (stream, text) in self.parts.drain(..) {
            emit(stream, &text);
        }
        self.bytes = 0;
        self.last_sent = now;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn many_tiny_reads_do_not_create_a_webview_event_for_every_line() {
        let mut p = Preview::default();
        let mut messages = Vec::new();
        for i in 0..15000 {
            p.push("stdout", &format!("LOG_{i}\n"));
            p.due(Duration::from_millis(30), |s, t| {
                messages.push((s.to_string(), t.to_string()))
            });
        }
        assert!(messages.is_empty());
        p.finish(Duration::from_millis(35), |s, t| {
            messages.push((s.to_string(), t.to_string()))
        });
        assert_eq!(messages.len(), 1);
        assert!(messages[0].1.ends_with("LOG_14999\n"));
        assert!(messages[0].1.len() <= mewu_agent_tools::MAX_OUTPUT);
    }
    #[test]
    fn retained_stream_order_and_unicode_boundaries_survive_trimming() {
        let mut p = Preview::default();
        p.push("stdout", &"界🙂".repeat(4000));
        p.push("stderr", "警告\n");
        p.push("stdout", "done\n");
        let mut messages = Vec::new();
        p.due(Duration::from_millis(40), |s, t| {
            messages.push((s.to_string(), t.to_string()))
        });
        assert_eq!(
            messages.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
            ["stdout", "stderr", "stdout"]
        );
        assert!(messages.iter().map(|v| v.1.len()).sum::<usize>() <= mewu_agent_tools::MAX_OUTPUT);
        assert_eq!(messages[1].1, "警告\n");
        assert_eq!(messages[2].1, "done\n");
    }
    #[test]
    fn alternating_streams_are_bounded_and_empty_ticks_emit_nothing() {
        let mut p = Preview::default();
        let mut messages = Vec::new();
        for i in 0..1000 {
            p.push(
                if i % 2 == 0 { "stdout" } else { "stderr" },
                &format!("{i}"),
            );
        }
        p.finish(Duration::ZERO, |s, t| {
            messages.push((s.to_string(), t.to_string()))
        });
        assert_eq!(messages.len(), MAX_PARTS);
        assert_eq!(messages.last().unwrap().1, "999");
        p.due(Duration::from_secs(1), |_, _| panic!("no pending output"));
    }
}
