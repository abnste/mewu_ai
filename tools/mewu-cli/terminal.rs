// SPDX-License-Identifier: MPL-2.0
//! Small native terminal view. No alternate-screen app, web server or subprocess.
use super::{chat::ChatUi, presentation::safe_text};
use crossterm::{
    cursor::MoveToColumn,
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
        text.dark_grey().to_string()
    } else {
        text.into()
    }
}
fn label(text: &str) -> String {
    safe_text(text).replace(['\n', '\t'], " ")
}
fn logo(writer: &mut impl Write) -> io::Result<()> {
    if terminal::size().map_or(true, |(width, _)| width >= 52) {
        writeln!(
            writer,
            "\n  {}    {}  {}\n  {}   {}\n  {}",
            accent(" /\\_/\\"),
            accent("mewu"),
            muted(concat!("v", env!("CARGO_PKG_VERSION"))),
            accent("( o.o )"),
            muted("DESKTOP AGENT"),
            accent(" > ^ <")
        )?;
    } else {
        writeln!(
            writer,
            "\n{} {}",
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
        "\n  {}  {}\n  {}  {}\n\n  {}\n",
        muted("Agent"),
        label(agent),
        muted("Model"),
        label(model),
        muted("Enter 发送  ·  ↑ 历史  ·  Ctrl+C 停止  ·  /help")
    );
}
pub(super) fn prompt() -> String {
    accent("› ")
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

pub(super) struct Turn {
    human: bool,
    raw: bool,
    tty: bool,
    started: Instant,
    frame: usize,
    waiting: bool,
    wrote: bool,
}
impl Turn {
    pub fn new(human: bool) -> io::Result<Self> {
        let raw = io::stdin().is_terminal();
        if raw {
            terminal::enable_raw_mode()?;
        }
        Ok(Self {
            human,
            raw,
            tty: io::stdout().is_terminal()
                && std::env::var("TERM").ok().as_deref() != Some("dumb"),
            started: Instant::now(),
            frame: 0,
            waiting: false,
            wrote: false,
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
        let width = terminal::size()
            .map_or(80, |(w, _)| w as usize)
            .saturating_sub(20);
        let mut clipped = String::new();
        let mut cells = 0;
        for c in label(text).chars() {
            let n = c.width().unwrap_or(0);
            if cells + n > width {
                break;
            }
            clipped.push(c);
            cells += n;
        }
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner = frames[self.frame % frames.len()];
        self.frame += 1;
        execute!(io::stdout(), MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        print!(
            "  {} {} {}",
            accent(&spinner.to_string()),
            clipped,
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
        if self.tty && (!self.wrote || replacement) {
            print!(
                "\r\n  {}\r\n\r\n",
                accent(if replacement {
                    "mewu · 更新答复"
                } else {
                    "mewu"
                })
            );
        } else if replacement {
            println!("\n[更新答复]");
        }
        let text = safe_text(text);
        if self.raw && self.tty {
            print!("{}", text.replace('\n', "\r\n"));
        } else {
            print!("{text}");
        }
        self.wrote = true;
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
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Release
                    && key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    return Ok(true);
                }
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
        if self.human && self.wrote {
            println!();
            let _ = io::stdout().flush();
        }
    }
}
