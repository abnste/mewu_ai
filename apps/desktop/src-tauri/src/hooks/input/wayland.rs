// SPDX-License-Identifier: MPL-2.0
//! A separate, fresh RemoteDesktop grant. A screenshot/region grant is never
//! promoted to global control. The portal owns revocation and monitor geometry.
use super::*;
use crate::recording_portal_linux::PortalSession;

pub(super) async fn execute(
    active: Active,
    fence: Fence,
    steps: Vec<InputStep>,
    _cancel: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let _active = active;
        let session = PortalSession::open_control(deadline, &mut || fence.check().is_ok())?;
        let (ox, oy) = session.logical_position.ok_or("系统未提供授权屏幕坐标")?;
        let (width, height) = session
            .logical_size
            .filter(|(w, h)| *w > 0 && *h > 0)
            .ok_or("系统未提供授权屏幕尺寸")?;
        // Check the complete plan before submitting its first input.
        for step in &steps {
            match step {
                InputStep::Move { x, y } => {
                    let (x, y) = (i64::from(*x) - i64::from(ox), i64::from(*y) - i64::from(oy));
                    if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
                        return Err("输入坐标超出本次授权屏幕".into());
                    }
                }
                InputStep::Key { key } => {
                    keys(key)?;
                }
                _ => {}
            }
        }
        let mut held = Vec::new();
        let result: Result<()> = (|| {
            for step in steps {
                fence.check()?;
                if !session.live() {
                    return Err("系统控制授权已结束".into());
                }
                let mut allowed = || fence.check().is_ok();
                match step {
                    InputStep::Move { x, y } => session.pointer(
                        f64::from(x) - f64::from(ox),
                        f64::from(y) - f64::from(oy),
                        &mut allowed,
                    )?,
                    InputStep::Click { button, count } => {
                        let button = button_code(button);
                        for _ in 0..count {
                            session.button_code(button, true, &mut allowed)?;
                            held.push(button);
                            session.button_code(button, false, &mut allowed)?;
                            held.pop();
                        }
                    }
                    InputStep::Button { button, direction } => {
                        let button = button_code(button);
                        match direction {
                            ButtonDirection::Press => {
                                if held.contains(&button) {
                                    return Err("鼠标按钮已经按住".into());
                                }
                                session.button_code(button, true, &mut allowed)?;
                                held.push(button);
                            }
                            ButtonDirection::Release => {
                                if !held.contains(&button) {
                                    return Err("没有待释放的鼠标按钮".into());
                                }
                                session.button_code(button, false, &mut allowed)?;
                                held.retain(|b| *b != button);
                            }
                        }
                    }
                    InputStep::Scroll { x, y } => {
                        if x != 0 {
                            session.scroll_axis(1, x, &mut allowed)?;
                        }
                        if y != 0 {
                            session.scroll_axis(0, y, &mut allowed)?;
                        }
                    }
                    InputStep::Key { key } => chord(&session, &fence, &keys(&key)?)?,
                    InputStep::Text { text } => {
                        for character in text.chars() {
                            fence.check()?;
                            let key = unicode(character);
                            session.keysym(key, true, &mut allowed)?;
                            let released = session.keysym(key, false, &mut || true);
                            released?;
                        }
                    }
                    InputStep::Delay { milliseconds } => {
                        let until =
                            (Instant::now() + Duration::from_millis(milliseconds)).min(deadline);
                        while Instant::now() < until {
                            fence.check()?;
                            if !session.live() {
                                return Err("系统控制授权已结束".into());
                            }
                            std::thread::sleep(
                                Duration::from_millis(10)
                                    .min(until.saturating_duration_since(Instant::now())),
                            );
                        }
                    }
                }
            }
            Ok(())
        })();
        for button in held.into_iter().rev() {
            let _ = session.button_code(button, false, &mut || true);
        }
        session.close();
        result
    })
    .await
    .map_err(|_| "Wayland 键鼠操作中断，未自动重试")?
}
fn button_code(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 272,
        MouseButton::Right => 273,
        MouseButton::Middle => 274,
    }
}
fn unicode(c: char) -> i32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        c if (c as u32) <= 0xff => c as i32,
        c => 0x01000000 | c as i32,
    }
}
fn keys(value: &str) -> Result<Vec<i32>> {
    let parts = value.split('+').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 5 {
        return Err("按键组合无效".into());
    }
    let mut keys = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let key = match *part {
            "Control" | "Ctrl" => 0xffe3,
            "Alt" | "Option" => 0xffe9,
            "Shift" => 0xffe1,
            "Super" | "Meta" | "Command" | "Cmd" | "Win" => 0xffeb,
            "Enter" | "Return" => 0xff0d,
            "Escape" | "Esc" => 0xff1b,
            "Tab" => 0xff09,
            "Space" => 0x20,
            "Backspace" => 0xff08,
            "Delete" => 0xffff,
            "Home" => 0xff50,
            "End" => 0xff57,
            "PageUp" => 0xff55,
            "PageDown" => 0xff56,
            "ArrowLeft" | "Left" => 0xff51,
            "ArrowUp" | "Up" => 0xff52,
            "ArrowRight" | "Right" => 0xff53,
            "ArrowDown" | "Down" => 0xff54,
            s if s.starts_with('F')
                && s[1..].parse::<i32>().is_ok_and(|n| (1..=24).contains(&n)) =>
            {
                0xffbd + s[1..].parse::<i32>().map_err(|_| "按键无效")?
            }
            s if s.chars().count() == 1 => unicode(s.chars().next().unwrap()),
            s if s.starts_with("Key") && s.len() == 4 && s.as_bytes()[3].is_ascii_uppercase() => {
                i32::from(s.as_bytes()[3].to_ascii_lowercase())
            }
            _ => return Err("不支持的按键名称".into()),
        };
        if index + 1 < parts.len() && !matches!(key, 0xffe3 | 0xffe9 | 0xffe1 | 0xffeb) {
            return Err("修饰键必须放在前面".into());
        }
        keys.push(key);
    }
    Ok(keys)
}
fn chord(session: &PortalSession, fence: &Fence, keys: &[i32]) -> Result<()> {
    let mut held = vec![];
    let result: Result<()> = (|| {
        for key in keys {
            fence.check()?;
            session.keysym(*key, true, &mut || fence.check().is_ok())?;
            held.push(*key);
        }
        Ok(())
    })();
    let mut release_error = None;
    for key in held.into_iter().rev() {
        if let Err(e) = session.keysym(key, false, &mut || true) {
            release_error = Some(e);
        }
    }
    result?;
    if let Some(error) = release_error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_follow_portal_xkb_keysyms_and_preserve_unicode() {
        assert_eq!(
            keys("Control+Shift+KeyA").unwrap(),
            vec![0xffe3, 0xffe1, 97]
        );
        assert_eq!(keys("F24").unwrap(), vec![0xffd5]);
        assert!(keys("a+Control").is_err());
        assert!(keys("F25").is_err());
        assert_eq!(unicode('中'), 0x01004e2d);
        assert_eq!(unicode('\n'), 0xff0d);
    }
}
