// SPDX-License-Identifier: MPL-2.0
//! Only saved native automation can use global input. Region-agent grants keep
//! their existing, independent target/window boundaries; no region API widens.
use super::{actions::Fence, *};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use mewu_hooks::{ButtonDirection, InputStep, MouseButton};
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;
struct Active(Arc<Shared>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.input_active.store(false, Ordering::Release);
    }
}
pub(super) async fn execute(work: &Execution, steps: Vec<InputStep>) -> Result<()> {
    work.check()?;
    if work
        .shared
        .input_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("已有键鼠操作正在执行".into());
    }
    let active = Active(work.shared.clone());
    // A bare function-key trigger may still be physically held. Waiting only
    // for modifiers lets a simulated release clear the repeat guard too early.
    let release_deadline = Instant::now() + Duration::from_secs(2);
    while super::shortcuts::awaiting_release(&work.shared)? {
        work.check()?;
        if Instant::now() >= release_deadline {
            return Err("请松开快捷键后重试".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let fence = Fence::new(work);
    #[cfg(target_os = "linux")]
    if crate::capture_shortcut_wayland_host::is_wayland() {
        return wayland::execute(active, fence, steps, work.cancel.clone(), work.deadline).await;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _active = active;
        #[cfg(target_os = "linux")]
        let physical = x11::PhysicalInput::new()?;
        #[cfg(not(target_os = "linux"))]
        let physical = PhysicalInput;
        for step in &steps {
            if let InputStep::Key { key: text } = step {
                let parts = text.split('+').collect::<Vec<_>>();
                if parts.is_empty() || parts.len() > 5 {
                    return Err("按键组合无效".into());
                }
                for (index, text) in parts.iter().enumerate() {
                    let key = key(text)?;
                    if index + 1 < parts.len()
                        && !matches!(key, Key::Control | Key::Alt | Key::Shift | Key::Meta)
                    {
                        return Err("修饰键必须放在前面".into());
                    }
                }
            }
        }
        wait_for_release(&fence, &physical)?;
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|_| "无法控制键鼠，请检查系统辅助功能权限或显示连接")?;
        let mut held: Vec<Button> = vec![];
        let result = (|| {
            for step in steps {
                fence.check()?;
                if physical.escape_down()? {
                    return Err("已按 Esc 停止键鼠操作".into());
                }
                let simulated = match step {
                    InputStep::Move { x, y } => enigo.move_mouse(x, y, Coordinate::Abs),
                    InputStep::Click { button, count } => {
                        let button = button_key(button);
                        for _ in 0..count {
                            enigo
                                .button(button, Direction::Click)
                                .map_err(|_| "点击未完成")?;
                        }
                        Ok(())
                    }
                    InputStep::Button { button, direction } => {
                        let button = button_key(button);
                        match direction {
                            ButtonDirection::Press => {
                                if held.contains(&button) {
                                    return Err("鼠标按钮已经按住".into());
                                }
                                enigo
                                    .button(button, Direction::Press)
                                    .map_err(|_| "无法按下鼠标")?;
                                held.push(button);
                            }
                            ButtonDirection::Release => {
                                if !held.contains(&button) {
                                    return Err("没有待释放的鼠标按钮".into());
                                }
                                enigo
                                    .button(button, Direction::Release)
                                    .map_err(|_| "无法释放鼠标")?;
                                held.retain(|b| b != &button);
                            }
                        }
                        Ok(())
                    }
                    InputStep::Key { key } => {
                        chord(&mut enigo, &key)?;
                        Ok(())
                    }
                    InputStep::Text { text } => {
                        let chars = text.chars().collect::<Vec<_>>();
                        for chunk in chars.chunks(64) {
                            fence.check()?;
                            if physical.escape_down()? {
                                return Err("已按 Esc 停止键鼠操作".into());
                            }
                            enigo
                                .text(&chunk.iter().collect::<String>())
                                .map_err(|_| "文字输入未完成")?;
                        }
                        Ok(())
                    }
                    InputStep::Scroll { x, y } => {
                        enigo
                            .scroll(x, Axis::Horizontal)
                            .map_err(|_| "滚动未完成")?;
                        enigo.scroll(y, Axis::Vertical)
                    }
                    InputStep::Delay { milliseconds } => {
                        let until = Instant::now() + Duration::from_millis(milliseconds);
                        while Instant::now() < until {
                            fence.check()?;
                            if physical.escape_down()? {
                                return Err("已按 Esc 停止键鼠操作".into());
                            }
                            std::thread::sleep(
                                Duration::from_millis(20)
                                    .min(until.saturating_duration_since(Instant::now())),
                            );
                        }
                        Ok(())
                    }
                };
                simulated.map_err(|_| "键鼠操作未完成，请检查系统控制权限")?;
            }
            Ok(())
        })();
        for button in held.into_iter().rev() {
            let _ = enigo.button(button, Direction::Release);
        }
        result
    })
    .await
    .map_err(|_| "键鼠执行中断，未自动重试")?
}
fn button_key(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}
fn chord(enigo: &mut Enigo, text: &str) -> Result<()> {
    let parts = text.split('+').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 5 {
        return Err("按键组合无效".into());
    }
    let keys = parts.iter().map(|s| key(s)).collect::<Result<Vec<_>>>()?;
    let mut held = vec![];
    let result = (|| {
        for k in keys.iter().take(keys.len().saturating_sub(1)) {
            if !matches!(k, Key::Control | Key::Alt | Key::Shift | Key::Meta) {
                return Err("修饰键必须放在前面".into());
            }
            enigo
                .key(*k, Direction::Press)
                .map_err(|_| "无法按下修饰键")?;
            held.push(*k);
        }
        enigo
            .key(*keys.last().ok_or("按键无效")?, Direction::Click)
            .map_err(|_| "按键未完成".into())
    })();
    for k in held.into_iter().rev() {
        let _ = enigo.key(k, Direction::Release);
    }
    result
}
pub(super) fn key(text: &str) -> Result<Key> {
    Ok(match text {
        "Control" | "Ctrl" => Key::Control,
        "Alt" | "Option" => Key::Alt,
        "Shift" => Key::Shift,
        "Super" | "Meta" | "Command" | "Cmd" | "Win" => Key::Meta,
        "Enter" | "Return" => Key::Return,
        "Escape" | "Esc" => Key::Escape,
        "Tab" => Key::Tab,
        "Space" => Key::Space,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "ArrowLeft" | "Left" => Key::LeftArrow,
        "ArrowRight" | "Right" => Key::RightArrow,
        "ArrowUp" | "Up" => Key::UpArrow,
        "ArrowDown" | "Down" => Key::DownArrow,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "F13" => Key::F13,
        "F14" => Key::F14,
        "F15" => Key::F15,
        "F16" => Key::F16,
        "F17" => Key::F17,
        "F18" => Key::F18,
        "F19" => Key::F19,
        "F20" => Key::F20,
        #[cfg(not(target_os = "macos"))]
        "F21" => Key::F21,
        #[cfg(not(target_os = "macos"))]
        "F22" => Key::F22,
        #[cfg(not(target_os = "macos"))]
        "F23" => Key::F23,
        #[cfg(not(target_os = "macos"))]
        "F24" => Key::F24,
        // A key label such as Control+A names the A key, not an uppercase
        // character. Case belongs to an explicit Shift or the Text action.
        s if s.chars().count() == 1 => {
            Key::Unicode(s.chars().next().unwrap().to_ascii_lowercase())
        }
        s if s.starts_with("Key") && s.len() == 4 && s.as_bytes()[3].is_ascii_uppercase() => {
            Key::Unicode((s.as_bytes()[3] as char).to_ascii_lowercase())
        }
        _ => return Err("不支持的按键名称".into()),
    })
}
#[cfg(test)]
mod key_tests {
    use super::*;
    #[test]
    fn letter_key_labels_do_not_add_implicit_shift() {
        assert_eq!(key("A").unwrap(), Key::Unicode('a'));
        assert_eq!(key("a").unwrap(), key("KeyA").unwrap());
        assert_eq!(key("Shift").unwrap(), Key::Shift);
        assert_eq!(key("中").unwrap(), Key::Unicode('中'));
    }
}
#[cfg(target_os = "linux")]
use x11::PhysicalInput;
#[cfg(not(target_os = "linux"))]
struct PhysicalInput;
#[cfg(not(target_os = "linux"))]
impl PhysicalInput {
    fn neutral(&self) -> Result<bool> {
        Ok(neutral())
    }
    fn escape_down(&self) -> Result<bool> {
        Ok(escape_down())
    }
}
fn wait_for_release(fence: &Fence, physical: &PhysicalInput) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(2);
    while !physical.neutral()? {
        fence.check()?;
        if Instant::now() >= until {
            return Err("请松开快捷键和鼠标后重试".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
#[cfg(windows)]
fn escape_down() -> bool {
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x1b) as u16 & 0x8000 != 0
    }
}
#[cfg(windows)]
fn neutral() -> bool {
    [1, 2, 4, 0x10, 0x11, 0x12, 0x5b, 0x5c]
        .iter()
        .all(|key| unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(*key) as u16 & 0x8000
                == 0
        })
}
#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceFlagsState(source: i32) -> u64;
    fn CGEventSourceButtonState(source: i32, button: u32) -> bool;
    fn CGEventSourceKeyState(source: i32, key: u16) -> bool;
}
#[cfg(target_os = "macos")]
fn escape_down() -> bool {
    unsafe { CGEventSourceKeyState(0, 53) }
}
#[cfg(target_os = "macos")]
fn neutral() -> bool {
    unsafe {
        CGEventSourceFlagsState(0) & ((1 << 17) | (1 << 18) | (1 << 19) | (1 << 20)) == 0
            && !CGEventSourceButtonState(0, 0)
            && !CGEventSourceButtonState(0, 1)
            && !CGEventSourceButtonState(0, 2)
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
fn escape_down() -> bool {
    false
}
#[cfg(not(any(windows, target_os = "macos")))]
fn neutral() -> bool {
    true
}
