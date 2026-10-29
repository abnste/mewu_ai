// SPDX-License-Identifier: MPL-2.0
//! A region-only Escape lease. Its HWND-less hotkey belongs to its own short
//! lived message thread, never to the Tauri recording/scroll shortcut manager.
//! Microsoft RegisterHotKey / UnregisterHotKey / PostThreadMessageW contracts.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::JoinHandle;
use windows_sys::Win32::{
    Foundation::WAIT_FAILED,
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, VK_ESCAPE},
        WindowsAndMessaging::{
            MsgWaitForMultipleObjectsEx, PeekMessageW, PostThreadMessageW, MSG,
            MWMO_INPUTAVAILABLE, PM_NOREMOVE, PM_REMOVE, QS_ALLINPUT, WM_HOTKEY, WM_QUIT, WM_USER,
        },
    },
};

const HOTKEY_ID: i32 = 0x4D57;
const STOP_UNAVAILABLE: &str = "无法注册区域停止键，请先结束其他使用 Esc 的操作";

pub(crate) struct EscapeLease {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

/// The invocation owns registration even when a failed begin drops its hooks
/// and the plugin catches the error. Retried observations reuse the same lease.
#[derive(Default)]
pub(crate) struct RunEscapeLease(Mutex<Option<Arc<EscapeLease>>>);
impl RunEscapeLease {
    pub(crate) fn acquire(
        &self,
        canceled: Arc<AtomicBool>,
    ) -> Result<Arc<EscapeLease>, &'static str> {
        let mut lease = self.0.lock().map_err(|_| STOP_UNAVAILABLE)?;
        if lease.is_none() {
            *lease = Some(Arc::new(EscapeLease::register(canceled)?));
        }
        Ok(lease
            .as_ref()
            .expect("registered region Escape lease")
            .clone())
    }
    pub(crate) fn release(&self) {
        let lease = self.0.lock().ok().and_then(|mut lease| lease.take());
        if let Some(lease) = lease {
            // In-flight hooks may still hold an Arc. Retire registration now,
            // without marking the parent model run as user-cancelled.
            lease.stop();
        }
    }
}

fn is_stop_message(message: u32, id: usize, data: isize) -> bool {
    message == WM_HOTKEY
        && id == HOTKEY_ID as usize
        && ((data as usize >> 16) & 0xffff) == usize::from(VK_ESCAPE)
}

impl EscapeLease {
    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        unsafe { PostThreadMessageW(self.thread_id, WM_USER, 0, 0); }
    }
    pub(crate) fn register(canceled: Arc<AtomicBool>) -> Result<Self, &'static str> {
        let (send, receive) = mpsc::sync_channel(0);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("mewu-region-escape".into())
            .spawn(move || {
                // The readiness handshake follows PeekMessage, so Drop can post to
                // an existing queue. No windows or general input hooks are created.
                let mut message: MSG = unsafe { std::mem::zeroed() };
                unsafe {
                    PeekMessageW(
                        &mut message,
                        std::ptr::null_mut(),
                        WM_USER,
                        WM_USER,
                        PM_NOREMOVE,
                    );
                }
                if unsafe {
                    RegisterHotKey(
                        std::ptr::null_mut(),
                        HOTKEY_ID,
                        MOD_NOREPEAT,
                        u32::from(VK_ESCAPE),
                    )
                } == 0
                {
                    let _ = send.send(Err(()));
                    return;
                }
                struct Registration;
                impl Drop for Registration {
                    fn drop(&mut self) {
                        unsafe {
                            UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
                        }
                    }
                }
                let _registration = Registration;
                if send.send(Ok(unsafe { GetCurrentThreadId() })).is_err() {
                    return;
                }
                while !stopping.load(Ordering::SeqCst) {
                    let status = unsafe {
                        MsgWaitForMultipleObjectsEx(
                            0,
                            std::ptr::null(),
                            100,
                            QS_ALLINPUT,
                            MWMO_INPUTAVAILABLE,
                        )
                    };
                    if status == WAIT_FAILED {
                        canceled.store(true, Ordering::SeqCst);
                        break;
                    }
                    while !stopping.load(Ordering::SeqCst)
                        && unsafe {
                            PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE)
                        } != 0
                    {
                        if message.message == WM_QUIT {
                            canceled.store(true, Ordering::SeqCst);
                            return;
                        }
                        if is_stop_message(message.message, message.wParam, message.lParam) {
                            canceled.store(true, Ordering::SeqCst);
                        }
                    }
                }
            })
            .map_err(|_| STOP_UNAVAILABLE)?;
        match receive.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
                stop,
            }),
            _ => {
                let _ = thread.join();
                Err(STOP_UNAVAILABLE)
            }
        }
    }
}
impl Drop for EscapeLease {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.stop();
            if !thread.is_finished() {
                unsafe {
                    PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
                }
            }
            // UnregisterHotKey executes on the registering thread before join
            // completes; this cannot unregister a manager-thread hotkey.
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static NATIVE_TEST: Mutex<()> = Mutex::new(());

    fn post_stop(lease: &EscapeLease) {
        assert_ne!(
            unsafe {
                PostThreadMessageW(
                    lease.thread_id,
                    WM_HOTKEY,
                    HOTKEY_ID as usize,
                    (VK_ESCAPE as isize) << 16,
                )
            },
            0
        );
    }
    fn wait_for_stop(canceled: &AtomicBool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !canceled.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(canceled.load(Ordering::SeqCst));
    }

    #[test]
    fn only_the_owned_escape_hotkey_message_cancels_the_region() {
        let data = (VK_ESCAPE as isize) << 16;
        assert!(is_stop_message(WM_HOTKEY, HOTKEY_ID as usize, data));
        assert!(!is_stop_message(WM_HOTKEY, HOTKEY_ID as usize + 1, data));
        assert!(!is_stop_message(WM_HOTKEY, HOTKEY_ID as usize, 0));
        assert!(!is_stop_message(WM_QUIT, HOTKEY_ID as usize, data));
    }

    #[test]
    fn actual_message_thread_cancels_and_drop_allows_reregister_without_key_injection() {
        let _serial = NATIVE_TEST.lock().unwrap();
        let canceled = Arc::new(AtomicBool::new(false));
        let lease = EscapeLease::register(canceled.clone()).expect("native Escape registration");
        post_stop(&lease);
        wait_for_stop(&canceled);
        drop(lease);
        let next = EscapeLease::register(Arc::new(AtomicBool::new(false)))
            .expect("Escape must be released after actual thread drain");
        drop(next);
    }

    #[test]
    fn actual_registration_conflict_does_not_unregister_the_existing_owner() {
        let _serial = NATIVE_TEST.lock().unwrap();
        let canceled = Arc::new(AtomicBool::new(false));
        let owner = EscapeLease::register(canceled.clone()).expect("native Escape registration");
        assert!(EscapeLease::register(Arc::new(AtomicBool::new(false))).is_err());
        post_stop(&owner);
        wait_for_stop(&canceled);
        drop(owner);
        drop(
            EscapeLease::register(Arc::new(AtomicBool::new(false)))
                .expect("old owner must release only its own hotkey"),
        );
    }

    #[test]
    fn run_lease_survives_failed_begin_and_reuses_registration_until_final_drain() {
        let _serial = NATIVE_TEST.lock().unwrap();
        let mut run = RunEscapeLease::default();
        let canceled = Arc::new(AtomicBool::new(false));
        let first_hooks = run.acquire(canceled.clone()).unwrap();
        let thread_id = first_hooks.thread_id;
        let registration = Arc::downgrade(&first_hooks);
        drop(first_hooks); // RegionSession::begin fails; plugin catches the error.
        assert!(EscapeLease::register(Arc::new(AtomicBool::new(false))).is_err());
        let retried_hooks = run.acquire(canceled.clone()).unwrap();
        assert!(Arc::ptr_eq(
            &registration.upgrade().unwrap(),
            &retried_hooks
        ));
        assert_eq!(retried_hooks.thread_id, thread_id);
        post_stop(&retried_hooks);
        wait_for_stop(&canceled);
        drop(retried_hooks);
        run.release(); // Context drop after actual worker drain.
        drop(EscapeLease::register(Arc::new(AtomicBool::new(false))).unwrap());
    }

    #[test]
    fn presentation_revocation_retires_escape_with_live_hooks_without_canceling_parent() {
        let _serial = NATIVE_TEST.lock().unwrap();
        let run = RunEscapeLease::default();
        let canceled = Arc::new(AtomicBool::new(false));
        let hooks = run.acquire(canceled.clone()).unwrap();
        run.release();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !hooks.thread.as_ref().unwrap().is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(hooks.thread.as_ref().unwrap().is_finished());
        assert!(!canceled.load(Ordering::SeqCst));
        drop(EscapeLease::register(Arc::new(AtomicBool::new(false))).expect("retired hooks cannot own Escape"));
        drop(hooks);
    }
}
