// SPDX-License-Identifier: MPL-2.0
//! Bound an unfinished SSE frame before the event parser can buffer it. JSON
//! remains the protocol adapter's job. CR, LF and split CRLF follow WHATWG SSE.
const MAX_WIRE_BYTES: usize = 32 * 1024 * 1024;
const FRAME_OVERHEAD: usize = 64 * 1024;

pub(crate) struct SseBudget {
    wire_bytes: usize,
    frame_bytes: usize,
    frame_limit: usize,
    line_empty: bool,
    previous_cr: bool,
}

impl SseBudget {
    pub fn new(max_data_bytes: usize) -> Self {
        Self {
            wire_bytes: 0,
            frame_bytes: 0,
            frame_limit: max_data_bytes.saturating_add(FRAME_OVERHEAD),
            line_empty: true,
            previous_cr: false,
        }
    }

    pub fn accept(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        self.wire_bytes = self.wire_bytes.saturating_add(bytes.len());
        if self.wire_bytes > MAX_WIRE_BYTES {
            return Err("回复数据超过上限");
        }
        for &byte in bytes {
            // CRLF is one delimiter even if its bytes arrive in different chunks.
            if self.previous_cr && byte == b'\n' {
                self.previous_cr = false;
                // Count both wire bytes unless the CR already ended the frame.
                if self.frame_bytes != 0 {
                    self.frame_bytes += 1;
                    if self.frame_bytes > self.frame_limit {
                        return Err("回复事件过大");
                    }
                }
                continue;
            }
            self.previous_cr = false;
            self.frame_bytes += 1;
            if matches!(byte, b'\r' | b'\n') {
                if self.line_empty {
                    self.frame_bytes = 0;
                }
                self.line_empty = true;
                self.previous_cr = byte == b'\r';
            } else {
                self.line_empty = false;
            }
            if self.frame_bytes > self.frame_limit {
                return Err("回复事件过大");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unfinished_lines_and_multiline_frames_are_bounded_before_dispatch() {
        for fragment in [&b"x"[..], &b"data: x\n"[..], &b": comment\r\n"[..]] {
            let mut budget = SseBudget::new(16);
            let mut accepted = 0;
            while budget.accept(fragment).is_ok() {
                accepted += fragment.len();
            }
            assert!(accepted <= 16 + FRAME_OVERHEAD + fragment.len());
            assert!(accepted < MAX_WIRE_BYTES);
        }
    }
    #[test]
    fn framing_matches_all_line_endings_at_every_byte_boundary() {
        for eol in ["\n", "\r", "\r\n"] {
            let frame = format!("data: {}{eol}{eol}", "🙂".repeat(9000));
            let frames = frame.repeat(4);
            for width in [1, 2, 3, 127, frame.len(), frames.len()] {
                let mut budget = SseBudget::new(16);
                for chunk in frames.as_bytes().chunks(width) {
                    budget.accept(chunk).unwrap();
                }
                assert_eq!(budget.frame_bytes, 0);
            }
        }
        // A split CRLF after a nonempty line must not reset an unfinished frame.
        let mut budget = SseBudget::new(16);
        budget.accept(&vec![b'x'; FRAME_OVERHEAD]).unwrap();
        budget.accept(b"\r").unwrap();
        budget.accept(b"\ndata: ").unwrap();
        assert!(budget.accept(b"01234567890123456789").is_err());
    }
    #[test]
    fn small_frames_do_not_evade_the_total_wire_budget() {
        let mut budget = SseBudget::new(16);
        let chunk = b": ping\n\n".repeat(4096);
        for _ in 0..MAX_WIRE_BYTES / chunk.len() {
            budget.accept(&chunk).unwrap();
        }
        assert!(budget.accept(b"x").is_err());
    }
}
