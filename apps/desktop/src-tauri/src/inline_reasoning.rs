// SPDX-License-Identifier: MPL-2.0
//! Split leading, explicit reasoning envelopes without rewriting model continuation.
//! MiniMax documents <think> in content; tags in answer prose/code remain literal.

const TAGS: [(&str, &str); 3] = [
    ("<think>", "</think>"),
    ("<thinking>", "</thinking>"),
    ("<reasoning>", "</reasoning>"),
];

fn starts_with_ascii_case(whole: &str, prefix: &str) -> bool {
    prefix.len() <= whole.len()
        && whole.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn prefix_whitespace(ch: char) -> bool {
    matches!(ch, '\u{feff}' | '\u{200b}') || (ch.is_whitespace() && ch != '\u{85}')
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct ReplyParts {
    pub text: String,
    pub reasoning: String,
}

#[derive(Default)]
enum Mode {
    #[default]
    Prefix,
    Thinking(&'static str),
    Answer,
}

#[derive(Default)]
pub(crate) struct InlineReasoning {
    mode: Mode,
    pending: String,
    whitespace: String,
    blocks: usize,
}

impl InlineReasoning {
    pub fn has_reasoning(&self) -> bool {
        self.blocks > 0
    }

    pub fn push(&mut self, text: &str) -> ReplyParts {
        let mut parts = ReplyParts::default();
        for ch in text.chars() {
            match self.mode {
                Mode::Answer => parts.text.push(ch),
                Mode::Prefix => {
                    if self.pending.is_empty() && prefix_whitespace(ch) {
                        self.whitespace.push(ch);
                        continue;
                    }
                    self.pending.push(ch);
                    if let Some((_, closing)) = TAGS
                        .iter()
                        .find(|(opening, _)| self.pending.eq_ignore_ascii_case(opening))
                    {
                        self.pending.clear();
                        self.whitespace.clear();
                        if self.blocks > 0 {
                            parts.reasoning.push_str("\n\n");
                        }
                        self.blocks += 1;
                        self.mode = Mode::Thinking(*closing);
                    } else if !TAGS
                        .iter()
                        .any(|(opening, _)| starts_with_ascii_case(opening, &self.pending))
                    {
                        parts.text.push_str(&self.whitespace);
                        parts.text.push_str(&self.pending);
                        self.whitespace.clear();
                        self.pending.clear();
                        self.mode = Mode::Answer;
                    }
                }
                Mode::Thinking(closing) => {
                    self.pending.push(ch);
                    while !starts_with_ascii_case(closing, &self.pending) {
                        let first = self.pending.chars().next().expect("nonempty pending text");
                        parts.reasoning.push(first);
                        self.pending.drain(..first.len_utf8());
                    }
                    if self.pending.eq_ignore_ascii_case(closing) {
                        self.pending.clear();
                        self.mode = Mode::Prefix;
                    }
                }
            }
        }
        parts
    }

    pub fn finish(&mut self) -> ReplyParts {
        let mut parts = ReplyParts::default();
        match self.mode {
            Mode::Thinking(_) => parts.reasoning.push_str(&self.pending),
            Mode::Prefix => {
                parts.text.push_str(&self.whitespace);
                parts.text.push_str(&self.pending);
            }
            Mode::Answer => {}
        }
        self.pending.clear();
        self.whitespace.clear();
        parts
    }
}

pub(crate) fn split_text(text: &str) -> ReplyParts {
    let mut parser = InlineReasoning::default();
    let mut parts = parser.push(text);
    let tail = parser.finish();
    parts.text.push_str(&tail.text);
    parts.reasoning.push_str(&tail.reasoning);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        id: String,
        input: String,
        text: String,
        reasoning: String,
    }

    #[test]
    fn inline_reasoning_shared_cases_and_every_utf8_fragment_boundary() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../tests/inline-reasoning.json")).unwrap();
        for case in cases {
            assert_eq!(
                split_text(&case.input),
                ReplyParts {
                    text: case.text.clone(),
                    reasoning: case.reasoning.clone()
                },
                "{}",
                case.id
            );
            for boundary in case
                .input
                .char_indices()
                .map(|(index, _)| index)
                .chain([case.input.len()])
            {
                let mut parser = InlineReasoning::default();
                let mut parts = parser.push(&case.input[..boundary]);
                let next = parser.push(&case.input[boundary..]);
                let tail = parser.finish();
                parts.text.push_str(&next.text);
                parts.text.push_str(&tail.text);
                parts.reasoning.push_str(&next.reasoning);
                parts.reasoning.push_str(&tail.reasoning);
                assert_eq!(parts.text, case.text, "{} / {}", case.id, boundary);
                assert_eq!(
                    parts.reasoning, case.reasoning,
                    "{} / {}",
                    case.id, boundary
                );
            }
        }
    }

    #[test]
    fn inline_reasoning_never_emits_a_partial_tag_or_thought_into_streamed_answer() {
        let mut parser = InlineReasoning::default();
        for fragment in ["<", "thi", "nk>", "SECRET🙂", "</th", "ink>"] {
            assert!(parser.push(fragment).text.is_empty());
        }
        assert_eq!(parser.push("Answer").text, "Answer");
        assert!(parser.has_reasoning());
        let mut canceled = InlineReasoning::default();
        assert!(canceled
            .push("<think>unfinished SECRET</th")
            .text
            .is_empty());
        assert!(canceled.finish().text.is_empty());
    }
}
