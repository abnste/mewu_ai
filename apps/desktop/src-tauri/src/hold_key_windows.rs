// SPDX-License-Identifier: MPL-2.0
//! Modifier-only hold transport. The hook callback only updates a tiny state
//! machine and posts messages; rule dispatch never runs inside the hook.
use std::{
    cell::RefCell,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex, OnceLock,
    },
    time::Instant,
};
use tauri::AppHandle;
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD,
            KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VSC_TO_VK_EX,
        },
        WindowsAndMessaging::*,
    },
};
const PRESS: u32 = WM_APP + 0x501;
const RELEASE: u32 = WM_APP + 0x502;
const STOP: u32 = WM_APP + 0x503;
const TAG: usize = 0x4d455755;
const STATE_TAG: usize = 0x4d455756;
// Unassigned, non-character menu mask documented by AutoHotkey.
// Never use Ctrl/Alt/Shift as a mask: that would change the user's real chord.
const MENU_MASK_KEY: u16 = 0xe8;
static ENABLED: AtomicBool = AtomicBool::new(false);
static THREAD: AtomicU32 = AtomicU32::new(0);
static THRESHOLD: AtomicU32 = AtomicU32::new(400);
static STOPPING: AtomicBool = AtomicBool::new(false);
static EDITING: AtomicBool = AtomicBool::new(false);
static FAULT: AtomicBool = AtomicBool::new(false);
pub fn warning() -> Option<&'static str> {
    FAULT
        .load(Ordering::Acquire)
        .then_some("系统长按快捷键接管失败，请重新启用该规则")
}
const RESET: u32 = WM_APP + 0x504;
pub fn set_editing(value: bool) {
    change_editing(&EDITING, value, || {
        post(RESET);
    });
}
fn change_editing(editing: &AtomicBool, value: bool, reset: impl FnOnce()) {
    // Capture invalidates shortcut editors even when no editor is open. Repeating
    // that inactive state must not manufacture a release of the hold that is
    // currently opening the capture, nor reseed away its suppressed Win key.
    if editing.swap(value, Ordering::AcqRel) != value {
        reset();
    }
}
fn seed() {
    KEYS.with(|cell| {
        let mut keys = cell.borrow_mut();
        *keys = Default::default();
        for key in 7..=254 {
            if crate::hold_sequence::Sequence::physical_keyboard(key)
                && unsafe { GetAsyncKeyState(key as i32) } < 0
            {
                keys.seed_down(key as u32, clock());
            }
        }
    });
}
fn physical_key(event: &KBDLLHOOKSTRUCT) -> u32 {
    match event.vkCode {
        0x10 => {
            let key = unsafe { MapVirtualKeyW(event.scanCode, MAPVK_VSC_TO_VK_EX) };
            if matches!(key, 0xa0 | 0xa1) {
                key
            } else {
                event.vkCode
            }
        }
        0x11 => {
            if event.flags & LLKHF_EXTENDED != 0 {
                0xa3
            } else {
                0xa2
            }
        }
        0x12 => {
            if event.flags & LLKHF_EXTENDED != 0 {
                0xa5
            } else {
                0xa4
            }
        }
        key => key,
    }
}
thread_local! {static KEYS:RefCell<crate::hold_sequence::Sequence>=RefCell::new(Default::default());}
fn clock() -> u64 {
    static CLOCK: OnceLock<Instant> = OnceLock::new();
    CLOCK
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn post(message: u32) -> bool {
    unsafe { PostThreadMessageW(THREAD.load(Ordering::Acquire), message, 0, 0) != 0 }
}
fn modifier_release_input(key: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key as u16,
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY,
                time: 0,
                dwExtraInfo: STATE_TAG,
            },
        },
    }
}
fn masked_modifier_release(key: u32) -> [INPUT; 3] {
    let mask = |up| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: MENU_MASK_KEY,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: TAG,
            },
        },
    };
    // The mask must reach the OS BEFORE the state-only Win release. Suppressing
    // all three messages leaves the shell's lone-Win Start action armed.
    [mask(false), mask(true), modifier_release_input(key)]
}
unsafe fn release_os_modifier(key: u32) -> bool {
    let inputs = masked_modifier_release(key);
    SendInput(
        inputs.len() as u32,
        inputs.as_ptr(),
        std::mem::size_of::<INPUT>() as i32,
    ) == inputs.len() as u32
}
#[derive(Debug, PartialEq)]
enum TaggedRoute {
    Forward,
    Suppress,
}
fn tagged_route(event: &KBDLLHOOKSTRUCT) -> Option<TaggedRoute> {
    if event.dwExtraInfo == STATE_TAG {
        // This key-up MUST reach Windows' input queue. Consuming it in our own
        // hook leaves the system modifier down, so Win+click can still activate
        // the shell even though Mewu consumed the physical keyboard message.
        return Some(
            if matches!(event.vkCode, 0x5b | 0x5c)
                && event.flags & (LLKHF_UP | LLKHF_EXTENDED) == (LLKHF_UP | LLKHF_EXTENDED)
            {
                TaggedRoute::Forward
            } else {
                TaggedRoute::Suppress
            },
        );
    }
    (event.dwExtraInfo == TAG).then_some(TaggedRoute::Forward)
}
unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 || l == 0 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let event = &*(l as *const KBDLLHOOKSTRUCT);
    if let Some(route) = tagged_route(event) {
        return match route {
            TaggedRoute::Forward => CallNextHookEx(null_mut(), code, w, l),
            TaggedRoute::Suppress => 1,
        };
    }
    if !ENABLED.load(Ordering::Acquire) {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    if EDITING.load(Ordering::Acquire) && KEYS.with(|keys| keys.borrow().held.is_none()) {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let down = matches!(w as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    let up = matches!(w as u32, WM_KEYUP | WM_SYSKEYUP);
    if !down && !up {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let key = physical_key(event);
    if event.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) != 0 {
        if up && KEYS.with(|cell| cell.borrow_mut().observed_release(key)) {
            post(RELEASE);
        }
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let consumed = std::panic::catch_unwind(|| {
        KEYS.with(|cell| {
            let decision =
                cell.borrow_mut()
                    .key(key, down, clock(), THRESHOLD.load(Ordering::Acquire));
            // Initial down is cleared by PRESS on this same hook thread AFTER
            // the original callback returns. LowLevelKeyboardProc runs before
            // async key state is updated; clearing inside that callback can be
            // overtaken by the original Win down and leave Win+click active.
            if let Some(key) = decision.clear_os_modifier.filter(|_| !decision.pressed) {
                if !release_os_modifier(key) {
                    cell.borrow_mut().forwarded = true;
                    FAULT.store(true, Ordering::Release);
                    ENABLED.store(false, Ordering::Release);
                    return false;
                }
            }
            if (decision.pressed && !post(PRESS)) || (decision.released && !post(RELEASE)) {
                FAULT.store(true, Ordering::Release);
                ENABLED.store(false, Ordering::Release);
                return false;
            }
            if !decision.replay.is_empty() {
                let inputs: Vec<_> = decision
                    .replay
                    .iter()
                    .map(|(key, up)| INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: *key as u16,
                                wScan: 0,
                                dwFlags: (if *up { KEYEVENTF_KEYUP } else { 0 })
                                    | (if matches!(*key, 0x5b | 0x5c) {
                                        KEYEVENTF_EXTENDEDKEY
                                    } else {
                                        0
                                    }),
                                time: 0,
                                dwExtraInfo: TAG,
                            },
                        },
                    })
                    .collect();
                if SendInput(
                    inputs.len() as u32,
                    inputs.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                ) != inputs.len() as u32
                {
                    FAULT.store(true, Ordering::Release);
                    ENABLED.store(false, Ordering::Release);
                    // The original physical key-up must still release a Win
                    // down if SendInput inserted only part of the tap batch.
                    if up && matches!(event.vkCode, 0x5b | 0x5c) {
                        return false;
                    }
                }
            }
            decision.consume
        })
    })
    .unwrap_or_else(|_| {
        FAULT.store(true, Ordering::Release);
        ENABLED.store(false, Ordering::Release);
        false
    });
    if consumed {
        1
    } else {
        CallNextHookEx(null_mut(), code, w, l)
    }
}
unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && matches!(w as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
        let event = &*(l as *const MSLLHOOKSTRUCT);
        if event.flags & LLMHF_INJECTED == 0 {
            crate::quick_capture::mouse_touch();
        }
    }
    CallNextHookEx(null_mut(), code, w, l)
}
fn worker() -> &'static Mutex<Option<std::thread::JoinHandle<()>>> {
    static WORKER: OnceLock<Mutex<Option<std::thread::JoinHandle<()>>>> = OnceLock::new();
    WORKER.get_or_init(Default::default)
}
pub async fn drain(timeout: std::time::Duration) -> Result<(), String> {
    disable();
    let began = Instant::now();
    loop {
        let finished = worker()
            .lock()
            .map_err(|_| "长按快捷键状态不可用")?
            .as_ref()
            .is_none_or(|thread| thread.is_finished());
        if finished {
            let old = worker().lock().map_err(|_| "长按快捷键状态不可用")?.take();
            if let Some(old) = old {
                old.join().map_err(|_| "长按快捷键线程异常结束")?;
            }
            return Ok(());
        }
        if began.elapsed() >= timeout {
            return Err("长按快捷键尚未停止，已取消退出".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
pub fn configure(app: &AppHandle, milliseconds: u32) -> Result<(), String> {
    let mut worker = worker().lock().map_err(|_| "长按快捷键不可用")?;
    THRESHOLD.store(milliseconds, Ordering::Release);
    FAULT.store(false, Ordering::Release);
    if STOPPING.load(Ordering::Acquire) {
        let until = Instant::now() + std::time::Duration::from_secs(3);
        while worker.as_ref().is_some_and(|w| !w.is_finished()) {
            if Instant::now() >= until {
                return Err("旧长按快捷键正在释放".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if let Some(old) = worker.take() {
            let _ = old.join();
        }
        STOPPING.store(false, Ordering::Release);
    }
    if worker.as_ref().is_some_and(|w| !w.is_finished()) && THREAD.load(Ordering::Acquire) != 0 {
        ENABLED.store(true, Ordering::Release);
        return Ok(());
    }
    let app = app.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    *worker = Some(
        std::thread::Builder::new()
            .name("mewu-hold-key".into())
            .spawn(move || unsafe {
                let mut message: MSG = std::mem::zeroed();
                PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                THREAD.store(GetCurrentThreadId(), Ordering::Release);
                let keyboard_hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard),
                    GetModuleHandleW(std::ptr::null()),
                    0,
                );
                let mut mouse_hook: HHOOK = std::ptr::null_mut();
                if keyboard_hook.is_null() {
                    THREAD.store(0, Ordering::Release);
                    let _ = tx.send(false);
                    return;
                }
                let state_timer = SetTimer(null_mut(), 0, 500, None);
                if state_timer == 0 {
                    UnhookWindowsHookEx(keyboard_hook);
                    THREAD.store(0, Ordering::Release);
                    let _ = tx.send(false);
                    return;
                }
                seed();
                ENABLED.store(true, Ordering::Release);
                let _ = tx.send(true);
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    match message.message {
                        WM_TIMER if message.wParam == state_timer => {
                            let released = KEYS.with(|cell| {
                                cell.borrow_mut()
                                    .reconcile_releases(|key| GetAsyncKeyState(key as i32) < 0)
                            });
                            if released {
                                post(RELEASE);
                            }
                        }
                        PRESS | RELEASE => {
                            let pressed = message.message == PRESS;
                            if pressed {
                                let owned = KEYS.with(|keys| keys.borrow().owned_modifier());
                                let Some(key) = owned else { continue };
                                if !release_os_modifier(key) {
                                    KEYS.with(|keys| keys.borrow_mut().forwarded = true);
                                    FAULT.store(true, Ordering::Release);
                                    ENABLED.store(false, Ordering::Release);
                                    continue;
                                }
                            }
                            if pressed && mouse_hook.is_null() {
                                mouse_hook = SetWindowsHookExW(
                                    WH_MOUSE_LL,
                                    Some(mouse),
                                    GetModuleHandleW(std::ptr::null()),
                                    0,
                                );
                            }
                            if !pressed && !mouse_hook.is_null() {
                                UnhookWindowsHookEx(mouse_hook);
                                mouse_hook = std::ptr::null_mut();
                            }
                            let target = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                crate::hooks::modifier_hold_changed(&target, pressed)
                            });
                        }
                        RESET => {
                            if !mouse_hook.is_null() {
                                UnhookWindowsHookEx(mouse_hook);
                                mouse_hook = std::ptr::null_mut();
                            }
                            let target = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                crate::hooks::modifier_hold_changed(&target, false)
                            });
                            seed();
                        }
                        STOP => break,
                        _ => {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                }
                ENABLED.store(false, Ordering::Release);
                // Cancel our suppressed gesture on shutdown. Never manufacture
                // a Win-down after the OS state was deliberately cleared.
                if let Some(key) = KEYS.with(|keys| keys.borrow().owned_modifier()) {
                    let _ = release_os_modifier(key);
                }
                UnhookWindowsHookEx(keyboard_hook);
                KillTimer(null_mut(), state_timer);
                if !mouse_hook.is_null() {
                    UnhookWindowsHookEx(mouse_hook);
                }
                THREAD.store(0, Ordering::Release);
            })
            .map_err(|_| "无法启动长按快捷键")?,
    );
    match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(true) => Ok(()),
        _ => {
            disable();
            Err("无法安装长按快捷键；普通截图快捷键仍可使用".into())
        }
    }
}
pub fn disable() {
    ENABLED.store(false, Ordering::Release);
    if THREAD.load(Ordering::Acquire) != 0 {
        STOPPING.store(true, Ordering::Release);
        post(STOP);
    }
}

#[cfg(test)]
mod modifier_tests {
    use super::*;
    #[test]
    fn logical_win_release_reaches_the_os_without_becoming_a_physical_release() {
        for key in [0x5b, 0x5c] {
            let mut event = KBDLLHOOKSTRUCT {
                vkCode: key,
                scanCode: 0,
                flags: LLKHF_INJECTED | LLKHF_UP | LLKHF_EXTENDED,
                time: 0,
                dwExtraInfo: STATE_TAG,
            };
            assert_eq!(tagged_route(&event), Some(TaggedRoute::Forward));
            let mut sequence = crate::hold_sequence::Sequence::default();
            sequence.key(key, true, 0, 400);
            // The transport forwards this tagged event before its physical-key
            // state machine, preserving ownership through the shell reset.
            assert_eq!(sequence.owned_modifier(), Some(key));
            event.flags &= !LLKHF_UP;
            assert_eq!(tagged_route(&event), Some(TaggedRoute::Suppress));
            event.vkCode = 0x43;
            event.flags |= LLKHF_UP;
            assert_eq!(tagged_route(&event), Some(TaggedRoute::Suppress));
            event.dwExtraInfo = TAG;
            assert_eq!(tagged_route(&event), Some(TaggedRoute::Forward));
            event.dwExtraInfo = 0;
            assert_eq!(tagged_route(&event), None);
        }
    }
    #[test]
    fn menu_mask_is_balanced_and_os_visible_before_each_owned_win_release() {
        for key in [0x5b, 0x5c] {
            let inputs = masked_modifier_release(key);
            let keyboard = inputs.map(|input| unsafe { input.Anonymous.ki });
            assert_eq!(
                keyboard.map(|key| key.wVk),
                [MENU_MASK_KEY, MENU_MASK_KEY, key as u16]
            );
            assert_eq!(keyboard.map(|key| key.dwExtraInfo), [TAG, TAG, STATE_TAG]);
            assert_eq!(
                keyboard.map(|key| key.dwFlags),
                [0, KEYEVENTF_KEYUP, KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY]
            );
            assert!(keyboard
                .iter()
                .all(|key| key.dwFlags & KEYEVENTF_KEYUP != 0 || key.wVk == MENU_MASK_KEY));
        }
    }
    #[test]
    fn modifier_state_reset_is_a_tagged_extended_key_up_not_a_new_win_press() {
        for key in [0x5b, 0x5c] {
            let input = modifier_release_input(key);
            assert_eq!(input.r#type, INPUT_KEYBOARD);
            let keyboard = unsafe { input.Anonymous.ki };
            assert_eq!(keyboard.wVk, key as u16);
            assert_eq!(keyboard.dwFlags, KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY);
            assert_eq!(keyboard.dwExtraInfo, STATE_TAG);
        }
    }
    #[test]
    fn capture_editor_invalidation_keeps_the_held_gesture_until_physical_release() {
        let editing = AtomicBool::new(false);
        let mut keys = crate::hold_sequence::Sequence::default();
        assert!(keys.key(0x5b, true, 0, 400).pressed);
        let mut resets = 0;
        // capture_screen_inner -> invalidate_all -> sync_system_editor(false).
        // A repeated inactive notification used to reset KEYS and post RELEASE.
        for _ in 0..3 {
            change_editing(&editing, false, || {
                resets += 1;
                keys = Default::default();
            });
        }
        assert_eq!(resets, 0);
        assert_eq!(keys.held, Some((0x5b, 0)));
        let released = keys.key(0x5b, false, 900, 400);
        assert!(released.consume && released.released && released.replay.is_empty());
        for value in [true, true, false, false] {
            change_editing(&editing, value, || resets += 1);
        }
        assert_eq!(
            resets, 2,
            "only actual editor transitions reset the transport"
        );
    }
    #[test]
    fn modifier_releases_use_the_same_physical_identity_as_startup_state() {
        for (vk, scan, flags, expected) in [
            (0x10, 0x2a, 0, 0xa0),
            (0x10, 0x36, 0, 0xa1),
            (0x11, 0, 0, 0xa2),
            (0x11, 0, LLKHF_EXTENDED, 0xa3),
            (0x12, 0, 0, 0xa4),
            (0x12, 0, LLKHF_EXTENDED, 0xa5),
        ] {
            let event = KBDLLHOOKSTRUCT {
                vkCode: vk,
                scanCode: scan,
                flags,
                time: 0,
                dwExtraInfo: 0,
            };
            assert_eq!(physical_key(&event), expected);
        }
    }
}
