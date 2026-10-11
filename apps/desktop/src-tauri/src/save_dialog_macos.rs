// SPDX-License-Identifier: MPL-2.0
//! A main-thread AppKit sheet, waited on only by the host's existing blocking
//! worker. No AppKit object, pointer, policy closure or operation lease crosses
//! from that main-thread session to another thread.
//!
//! The worker keeps its original Work/source leases until a separate native
//! cleanup acknowledgement. AppKit's completion callback alone is not proof
//! that the save sheet has left the screen.

use super::*;
use block2::RcBlock;
use objc2::{
    define_class, msg_send, rc::Retained, runtime::AnyObject, sel, DefinedClass, MainThreadMarker,
    MainThreadOnly,
};
use objc2_app_kit::{
    NSModalResponse, NSModalResponseCancel, NSModalResponseOK, NSPopUpButton, NSSavePanel, NSView,
    NSWindow,
};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use objc2_uniform_type_identifiers::UTType;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};

const WATCH_INTERVAL: Duration = Duration::from_millis(50);
const MAX_SESSIONS: usize = 8;
const MAX_PATH_BYTES: usize = 32_767;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentKind {
    Png,
    Jpeg,
}

impl ContentKind {
    fn identifier(self) -> &'static str {
        match self {
            Self::Png => "public.png",
            Self::Jpeg => "public.jpeg",
        }
    }
}

fn content_kinds<F: Copy + Eq>(spec: &DialogSpec<F>) -> Result<Vec<ContentKind>, SaveDialogError> {
    spec.validate()?;
    spec.formats
        .iter()
        .map(|format| match format.extensions {
            ["png"] => Ok(ContentKind::Png),
            extensions
                if extensions
                    .iter()
                    .all(|extension| ["jpg", "jpeg"].contains(extension)) =>
            {
                Ok(ContentKind::Jpeg)
            }
            _ => Err(SaveDialogError::InvalidFormat),
        })
        .collect()
}

fn selected_index(native_index: isize, count: usize) -> Result<u32, SaveDialogError> {
    let index = usize::try_from(native_index).map_err(|_| SaveDialogError::InvalidFormat)?;
    if index >= count {
        return Err(SaveDialogError::InvalidFormat);
    }
    u32::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .ok_or(SaveDialogError::InvalidFormat)
}

/// The first request/source failure is terminal, even if a later source check
/// would pass. A native error remains an error, independently of this latch.
#[derive(Debug, Default, PartialEq, Eq)]
enum AdmissionLatch {
    #[default]
    Current,
    RequestCanceled,
    Rejected(String),
}

impl AdmissionLatch {
    fn observe(&mut self, cancel: &AtomicBool, allowed: &dyn Fn() -> Result<(), String>) {
        if !matches!(self, Self::Current) {
            return;
        }
        if cancel.load(Ordering::Acquire) {
            *self = Self::RequestCanceled;
            return;
        }
        // A Rust panic in the caller must still cancel and await its native
        // sheet. This does not claim to catch Objective-C exceptions or aborts.
        let result = match catch_unwind(AssertUnwindSafe(allowed)) {
            Ok(result) => result,
            Err(payload) => {
                std::mem::forget(payload);
                Err("无法核对保存来源".into())
            }
        };
        if cancel.load(Ordering::Acquire) {
            *self = Self::RequestCanceled;
        } else if let Err(message) = result {
            *self = Self::Rejected(message);
        }
    }

    fn terminal<F>(&self) -> Option<Result<SaveDialogOutcome<F>, SaveDialogError>> {
        match self {
            Self::Current => None,
            Self::RequestCanceled => Some(Ok(SaveDialogOutcome::RequestCanceled)),
            Self::Rejected(message) => Some(Err(SaveDialogError::Rejected(message.clone()))),
        }
    }
}

enum NativeOutcome {
    Selected { path: PathBuf, index: u32 },
    Canceled,
}

enum Reply {
    /// Only a wakeup. The worker schedules cleanup after the callback; this
    /// message never releases the host lease or authorizes publication.
    Completion,
    Finished(Result<NativeOutcome, SaveDialogError>),
}

struct Configuration {
    name: String,
    labels: Vec<String>,
    kinds: Vec<ContentKind>,
    default_index: u32,
}

struct Session {
    panel: Retained<NSSavePanel>,
    _parent: Retained<NSWindow>,
    popup: Retained<NSPopUpButton>,
    _accessory: Retained<NSView>,
    _target: Retained<FormatTarget>,
    types: Vec<Retained<UTType>>,
    kinds: Vec<ContentKind>,
    selected: u32,
    callback_active: std::rc::Rc<Cell<bool>>,
    presentation_attempted: bool,
    completed: bool,
    outcome: Option<Result<NativeOutcome, SaveDialogError>>,
    fault: Option<SaveDialogError>,
    cancel_native: Arc<AtomicBool>,
    sender: Sender<Reply>,
    #[cfg(debug_assertions)]
    diagnostic_logged: bool,
}

thread_local! {
    static SESSIONS: RefCell<HashMap<uuid::Uuid, Session>> = RefCell::new(HashMap::new());
}

struct FormatTargetIvars {
    session_id: uuid::Uuid,
}

define_class!(
    // SAFETY: NSObject imposes no additional subclassing requirements. All
    // state and native calls stay on the main thread; no custom Drop exists.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = FormatTargetIvars]
    struct FormatTarget;

    unsafe impl NSObjectProtocol for FormatTarget {}

    impl FormatTarget {
        #[unsafe(method(formatChanged:))]
        fn format_changed(&self, _sender: Option<&AnyObject>) {
            let id = self.ivars().session_id;
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| update_format(id))) {
                std::mem::forget(payload);
                fault(id, SaveDialogError::InvalidFormat);
            }
        }
    }
);

impl FormatTarget {
    fn new(mtm: MainThreadMarker, session_id: uuid::Uuid) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FormatTargetIvars { session_id });
        // SAFETY: This is NSObject's correctly typed initializer, called on
        // the main thread before assigning this instance as the action target.
        unsafe { msg_send![super(this), init] }
    }
}

fn fault(id: uuid::Uuid, failure: SaveDialogError) {
    SESSIONS.with(|sessions| {
        if let Ok(mut sessions) = sessions.try_borrow_mut() {
            if let Some(session) = sessions.get_mut(&id) {
                if session.fault.is_none() {
                    session.fault = Some(failure);
                }
                session.cancel_native.store(true, Ordering::Release);
            }
        }
    });
}

fn update_format(id: uuid::Uuid) {
    let values = SESSIONS.with(|sessions| {
        let sessions = sessions.try_borrow().ok()?;
        let session = sessions.get(&id)?;
        if session.completed || session.fault.is_some() {
            return None;
        }
        Some((
            session.panel.clone(),
            session.popup.clone(),
            session.types.clone(),
        ))
    });
    let Some((panel, popup, types)) = values else {
        return;
    };
    let index = match selected_index(popup.indexOfSelectedItem(), types.len()) {
        Ok(index) => index,
        Err(error) => {
            fault(id, error);
            return;
        }
    };
    // One type follows the explicit popup choice. Suffixes never select the
    // encoder. No TLS borrow or Host/Engine/operation lock survives this call.
    panel.setAllowedContentTypes(&NSArray::from_slice(&[&*types[index as usize - 1]]));
    SESSIONS.with(|sessions| {
        if let Ok(mut sessions) = sessions.try_borrow_mut() {
            if let Some(session) = sessions.get_mut(&id) {
                session.selected = index;
            }
        }
    });
}

struct CallbackGuard(std::rc::Rc<Cell<bool>>);

impl CallbackGuard {
    fn enter(active: std::rc::Rc<Cell<bool>>) -> Self {
        active.set(true);
        Self(active)
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

fn response_error(response: NSModalResponse) -> SaveDialogError {
    SaveDialogError::NativeMac {
        stage: "show",
        response,
    }
}

fn selected_path(panel: &NSSavePanel) -> Result<PathBuf, SaveDialogError> {
    let url = panel.URL().ok_or(SaveDialogError::InvalidPath)?;
    if !url.isFileURL() {
        return Err(SaveDialogError::InvalidPath);
    }
    let raw = url.fileSystemRepresentation();
    // SAFETY: NSURL supplies a NUL-terminated filesystem representation whose
    // lifetime is covered by this retained URL. Stop at a bounded byte count.
    let count = unsafe { libc::strnlen(raw.as_ptr(), MAX_PATH_BYTES + 1) };
    if count == 0 || count > MAX_PATH_BYTES {
        return Err(SaveDialogError::InvalidPath);
    }
    // SAFETY: These are the bytes just counted from the still-live NSURL.
    let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), count) };
    let path = PathBuf::from(OsString::from_vec(bytes.to_vec()));
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(SaveDialogError::InvalidPath);
    }
    Ok(path)
}

fn completed(id: uuid::Uuid, response: NSModalResponse) {
    let values = SESSIONS.with(|sessions| {
        let sessions = sessions.try_borrow().ok()?;
        let session = sessions.get(&id)?;
        Some((
            session.panel.clone(),
            session.popup.clone(),
            session.kinds.clone(),
            session.selected,
            session.completed,
        ))
    });
    let Some((panel, popup, kinds, selected, duplicate)) = values else {
        return;
    };
    let outcome = if duplicate {
        Err(SaveDialogError::ThreadInterrupted)
    } else if response == NSModalResponseOK {
        (|| {
            let index = selected_index(popup.indexOfSelectedItem(), kinds.len())?;
            if index != selected {
                return Err(SaveDialogError::InvalidFormat);
            }
            let types = panel.allowedContentTypes();
            if types.len() != 1
                || types.objectAtIndex(0).identifier().to_string()
                    != kinds[index as usize - 1].identifier()
            {
                return Err(SaveDialogError::InvalidFormat);
            }
            Ok(NativeOutcome::Selected {
                path: selected_path(&panel)?,
                index,
            })
        })()
    } else if response == NSModalResponseCancel {
        Ok(NativeOutcome::Canceled)
    } else {
        // Abort, failed presentation and unknown responses are actual errors,
        // never reported as an ordinary user/request cancellation.
        Err(response_error(response))
    };
    SESSIONS.with(|sessions| {
        if let Ok(mut sessions) = sessions.try_borrow_mut() {
            if let Some(session) = sessions.get_mut(&id) {
                session.completed = true;
                session.outcome = Some(outcome);
            }
        }
    });
    // Apple explicitly permits an onscreen sheet during completion. This is a
    // direct AppKit call, not queued Tauri hide. Cleanup separately proves gone.
    panel.orderOut(None);
}

fn start(
    id: uuid::Uuid,
    window: WebviewWindow,
    token: usize,
    configuration: Configuration,
    cancel_native: Arc<AtomicBool>,
    sender: Sender<Reply>,
) -> Result<bool, SaveDialogError> {
    let mtm = MainThreadMarker::new().ok_or(SaveDialogError::ThreadUnavailable)?;
    if !crate::window_witness_macos::current_for(token, window.label()) {
        return Err(SaveDialogError::InvalidOwner);
    }
    let raw = window
        .ns_window()
        .map_err(|_| SaveDialogError::InvalidOwner)?;
    // SAFETY: Tauri returns this trusted WebviewWindow's +0 NSWindow. Retain
    // immediately on the same main thread; do not consume it as a +1 pointer.
    let parent =
        unsafe { Retained::<NSWindow>::retain(raw.cast()) }.ok_or(SaveDialogError::InvalidOwner)?;
    if !crate::window_witness_macos::current_for(token, window.label())
        || !parent.isVisible()
        || parent.attachedSheet().is_some()
    {
        return Err(SaveDialogError::InvalidOwner);
    }
    if cancel_native.load(Ordering::Acquire) {
        return Ok(false);
    }
    let panel = NSSavePanel::savePanel(mtm);
    let types: Vec<_> = configuration
        .kinds
        .iter()
        .map(|kind| {
            UTType::typeWithIdentifier(&NSString::from_str(kind.identifier()))
                .ok_or(SaveDialogError::InvalidFormat)
        })
        .collect::<Result<_, _>>()?;
    let initial = configuration.default_index as usize - 1;
    let kind = types.get(initial).ok_or(SaveDialogError::InvalidFormat)?;
    panel.setAllowedContentTypes(&NSArray::from_slice(&[&**kind]));
    panel.setAllowsOtherFileTypes(false);
    panel.setCanSelectHiddenExtension(false);
    panel.setExtensionHidden(false);
    panel.setCanCreateDirectories(true);
    panel.setNameFieldStringValue(&NSString::from_str(&configuration.name));
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(NSPoint::new(12.0, 10.0), NSSize::new(280.0, 28.0)),
        false,
    );
    for label in configuration.labels {
        popup.addItemWithTitle(&NSString::from_str(&label));
    }
    popup.selectItemAtIndex(initial as isize);
    popup.setAutoenablesItems(false);
    if popup.numberOfItems() != types.len() as isize {
        return Err(SaveDialogError::InvalidFormat);
    }
    let target = FormatTarget::new(mtm, id);
    // SAFETY: This retained main-thread target implements formatChanged: with
    // the required sender signature. Session retains it because target is weak.
    unsafe {
        popup.setTarget(Some(&target));
        popup.setAction(Some(sel!(formatChanged:)));
    }
    // Give the format control its own stable accessory container and padding.
    let accessory = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(304.0, 48.0)),
    );
    accessory.addSubview(&popup);
    panel.setAccessoryView(Some(&accessory));
    let callback_active = std::rc::Rc::new(Cell::new(false));
    let callback_flag = callback_active.clone();
    let completion_sender = sender.clone();
    let completion = RcBlock::new(move |response: NSModalResponse| {
        let _guard = CallbackGuard::enter(callback_flag.clone());
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| completed(id, response))) {
            std::mem::forget(payload);
            fault(id, SaveDialogError::ThreadInterrupted);
            // The callback happened even if result extraction failed. Native
            // maintenance must still cancel/order out and prove it gone.
            SESSIONS.with(|sessions| {
                if let Ok(mut sessions) = sessions.try_borrow_mut() {
                    if let Some(session) = sessions.get_mut(&id) {
                        session.completed = true;
                    }
                }
            });
        }
        let _ = completion_sender.send(Reply::Completion);
    });
    // Recheck the original token rather than admitting a replacement window.
    // This precedes TLS installation so a failed admission cannot leave an
    // unpresented session waiting for a completion that will never happen.
    if !crate::window_witness_macos::current_for(token, window.label()) {
        return Err(SaveDialogError::InvalidOwner);
    }
    if cancel_native.load(Ordering::Acquire) {
        return Ok(false);
    }
    SESSIONS.with(|sessions| {
        let mut sessions = sessions
            .try_borrow_mut()
            .map_err(|_| SaveDialogError::ThreadInterrupted)?;
        if sessions.len() >= MAX_SESSIONS || sessions.contains_key(&id) {
            return Err(SaveDialogError::CancelWatcherUnavailable);
        }
        sessions.insert(
            id,
            Session {
                panel: panel.clone(),
                _parent: parent.clone(),
                popup,
                _accessory: accessory,
                _target: target,
                types,
                kinds: configuration.kinds,
                selected: configuration.default_index,
                callback_active,
                presentation_attempted: false,
                completed: false,
                outcome: None,
                fault: None,
                cancel_native,
                sender,
                #[cfg(debug_assertions)]
                diagnostic_logged: false,
            },
        );
        Ok(())
    })?;
    SESSIONS.with(|sessions| {
        if let Some(session) = sessions.borrow_mut().get_mut(&id) {
            session.presentation_attempted = true;
        }
    });
    // Returns immediately. AppKit copies/retains this heap completion block;
    // it captures no panel/session, so it cannot create a panel retain cycle.
    panel.beginSheetModalForWindow_completionHandler(&parent, &completion);
    Ok(true)
}

#[cfg(debug_assertions)]
fn diagnose(id: uuid::Uuid) {
    if std::env::var("MEWU_MAC_SAVE_DIAGNOSTICS").as_deref() != Ok("1") {
        return;
    }
    let Some((panel, parent, container, popup)) = SESSIONS.with(|sessions| {
        let sessions = sessions.try_borrow().ok()?;
        let session = sessions.get(&id)?;
        if session.diagnostic_logged { return None; }
        Some((session.panel.clone(), session._parent.clone(), session._accessory.clone(), session.popup.clone()))
    }) else { return; };
    if !panel.isVisible() { return; }
    SESSIONS.with(|sessions| {
        if let Ok(mut sessions) = sessions.try_borrow_mut() {
            if let Some(session) = sessions.get_mut(&id) { session.diagnostic_logged = true; }
        }
    });
    let accessory = panel.accessoryView();
    let popup_window = popup.window();
    // Own panel/control geometry only. No selected path, filename, title,
    // screen pixels, source data or other applications' windows are logged.
    eprintln!(
        "mewu.mac.save.panel parentLevel={} panelLevel={} sheet={} accessory={} accessoryFrame={:?} accessoryHidden={} popupFrame={:?} popupHidden={} popupWindow={} popupLevel={:?} popupItems={} selected={}",
        parent.level(), panel.level(),
        panel.sheetParent().is_some(), accessory.is_some(),
        container.frame(), container.isHidden(),
        popup.frame(), popup.isHidden(), popup_window.is_some(),
        popup_window.as_ref().map(|window| window.level()),
        popup.numberOfItems(), popup.indexOfSelectedItem(),
    );
}

fn maintain(id: uuid::Uuid) {
    let Some(_mtm) = MainThreadMarker::new() else {
        return;
    };
    #[cfg(debug_assertions)]
    diagnose(id);
    let pending = SESSIONS.with(|sessions| {
        let sessions = sessions.try_borrow().ok()?;
        let session = sessions.get(&id)?;
        if session.completed
            || !session.presentation_attempted
            || !session.cancel_native.load(Ordering::Acquire)
        {
            return None;
        }
        Some(session.panel.clone())
    });
    if let Some(panel) = pending {
        // SAFETY: NSSavePanel's standard cancel action receives nil, on its
        // owning main thread. No TLS or host lock is held during reentrancy.
        unsafe { panel.cancel(None) };
    }
    let completed_panel = SESSIONS.with(|sessions| {
        let sessions = sessions.try_borrow().ok()?;
        let session = sessions.get(&id)?;
        if !(session.completed || (!session.presentation_attempted && session.fault.is_some()))
            || session.callback_active.get()
        {
            return None;
        }
        Some(session.panel.clone())
    });
    let Some(panel) = completed_panel else {
        return;
    };
    panel.orderOut(None);
    if panel.isVisible() || panel.sheetParent().is_some() {
        return;
    }
    drop(panel);
    let finished = SESSIONS.with(|sessions| {
        let mut sessions = sessions.try_borrow_mut().ok()?;
        let session = sessions.get(&id)?;
        if session.callback_active.get()
            || !(session.completed || (!session.presentation_attempted && session.fault.is_some()))
        {
            return None;
        }
        sessions.remove(&id)
    });
    let Some(mut session) = finished else {
        return;
    };
    let result = match session.fault.take() {
        Some(error) => Err(error),
        None => session
            .outcome
            .take()
            .unwrap_or(Err(SaveDialogError::ThreadInterrupted)),
    };
    let sender = session.sender.clone();
    // Clear the weak action linkage and drop every locally retained AppKit
    // object on its owning thread BEFORE publishing the final cleanup ACK.
    unsafe {
        session.popup.setAction(None);
        session.popup.setTarget(None);
    }
    drop(session);
    let _ = sender.send(Reply::Finished(result));
}

struct DispatchGuard(Arc<AtomicBool>);

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn dispatch_maintenance(app: &AppHandle, id: uuid::Uuid, dispatch_pending: &Arc<AtomicBool>) {
    if dispatch_pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let pending = dispatch_pending.clone();
    if app
        .run_on_main_thread(move || {
            let _guard = DispatchGuard(pending);
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| maintain(id))) {
                std::mem::forget(payload);
                fault(id, SaveDialogError::ThreadInterrupted);
            }
        })
        .is_err()
    {
        // An enqueue failure is not proof that an already presented native
        // sheet is gone. Keep the host lease and retry on the next bounded tick.
        dispatch_pending.store(false, Ordering::Release);
    }
}

fn wait<F: Copy + Eq>(
    app: &AppHandle,
    id: uuid::Uuid,
    spec: &DialogSpec<F>,
    cancel: &AtomicBool,
    allowed: &dyn Fn() -> Result<(), String>,
    cancel_native: &Arc<AtomicBool>,
    dispatch_pending: &Arc<AtomicBool>,
    receiver: Receiver<Reply>,
) -> Result<SaveDialogOutcome<F>, SaveDialogError> {
    let mut admission = AdmissionLatch::default();
    loop {
        admission.observe(cancel, allowed);
        if !matches!(admission, AdmissionLatch::Current) {
            cancel_native.store(true, Ordering::Release);
        }
        match receiver.recv_timeout(WATCH_INTERVAL) {
            Ok(Reply::Finished(result)) => {
                // Native errors must not be hidden by a concurrent cancel.
                let result = result?;
                admission.observe(cancel, allowed);
                if let Some(result) = admission.terminal() {
                    return result;
                }
                return match result {
                    NativeOutcome::Canceled => Ok(SaveDialogOutcome::UserCanceled),
                    NativeOutcome::Selected { path, index } => {
                        let format = spec.destination(&path, index)?;
                        admission.observe(cancel, allowed);
                        if let Some(result) = admission.terminal() {
                            result
                        } else {
                            Ok(SaveDialogOutcome::Selected { path, format })
                        }
                    }
                };
            }
            Ok(Reply::Completion) | Err(RecvTimeoutError::Timeout) => {
                // Called only from this blocking worker, after all admission
                // locks were released. Never use main-thread inline dispatch
                // as a supposed barrier after a still-running callback.
                dispatch_maintenance(app, id, dispatch_pending);
            }
            Err(RecvTimeoutError::Disconnected) => {
                // No native cleanup ACK: do not release the original worker.
                cancel_native.store(true, Ordering::Release);
                dispatch_maintenance(app, id, dispatch_pending);
                std::thread::park_timeout(WATCH_INTERVAL);
            }
        }
    }
}

/// Call only from the existing host blocking worker, with its original source
/// and Work leases. `token` is an opaque host witness, never a native pointer.
pub fn pick_destination<F: Copy + Eq + Send + Sync + 'static>(
    window: WebviewWindow,
    token: usize,
    spec: DialogSpec<F>,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome<F>, SaveDialogError> {
    if MainThreadMarker::new().is_some() {
        return Err(SaveDialogError::ThreadUnavailable);
    }
    let kinds = content_kinds(&spec)?;
    let mut admission = AdmissionLatch::default();
    admission.observe(&cancel, &allowed);
    if let Some(result) = admission.terminal() {
        return result;
    }
    let configuration = Configuration {
        name: default_filename(spec.basename, chrono::Local::now().naive_local()),
        labels: spec
            .formats
            .iter()
            .map(|format| crate::ui_language::translated(format.label))
            .collect(),
        kinds,
        default_index: spec.default_index,
    };
    let app = window.app_handle().clone();
    let id = uuid::Uuid::new_v4();
    let cancel_native = Arc::new(AtomicBool::new(false));
    let dispatch_pending = Arc::new(AtomicBool::new(true));
    let (sender, receiver) = mpsc::channel();
    // Retain a worker-side sender so disconnected is never mistaken for a
    // successful native cleanup, even if startup/main-loop work is delayed.
    let startup_sender = sender.clone();
    let native_cancel = cancel_native.clone();
    let pending = dispatch_pending.clone();
    app.run_on_main_thread(move || {
        let _guard = DispatchGuard(pending);
        let result = catch_unwind(AssertUnwindSafe(|| {
            start(
                id,
                window,
                token,
                configuration,
                native_cancel,
                startup_sender.clone(),
            )
        }));
        match result {
            Ok(Ok(true)) => (),
            Ok(Ok(false)) => {
                // start's locally retained objects have already been dropped.
                let _ = startup_sender.send(Reply::Finished(Ok(NativeOutcome::Canceled)));
            }
            Ok(Err(error)) => {
                let _ = startup_sender.send(Reply::Finished(Err(error)));
            }
            Err(payload) => {
                std::mem::forget(payload);
                // A panic after session installation must still await native
                // cleanup; never send a premature terminal error for it.
                let installed = SESSIONS.with(|sessions| {
                    sessions
                        .try_borrow()
                        .map(|sessions| sessions.contains_key(&id))
                        .unwrap_or(true)
                });
                if installed {
                    fault(id, SaveDialogError::ThreadInterrupted);
                } else {
                    let _ = startup_sender
                        .send(Reply::Finished(Err(SaveDialogError::ThreadInterrupted)));
                }
            }
        }
    })
    .map_err(|_| SaveDialogError::ThreadUnavailable)?;
    wait(
        &app,
        id,
        &spec,
        &cancel,
        &allowed,
        &cancel_native,
        &dispatch_pending,
        receiver,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMATS: &[FormatSpec<u8>] = &[
        FormatSpec {
            format: 11,
            label: "PNG",
            extensions: &["png"],
        },
        FormatSpec {
            format: 22,
            label: "JPEG",
            extensions: &["jpg", "jpeg"],
        },
    ];

    fn spec() -> DialogSpec<u8> {
        DialogSpec {
            basename: "截图",
            formats: FORMATS,
            default_index: 2,
        }
    }

    #[test]
    fn actual_popup_index_is_bounded_and_suffix_only_validates_that_choice() {
        let spec = spec();
        assert_eq!(
            content_kinds(&spec).unwrap(),
            [ContentKind::Png, ContentKind::Jpeg]
        );
        assert_eq!(selected_index(0, 2).unwrap(), 1);
        assert_eq!(selected_index(1, 2).unwrap(), spec.default_index);
        for invalid in [-1, 2, isize::MAX] {
            assert!(selected_index(invalid, 2).is_err());
        }
        assert_eq!(
            spec.destination(Path::new("/private/tmp/回答.PNG"), 1)
                .unwrap(),
            11
        );
        assert_eq!(
            spec.destination(Path::new("/private/tmp/回答.JPEG"), 2)
                .unwrap(),
            22
        );
        assert!(matches!(
            spec.destination(Path::new("/private/tmp/回答.png"), 2),
            Err(SaveDialogError::ExtensionConflict)
        ));
        let unsupported = DialogSpec {
            formats: &[FormatSpec {
                format: 33,
                label: "WebP",
                extensions: &["webp"],
            }],
            default_index: 1,
            ..spec
        };
        assert!(matches!(
            content_kinds(&unsupported),
            Err(SaveDialogError::InvalidFormat)
        ));
    }

    #[test]
    fn cancellation_and_source_rejection_latch_without_becoming_user_cancel() {
        let cancel = AtomicBool::new(false);
        let current = Cell::new(true);
        let allowed = || {
            if current.get() {
                Ok(())
            } else {
                Err("source changed".into())
            }
        };
        let mut latch = AdmissionLatch::default();
        latch.observe(&cancel, &allowed);
        assert_eq!(latch, AdmissionLatch::Current);
        current.set(false);
        latch.observe(&cancel, &allowed);
        current.set(true);
        cancel.store(true, Ordering::Release);
        latch.observe(&cancel, &allowed);
        assert_eq!(latch, AdmissionLatch::Rejected("source changed".into()));
        assert!(matches!(
            latch.terminal::<u8>(),
            Some(Err(SaveDialogError::Rejected(_)))
        ));

        let mut latch = AdmissionLatch::default();
        latch.observe(&cancel, &|| {
            panic!("canceled admission must not query policy")
        });
        cancel.store(false, Ordering::Release);
        latch.observe(&cancel, &|| Ok(()));
        assert!(matches!(
            latch.terminal::<u8>(),
            Some(Ok(SaveDialogOutcome::RequestCanceled))
        ));
    }
}
