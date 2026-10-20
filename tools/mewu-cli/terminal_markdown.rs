// SPDX-License-Identifier: MPL-2.0
//! CommonMark text -> terminal spans; model text never supplies terminal escapes.
use super::super::presentation::safe_text;
use crossterm::style::{Color, Stylize};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: bool,
    heading: bool,
}
#[derive(Clone, Debug)]
struct Span {
    text: String,
    style: Style,
}

fn push(spans: &mut Vec<Span>, text: impl Into<String>, style: Style) {
    // CommonMark can decode entities into controls after source sanitization.
    let text = safe_text(&text.into())
        .replace('\t', "    ")
        .replace(['\n', '\r'], " ");
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut().filter(|last| last.style == style) {
        last.text.push_str(&text);
    } else {
        spans.push(Span { text, style });
    }
}

fn spans(source: &str, code: bool) -> Vec<Span> {
    let mut result = Vec::new();
    if code {
        push(
            &mut result,
            source,
            Style {
                code: true,
                ..Style::default()
            },
        );
        return result;
    }
    let mut style = Style::default();
    let mut link = None;
    let mut ordered = None;
    let indent = source.chars().take_while(|c| *c == ' ').count().min(16);
    if indent > 0 {
        push(&mut result, " ".repeat(indent), style);
    }
    for event in Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    ) {
        match event {
            Event::Start(Tag::Strong) => style.bold = true,
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::Start(Tag::Strikethrough) => style.strike = true,
            Event::Start(Tag::Heading { .. }) => {
                style.bold = true;
                style.heading = true;
            }
            Event::Start(Tag::List(start)) => ordered = start,
            Event::Start(Tag::Item) => push(
                &mut result,
                ordered.map_or_else(|| "• ".into(), |n| format!("{n}. ")),
                style,
            ),
            Event::Start(Tag::BlockQuote(_)) => push(
                &mut result,
                "│ ",
                Style {
                    link: true,
                    ..style
                },
            ),
            Event::Start(Tag::Link { dest_url, .. })
            | Event::Start(Tag::Image { dest_url, .. }) => {
                link = Some(dest_url.to_string());
                style.link = true;
            }
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            Event::End(TagEnd::Strikethrough) => style.strike = false,
            Event::End(TagEnd::Link | TagEnd::Image) => {
                style.link = false;
                if let Some(url) = link.take() {
                    push(
                        &mut result,
                        format!(" ({url})"),
                        Style {
                            link: true,
                            ..style
                        },
                    );
                }
            }
            Event::Code(text) => push(
                &mut result,
                text.into_string(),
                Style {
                    code: true,
                    ..style
                },
            ),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                push(&mut result, text.into_string(), style)
            }
            Event::SoftBreak | Event::HardBreak => push(&mut result, " ", style),
            Event::Rule => push(
                &mut result,
                "────────────────",
                Style {
                    link: true,
                    ..style
                },
            ),
            Event::TaskListMarker(done) => {
                push(&mut result, if done { "[x] " } else { "[ ] " }, style)
            }
            Event::FootnoteReference(text) => push(&mut result, format!("[{text}]"), style),
            Event::InlineMath(text) | Event::DisplayMath(text) => push(
                &mut result,
                text.into_string(),
                Style {
                    code: true,
                    ..style
                },
            ),
            _ => {}
        }
    }
    result
}

fn paint(text: &str, style: Style, color: bool) -> String {
    if !color {
        return text.into();
    }
    let mut painted = if style.code {
        text.with(Color::Rgb {
            r: 130,
            g: 215,
            b: 200,
        })
    } else if style.link {
        text.with(Color::Rgb {
            r: 143,
            g: 183,
            b: 236,
        })
    } else if style.heading {
        text.with(Color::Rgb {
            r: 191,
            g: 175,
            b: 255,
        })
    } else {
        text.stylize()
    };
    if style.bold || style.code {
        painted = painted.bold();
    }
    if style.italic {
        painted = painted.italic();
    }
    if style.strike {
        painted = painted.crossed_out();
    }
    painted.to_string()
}

/// Wrap before adding ANSI styles. Width counts CJK cells, never escape bytes.
pub(super) fn rows(
    source: &str,
    width: usize,
    code: bool,
    literal: bool,
    color: bool,
) -> Vec<String> {
    let source = safe_text(source);
    let source = source.trim_end_matches('\r');
    let spans = if literal {
        vec![Span {
            text: source.replace('\t', "    "),
            style: Style::default(),
        }]
    } else {
        spans(source, code)
    };
    let mut rows = Vec::new();
    let mut row: Vec<(String, Style)> = Vec::new();
    let mut cells = 0;
    let width = width.max(4);
    for span in spans {
        // Prefer word boundaries while permitting long identifiers/CJK to wrap.
        for word in span.text.split_inclusive(char::is_whitespace) {
            let word_cells: usize = word.chars().map(|c| c.width().unwrap_or(0)).sum();
            if cells > 0 && word_cells <= width && cells + word_cells > width {
                rows.push(row.iter().map(|(s, k)| paint(s, *k, color)).collect());
                row.clear();
                cells = 0;
            }
            for c in word.chars() {
                let n = c.width().unwrap_or(0);
                if cells > 0 && cells + n > width {
                    rows.push(row.iter().map(|(s, k)| paint(s, *k, color)).collect());
                    row.clear();
                    cells = 0;
                }
                if let Some((text, _)) = row.last_mut().filter(|(_, s)| *s == span.style) {
                    text.push(c);
                } else {
                    row.push((c.to_string(), span.style));
                }
                cells += n;
            }
        }
    }
    rows.push(row.iter().map(|(s, k)| paint(s, *k, color)).collect());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_is_parsed_and_controls_cannot_supply_ansi() {
        assert_eq!(
            rows("**Mini** 和 `code`", 80, false, false, false),
            ["Mini 和 code"]
        );
        assert_eq!(rows("### 标题", 80, false, false, false), ["标题"]);
        assert_eq!(rows("- 第一项", 80, false, false, false), ["• 第一项"]);
        let out = rows("\x1b[2J**正常**\r", 80, false, false, true).join("");
        assert!(!out.contains("\x1b[2J"));
        assert!(!out.contains("**"));
        let decoded = rows("&#27;[2J &#13;X", 80, false, false, false).join("");
        assert!(!decoded.contains('\x1b'));
        assert!(!decoded.contains('\r'));
    }
    #[test]
    fn cjk_wraps_with_margins_and_plain_code_is_preserved() {
        let input = "中文测试abcdef中文测试";
        let out = rows(input, 10, false, true, false);
        assert!(out
            .iter()
            .all(|r| r.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>() <= 10));
        assert_eq!(out.concat(), input);
        assert_eq!(
            rows("  x = **literal**", 80, true, false, false),
            ["  x = **literal**"]
        );
        assert!(rows("plain", 80, false, false, false)
            .iter()
            .all(|r| !r.contains('\x1b')));
    }
    #[test]
    fn latin_words_links_and_unbroken_long_identifiers_survive() {
        assert_eq!(
            rows("one two three", 8, false, false, false),
            ["one two ", "three"]
        );
        assert_eq!(
            rows("[官网](https://example.com)", 80, false, false, false),
            ["官网 (https://example.com)"]
        );
        assert_eq!(
            rows(&"a".repeat(10000), 32, false, true, false).concat(),
            "a".repeat(10000)
        );
    }
}
