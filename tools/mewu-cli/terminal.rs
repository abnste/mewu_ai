// SPDX-License-Identifier: MPL-2.0
//! Native scrollback chat. Only the unfinished reply line is repainted.
use super::{chat::ChatUi, presentation::safe_text};
#[path = "terminal_markdown.rs"]
mod markdown;
use crossterm::{
    cursor::{MoveToColumn, MoveUp},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Color, Stylize},
    terminal::{self, Clear, ClearType},
};
use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthChar;

fn styled() -> bool {
    io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").ok().as_deref() != Some("dumb")
}
fn accent(text: &str) -> String {
    if styled() {
        text.with(Color::Cyan).bold().to_string()
    } else {
        text.into()
    }
}
fn muted(text: &str) -> String {
    if styled() {
        text.with(Color::Rgb {
            r: 146,
            g: 154,
            b: 171,
        })
        .to_string()
    } else {
        text.into()
    }
}
fn label(text: &str) -> String {
    safe_text(text).replace(['\n', '\t'], " ")
}
fn columns() -> usize {
    terminal::size()
        .map_or(80, |(w, _)| w as usize)
        .saturating_sub(4)
        .clamp(12, 92)
}
fn clip(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut cells = 0;
    for c in label(text).chars() {
        let n = c.width().unwrap_or(0);
        if cells + n > width {
            out.push('…');
            break;
        }
        out.push(c);
        cells += n;
    }
    out
}
fn logo(writer: &mut impl Write) -> io::Result<()> {
    if terminal::size().map_or(true, |(w, _)| w >= 52) {
        writeln!(
            writer,
            "\n  {}    {}  {}\n  {}\n  {}",
            accent(" /\\_/\\"),
            accent("mewu"),
            muted(concat!("v", env!("CARGO_PKG_VERSION"))),
            accent("( o.o )"),
            accent(" > ^ <")
        )?;
    } else {
        writeln!(
            writer,
            "\n  {} {}",
            accent("mewu"),
            muted(env!("CARGO_PKG_VERSION"))
        )?;
    }
    Ok(())
}
pub(super) fn help(writer: &mut impl Write, usage: &str) -> io::Result<()> {
    logo(writer)?;
    for line in usage.lines().skip(2) {
        if !line.is_empty()
            && !line.starts_with(' ')
            && !line.contains('。')
            && line.chars().count() < 12
        {
            writeln!(writer, "{}", accent(line))?;
        } else {
            writeln!(writer, "{line}")?;
        }
    }
    writeln!(writer)?;
    writer.flush()
}
pub(super) fn welcome(agent: &str, model: &str) {
    let mut out = io::stdout().lock();
    let _ = logo(&mut out);
    let _ = writeln!(
        out,
        "\n  {}\n  {}\n",
        clip(
            &format!("{}  ·  {}", label(agent), label(model)),
            columns().saturating_sub(2)
        ),
        muted("Enter 发送  ·  ↑ 历史  ·  Ctrl+C 停止 / 退出  ·  /help")
    );
}
pub(super) fn before_prompt() {
    println!("\n  {}", muted(&"─".repeat(columns())));
}
pub(super) fn prompt() -> String {
    format!("  {}", accent("› "))
}
pub(super) fn notice(text: &str) {
    println!("\n  {}\n", muted(&label(text)));
}
pub(super) fn error(text: &str) {
    let clean = label(text);
    if styled() {
        eprintln!("\n  {}\n", clean.with(Color::Red));
    } else {
        eprintln!("{clean}");
    }
}
pub(super) fn resume(scene: &str) {
    eprintln!(
        "\n  {}\n",
        muted(&format!("继续会话  mewu chat --scene {}", label(scene)))
    );
}
pub(super) fn commands() {
    println!("\n  {}\n  /new             新会话\n  /agent [名称或ID]  列出 / 切换 Agent\n  /model [名称或ID]  列出 / 切换模型\n  /session         会话 ID\n  /use ID          继续已有会话\n  /clear           清理终端显示\n  /exit            退出\n",accent("对话命令"));
}
pub(super) fn clear() {
    let _ = execute!(
        io::stdout(),
        Clear(ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    );
}

struct Reply {
    pending: String,
    shown: usize,
    committed: usize,
    literal: bool,
    fence: Option<(char, usize)>,
    width: usize,
    tail_limit: usize,
    color: bool,
}
impl Reply {
    fn new(width: usize, height: usize, color: bool) -> Self {
        Self {
            pending: String::new(),
            shown: 0,
            committed: 0,
            literal: false,
            fence: None,
            width: width.max(4),
            tail_limit: height.saturating_sub(4).clamp(1, 10),
            color,
        }
    }
    fn clear_tail(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.shown > 0 {
            execute!(
                out,
                MoveToColumn(0),
                MoveUp(self.shown as u16),
                Clear(ClearType::FromCursorDown)
            )?;
            self.shown = 0;
        }
        Ok(())
    }
    fn write_rows(&self, out: &mut impl Write, rows: &[String]) -> io::Result<()> {
        for row in rows {
            let gutter = if self.color {
                "│"
                    .with(Color::Rgb {
                        r: 87,
                        g: 98,
                        b: 119,
                    })
                    .to_string()
            } else {
                "│".into()
            };
            write!(out, "  {gutter} {row}\r\n")?;
        }
        Ok(())
    }
    fn complete_line(&mut self, out: &mut impl Write, line: &str) -> io::Result<()> {
        let trimmed = line.trim_start_matches(' ');
        let fence = trimmed
            .chars()
            .next()
            .filter(|c| *c == '`' || *c == '~')
            .map(|c| (c, trimmed.chars().take_while(|x| *x == c).count()));
        let mut decoration = None;
        if line.len() - trimmed.len() <= 3 {
            if let Some((c, n)) = fence.filter(|(_, n)| *n >= 3) {
                if self
                    .fence
                    .is_some_and(|(open, size)| open == c && n >= size)
                    && trimmed[n..].trim().is_empty()
                {
                    self.fence = None;
                    decoration = Some("╰─".to_string());
                } else if self.fence.is_none() {
                    self.fence = Some((c, n));
                    decoration = Some(format!(
                        "╭─ {}",
                        clip(trimmed[n..].trim(), self.width.saturating_sub(5))
                    ));
                }
            }
        }
        let rows = if let Some(text) = decoration {
            vec![if self.color {
                text.with(Color::Rgb {
                    r: 146,
                    g: 154,
                    b: 171,
                })
                .to_string()
            } else {
                text
            }]
        } else {
            markdown::rows(
                line,
                self.width,
                self.fence.is_some(),
                self.literal,
                self.color,
            )
        };
        self.write_rows(out, &rows[self.committed.min(rows.len())..])?;
        self.committed = 0;
        self.literal = false;
        Ok(())
    }
    fn delta(&mut self, out: &mut impl Write, text: &str) -> io::Result<()> {
        let mut frame = Vec::new();
        self.clear_tail(&mut frame)?;
        self.pending.push_str(&safe_text(text));
        while let Some(index) = self.pending.find('\n') {
            let tail = self.pending.split_off(index + 1);
            let line = std::mem::replace(&mut self.pending, tail);
            self.complete_line(&mut frame, line.trim_end_matches('\n'))?;
        }
        if !self.pending.is_empty() {
            let mut rows = markdown::rows(
                &self.pending,
                self.width,
                self.fence.is_some(),
                self.literal,
                self.color,
            );
            // Never move the cursor above the visible tail. Very long physical
            // lines use literal streaming, keeping their committed prefix stable.
            if !self.literal && rows.len() > self.tail_limit {
                self.literal = true;
                rows = markdown::rows(
                    &self.pending,
                    self.width,
                    self.fence.is_some(),
                    true,
                    self.color,
                );
            }
            self.write_rows(&mut frame, &rows[self.committed.min(rows.len())..])?;
            self.committed = rows.len().saturating_sub(self.tail_limit);
            self.shown = rows.len() - self.committed;
        }
        // One write per frame: no separately flushed clear that would make
        // legacy Windows consoles flash between clearing and painting a line.
        out.write_all(&frame)?;
        out.flush()
    }
    fn finish(&mut self, out: &mut impl Write) -> io::Result<()> {
        let mut frame = Vec::new();
        self.clear_tail(&mut frame)?;
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.complete_line(&mut frame, &line)?;
        }
        if self.fence.take().is_some() {
            self.write_rows(&mut frame, &["╰─".into()])?;
        }
        out.write_all(&frame)?;
        out.flush()
    }
    fn resized(&mut self, out: &mut impl Write, width: usize, height: usize) -> io::Result<()> {
        // The terminal may already have reflowed its scrollback. Do not erase
        // using stale coordinates; keep displayed text and continue below it.
        if !self.pending.is_empty() {
            write!(out, "\r\n")?;
            self.pending.clear();
        }
        self.shown = 0;
        self.committed = 0;
        self.literal = false;
        self.width = width.saturating_sub(6).clamp(4, 86);
        self.tail_limit = height.saturating_sub(4).clamp(1, 10);
        Ok(())
    }
}

pub(super) struct Turn {
    human: bool,
    raw: bool,
    tty: bool,
    started: Instant,
    frame: usize,
    waiting: bool,
    wrote: bool,
    finished: bool,
    reply: Reply,
}
impl Turn {
    pub fn new(human: bool) -> io::Result<Self> {
        let raw = io::stdin().is_terminal();
        if raw {
            terminal::enable_raw_mode()?;
        }
        let (w, h) = terminal::size().unwrap_or((80, 24));
        Ok(Self {
            human,
            raw,
            tty: io::stdout().is_terminal()
                && std::env::var("TERM").ok().as_deref() != Some("dumb"),
            started: Instant::now(),
            frame: 0,
            waiting: false,
            wrote: false,
            finished: false,
            reply: Reply::new(
                (w as usize).saturating_sub(6).clamp(4, 86),
                h as usize,
                styled(),
            ),
        })
    }
    fn clear_wait(&mut self) -> io::Result<()> {
        if self.waiting {
            execute!(io::stdout(), MoveToColumn(0), Clear(ClearType::CurrentLine))?;
            self.waiting = false;
        }
        Ok(())
    }
}
impl ChatUi for Turn {
    fn progress(&mut self, text: &str) -> io::Result<()> {
        if !self.human || !self.tty || self.wrote {
            return Ok(());
        }
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner = frames[self.frame % frames.len()];
        self.frame += 1;
        execute!(io::stdout(), MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        print!(
            "  {} {} {}",
            accent(&spinner.to_string()),
            clip(text, columns().saturating_sub(16)),
            muted(&format!("{:.1}s", self.started.elapsed().as_secs_f32()))
        );
        self.waiting = true;
        io::stdout().flush()
    }
    fn delta(&mut self, text: &str, replacement: bool) -> io::Result<()> {
        if !self.human {
            return Ok(());
        }
        self.clear_wait()?;
        if self.tty {
            if !self.wrote || replacement {
                if replacement {
                    self.reply.finish(&mut io::stdout().lock())?;
                }
                write!(
                    io::stdout(),
                    "\r\n  {} {}\r\n\r\n",
                    accent("●"),
                    accent(if replacement {
                        "mewu · 更新答复"
                    } else {
                        "mewu"
                    })
                )?;
            }
            self.reply.delta(&mut io::stdout().lock(), text)?;
        } else {
            if replacement {
                println!("\n[更新答复]");
            }
            print!("{}", safe_text(text));
            io::stdout().flush()?;
        }
        self.wrote = true;
        Ok(())
    }
    fn finish(&mut self) -> io::Result<()> {
        self.clear_wait()?;
        if self.human && self.tty && self.wrote {
            self.reply.finish(&mut io::stdout().lock())?;
            write!(
                io::stdout(),
                "\r\n  {}\r\n",
                muted(&format!("{:.1}s", self.started.elapsed().as_secs_f32()))
            )?;
        }
        self.finished = true;
        io::stdout().flush()
    }
    fn interrupted(&mut self) -> io::Result<bool> {
        if !self.raw {
            return Ok(false);
        }
        for _ in 0..16 {
            if !event::poll(Duration::ZERO)? {
                break;
            }
            match event::read()? {
                Event::Key(key)
                    if key.kind != KeyEventKind::Release
                        && key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    return Ok(true)
                }
                Event::Resize(w, h) if self.tty => {
                    self.reply
                        .resized(&mut io::stdout().lock(), w as usize, h as usize)?
                }
                _ => {}
            }
        }
        Ok(false)
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        let _ = self.clear_wait();
        if self.raw {
            let _ = terminal::disable_raw_mode();
        }
        if self.human && self.wrote && (!self.tty || !self.finished) {
            println!();
            let _ = io::stdout().flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_markdown_updates_only_the_tail_and_keeps_completed_lines() {
        let mut r = Reply::new(30, 24, false);
        let mut out = Vec::new();
        r.delta(&mut out, "**Mi").unwrap();
        out.clear();
        r.delta(&mut out, "ni** 是名字\n第二段").unwrap();
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(s.contains("Mini 是名字"));
        assert!(!s.contains("**Mini**"));
        assert_eq!(r.pending, "第二段");
        out.clear();
        r.delta(&mut out, "。\n").unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("Mini"));
        assert!(s.contains("第二段。"));
    }
    #[test]
    fn huge_lines_never_erase_above_visible_tail_and_finish_once() {
        let mut r = Reply::new(12, 8, false);
        let mut out = Vec::new();
        for _ in 0..100 {
            r.delta(&mut out, "中文abc中文").unwrap();
            assert!(r.shown <= 4);
        }
        let committed = r.committed;
        assert!(committed > 0);
        out.clear();
        r.finish(&mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().lines().count() < 10);
        assert!(r.pending.is_empty());
    }
    #[test]
    fn code_fences_keep_literal_code_and_resize_never_uses_stale_cursor_rows() {
        let mut r = Reply::new(30, 24, false);
        let mut out = Vec::new();
        r.delta(&mut out, "```rust\nlet x = **a**;\n```\n").unwrap();
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(s.contains("╭─ rust"));
        assert!(s.contains("**a**"));
        assert!(s.contains("╰─"));
        assert!(r.fence.is_none());
        r.delta(&mut out, "未完成").unwrap();
        out.clear();
        r.resized(&mut out, 20, 6).unwrap();
        r.delta(&mut out, "后续").unwrap();
        assert!(!String::from_utf8(out).unwrap().contains("\x1b[1A"));
    }
}
