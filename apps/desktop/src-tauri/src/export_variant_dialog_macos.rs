// SPDX-License-Identifier: MPL-2.0
//! AppKit presentation choice. Only the existing blocking worker checks source
//! policy; only the main thread owns native objects. Its lease outlives the
//! callback until the sheet is detached and every session object is dropped.
use super::*;
use block2::RcBlock;
use objc2::{rc::Retained, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAlert, NSModalResponse, NSModalResponseCancel, NSWindow};
use objc2_foundation::NSString;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};

const TICK: Duration = Duration::from_millis(50);
const FAILED: &str = "无法打开导出选项";

fn response(value: NSModalResponse) -> Result<Outcome, String> {
    match value {
        1000 => Ok(Outcome::Selected(ExportVariant::Annotated)),
        1001 => Ok(Outcome::Selected(ExportVariant::Clean)),
        1002 | NSModalResponseCancel => Ok(Outcome::UserCanceled),
        _ => Err("导出选项已中断".into()),
    }
}

enum Reply {
    Completion,
    Finished(Result<Outcome, String>),
}
struct Session {
    alert: Retained<NSAlert>,
    parent: Retained<NSWindow>,
    active: std::rc::Rc<Cell<bool>>,
    presentation_attempted: bool,
    completed: bool,
    outcome: Option<Result<Outcome, String>>,
    fault: Option<String>,
    cancel: Arc<AtomicBool>,
    sender: Sender<Reply>,
}
thread_local! { static SESSIONS: RefCell<HashMap<uuid::Uuid, Session>> = RefCell::new(HashMap::new()); }

struct CallbackGuard(std::rc::Rc<Cell<bool>>);
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
struct DispatchGuard(Arc<AtomicBool>);
impl Drop for DispatchGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn fault(id: uuid::Uuid) {
    SESSIONS.with(|sessions| {
        if let Ok(mut sessions) = sessions.try_borrow_mut() {
            if let Some(session) = sessions.get_mut(&id) {
                session.fault.get_or_insert_with(|| FAILED.into());
                session.cancel.store(true, Ordering::Release);
            }
        }
    });
}

fn completed(id: uuid::Uuid, value: NSModalResponse) {
    let alert = SESSIONS.with(|sessions| {
        let mut sessions = sessions.try_borrow_mut().ok()?;
        let session = sessions.get_mut(&id)?;
        session.outcome = Some(if session.completed {
            Err(FAILED.into())
        } else {
            response(value)
        });
        session.completed = true;
        Some(session.alert.clone())
    });
    if let Some(alert) = alert {
        alert.window().orderOut(None);
    }
}

fn start(
    id: uuid::Uuid,
    window: WebviewWindow,
    token: usize,
    media: Media,
    cancel: Arc<AtomicBool>,
    sender: Sender<Reply>,
) -> Result<bool, String> {
    let mtm = MainThreadMarker::new().ok_or(FAILED)?;
    if !crate::window_witness_macos::current_for(token, window.label()) {
        return Err(FAILED.into());
    }
    let raw = window.ns_window().map_err(|_| FAILED)?;
    // SAFETY: Tauri's trusted window supplies a +0 pointer. Retain immediately
    // on the owning main thread; the opaque token is never cast to a pointer.
    let parent = unsafe { Retained::<NSWindow>::retain(raw.cast()) }.ok_or(FAILED)?;
    if !crate::window_witness_macos::current_for(token, window.label())
        || !parent.isVisible()
        || parent.attachedSheet().is_some()
    {
        return Err(FAILED.into());
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&crate::ui_language::translated(
        "导出内容",
    )));
    let message = match media {
        Media::Image => "选择保存带标注的图片，或没有标注的原图。",
        Media::Video => "选择保存带标注的视频，或没有标注的原视频。",
    };
    alert.setInformativeText(&NSString::from_str(&crate::ui_language::translated(
        message,
    )));
    let clean = match media {
        Media::Image => "原图",
        Media::Video => "原视频",
    };
    for label in ["带标注", clean, "取消"] {
        alert.addButtonWithTitle(&NSString::from_str(&crate::ui_language::translated(label)));
    }
    let active = std::rc::Rc::new(Cell::new(false));
    let flag = active.clone();
    let wake = sender.clone();
    let callback = RcBlock::new(move |value: NSModalResponse| {
        flag.set(true);
        let _guard = CallbackGuard(flag.clone());
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| completed(id, value))) {
            std::mem::forget(payload);
            fault(id);
            SESSIONS.with(|sessions| {
                if let Ok(mut sessions) = sessions.try_borrow_mut() {
                    if let Some(session) = sessions.get_mut(&id) {
                        session.completed = true;
                    }
                }
            });
        }
        let _ = wake.send(Reply::Completion);
    });
    if !crate::window_witness_macos::current_for(token, window.label()) {
        return Err(FAILED.into());
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.try_borrow_mut().map_err(|_| FAILED)?;
        if sessions.len() >= 8 || sessions.contains_key(&id) {
            return Err(FAILED);
        }
        sessions.insert(
            id,
            Session {
                alert: alert.clone(),
                parent: parent.clone(),
                active,
                presentation_attempted: false,
                completed: false,
                outcome: None,
                fault: None,
                cancel,
                sender,
            },
        );
        Ok(())
    })?;
    SESSIONS.with(|sessions| {
        if let Some(session) = sessions.borrow_mut().get_mut(&id) {
            session.presentation_attempted = true;
        }
    });
    // AppKit copies this block; it captures no retained alert/parent/session.
    alert.beginSheetModalForWindow_completionHandler(&parent, Some(&callback));
    Ok(true)
}

fn maintain(id: uuid::Uuid) {
    let pending = SESSIONS.with(|sessions| {
        let sessions = sessions.try_borrow().ok()?;
        let session = sessions.get(&id)?;
        if session.completed
            || !session.presentation_attempted
            || !session.cancel.load(Ordering::Acquire)
        {
            return None;
        }
        Some((session.parent.clone(), session.alert.clone()))
    });
    if let Some((parent, alert)) = pending {
        // No RefCell borrow or host lock survives this reentrant AppKit call.
        parent.endSheet_returnCode(&alert.window(), NSModalResponseCancel);
    }
    let alert = SESSIONS.with(|sessions| {
        let sessions = sessions.try_borrow().ok()?;
        let session = sessions.get(&id)?;
        if !(session.completed || (!session.presentation_attempted && session.fault.is_some()))
            || session.active.get()
        {
            return None;
        }
        Some(session.alert.clone())
    });
    let Some(alert) = alert else {
        return;
    };
    let panel = alert.window();
    panel.orderOut(None);
    if panel.isVisible() || panel.sheetParent().is_some() {
        return;
    }
    drop(panel);
    drop(alert);
    let removed = SESSIONS.with(|sessions| {
        let mut sessions = sessions.try_borrow_mut().ok()?;
        let session = sessions.get(&id)?;
        if !(session.completed || (!session.presentation_attempted && session.fault.is_some()))
            || session.active.get()
        {
            return None;
        }
        sessions.remove(&id)
    });
    if let Some(mut session) = removed {
        let sender = session.sender.clone();
        let result = match session.fault.take() {
            Some(error) => Err(error),
            None => session.outcome.take().unwrap_or_else(|| Err(FAILED.into())),
        };
        drop(session);
        let _ = sender.send(Reply::Finished(result));
    }
}

fn dispatch(app: &AppHandle, id: uuid::Uuid, pending: &Arc<AtomicBool>) {
    if pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let flag = pending.clone();
    if app
        .run_on_main_thread(move || {
            let _guard = DispatchGuard(flag);
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| maintain(id))) {
                std::mem::forget(payload);
                fault(id);
            }
        })
        .is_err()
    {
        pending.store(false, Ordering::Release);
    }
}

#[derive(Default)]
enum Admission {
    #[default]
    Current,
    Canceled,
    Rejected(String),
}
impl Admission {
    fn observe(&mut self, cancel: &AtomicBool, allowed: &dyn Fn() -> Result<(), String>) {
        if !matches!(self, Self::Current) {
            return;
        }
        if cancel.load(Ordering::Acquire) {
            *self = Self::Canceled;
            return;
        }
        let result = match catch_unwind(AssertUnwindSafe(allowed)) {
            Ok(result) => result,
            Err(payload) => {
                std::mem::forget(payload);
                Err("无法核对导出来源".into())
            }
        };
        if cancel.load(Ordering::Acquire) {
            *self = Self::Canceled;
        } else if let Err(error) = result {
            *self = Self::Rejected(error);
        }
    }
    fn terminal(&self) -> Option<Result<Outcome, String>> {
        match self {
            Self::Current => None,
            Self::Canceled => Some(Ok(Outcome::RequestCanceled)),
            Self::Rejected(error) => Some(Err(error.clone())),
        }
    }
}

fn wait(
    app: &AppHandle,
    id: uuid::Uuid,
    cancel: &AtomicBool,
    allowed: &dyn Fn() -> Result<(), String>,
    native: &Arc<AtomicBool>,
    pending: &Arc<AtomicBool>,
    receiver: Receiver<Reply>,
) -> Result<Outcome, String> {
    let mut admission = Admission::default();
    loop {
        admission.observe(cancel, allowed);
        if admission.terminal().is_some() {
            native.store(true, Ordering::Release);
        }
        match receiver.recv_timeout(TICK) {
            Ok(Reply::Finished(result)) => {
                let result = result?;
                admission.observe(cancel, allowed);
                return admission.terminal().unwrap_or(Ok(result));
            }
            Ok(Reply::Completion) | Err(RecvTimeoutError::Timeout) => dispatch(app, id, pending),
            Err(RecvTimeoutError::Disconnected) => {
                native.store(true, Ordering::Release);
                dispatch(app, id, pending);
                std::thread::park_timeout(TICK);
            }
        }
    }
}

pub(super) fn show(
    window: WebviewWindow,
    token: usize,
    media: Media,
    cancel: &Arc<AtomicBool>,
    allowed: &dyn Fn() -> Result<(), String>,
) -> Result<Outcome, String> {
    if MainThreadMarker::new().is_some() {
        return Err(FAILED.into());
    }
    let mut admission = Admission::default();
    admission.observe(cancel, allowed);
    if let Some(result) = admission.terminal() {
        return result;
    }
    let app = window.app_handle().clone();
    let id = uuid::Uuid::new_v4();
    let native = Arc::new(AtomicBool::new(false));
    let pending = Arc::new(AtomicBool::new(true));
    let native_clone = native.clone();
    let flag = pending.clone();
    let (sender, receiver) = mpsc::channel();
    let startup = sender.clone();
    app.run_on_main_thread(move || {
        let _guard = DispatchGuard(flag);
        match catch_unwind(AssertUnwindSafe(|| {
            start(id, window, token, media, native_clone, startup.clone())
        })) {
            Ok(Ok(true)) => (),
            Ok(Ok(false)) => {
                let _ = startup.send(Reply::Finished(Ok(Outcome::RequestCanceled)));
            }
            Ok(Err(error)) => {
                let _ = startup.send(Reply::Finished(Err(error)));
            }
            Err(payload) => {
                std::mem::forget(payload);
                let installed = SESSIONS.with(|sessions| {
                    sessions
                        .try_borrow()
                        .map(|sessions| sessions.contains_key(&id))
                        .unwrap_or(true)
                });
                if installed {
                    fault(id);
                } else {
                    let _ = startup.send(Reply::Finished(Err(FAILED.into())));
                }
            }
        }
    })
    .map_err(|_| FAILED)?;
    wait(&app, id, cancel, allowed, &native, &pending, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_native_response_is_an_error_and_cancel_kinds_stay_separate() {
        assert_eq!(
            response(1000).unwrap(),
            Outcome::Selected(ExportVariant::Annotated)
        );
        assert_eq!(
            response(1001).unwrap(),
            Outcome::Selected(ExportVariant::Clean)
        );
        assert_eq!(response(1002).unwrap(), Outcome::UserCanceled);
        assert_eq!(
            response(NSModalResponseCancel).unwrap(),
            Outcome::UserCanceled
        );
        assert!(response(-1000).is_err());
        let cancel = AtomicBool::new(false);
        let mut admission = Admission::default();
        admission.observe(&cancel, &|| Err("source changed".into()));
        cancel.store(true, Ordering::Release);
        admission.observe(&cancel, &|| Ok(()));
        assert_eq!(admission.terminal().unwrap(), Err("source changed".into()));
        let mut admission = Admission::default();
        admission.observe(&cancel, &|| panic!("cancel must skip source policy"));
        assert_eq!(
            admission.terminal().unwrap().unwrap(),
            Outcome::RequestCanceled
        );
    }
}
