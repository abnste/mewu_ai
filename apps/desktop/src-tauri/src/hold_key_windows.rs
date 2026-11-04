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
    Foundation::{GetLastError, SetLastError, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD,
            KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC,
            MAPVK_VSC_TO_VK_EX,
        },
        WindowsAndMessaging::*,
    },
};
const PRESS: u32 = WM_APP + 0x501;
const RELEASE: u32 = WM_APP + 0x502;
const STOP: u32 = WM_APP + 0x503;
const TAG: usize = 0x4d455755;
const STATE_TAG: usize = 0x4d455756;
// PowerToys uses this non-character virtual key to cancel the shell's pending
// lone-modifier action. Do not substitute Ctrl/Alt/Shift (real user chords).
const MENU_MASK_KEY: u16 = 0xff;
const MASK_TAG: usize = 0x4d455757;
const CLEAR_STATE: u32 = WM_APP + 0x505;
const IDLE_STATE_INTERVAL: u32 = 500;
const OWNED_STATE_INTERVAL: u32 = 16;
static ENABLED: AtomicBool = AtomicBool::new(false);
static THREAD: AtomicU32 = AtomicU32::new(0);
static THRESHOLD: AtomicU32 = AtomicU32::new(400);
static STOPPING: AtomicBool = AtomicBool::new(false);
static EDITING: AtomicBool = AtomicBool::new(false);
static FAULT: AtomicBool = AtomicBool::new(false);
type TraceRecord = (u64, &'static str, bool, bool, u32, u32);
static TRACE: OnceLock<Option<std::sync::mpsc::SyncSender<TraceRecord>>> = OnceLock::new();
fn initialize_trace() {
    TRACE.get_or_init(|| {
        let path = std::env::var_os("MEWU_HOLD_TRACE")?;
        let (sender, receiver) = std::sync::mpsc::sync_channel::<TraceRecord>(32);
        std::thread::Builder::new()
            .name("mewu-hold-diagnostics".into())
            .spawn(move || {
                use std::io::Write;
                let Ok(mut file) = std::fs::File::create(path) else { return };
                // Explicit opt-in, bounded output, no ordinary key codes,
                // typed text, screen contents or foreground application data.
                // All file I/O stays off the hook/message-pump thread.
                for (at, event, owned, suppressed, inserted, error) in receiver.into_iter().take(512) {
                    if writeln!(file, "{{\"at\":{at},\"event\":\"{event}\",\"owned\":{owned},\"suppressed\":{suppressed},\"inserted\":{inserted},\"error\":{error}}}").is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(sender)
    });
}
fn trace(event: &'static str, owned: bool, suppressed: bool) {
    if let Some(Some(sender)) = TRACE.get() {
        let _ = sender.try_send((clock(), event, owned, suppressed, 0, 0));
    }
}
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
                wScan: unsafe { MapVirtualKeyW(key, MAPVK_VK_TO_VSC) } as u16,
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
                dwExtraInfo: MASK_TAG,
            },
        },
    };
    // A suppressed Win-down can already arm the shell's lone-Win action.
    // Resetting only its low-level state is therefore insufficient: the shell
    // can open Start on our synthetic release while the physical key is held.
    // Send the balanced, non-character mask BEFORE the private state reset in
    // one batch. The mask is visible; the Win-up itself remains suppressed.
    // https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardmanager.md#dummy-key-events
    [mask(false), mask(true), modifier_release_input(key)]
}
unsafe fn release_os_modifier(key: u32) -> bool {
    let inputs = masked_modifier_release(key);
    SetLastError(0);
    let inserted = SendInput(
        inputs.len() as u32,
        inputs.as_ptr(),
        std::mem::size_of::<INPUT>() as i32,
    );
    let complete = inserted == inputs.len() as u32;
    if !complete {
        let error = GetLastError();
        if let Some(Some(sender)) = TRACE.get() {
            let _ = sender.try_send((clock(), "reset-failed", true, true, inserted, error));
        }
    }
    complete
}
#[derive(Debug, PartialEq)]
enum TaggedRoute {
    Forward,
    Suppress,
}
fn tagged_route(event: &KBDLLHOOKSTRUCT) -> Option<TaggedRoute> {
    if event.dwExtraInfo == STATE_TAG {
        // State-only releases follow PowerToys' lower-level modifier reset:
        // SendInput updates low-level state, but this hook must consume the
        // tagged message. Forwarding it lets the shell treat our bookkeeping
        // as a real Win release and open Start/Click to Do. Only actual tap or
        // chord replay (TAG) belongs in the application's input queue.
        // https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardmanager.md#modifier-caps-lock-interaction-on-japanese-ime-keyboards
        return Some(TaggedRoute::Suppress);
    }
    (event.dwExtraInfo == TAG
        || (event.dwExtraInfo == MASK_TAG && event.vkCode == u32::from(MENU_MASK_KEY)))
    .then_some(TaggedRoute::Forward)
}
unsafe fn replay_keys(keys: &[(u32, bool)]) -> bool {
    if keys.is_empty() {
        return true;
    }
    let inputs: Vec<_> = keys
        .iter()
        .map(|(key, up)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: *key as u16,
                    wScan: MapVirtualKeyW(*key, MAPVK_VK_TO_VSC) as u16,
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
    SendInput(
        inputs.len() as u32,
        inputs.as_ptr(),
        std::mem::size_of::<INPUT>() as i32,
    ) == inputs.len() as u32
}
fn fault() {
    FAULT.store(true, Ordering::Release);
    // An injection failure does not disable our keyboard hook. The physical
    // Win-down has already been consumed: forwarding its later repeats/up
    // would hand half a gesture to the shell and reopen Start. Only explicit
    // configure/disable owns ENABLED; keep receiving physical releases here.
    trace("transport-fault", true, true);
}

fn deliver_decision(
    decision: crate::hold_sequence::Decision,
    key: u32,
    down: bool,
    mut clear: impl FnMut(u32, bool) -> bool,
    mut notify: impl FnMut(u32) -> bool,
    mut replay: impl FnMut(&[(u32, bool)]) -> bool,
    mut failed: impl FnMut(),
) -> bool {
    if let Some(key) = decision.clear_os_modifier.filter(|_| !decision.pressed) {
        if !clear(key, down) {
            failed();
        }
    }
    if (decision.pressed && !notify(PRESS)) || (decision.released && !notify(RELEASE)) {
        failed();
    }
    if !replay(&decision.replay) {
        failed();
        // Only an actual tap/chord replay may have inserted Win-down. Its
        // physical key-up is the fallback release for a partial replay batch.
        if !down && matches!(key, 0x5b | 0x5c) && !decision.replay.is_empty() {
            return false;
        }
    }
    decision.consume
}
fn suppress_foreign_modifier(
    event: &KBDLLHOOKSTRUCT,
    sequence: &crate::hold_sequence::Sequence,
) -> bool {
    event.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) != 0
        && matches!(event.vkCode, 0x5b | 0x5c)
        && sequence.owns_modifier(event.vkCode)
}
unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 || l == 0 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let event = &*(l as *const KBDLLHOOKSTRUCT);
    if let Some(route) = tagged_route(event) {
        if matches!(event.vkCode, 0x5b | 0x5c) {
            trace(
                if event.dwExtraInfo == STATE_TAG {
                    "state-reset"
                } else {
                    "replay"
                },
                false,
                route == TaggedRoute::Suppress,
            );
        }
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
        if matches!(event.vkCode, 0x5b | 0x5c) {
            let owned = KEYS.with(|cell| cell.borrow().owns_modifier(key));
            trace(
                if down { "foreign-down" } else { "foreign-up" },
                owned,
                owned,
            );
        }
        // Keeping physical ownership is not enough: forwarding a foreign
        // synthetic Win-up still lets the shell open Start during our hold.
        // Our own short-tap/chord replay was handled by TAG above. Only a
        // modifier currently owned by this gesture is suppressed here.
        if KEYS.with(|cell| suppress_foreign_modifier(event, &cell.borrow())) {
            if down
                && PostThreadMessageW(
                    THREAD.load(Ordering::Acquire),
                    CLEAR_STATE,
                    key as WPARAM,
                    0,
                ) == 0
            {
                fault();
            }
            return 1;
        }
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
            deliver_decision(
                decision,
                key,
                down,
                |key, down| {
                    // Defer down-state resets until the callback returns and
                    // recheck ownership before touching a possible Win chord.
                    if down {
                        PostThreadMessageW(
                            THREAD.load(Ordering::Acquire),
                            CLEAR_STATE,
                            key as WPARAM,
                            0,
                        ) != 0
                    } else {
                        release_os_modifier(key)
                    }
                },
                post,
                |keys| replay_keys(keys),
                || {
                    cell.borrow_mut().cancel_owned();
                    fault();
                },
            )
        })
    })
    .unwrap_or_else(|_| {
        fault();
        KEYS.with(|cell| {
            let mut keys = cell.borrow_mut();
            keys.cancel_owned();
            keys.has_owned_modifiers()
        })
    });
    if matches!(key, 0x5b | 0x5c) {
        trace(
            if down { "physical-down" } else { "physical-up" },
            KEYS.with(|cell| cell.borrow().has_owned_modifiers()),
            consumed,
        );
    }
    if consumed {
        1
    } else {
        CallNextHookEx(null_mut(), code, w, l)
    }
}
unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0
        && l != 0
        && ENABLED.load(Ordering::Acquire)
        && matches!(
            w as u32,
            WM_LBUTTONDOWN
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_XBUTTONDOWN
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
        )
    {
        let event = &*(l as *const MSLLHOOKSTRUCT);
        if event.flags & LLMHF_INJECTED == 0 {
            let handled = std::panic::catch_unwind(|| {
                let decision = KEYS.with(|keys| {
                    keys.borrow_mut()
                        .mouse(clock(), THRESHOLD.load(Ordering::Acquire))
                });
                // The click must not reach a lower hook with Win logically
                // down after a consumed repeat. Before the threshold, restore
                // the ordinary Win+mouse chord instead of stealing it.
                let mut cleared = true;
                for key in [0x5b, 0x5c] {
                    if KEYS.with(|keys| keys.borrow().owns_modifier(key)) {
                        cleared &= release_os_modifier(key);
                    }
                }
                if !cleared
                    || !replay_keys(&decision.replay)
                    || (decision.released && !post(RELEASE))
                {
                    fault();
                }
            });
            if handled.is_err() {
                fault();
            }
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
    initialize_trace();
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
                if keyboard_hook.is_null() {
                    THREAD.store(0, Ordering::Release);
                    let _ = tx.send(false);
                    return;
                }
                // Listen from startup, not only after PRESS is dequeued: a
                // quick Win+click/scroll is a normal chord and must cancel the
                // pending hold before it can open Mewu.
                let mouse_hook = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(mouse),
                    GetModuleHandleW(std::ptr::null()),
                    0,
                );
                if mouse_hook.is_null() {
                    UnhookWindowsHookEx(keyboard_hook);
                    THREAD.store(0, Ordering::Release);
                    let _ = tx.send(false);
                    return;
                }
                let mut state_interval = IDLE_STATE_INTERVAL;
                let mut state_timer = SetTimer(null_mut(), 0, state_interval, None);
                if state_timer == 0 {
                    UnhookWindowsHookEx(keyboard_hook);
                    UnhookWindowsHookEx(mouse_hook);
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
                            // Suppression is not a one-shot operation: a later
                            // native repeat/state update can land after CLEAR_STATE.
                            // Keep the system neutral for the duration of our own
                            // physical hold, without touching forwarded Win chords.
                            let reasserted = KEYS.with(|cell| {
                                cell.borrow()
                                    .reasserted_modifiers(|key| GetAsyncKeyState(key as i32) < 0)
                            });
                            for key in reasserted {
                                if !release_os_modifier(key) {
                                    fault();
                                }
                            }
                            let released = KEYS.with(|cell| {
                                cell.borrow_mut()
                                    .reconcile_releases(|key| GetAsyncKeyState(key as i32) < 0)
                            });
                            if released {
                                post(RELEASE);
                            }
                        }
                        CLEAR_STATE => {
                            let key = message.wParam as u32;
                            if KEYS.with(|keys| keys.borrow().owns_modifier(key))
                                && !release_os_modifier(key)
                            {
                                fault();
                            }
                        }
                        PRESS | RELEASE => {
                            let pressed = message.message == PRESS;
                            if pressed {
                                let owned = KEYS.with(|keys| keys.borrow().owned_modifier());
                                let Some(key) = owned else { continue };
                                if !release_os_modifier(key) {
                                    KEYS.with(|keys| keys.borrow_mut().cancel_owned());
                                    fault();
                                    continue;
                                }
                            }
                            let target = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                crate::hooks::modifier_hold_changed(&target, pressed)
                            });
                        }
                        RESET => {
                            let target = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                crate::hooks::modifier_hold_changed(&target, false)
                            });
                            if KEYS.with(|keys| keys.borrow().has_owned_modifiers()) {
                                KEYS.with(|keys| keys.borrow_mut().cancel_owned());
                            } else {
                                seed();
                            }
                        }
                        STOP => break,
                        _ => {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    let wanted_interval = if KEYS.with(|keys| keys.borrow().has_owned_modifiers()) {
                        OWNED_STATE_INTERVAL
                    } else {
                        IDLE_STATE_INTERVAL
                    };
                    if wanted_interval != state_interval {
                        let timer = SetTimer(null_mut(), state_timer, wanted_interval, None);
                        if timer == 0 {
                            fault();
                        } else {
                            state_timer = timer;
                            state_interval = wanted_interval;
                        }
                    }
                }
                ENABLED.store(false, Ordering::Release);
                // Cancel our suppressed gesture on shutdown. Never manufacture
                // a Win-down after the OS state was deliberately cleared.
                for key in [0x5b, 0x5c] {
                    if KEYS.with(|keys| keys.borrow().owns_modifier(key)) {
                        let _ = release_os_modifier(key);
                    }
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
    fn reset_delivery_failure_keeps_repeat_and_release_consumed_then_allows_next_tap() {
        for win in [0x5b, 0x5c] {
            let mut sequence = crate::hold_sequence::Sequence::default();
            sequence.key(win, true, 0, 400);
            let mut failures = 0;
            let mut notifications = Vec::new();
            for (down, at) in [(true, 500), (true, 600), (false, 800)] {
                let decision = sequence.key(win, down, at, 400);
                assert!(deliver_decision(
                    decision,
                    win,
                    down,
                    |_, _| false,
                    |message| {
                        notifications.push(message);
                        true
                    },
                    |keys| {
                        assert!(keys.is_empty());
                        true
                    },
                    || {
                        failures += 1;
                        sequence.cancel_owned();
                    }
                ));
            }
            assert_eq!(failures, 3);
            assert_eq!(notifications, vec![RELEASE]);
            assert!(!sequence.has_owned_modifiers());
            sequence.key(win, true, 1000, 400);
            assert_eq!(
                sequence.key(win, false, 1100, 400).replay,
                vec![(win, false), (win, true)]
            );
        }
    }
    #[test]
    fn failed_hold_dispatch_cannot_forward_half_a_gesture_or_replay_it_as_a_tap() {
        let mut sequence = crate::hold_sequence::Sequence::default();
        let decision = sequence.key(0x5b, true, 0, 400);
        assert!(deliver_decision(
            decision,
            0x5b,
            true,
            |_, _| true,
            |_| false,
            |_| true,
            || sequence.cancel_owned()
        ));
        assert_eq!(sequence.owned_modifier(), Some(0x5b));
        let decision = sequence.key(0x5b, false, 100, 400);
        assert!(decision.replay.is_empty());
        assert!(deliver_decision(
            decision,
            0x5b,
            false,
            |_, _| false,
            |_| true,
            |_| true,
            || {}
        ));
        assert!(!sequence.has_owned_modifiers());
    }
    #[test]
    fn only_failed_real_replay_can_forward_physical_up_as_fallback() {
        for duration in [100, 800] {
            let mut sequence = crate::hold_sequence::Sequence::default();
            sequence.key(0x5b, true, 0, 400);
            let decision = sequence.key(0x5b, false, duration, 400);
            let consume = deliver_decision(
                decision,
                0x5b,
                false,
                |_, _| false,
                |_| true,
                |_| false,
                || {},
            );
            assert_eq!(consume, duration >= 400);
        }
    }
    #[test]
    fn reporting_transport_failure_does_not_disable_the_registered_hook() {
        let previous_enabled = ENABLED.swap(true, Ordering::AcqRel);
        let previous_fault = FAULT.load(Ordering::Acquire);
        fault();
        let still_enabled = ENABLED.load(Ordering::Acquire);
        let warned = FAULT.load(Ordering::Acquire);
        ENABLED.store(previous_enabled, Ordering::Release);
        FAULT.store(previous_fault, Ordering::Release);
        assert!(still_enabled && warned);
    }
    #[test]
    fn menu_cancellation_precedes_private_release_without_replaying_win_down() {
        for win in [0x5b, 0x5c] {
            let inputs = masked_modifier_release(win);
            let keys = inputs.map(|input| unsafe { input.Anonymous.ki });
            assert_eq!(
                keys.map(|key| key.wVk),
                [MENU_MASK_KEY, MENU_MASK_KEY, win as u16]
            );
            assert_eq!(
                keys.map(|key| key.dwFlags),
                [0, KEYEVENTF_KEYUP, KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY]
            );
            let routes = keys.map(|key| {
                tagged_route(&KBDLLHOOKSTRUCT {
                    vkCode: key.wVk.into(),
                    scanCode: key.wScan.into(),
                    flags: LLKHF_INJECTED
                        | if key.dwFlags & KEYEVENTF_KEYUP != 0 {
                            LLKHF_UP
                        } else {
                            0
                        },
                    time: 0,
                    dwExtraInfo: key.dwExtraInfo,
                })
            });
            assert_eq!(
                routes,
                [
                    Some(TaggedRoute::Forward),
                    Some(TaggedRoute::Forward),
                    Some(TaggedRoute::Suppress)
                ]
            );
            assert!(keys
                .iter()
                .all(|key| key.dwFlags & KEYEVENTF_KEYUP != 0 || key.wVk == MENU_MASK_KEY));
        }
    }
    #[test]
    fn menu_mask_does_not_convert_owned_holds_into_chords_or_replace_short_taps() {
        for win in [0x5b, 0x5c] {
            let mut sequence = crate::hold_sequence::Sequence::default();
            sequence.key(win, true, 0, 400);
            for input in masked_modifier_release(win) {
                let key = unsafe { input.Anonymous.ki };
                let event = KBDLLHOOKSTRUCT {
                    vkCode: key.wVk.into(),
                    scanCode: key.wScan.into(),
                    flags: LLKHF_INJECTED,
                    time: 0,
                    dwExtraInfo: key.dwExtraInfo,
                };
                // All private events are routed before the physical state machine.
                assert!(tagged_route(&event).is_some());
            }
            assert_eq!(sequence.owned_modifier(), Some(win));
            assert!(sequence.key(win, false, 800, 400).replay.is_empty());
            sequence.key(win, true, 1000, 400);
            assert_eq!(
                sequence.key(win, false, 1100, 400).replay,
                vec![(win, false), (win, true)]
            );
            sequence.key(win, true, 2000, 400);
            assert_eq!(
                sequence.key(0x45, true, 2100, 400).replay,
                vec![(win, false)]
            );
            assert!(!sequence.key(win, false, 2200, 400).consume);
        }
    }
    #[test]
    fn foreign_win_events_cannot_escape_an_owned_hold_but_normal_chords_still_pass() {
        for win in [0x5b, 0x5c] {
            let mut sequence = crate::hold_sequence::Sequence::default();
            let mut event = KBDLLHOOKSTRUCT {
                vkCode: win,
                scanCode: 0,
                flags: LLKHF_INJECTED | LLKHF_EXTENDED,
                time: 0,
                dwExtraInfo: 0,
            };
            assert!(!suppress_foreign_modifier(&event, &sequence));
            sequence.key(win, true, 0, 400);
            assert!(suppress_foreign_modifier(&event, &sequence));
            event.flags |= LLKHF_UP;
            assert!(suppress_foreign_modifier(&event, &sequence));
            assert!(!sequence.observed_release(win));
            event.vkCode = 0x45;
            assert!(!suppress_foreign_modifier(&event, &sequence));
            event.vkCode = win;
            sequence.key(0x45, true, 100, 400);
            assert!(!suppress_foreign_modifier(&event, &sequence));
            sequence.key(win, false, 150, 400);
            sequence.key(0x45, false, 150, 400);
            sequence.key(win, true, 200, 400);
            event.flags = LLKHF_UP;
            assert!(!suppress_foreign_modifier(&event, &sequence));
            event.flags |= LLKHF_INJECTED;
            assert!(suppress_foreign_modifier(&event, &sequence));
            assert!(sequence.key(win, false, 800, 400).replay.is_empty());
            assert!(!suppress_foreign_modifier(&event, &sequence));
        }
    }
    #[test]
    fn actual_hook_consumes_private_resets_before_any_shell_or_physical_key_handling() {
        // Invoke the real callback with a private message in memory. This test
        // never calls SendInput or injects a Windows key into the desktop.
        for key in [0x5b, 0x5c] {
            let event = KBDLLHOOKSTRUCT {
                vkCode: key,
                scanCode: key,
                flags: LLKHF_INJECTED | LLKHF_UP | LLKHF_EXTENDED,
                time: 0,
                dwExtraInfo: STATE_TAG,
            };
            let result = unsafe {
                keyboard(
                    HC_ACTION as i32,
                    WM_KEYUP as WPARAM,
                    &event as *const KBDLLHOOKSTRUCT as LPARAM,
                )
            };
            assert_eq!(result, 1, "state bookkeeping must not reach the shell");
        }
    }
    #[test]
    fn state_reset_is_consumed_without_delivering_a_shell_win_release() {
        for key in [0x5b, 0x5c] {
            let mut event = KBDLLHOOKSTRUCT {
                vkCode: key,
                scanCode: 0,
                flags: LLKHF_INJECTED | LLKHF_UP | LLKHF_EXTENDED,
                time: 0,
                dwExtraInfo: STATE_TAG,
            };
            assert_eq!(tagged_route(&event), Some(TaggedRoute::Suppress));
            let mut sequence = crate::hold_sequence::Sequence::default();
            sequence.key(key, true, 0, 400);
            // Physical ownership survives our own hidden reset. Lower hooks
            // and the shell must never see this artificial key-up.
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
    fn an_owned_state_reset_contains_no_shell_visible_keys_or_dummy_key_downs() {
        for key in [0x5b, 0x5c] {
            let input = modifier_release_input(key);
            let keyboard = unsafe { input.Anonymous.ki };
            assert_eq!(keyboard.wVk, key as u16);
            assert_eq!(keyboard.dwExtraInfo, STATE_TAG);
            assert_eq!(keyboard.dwFlags, KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY);
            let event = KBDLLHOOKSTRUCT {
                vkCode: keyboard.wVk as u32,
                scanCode: keyboard.wScan as u32,
                flags: LLKHF_INJECTED | LLKHF_UP | LLKHF_EXTENDED,
                time: 0,
                dwExtraInfo: keyboard.dwExtraInfo,
            };
            assert_eq!(tagged_route(&event), Some(TaggedRoute::Suppress));
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
