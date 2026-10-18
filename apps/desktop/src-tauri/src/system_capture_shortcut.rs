// SPDX-License-Identifier: MPL-2.0
//! Process-scoped Windows screenshot shortcut. Never changes Windows policies,
//! protocol associations, executable registrations, or the user's other hotkeys.

#[cfg(target_os = "windows")]
mod native {
    use std::{
        cell::RefCell,
        panic::{catch_unwind, AssertUnwindSafe},
        ptr::{null, null_mut},
        sync::{
            atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
            mpsc::{self, Receiver},
            Arc, Mutex, OnceLock,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };
    use tauri::AppHandle;
    use windows_sys::Win32::{
        Foundation::{GetLastError, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_KEYUP,
            },
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
                SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
                LLKHF_ALTDOWN, LLKHF_LOWER_IL_INJECTED, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_APP,
                WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
            },
        },
    };

    const CAPTURE: u32 = WM_APP + 0x3a1;
    const STOP: u32 = WM_APP + 0x3a2;
    const CALLBACK_FAILED: u32 = u32::MAX;
    const MASK_FAILED: u32 = u32::MAX - 1;
    const WAIT: Duration = Duration::from_secs(3);
    static ENABLED: AtomicBool = AtomicBool::new(false);
    static EDITING: AtomicBool = AtomicBool::new(false);
    static STOPPING: AtomicBool = AtomicBool::new(false);
    static PENDING: AtomicU64 = AtomicU64::new(0);
    static THREAD_ID: AtomicU32 = AtomicU32::new(0);
    static FAULT: AtomicU32 = AtomicU32::new(0);
    static EPOCH: AtomicU64 = AtomicU64::new(0);
    static RUNTIME: OnceLock<Mutex<Runtime>> = OnceLock::new();
    thread_local! {
        static KEYS: RefCell<Option<super::Sequence>> = const { RefCell::new(None) };
    }

    #[derive(Default)]
    struct Runtime {
        worker: Option<Worker>,
        generation: u64,
        warning: Option<String>,
    }
    struct Worker {
        signal: Arc<Signal>,
        ready: Receiver<Result<u32, u32>>,
        stopped: Receiver<Result<(), u32>>,
        thread: JoinHandle<()>,
    }
    #[derive(Default)]
    struct Signal {
        id: AtomicU32,
        stop: AtomicBool,
    }
    fn invalidate() {
        EPOCH.fetch_add(1, Ordering::AcqRel);
        PENDING.store(0, Ordering::Release);
    }
    fn cleanup_pending() -> String {
        "系统截图快捷键尚未释放，请稍后重试".into()
    }
    fn runtime() -> &'static Mutex<Runtime> {
        RUNTIME.get_or_init(|| Mutex::new(Runtime::default()))
    }
    fn failure(code: u32) -> String {
        if code == MASK_FAILED {
            return "系统截图快捷键未能接管此次按键".into();
        }
        if code == 0 || code == CALLBACK_FAILED {
            "无法接管系统截图快捷键".into()
        } else {
            format!("无法接管系统截图快捷键（Windows，0x{code:08X}）")
        }
    }

    impl Worker {
        fn spawn(app: &AppHandle) -> Result<Self, String> {
            Self::spawn_internal(Some(app.clone()))
        }
        fn spawn_internal(app: Option<AppHandle>) -> Result<Self, String> {
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let (stopped_tx, stopped_rx) = mpsc::channel();
            let signal = Arc::new(Signal::default());
            let thread_signal = signal.clone();
            let thread = thread::Builder::new()
                .name("mewu-system-capture".into())
                .spawn(move || {
                    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                        let mut message = MSG::default();
                        // Create the queue before publishing its thread ID.
                        PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                        let id = GetCurrentThreadId();
                        thread_signal.id.store(id, Ordering::Release);
                        if thread_signal.stop.load(Ordering::Acquire) {
                            return;
                        }
                        let mut keys = super::Sequence::default();
                        keys.s_down = GetAsyncKeyState(super::S as i32) < 0;
                        KEYS.with(|state| *state.borrow_mut() = Some(keys));
                        let module = GetModuleHandleW(null());
                        if module.is_null() {
                            let _ = ready_tx.send(Err(GetLastError()));
                            return;
                        }
                        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0);
                        if hook.is_null() {
                            let _ = ready_tx.send(Err(GetLastError()));
                            return;
                        }
                        let mut owned = OwnedHook(Some(hook));
                        THREAD_ID.store(id, Ordering::Release);
                        if ready_tx.send(Ok(id)).is_err() {
                            return;
                        }
                        loop {
                            // A start timeout may precede publication of our queue.
                            // Its retained owner sets stop before attempting Post.
                            let result = if thread_signal.stop.load(Ordering::Acquire) {
                                message.message = STOP;
                                1
                            } else {
                                GetMessageW(&mut message, null_mut(), 0, 0)
                            };
                            if result <= 0 {
                                let code = if result < 0 {
                                    GetLastError()
                                } else {
                                    CALLBACK_FAILED
                                };
                                FAULT.store(code.max(1), Ordering::Release);
                                let _ = stopped_tx.send(Err(code));
                                break;
                            }
                            match message.message {
                                CAPTURE => {
                                    let epoch =
                                        super::epoch_from_parts(message.wParam, message.lParam);
                                    if epoch != 0
                                        && PENDING
                                            .compare_exchange(
                                                epoch,
                                                0,
                                                Ordering::AcqRel,
                                                Ordering::Acquire,
                                            )
                                            .is_ok()
                                        && dispatch_allowed(epoch)
                                    {
                                        // This entry only posts to the Tauri main loop. No
                                        // WebView work, disk access or waiting on that loop.
                                        if let Some(app) = &app {
                                            crate::capture_system_shortcut_pressed(app, epoch);
                                        }
                                    }
                                }
                                STOP => {
                                    let hook = owned.0.expect("owned keyboard hook");
                                    if UnhookWindowsHookEx(hook) == 0 {
                                        let _ = stopped_tx.send(Err(GetLastError()));
                                        // Admission is closed. Keep the exact handle and
                                        // message pump alive so cleanup can be retried.
                                        thread_signal.stop.store(false, Ordering::Release);
                                        continue;
                                    }
                                    owned.0 = None;
                                    let _ = stopped_tx.send(Ok(()));
                                    break;
                                }
                                _ => {
                                    TranslateMessage(&message);
                                    DispatchMessageW(&message);
                                }
                            }
                        }
                    }));
                    ENABLED.store(false, Ordering::Release);
                    PENDING.store(0, Ordering::Release);
                    THREAD_ID.store(0, Ordering::Release);
                    thread_signal.id.store(0, Ordering::Release);
                    KEYS.with(|state| *state.borrow_mut() = None);
                    if result.is_err() {
                        FAULT.store(CALLBACK_FAILED, Ordering::Release);
                    }
                })
                .map_err(|_| failure(0))?;
            Ok(Self {
                signal,
                ready: ready_rx,
                stopped: stopped_rx,
                thread,
            })
        }
        fn wait_ready(&mut self) -> Result<(), String> {
            match self.ready.recv_timeout(WAIT) {
                Ok(Ok(id))
                    if !self.thread.is_finished() && THREAD_ID.load(Ordering::Acquire) == id =>
                {
                    Ok(())
                }
                Ok(Err(code)) => Err(failure(code)),
                _ => {
                    // The Runtime already owns self, including a late-created hook.
                    self.request_stop();
                    Err(cleanup_pending())
                }
            }
        }
        fn request_stop(&self) {
            ENABLED.store(false, Ordering::Release);
            self.signal.stop.store(true, Ordering::Release);
            let id = self.signal.id.load(Ordering::Acquire);
            if id != 0 {
                unsafe {
                    PostThreadMessageW(id, STOP, 0, 0);
                }
            }
        }

        fn stop(&mut self) -> Result<(), String> {
            ENABLED.store(false, Ordering::Release);
            PENDING.store(0, Ordering::Release);
            if self.thread.is_finished() {
                return Ok(());
            }
            self.request_stop();
            let deadline = Instant::now() + WAIT;
            match self.stopped.recv_timeout(WAIT) {
                Ok(Ok(())) => Ok(()),
                Ok(Err(code)) => Err(failure(code)),
                Err(_) if self.thread.is_finished() => return Ok(()),
                Err(_) => return Err(cleanup_pending()),
            }?;
            while !self.thread.is_finished() {
                if Instant::now() >= deadline {
                    return Err(cleanup_pending());
                }
                thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        }
    }
    struct OwnedHook(Option<HHOOK>);
    impl Drop for OwnedHook {
        fn drop(&mut self) {
            if let Some(hook) = self.0.take() {
                unsafe {
                    UnhookWindowsHookEx(hook);
                }
            }
        }
    }

    // WH_KEYBOARD_LL executes on our own message-pump thread. Thread-local fixed
    // key state and a coalesced thread message keep this callback free of locks,
    // allocations, application callbacks and waits. Only the matched chord gets
    // the shell modifier mask below; Win, Shift and S are never replayed.
    unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code != 0 || lparam == 0 {
            return CallNextHookEx(null_mut(), code, wparam, lparam);
        }
        let handled = catch_unwind(AssertUnwindSafe(|| {
            let down = match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => true,
                WM_KEYUP | WM_SYSKEYUP => false,
                _ => return false,
            };
            let event = &*(lparam as *const KBDLLHOOKSTRUCT);
            if event.vkCode == super::MASK_KEY as u32 && event.dwExtraInfo == super::MASK_MARKER {
                // The shell must see our dummy pair. In particular, do not try
                // to borrow the key state recursively during SendInput.
                return false;
            }
            if !super::accepts_input(event.flags & LLKHF_LOWER_IL_INJECTED != 0) {
                return false;
            }
            // Bind the request before observing the original key sequence. A
            // later edit/off/on transition cannot turn it into a new request.
            let epoch = EPOCH.load(Ordering::Acquire);
            let observed = KEYS.with(|state| {
                let Ok(mut state) = state.try_borrow_mut() else {
                    return None;
                };
                let Some(keys) = state.as_mut() else {
                    return None;
                };
                let first_s_down = event.vkCode == super::S && down && !keys.s_down;
                let mut blocked_extra = event.flags & LLKHF_ALTDOWN != 0;
                if event.vkCode == super::S && down {
                    // Only clear stale modifier state; never acquire a modifier
                    // from an unobserved/lower-integrity injected key event.
                    keys.clear_released_modifiers(|key| GetAsyncKeyState(key as i32) < 0);
                    blocked_extra |= super::MODIFIERS[4..]
                        .iter()
                        .any(|key| GetAsyncKeyState(*key as i32) < 0);
                }
                let decision = keys.key(
                    event.vkCode,
                    down,
                    ENABLED.load(Ordering::Acquire) && !blocked_extra,
                    !EDITING.load(Ordering::Acquire),
                );
                let mask = first_s_down && decision.consume;
                Some((decision, mask))
            });
            let Some((decision, mask)) = observed else {
                return false;
            };
            if decision.consume
                && (EPOCH.load(Ordering::Acquire) != epoch || !ENABLED.load(Ordering::Acquire))
            {
                release_suppression();
                return false;
            }
            if mask && !mask_windows_modifiers() {
                // SendInput does not report a reliable UIPI-specific last error.
                // Do not guess a code, replay the chord, or dispatch a screenshot.
                FAULT.store(MASK_FAILED, Ordering::Release);
                release_suppression();
                return false;
            }
            if mask {
                let _ = FAULT.compare_exchange(MASK_FAILED, 0, Ordering::AcqRel, Ordering::Acquire);
            }
            if decision.fire && !dispatch_allowed(epoch) {
                release_suppression();
                return false;
            }
            if decision.fire
                && PENDING
                    .compare_exchange(0, epoch, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                let (low, high) = super::epoch_parts(epoch);
                if PostThreadMessageW(THREAD_ID.load(Ordering::Acquire), CAPTURE, low, high) == 0 {
                    let _ = PENDING.compare_exchange(epoch, 0, Ordering::AcqRel, Ordering::Acquire);
                    FAULT.store(GetLastError().max(1), Ordering::Release);
                    release_suppression();
                    return false;
                }
            }
            decision.consume
        }));
        match handled {
            Ok(true) => 1,
            Ok(false) => CallNextHookEx(null_mut(), code, wparam, lparam),
            Err(_) => {
                ENABLED.store(false, Ordering::Release);
                FAULT.store(CALLBACK_FAILED, Ordering::Release);
                CallNextHookEx(null_mut(), code, wparam, lparam)
            }
        }
    }

    fn release_suppression() {
        KEYS.with(|state| {
            if let Ok(mut state) = state.try_borrow_mut() {
                if let Some(keys) = state.as_mut() {
                    keys.reject_capture();
                }
            }
        });
    }
    unsafe fn mask_windows_modifiers() -> bool {
        // PowerToys uses VK 0xFF to prevent releasing a remapped Win chord from
        // opening Start. It is a shell convention, not a Win32 compatibility
        // guarantee; actual Win-key/Start behavior still needs device testing.
        // https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardmanager.md
        super::send_mask(|events| {
            let mut input = [INPUT::default(); 2];
            for (index, event) in events.iter().enumerate() {
                input[index] = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: event.key,
                            wScan: 0,
                            dwFlags: if event.up { KEYEVENTF_KEYUP } else { 0 },
                            time: 0,
                            dwExtraInfo: event.marker,
                        },
                    },
                };
            }
            SendInput(
                events.len() as u32,
                input.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        })
    }

    fn set(runtime: &mut Runtime, app: &AppHandle, enabled: bool) -> Result<(), String> {
        if enabled && STOPPING.load(Ordering::Acquire) {
            return Err("正在退出，无法接管系统截图快捷键".into());
        }
        if let Some(worker) = runtime.worker.as_ref() {
            if worker.thread.is_finished() {
                let worker = runtime.worker.take().expect("existing worker");
                let _ = worker.thread.join();
            }
        }
        if enabled
            && ENABLED.load(Ordering::Acquire)
            && runtime.worker.is_some()
            && FAULT.load(Ordering::Acquire) == 0
        {
            return Ok(());
        }
        if let Some(worker) = runtime.worker.as_mut() {
            worker.stop()?;
            let worker = runtime.worker.take().expect("stopped worker");
            worker.thread.join().map_err(|_| failure(CALLBACK_FAILED))?;
        }
        FAULT.store(0, Ordering::Release);
        if enabled {
            runtime.worker = Some(Worker::spawn(app)?);
            runtime
                .worker
                .as_mut()
                .expect("owned starting worker")
                .wait_ready()?;
            if STOPPING.load(Ordering::Acquire) {
                runtime
                    .worker
                    .as_ref()
                    .expect("owned starting worker")
                    .request_stop();
                return Err("正在退出，无法接管系统截图快捷键".into());
            }
            ENABLED.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// Called from setup or a worker; this waits for the independent hook thread.
    /// A failure does not affect the tray or the existing capture shortcut.
    pub fn initialize(app: &AppHandle, enabled: bool) -> Result<(), String> {
        let mut runtime = runtime().lock().map_err(|_| failure(0))?;
        if STOPPING.load(Ordering::Acquire) {
            return Ok(());
        }
        // A late startup task must not overwrite an already applied settings save.
        if runtime.generation != 0 {
            return Ok(());
        }
        invalidate();
        runtime.generation = 1;
        match set(&mut runtime, app, enabled) {
            Ok(()) => {
                runtime.warning = None;
                Ok(())
            }
            Err(error) => {
                runtime.warning = Some(error.clone());
                Err(error)
            }
        }
    }

    #[must_use = "commit the preference change or explicitly roll it back"]
    pub struct Change {
        app: AppHandle,
        previous: bool,
        previous_warning: Option<String>,
        previous_fault: u32,
        generation: u64,
    }
    impl Change {
        pub fn apply(app: &AppHandle, enabled: bool) -> Result<Self, String> {
            let mut runtime = runtime().lock().map_err(|_| failure(0))?;
            let previous = ENABLED.load(Ordering::Acquire);
            let previous_warning = runtime.warning.clone();
            let previous_fault = FAULT.load(Ordering::Acquire);
            let generation = runtime
                .generation
                .checked_add(1)
                .ok_or_else(|| failure(0))?;
            invalidate();
            if let Err(error) = set(&mut runtime, app, enabled) {
                runtime.warning = Some(error.clone());
                return Err(error);
            }
            runtime.generation = generation;
            runtime.warning = None;
            Ok(Self {
                app: app.clone(),
                previous,
                previous_warning,
                previous_fault,
                generation: runtime.generation,
            })
        }
        pub fn commit(self) {}
        pub fn rollback(self) -> Result<(), String> {
            let mut runtime = runtime().lock().map_err(|_| failure(0))?;
            if runtime.generation != self.generation {
                return Err("系统截图快捷键设置已改变".into());
            }
            invalidate();
            if let Err(error) = set(&mut runtime, &self.app, self.previous) {
                runtime.warning = Some(error.clone());
                return Err(error);
            }
            runtime.warning = self.previous_warning;
            FAULT.store(self.previous_fault, Ordering::Release);
            Ok(())
        }
    }

    /// Lock-free because callers may own the existing shortcut editor lock.
    pub fn set_editing(_app: &AppHandle, editing: bool) {
        if EDITING.swap(editing, Ordering::AcqRel) != editing {
            invalidate();
        }
    }
    pub fn dispatch_epoch() -> u64 {
        EPOCH.load(Ordering::Acquire)
    }
    pub fn dispatch_allowed(epoch: u64) -> bool {
        ENABLED.load(Ordering::Acquire)
            && !EDITING.load(Ordering::Acquire)
            && EPOCH.load(Ordering::Acquire) == epoch
    }
    pub fn warning() -> Option<String> {
        let fault = FAULT.load(Ordering::Acquire);
        if fault != 0 {
            return Some(failure(fault));
        }
        match runtime().try_lock() {
            Ok(runtime) => runtime.warning.clone(),
            Err(std::sync::TryLockError::WouldBlock) => Some("系统截图快捷键正在切换".into()),
            Err(std::sync::TryLockError::Poisoned(_)) => Some(failure(0)),
        }
    }
    /// No new callbacks are admitted before exact-handle unhook and thread join.
    pub fn shutdown(app: &AppHandle) -> Result<(), String> {
        STOPPING.store(true, Ordering::Release);
        ENABLED.store(false, Ordering::Release);
        invalidate();
        Change::apply(app, false).map(Change::commit)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn system_capture_shortcut_real_hook_installs_and_unhooks_without_input() {
            let runtime = runtime().lock().unwrap();
            assert!(runtime.worker.is_none());
            assert!(!ENABLED.load(Ordering::Acquire));
            let mut worker = Worker::spawn_internal(None).unwrap();
            worker.wait_ready().unwrap();
            let id = worker.signal.id.load(Ordering::Acquire);
            assert_ne!(id, 0);
            assert_eq!(THREAD_ID.load(Ordering::Acquire), id);
            // Admission stays closed: no synthesized keys, foreground changes,
            // screenshot, or real keystroke suppression are part of this test.
            worker.stop().unwrap();
            worker.thread.join().unwrap();
            assert_eq!(THREAD_ID.load(Ordering::Acquire), 0);
            assert!(!ENABLED.load(Ordering::Acquire));
        }
    }
}

#[cfg(target_os = "windows")]
pub use native::{
    dispatch_allowed, dispatch_epoch, initialize, set_editing, shutdown, warning, Change,
};

#[cfg(not(target_os = "windows"))]
pub fn initialize(_app: &tauri::AppHandle, _enabled: bool) -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "windows"))]
pub fn set_editing(_app: &tauri::AppHandle, _editing: bool) {}
#[cfg(not(target_os = "windows"))]
pub fn dispatch_epoch() -> u64 {
    0
}
#[cfg(not(target_os = "windows"))]
pub fn dispatch_allowed(_epoch: u64) -> bool {
    false
}
#[cfg(not(target_os = "windows"))]
pub fn warning() -> Option<String> {
    None
}
#[cfg(not(target_os = "windows"))]
pub fn shutdown(_app: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "windows"))]
#[must_use]
pub struct Change;
#[cfg(not(target_os = "windows"))]
impl Change {
    pub fn apply(_app: &tauri::AppHandle, _enabled: bool) -> Result<Self, String> {
        Ok(Self)
    }
    pub fn commit(self) {}
    pub fn rollback(self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(any(target_os = "windows", test))]
const S: u32 = 0x53;
#[cfg(any(target_os = "windows", test))]
fn epoch_parts(epoch: u64) -> (usize, isize) {
    (epoch as u32 as usize, (epoch >> 32) as u32 as isize)
}
#[cfg(any(target_os = "windows", test))]
fn epoch_from_parts(low: usize, high: isize) -> u64 {
    low as u32 as u64 | ((high as u32 as u64) << 32)
}
#[cfg(any(target_os = "windows", test))]
const MASK_KEY: u16 = 0xff;
#[cfg(any(target_os = "windows", test))]
const MASK_MARKER: usize = 0x4d45_5755_5353_5031u64 as usize;
#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MaskEvent {
    key: u16,
    up: bool,
    marker: usize,
}
#[cfg(any(target_os = "windows", test))]
const MASK_EVENTS: [MaskEvent; 2] = [
    MaskEvent {
        key: MASK_KEY,
        up: false,
        marker: MASK_MARKER,
    },
    MaskEvent {
        key: MASK_KEY,
        up: true,
        marker: MASK_MARKER,
    },
];
#[cfg(any(target_os = "windows", test))]
fn send_mask(mut send: impl FnMut(&[MaskEvent]) -> u32) -> bool {
    match send(&MASK_EVENTS) {
        2 => true,
        1 => {
            // Release only our own partially inserted dummy down. Never replay
            // real modifiers/S or claim success after a partial insertion.
            send(&MASK_EVENTS[1..]);
            false
        }
        _ => false,
    }
}
#[cfg(any(target_os = "windows", test))]
const MODIFIERS: [u32; 8] = [0x5b, 0x5c, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5];
#[cfg(any(target_os = "windows", test))]
fn accepts_input(lower_integrity_injected: bool) -> bool {
    // Same-integrity accessibility/automation input follows the same complete
    // sequence as physical input. Lower-integrity events never alter our state.
    !lower_integrity_injected
}
#[cfg(any(target_os = "windows", test))]
#[derive(Default)]
struct Sequence {
    modifiers: [bool; 8],
    s_down: bool,
    s_suppressed: bool,
}
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Default, PartialEq, Eq)]
struct Decision {
    consume: bool,
    fire: bool,
}
#[cfg(any(target_os = "windows", test))]
impl Sequence {
    fn reject_capture(&mut self) {
        self.s_suppressed = false;
    }
    fn clear_released_modifiers(&mut self, mut held: impl FnMut(u32) -> bool) {
        for (index, key) in MODIFIERS.iter().enumerate() {
            if self.modifiers[index] && !held(*key) {
                self.modifiers[index] = false;
            }
        }
    }
    fn key(&mut self, key: u32, down: bool, enabled: bool, dispatch: bool) -> Decision {
        if let Some(index) = MODIFIERS.iter().position(|candidate| *candidate == key) {
            self.modifiers[index] = down;
            return Decision::default();
        }
        if key != S {
            return Decision::default();
        }
        if !down {
            self.s_down = false;
            let consume = std::mem::take(&mut self.s_suppressed);
            return Decision {
                consume,
                fire: false,
            };
        }
        if self.s_down {
            return Decision {
                consume: self.s_suppressed,
                fire: false,
            };
        }
        self.s_down = true;
        let modifiers = &self.modifiers;
        let exact = (modifiers[0] || modifiers[1])
            && (modifiers[2] || modifiers[3])
            && !modifiers[4..].iter().any(|down| *down);
        self.s_suppressed = enabled && exact;
        Decision {
            consume: self.s_suppressed,
            fire: self.s_suppressed && dispatch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(keys: &mut Sequence, key: u32) -> Decision {
        keys.key(key, true, true, true)
    }
    fn release(keys: &mut Sequence, key: u32) -> Decision {
        keys.key(key, false, true, true)
    }
    #[test]
    fn system_capture_shortcut_epoch_messages_keep_both_halves_and_do_not_consume_new_requests() {
        use std::sync::atomic::{AtomicU64, Ordering};
        for epoch in [1, u32::MAX as u64, 1u64 << 32, u64::MAX] {
            let (low, high) = epoch_parts(epoch);
            assert_eq!(epoch_from_parts(low, high), epoch);
        }
        let pending = AtomicU64::new(12);
        assert!(pending
            .compare_exchange(11, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_err());
        assert_eq!(pending.load(Ordering::Acquire), 12);
        assert!(pending
            .compare_exchange(12, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok());
    }
    #[test]
    fn system_capture_shortcut_matches_both_sides_and_release_orders() {
        for win in [0x5b, 0x5c] {
            for shift in [0xa0, 0xa1] {
                for first in [win, shift] {
                    let mut keys = Sequence::default();
                    press(&mut keys, first);
                    press(&mut keys, if first == win { shift } else { win });
                    assert_eq!(
                        press(&mut keys, S),
                        Decision {
                            consume: true,
                            fire: true
                        }
                    );
                    assert_eq!(
                        press(&mut keys, S),
                        Decision {
                            consume: true,
                            fire: false
                        }
                    );
                    release(&mut keys, first);
                    assert_eq!(
                        release(&mut keys, S),
                        Decision {
                            consume: true,
                            fire: false
                        }
                    );
                    assert_eq!(release(&mut keys, S), Decision::default());
                }
            }
        }
    }
    #[test]
    fn system_capture_shortcut_keeps_independent_modifiers_and_other_chords() {
        let mut keys = Sequence::default();
        for key in [0x5b, 0x5c, 0xa0, 0xa1] {
            press(&mut keys, key);
        }
        release(&mut keys, 0x5b);
        release(&mut keys, 0xa0);
        assert!(press(&mut keys, S).fire);
        release(&mut keys, S);
        for extra in [0xa2, 0xa3, 0xa4, 0xa5] {
            press(&mut keys, extra);
            assert_eq!(press(&mut keys, S), Decision::default());
            release(&mut keys, S);
            release(&mut keys, extra);
        }
        release(&mut keys, 0xa1);
        assert_eq!(press(&mut keys, S), Decision::default());
    }
    #[test]
    fn system_capture_shortcut_editing_disabled_and_held_s_keep_key_pairs() {
        let mut keys = Sequence::default();
        press(&mut keys, 0x5b);
        press(&mut keys, 0xa0);
        assert_eq!(
            keys.key(S, true, true, false),
            Decision {
                consume: true,
                fire: false
            }
        );
        assert_eq!(
            keys.key(S, false, false, false),
            Decision {
                consume: true,
                fire: false
            }
        );
        assert_eq!(keys.key(S, true, false, true), Decision::default());
        assert_eq!(keys.key(S, true, true, true), Decision::default());
        release(&mut keys, S);
        assert!(press(&mut keys, S).fire);
    }
    #[test]
    fn system_capture_shortcut_does_not_acquire_unobserved_or_stale_modifiers() {
        let mut keys = Sequence::default();
        press(&mut keys, 0x5b);
        press(&mut keys, 0xa0);
        keys.clear_released_modifiers(|key| key != 0x5b);
        assert_eq!(press(&mut keys, S), Decision::default());
        release(&mut keys, S);
        // An OS snapshot alone cannot introduce unobserved injected modifiers.
        keys.clear_released_modifiers(|_| true);
        assert_eq!(press(&mut keys, S), Decision::default());
    }
    #[test]
    fn system_capture_shortcut_injection_policy_does_not_leave_ghost_modifiers() {
        let mut keys = Sequence::default();
        for key in [0x5b, 0xa0] {
            if accepts_input(true) {
                press(&mut keys, key);
            }
        }
        assert_eq!(press(&mut keys, S), Decision::default());
        release(&mut keys, S);
        for key in [0x5b, 0xa0] {
            if accepts_input(false) {
                press(&mut keys, key);
            }
        }
        assert!(press(&mut keys, S).fire);
        release(&mut keys, S);
        for key in [0x5b, 0xa0] {
            if accepts_input(false) {
                release(&mut keys, key);
            }
        }
        assert_eq!(press(&mut keys, S), Decision::default());
    }
    #[test]
    fn system_capture_shortcut_shell_mask_is_one_marked_dummy_pair_and_partial_failure_falls_back()
    {
        let mut calls = Vec::new();
        assert!(send_mask(|events| {
            calls.push(events.to_vec());
            2
        }));
        assert_eq!(calls, vec![MASK_EVENTS.to_vec()]);
        assert_eq!(
            MASK_EVENTS[0],
            MaskEvent {
                key: 0xff,
                up: false,
                marker: MASK_MARKER
            }
        );
        assert_eq!(
            MASK_EVENTS[1],
            MaskEvent {
                key: 0xff,
                up: true,
                marker: MASK_MARKER
            }
        );
        calls.clear();
        assert!(!send_mask(|events| {
            calls.push(events.to_vec());
            0
        }));
        assert_eq!(calls, vec![MASK_EVENTS.to_vec()]);
        calls.clear();
        assert!(!send_mask(|events| {
            calls.push(events.to_vec());
            1
        }));
        assert_eq!(calls, vec![MASK_EVENTS.to_vec(), vec![MASK_EVENTS[1]]]);
        let mut keys = Sequence::default();
        press(&mut keys, 0x5b);
        press(&mut keys, 0xa0);
        assert!(press(&mut keys, S).consume);
        keys.reject_capture();
        assert_eq!(press(&mut keys, S), Decision::default());
        assert_eq!(release(&mut keys, S), Decision::default());
        assert!(press(&mut keys, S).fire);
    }
}
