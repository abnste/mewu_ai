// SPDX-License-Identifier: MPL-2.0
//! Host adapter around the single pure Model. Model/disk/OS work runs outside
//! every shared mutex. Readers and event routing never wait for that work.
use crate::{
    capture_shortcut::{
        Backend, CaptureShortcutState, CommitReceipt, FileStamp, HostPreferences, PreferencesError,
        PreferencesFile, RegistrationError, ShortcutChord, ShortcutModel,
    },
    Host,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

const LEASE_TTL: Duration = Duration::from_secs(30);
const TOMBSTONE_TTL: Duration = Duration::from_secs(300);
const MAX_TOMBSTONES: usize = 512;
const BUSY: &str = "快捷键设置正在保存，请稍后再试";
const CANCELED: &str = "快捷键编辑已结束";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutEditLease {
    lease_id: String,
    expires_at: u64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EditEnded {
    lease_id: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Recorded {
    lease_id: String,
    shortcut: ShortcutChord,
}

#[derive(Clone)]
struct Editor {
    owner: String,
    window: SettingsOwner,
    public: ShortcutEditLease,
    until: Instant,
    stop: Arc<Notify>,
}
impl Editor {
    fn same_lease(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.window == other.window
            && self.public.lease_id == other.public.lease_id
            && self.until == other.until
    }
}
#[derive(Default)]
struct Editors {
    active: Option<Editor>,
    // End-before-begin must be remembered without unbounded request-ID growth.
    ended: HashMap<String, (String, Instant)>,
}
impl Editors {
    fn matches_observation(&self, observed: Option<&Editor>) -> bool {
        match (self.active.as_ref(), observed) {
            (None, None) => true,
            (Some(active), Some(observed)) => active.same_lease(observed),
            _ => false,
        }
    }
    fn prune(&mut self, now: Instant) -> Option<Editor> {
        self.ended
            .retain(|_, (_, at)| now.saturating_duration_since(*at) < TOMBSTONE_TTL);
        if self.active.as_ref().is_some_and(|lease| now >= lease.until) {
            let previous = self.active.take().unwrap();
            self.remember(&previous, now);
            Some(previous)
        } else {
            None
        }
    }
    fn remember(&mut self, lease: &Editor, now: Instant) {
        // Begin reserves capacity for one active lease. It cannot be silently
        // replaced; all successful begin IDs can therefore acquire a tombstone.
        self.ended
            .insert(lease.public.lease_id.clone(), (lease.owner.clone(), now));
        // notify_one retains a permit even if the timer has not started polling.
        lease.stop.notify_one();
    }
    fn begin(
        &mut self,
        owner: &str,
        window: SettingsOwner,
        id: &str,
        now: Instant,
        unix_ms: u64,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<ShortcutEditLease, String> {
        valid_id(id)?;
        // Caller prunes while holding the same mutex before calling begin.
        // Native foreground observations are made before taking this mutex;
        // their revocation epoch and exit gates are rechecked here, so an
        // invalidating owner cannot admit a lease from an old observation.
        admit()?;
        if self.ended.contains_key(id) {
            return Err(CANCELED.into());
        }
        if let Some(current) = &self.active {
            return if current.owner == owner
                && current.window == window
                && current.public.lease_id == id
            {
                Ok(current.public.clone()) // Idempotent; no lifetime extension.
            } else {
                Err("请先结束当前快捷键编辑".into())
            };
        }
        if self.ended.len() >= MAX_TOMBSTONES - 1 {
            return Err("快捷键编辑请求过多，请稍后重试".into());
        }
        let public = ShortcutEditLease {
            lease_id: id.into(),
            expires_at: unix_ms.saturating_add(30_000),
        };
        self.active = Some(Editor {
            owner: owner.into(),
            window,
            public: public.clone(),
            until: now + LEASE_TTL,
            stop: Arc::new(Notify::new()),
        });
        Ok(public)
    }
    fn end(&mut self, owner: &str, id: &str, now: Instant) -> Result<Option<Editor>, String> {
        valid_id(id)?;
        if let Some(current) = self
            .active
            .as_ref()
            .filter(|lease| lease.public.lease_id == id)
        {
            if current.owner != owner {
                return Err("快捷键编辑不属于此窗口".into());
            }
            let previous = self.active.take().unwrap();
            self.remember(&previous, now);
            return Ok(Some(previous));
        }
        if let Some((previous, _)) = self.ended.get(id) {
            if previous != owner {
                return Err("快捷键编辑不属于此窗口".into());
            }
            return Ok(None); // Do not extend an old tombstone with repeated end.
        }
        let limit = MAX_TOMBSTONES - usize::from(self.active.is_some());
        if self.ended.len() >= limit {
            return Err("快捷键编辑请求过多，请稍后重试".into());
        }
        self.ended.insert(id.into(), (owner.into(), now));
        Ok(None)
    }
    fn invalidate(&mut self, owner: Option<&str>, now: Instant) -> Option<Editor> {
        if !self
            .active
            .as_ref()
            .is_some_and(|v| owner.is_none_or(|owner| owner == v.owner))
        {
            return None;
        }
        let previous = self.active.take().unwrap();
        self.remember(&previous, now);
        Some(previous)
    }
    fn invalidate_owned(
        &mut self,
        owner: &str,
        expected: SettingsOwner,
        now: Instant,
    ) -> Option<Editor> {
        // A save admitted before a UI query/claim gap may belong to an old
        // settings lifetime. Never retire a replacement window's input lease.
        if !self
            .active
            .as_ref()
            .is_some_and(|lease| lease.window == expected)
        {
            return None;
        }
        self.invalidate(Some(owner), now)
    }
    fn expire_exact(&mut self, lease: &Editor, now: Instant) -> Option<Editor> {
        let exact = self
            .active
            .as_ref()
            .is_some_and(|active| active.same_lease(lease));
        if exact {
            self.prune(now)
        } else {
            None
        }
    }
}

#[derive(Clone, Default)]
struct Route {
    active: Option<u32>,
    owned: Vec<(u32, ShortcutChord)>,
}
struct Inner {
    busy: AtomicBool,
    abort: AtomicBool,
    faulted: AtomicBool,
    // Focus loss/destruction revokes observations made before native getters.
    // This counter is checked again under the editor mutex, never by waiting
    // on the UI thread while that mutex is held.
    owner_epoch: AtomicU64,
    model: Mutex<Option<ShortcutModel>>,
    view: Mutex<CaptureShortcutState>,
    route: Mutex<Route>,
    editors: Mutex<Editors>,
    // The canceled previous timer may still be exiting when the next lease starts.
    // Its slot stays held until that task actually exits; no unbounded task queue.
    expiry_slots: Arc<Semaphore>,
    preferences: Option<PreferencesFile>,
}
#[derive(Clone)]
pub struct HostRuntime(Arc<Inner>);
impl HostRuntime {
    fn view(&self) -> CaptureShortcutState {
        self.0
            .view
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn claim(&self) -> Result<ModelWork, String> {
        if self.0.faulted.load(Ordering::Acquire) {
            return Err("快捷键状态不可用，请重启后检查".into());
        }
        self.0
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| BUSY)?;
        let permit = Permit(self.0.clone());
        self.0.abort.store(false, Ordering::Release);
        let model = self
            .0
            .model
            .lock()
            .map_err(|_| "快捷键状态不可用")?
            .take()
            .ok_or("快捷键状态不可用")?;
        Ok(ModelWork {
            inner: self.0.clone(),
            model: Some(model),
            _permit: permit,
        })
    }
}
struct Permit(Arc<Inner>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}
struct ModelWork {
    inner: Arc<Inner>,
    model: Option<ShortcutModel>,
    _permit: Permit,
}
impl Drop for ModelWork {
    fn drop(&mut self) {
        // Restores ownership before Permit drops, even if the invoke was dropped.
        // A panic makes future editing fail closed; it does not lose known routing.
        if std::thread::panicking() {
            self.inner.faulted.store(true, Ordering::Release);
            let mut view = self.inner.view.lock().unwrap_or_else(|e| e.into_inner());
            view.editable = false;
        }
        *self.inner.model.lock().unwrap_or_else(|e| e.into_inner()) = self.model.take();
    }
}

fn valid_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id) {
        Ok(())
    } else {
        Err("快捷键编辑标识无效".into())
    }
}
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn shortcut(chord: &ShortcutChord) -> Result<Shortcut, RegistrationError> {
    chord
        .validate()
        .map_err(|_| RegistrationError::Unavailable)?;
    // All pieces come from validated structured fields. No arbitrary accelerator
    // is accepted, and DOM physical code is not treated as a scan-code mapping.
    let mut text = String::new();
    if chord.ctrl {
        text.push_str("Control+");
    }
    if chord.shift {
        text.push_str("Shift+");
    }
    if chord.alt {
        text.push_str("Alt+");
    }
    text.push(chord.code.as_bytes()[3] as char);
    text.parse().map_err(|_| RegistrationError::Unavailable)
}
fn display(chord: &ShortcutChord) -> String {
    let mut values = Vec::new();
    if chord.ctrl {
        values.push("Ctrl");
    }
    if chord.shift {
        values.push("Shift");
    }
    if chord.alt {
        values.push("Alt");
    }
    values.push(chord.code.strip_prefix("Key").unwrap_or("?"));
    values.join(" + ")
}
fn window_handle(window: &WebviewWindow) -> Result<usize, String> {
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|h| h.0 as usize)
            .map_err(|_| "设置窗口不可用".into())
    }
    #[cfg(target_os = "macos")]
    {
        // Only called in observe_settings_now on the owning UI thread. Do not
        // reuse capture witnesses here: their screen-capture permission check
        // must not prevent editing a shortcut before the first capture.
        objc2::MainThreadMarker::new().ok_or("设置窗口只能在界面线程读取")?;
        window
            .ns_window()
            .map(|value| value as usize)
            .map_err(|_| "设置窗口不可用".into())
    }
    #[cfg(target_os = "linux")]
    {
        crate::window_actions_linux::window_handle(window)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = window;
        Err("此平台尚不支持编辑截图快捷键".into())
    }
}
fn visible(handle: usize) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        handle != 0 && IsWindow(handle as _) != 0 && IsWindowVisible(handle as _) != 0
    }
    #[cfg(target_os = "macos")]
    {
        // The live AppHandle window and its native identity are resolved in
        // one main-thread closure; its actual visibility is queried there too.
        handle != 0
    }
    #[cfg(target_os = "linux")]
    {
        crate::window_actions_linux::current_for(handle, "settings")
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = handle;
        false
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SettingsOwner {
    handle: usize,
    epoch: u64,
}
#[derive(Clone, Copy)]
struct OwnerObservation {
    handle: usize,
    visible: bool,
    minimized: bool,
    focused: bool,
}
fn owner_matches(
    owner: SettingsOwner,
    before: u64,
    after: u64,
    observed: Option<OwnerObservation>,
) -> bool {
    owner.handle != 0
        && owner.epoch == before
        && before == after
        && observed.is_some_and(|value| {
            value.handle == owner.handle && value.visible && !value.minimized && value.focused
        })
}
/// Resolve the *current* owned settings window on every check. Tauri window
/// equality compares labels only, so it is not a native lifetime witness.
/// GTK uses existing generation tokens. AppKit native identities are compared
/// only in the UI closure; an old address is never dereferenced or used to find
/// another window. Focus/destruction epochs prevent reusing the same address.
fn observe_settings_now(
    app: &AppHandle,
    caller: Option<&WebviewWindow>,
) -> Option<OwnerObservation> {
    let window = app.get_webview_window("settings")?;
    let handle = window_handle(&window).ok()?;
    if let Some(caller) = caller {
        if caller.label() != "settings" || window_handle(caller).ok()? != handle {
            return None;
        }
    }
    let focused = window.is_focused().ok()?;
    #[cfg(target_os = "macos")]
    let focused = focused
        && objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new()?)
            .isActive();
    Some(OwnerObservation {
        handle,
        visible: visible(handle) && window.is_visible().ok()?,
        minimized: window.is_minimized().ok()?,
        focused,
    })
}
/// No editor/model mutex may be held across these native/UI getters. A stale
/// caller dispatcher fails closed; current Mac native handles cannot outlive
/// this single UI closure. Timeout performs no native mutation or late admit.
fn observe_settings(app: &AppHandle, caller: Option<&WebviewWindow>) -> Option<OwnerObservation> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        #[cfg(target_os = "macos")]
        let ui_thread = objc2::MainThreadMarker::new().is_some();
        #[cfg(target_os = "linux")]
        let ui_thread = gtk::is_initialized_main_thread();
        if ui_thread {
            return observe_settings_now(app, caller);
        }
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let target = app.clone();
        let caller = caller.cloned();
        app.run_on_main_thread(move || {
            let _ = send.send(observe_settings_now(&target, caller.as_ref()));
        })
        .ok()?;
        receive.recv_timeout(Duration::from_secs(2)).ok().flatten()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        observe_settings_now(app, caller)
    }
}
fn foreground(app: &AppHandle, inner: &Inner, owner: SettingsOwner) -> bool {
    let before = inner.owner_epoch.load(Ordering::Acquire);
    if before != owner.epoch {
        return false;
    }
    let observed = observe_settings(app, None);
    let matched = owner_matches(
        owner,
        before,
        inner.owner_epoch.load(Ordering::Acquire),
        observed,
    );
    #[cfg(windows)]
    unsafe {
        matched
            && windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as usize
                == owner.handle
    }
    #[cfg(not(windows))]
    {
        matched
    }
}
fn settings_owner(
    app: &AppHandle,
    inner: &Inner,
    window: &WebviewWindow,
) -> Result<SettingsOwner, String> {
    if window.label() != "settings" {
        return Err("请在设置窗口编辑快捷键".into());
    }
    let epoch = inner.owner_epoch.load(Ordering::Acquire);
    let observed = observe_settings(app, Some(window)).ok_or("设置窗口不可用")?;
    let owner = SettingsOwner {
        handle: observed.handle,
        epoch,
    };
    if owner_matches(
        owner,
        epoch,
        inner.owner_epoch.load(Ordering::Acquire),
        Some(observed),
    ) {
        #[cfg(windows)]
        if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as usize }
            != owner.handle
        {
            return Err("请在设置窗口编辑快捷键".into());
        }
        Ok(owner)
    } else {
        Err("请在设置窗口编辑快捷键".into())
    }
}
fn admission(app: &AppHandle, inner: &Inner, owner: Option<SettingsOwner>) -> bool {
    let Some(host) = app.try_state::<Host>() else {
        return false;
    };
    !inner.faulted.load(Ordering::Acquire)
        && host.exit.is_running()
        && !host.capturing.load(Ordering::Acquire)
        && owner.is_none_or(|owner| foreground(app, inner, owner))
        && crate::recording::ensure_idle(app).is_ok()
}
fn gate(app: &AppHandle, inner: &Inner, owner: Option<SettingsOwner>) -> bool {
    !inner.abort.load(Ordering::Acquire) && admission(app, inner, owner)
}
fn emit_ended(app: &AppHandle, lease: Option<Editor>) {
    if let Some(lease) = lease {
        let _ = app.emit_to(
            lease.owner,
            "capture-shortcut-edit-ended",
            EditEnded {
                lease_id: lease.public.lease_id,
            },
        );
    }
}

fn sync_system_editor(app: &AppHandle, editors: &Editors) {
    // Serialize with editor changes so an old expiry cannot reopen a newer
    // lease. The Windows adapter only stores atomics here, never waits.
    crate::system_capture_shortcut::set_editing(app, editors.active.is_some());
    #[cfg(windows)]
    crate::hold_key_windows::set_editing(editors.active.is_some());
}

#[cfg(windows)]
pub(crate) fn system_capture_allowed(app: &AppHandle) -> bool {
    let Some(host) = app.try_state::<Host>() else {
        return false;
    };
    if !host.exit.accepts_new_operations()
        || host.capturing.load(Ordering::Acquire)
        || crate::recording::ensure_idle(app).is_err()
    {
        return false;
    }
    app.try_state::<HostRuntime>().is_some_and(|runtime| {
        runtime
            .0
            .editors
            .lock()
            .map(|editors| editors.active.is_none())
            .unwrap_or(false)
    })
}
struct NativeBackend {
    app: AppHandle,
    inner: Arc<Inner>,
    owner: Option<SettingsOwner>,
}
impl Backend for NativeBackend {
    fn register(&mut self, chord: &ShortcutChord) -> Result<(), RegistrationError> {
        let key = shortcut(chord)?;
        // Tauri 2.4.0 flattens underlying error into a String. Do not classify
        // occupied by matching English text or expose raw OS errors to the UI.
        self.app
            .global_shortcut()
            .register(key)
            .map_err(|_| RegistrationError::Unavailable)
    }
    fn unregister(&mut self, chord: &ShortcutChord) -> Result<(), RegistrationError> {
        self.app
            .global_shortcut()
            .unregister(shortcut(chord)?)
            .map_err(|_| RegistrationError::Unavailable)
    }
    fn persist(
        &mut self,
        expected: &FileStamp,
        next: &HostPreferences,
    ) -> Result<CommitReceipt, PreferencesError> {
        let file = self
            .inner
            .preferences
            .as_ref()
            .ok_or(PreferencesError::Access)?;
        file.commit(expected, next, || self.allowed())
    }
    fn check_current(&mut self, expected: &FileStamp) -> Result<(), PreferencesError> {
        self.inner
            .preferences
            .as_ref()
            .ok_or(PreferencesError::Access)?
            .check_current(expected)
    }
    fn allowed(&self) -> bool {
        gate(&self.app, &self.inner, self.owner)
    }
    fn switch_route(&mut self, active: Option<&ShortcutChord>, owned: &[ShortcutChord]) {
        let route = Route {
            active: active.and_then(|value| shortcut(value).ok().map(|key| key.id())),
            owned: owned
                .iter()
                .filter_map(|value| shortcut(value).ok().map(|key| (key.id(), value.clone())))
                .collect(),
        };
        *self.inner.route.lock().unwrap_or_else(|e| e.into_inner()) = route;
    }
    fn publish(&mut self, value: &CaptureShortcutState) {
        {
            let mut previous = self.inner.view.lock().unwrap_or_else(|e| e.into_inner());
            if value.sequence < previous.sequence {
                return;
            }
            *previous = value.clone();
        }
        // No shared state guard is live while delivering events.
        let _ = self
            .app
            .emit_to("settings", "capture-shortcut-state", value.clone());
        let _ = self
            .app
            .emit_to("space", "capture-shortcut-state", value.clone());
    }
}

/// Root calls once during setup, after Host and global-shortcut plugin exist.
/// Even a corrupt preference creates a readonly Model, retaining app/tray access.
pub fn initialize(app: &AppHandle, root: &Path) -> Result<(), String> {
    if app.try_state::<HostRuntime>().is_some() {
        return Err("快捷键状态已初始化".into());
    }
    // Only the shared settings recorder remains here. Actual bindings are
    // imported into Hooks once and registered/executed by that rule runtime.
    let _ = root;
    let model = ShortcutModel::new(Ok(crate::host_preferences::LoadedPreferences::missing()));
    let runtime = HostRuntime(Arc::new(Inner {
        busy: AtomicBool::new(false),
        abort: AtomicBool::new(false),
        faulted: AtomicBool::new(false),
        owner_epoch: AtomicU64::new(0),
        view: Mutex::new(model.view()),
        model: Mutex::new(Some(model)),
        route: Mutex::new(Route::default()),
        editors: Mutex::new(Editors::default()),
        expiry_slots: Arc::new(Semaphore::new(2)),
        preferences: None,
    }));
    app.manage(runtime.clone());
    Ok(())
}
fn start_expiry_timer(
    app: AppHandle,
    runtime: HostRuntime,
    lease: Editor,
    slot: OwnedSemaphorePermit,
) {
    tauri::async_runtime::spawn(async move {
        let _slot = slot;
        tokio::select! {
            biased;
            _ = lease.stop.notified() => return,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(lease.until)) => {}
        }
        let expired = {
            let mut editors = runtime.0.editors.lock().unwrap_or_else(|e| e.into_inner());
            let expired = editors.expire_exact(&lease, Instant::now());
            sync_system_editor(&app, &editors);
            expired
        };
        emit_ended(&app, expired);
    });
}

#[tauri::command]
pub fn get_capture_shortcut(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<CaptureShortcutState, String> {
    if !matches!(window.label(), "settings" | "space") {
        return Err("此窗口不能读取截图快捷键".into());
    }
    crate::hooks::shortcut_state(&app, "capture.open")
}

#[tauri::command]
pub async fn set_capture_shortcut(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: u64,
    shortcut: Option<ShortcutChord>,
) -> Result<CaptureShortcutState, String> {
    if window.label() != "settings" {
        return Err("请在设置中修改截图快捷键".into());
    }
    let runtime = app.state::<HostRuntime>().inner().clone();
    let owner = settings_owner(&app, &runtime.0, &window)?; // No shared mutex.
    if !admission(&app, &runtime.0, Some(owner)) {
        return Err("当前无法修改截图快捷键".into());
    }
    // Begin checks the busy flag under the editor mutex. Either it was already
    // admitted and is ended here, or it observes this permit and is rejected.
    retire_editor(&app, "settings", Some(owner));
    if !gate(&app, &runtime.0, Some(owner)) {
        return Err("当前无法修改截图快捷键".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::hooks::change_action_shortcut(&app, expected_revision, "capture.open", shortcut)
    })
    .await
    .map_err(|_| "快捷键设置任务中断，请重启后检查".to_string())?
}

#[tauri::command]
pub fn begin_capture_shortcut_edit(
    app: AppHandle,
    window: WebviewWindow,
    lease_id: String,
) -> Result<ShortcutEditLease, String> {
    if window.label() != "settings" {
        return Err("请在设置中编辑截图快捷键".into());
    }
    let runtime = app.state::<HostRuntime>().inner().clone();
    crate::recording::ensure_idle(&app)?;
    let owner = settings_owner(&app, &runtime.0, &window)?;
    let now = Instant::now();
    let (expired, result, timer) = {
        let mut editors = runtime
            .0
            .editors
            .lock()
            .map_err(|_| "快捷键编辑状态不可用")?;
        let expired = editors.prune(now);
        let had_active = editors.active.is_some();
        let mut timer_slot = None;
        let result = editors.begin("settings", owner, &lease_id, now, unix_ms(), || {
            app.state::<Host>().exit.ensure_running()?;
            if !runtime.view().editable || runtime.0.faulted.load(Ordering::Acquire) {
                return Err("快捷键设置当前不可编辑".into());
            }
            if runtime.0.busy.load(Ordering::Acquire) {
                return Err(BUSY.into());
            }
            if app.state::<Host>().capturing.load(Ordering::Acquire)
                || runtime.0.owner_epoch.load(Ordering::Acquire) != owner.epoch
            {
                return Err("请在设置窗口编辑快捷键".into());
            }
            if !had_active {
                timer_slot = Some(
                    runtime
                        .0
                        .expiry_slots
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| BUSY)?,
                );
            }
            Ok(())
        });
        let timer = if result.is_ok() {
            timer_slot.map(|slot| (editors.active.as_ref().unwrap().clone(), slot))
        } else {
            None
        };
        sync_system_editor(&app, &editors);
        (expired, result, timer)
    };
    emit_ended(&app, expired);
    if let Some((lease, slot)) = timer {
        start_expiry_timer(app, runtime, lease, slot);
    }
    result
}

#[tauri::command]
pub fn end_capture_shortcut_edit(
    app: AppHandle,
    window: WebviewWindow,
    lease_id: String,
) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("请在设置中结束快捷键编辑".into());
    }
    end_editor(&app, &lease_id)
}
pub(crate) fn end_editor(app: &AppHandle, lease_id: &str) -> Result<(), String> {
    // Cleanup must remain legal in PREPARING/DRAINING and after focus loss.
    let runtime = app.state::<HostRuntime>();
    let now = Instant::now();
    let (expired, ended) = {
        let mut editors = runtime
            .0
            .editors
            .lock()
            .map_err(|_| "快捷键编辑状态不可用")?;
        let expired = editors.prune(now);
        sync_system_editor(&app, &editors);
        let ended = editors.end("settings", lease_id, now)?;
        sync_system_editor(&app, &editors);
        (expired, ended)
    };
    emit_ended(&app, expired);
    emit_ended(&app, ended);
    Ok(())
}

pub fn active_display(app: &AppHandle) -> String {
    crate::hooks::shortcut_label(app, "capture.open")
}
/// Shared settings recorder admission, also used by other registered actions.
/// No UI/DB calls or feature-specific IDs on the native shortcut callback.
pub(crate) fn editor_active(app: &AppHandle) -> bool {
    app.try_state::<HostRuntime>().is_some_and(|r| {
        r.0.editors.lock().is_ok_and(|editors| {
            editors
                .active
                .as_ref()
                .is_some_and(|e| Instant::now() < e.until)
        })
    })
}
/// Call only for ShortcutState::Pressed. Other feature handlers remain untouched.
/// True means root should call its existing start_capture; false is consumed/ignored.
pub fn route_pressed(app: &AppHandle, key: &Shortcut) -> bool {
    let Some(runtime) = app.try_state::<HostRuntime>() else {
        return false;
    };
    let Some(host) = app.try_state::<Host>() else {
        return false;
    };
    if !host.exit.is_running() {
        return false;
    }
    let route = runtime
        .0
        .route
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some((_, chord)) = route.owned.iter().find(|(id, _)| *id == key.id()) else {
        return false;
    };
    // A foreground query can visit the UI thread. Observe outside the lease
    // mutex, then match that exact owner again so neither a replacement nor a
    // lease beginning during the query is canceled or routed by an old result.
    let observed_lease = runtime
        .0
        .editors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .active
        .clone();
    let focused = observed_lease
        .as_ref()
        .is_some_and(|lease| foreground(app, &runtime.0, lease.window));
    let now = Instant::now(); // Native dispatch must not extend an expired lease.
    let (ended, recorded, changed_lease) = {
        let mut editors = runtime.0.editors.lock().unwrap_or_else(|e| e.into_inner());
        let mut ended = editors.prune(now);
        let changed_lease = !editors.matches_observation(observed_lease.as_ref());
        if editors.active.as_ref().is_some_and(|lease| {
            !changed_lease
                && (!focused
                    || runtime.0.owner_epoch.load(Ordering::Acquire) != lease.window.epoch
                    || !host.exit.is_running()
                    || host.capturing.load(Ordering::Acquire))
        }) {
            ended = ended.or_else(|| editors.invalidate(None, now));
        }
        let recorded = editors
            .active
            .as_ref()
            .filter(|_| !changed_lease)
            .map(|lease| {
                (
                    lease.owner.clone(),
                    Recorded {
                        lease_id: lease.public.lease_id.clone(),
                        shortcut: chord.clone(),
                    },
                )
            });
        sync_system_editor(app, &editors);
        (ended, recorded, changed_lease)
    };
    emit_ended(app, ended);
    if changed_lease {
        return false;
    }
    if let Some((owner, value)) = recorded {
        let _ = app.emit_to(owner, "capture-shortcut-recorded", value);
        return false;
    }
    if !host.exit.is_running()
        || host.capturing.load(Ordering::Acquire)
        || crate::recording::ensure_idle(app).is_err()
    {
        return false;
    }
    // Check the live route again: a worker may have switched it during lease work.
    let active = runtime
        .0
        .route
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .active;
    active == Some(key.id())
}
pub fn invalidate_owner(app: &AppHandle, owner: &str) {
    #[cfg(target_os = "linux")]
    if owner == "settings" {
        if let Some(runtime) = app.try_state::<crate::capture_shortcut_portal_linux::Runtime>() {
            runtime.invalidate_settings();
        }
    }
    if owner == "settings" {
        if let Some(runtime) = app.try_state::<HostRuntime>() {
            revoke_owner_epoch(&runtime.0);
        }
    }
    retire_editor(app, owner, None);
}
fn revoke_owner_epoch(inner: &Inner) {
    if inner
        .owner_epoch
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .is_err()
    {
        inner.faulted.store(true, Ordering::Release);
        inner.abort.store(true, Ordering::Release);
    }
}
/// Saving retires the input lease without revoking its own still-foreground
/// owner. Native focus/destruction invalidation uses invalidate_owner instead.
fn retire_editor(app: &AppHandle, owner: &str, expected: Option<SettingsOwner>) {
    let Some(runtime) = app.try_state::<HostRuntime>() else {
        return;
    };
    let old = {
        let mut editors = runtime.0.editors.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let old = match expected {
            Some(window) => editors.invalidate_owned(owner, window, now),
            None => editors.invalidate(Some(owner), now),
        };
        sync_system_editor(app, &editors);
        old
    };
    emit_ended(app, old);
}
/// Wayland consent legitimately moves focus to the system dialog. The portal
/// adapter separately observes actual GTK unmap/unrealize; other backends keep
/// their existing foreground recorder cancellation behavior.
pub(crate) fn owner_event_invalidates(event: &tauri::WindowEvent, wayland_portal: bool) -> bool {
    matches!(event, tauri::WindowEvent::Destroyed)
        || (!wayland_portal && matches!(event, tauri::WindowEvent::Focused(false)))
}
pub fn invalidate_all(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    if let Some(runtime) = app.try_state::<crate::capture_shortcut_portal_linux::Runtime>() {
        runtime.invalidate_settings();
    }
    let Some(runtime) = app.try_state::<HostRuntime>() else {
        return;
    };
    revoke_owner_epoch(&runtime.0);
    let old = {
        let mut editors = runtime.0.editors.lock().unwrap_or_else(|e| e.into_inner());
        let old = editors.invalidate(None, Instant::now());
        sync_system_editor(app, &editors);
        old
    };
    emit_ended(app, old);
}
pub async fn drain(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    #[cfg(windows)]
    crate::hold_key_windows::drain(timeout).await?;
    #[cfg(target_os = "linux")]
    if let Some(runtime) = app.try_state::<crate::capture_shortcut_portal_linux::Runtime>() {
        return runtime.inner().clone().drain(timeout).await;
    }
    let Some(runtime) = app
        .try_state::<HostRuntime>()
        .map(|state| state.inner().clone())
    else {
        return Ok(());
    };
    runtime.0.abort.store(true, Ordering::Release);
    invalidate_all(app);
    let started = Instant::now();
    while runtime.0.busy.load(Ordering::Acquire) {
        if started.elapsed() >= timeout {
            return Err("快捷键设置尚未结束，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wayland_system_dialog_focus_loss_preserves_request_but_destroy_always_revokes() {
        use tauri::WindowEvent;
        assert!(!owner_event_invalidates(&WindowEvent::Focused(false), true));
        assert!(owner_event_invalidates(&WindowEvent::Focused(false), false));
        assert!(owner_event_invalidates(&WindowEvent::Destroyed, true));
        assert!(owner_event_invalidates(&WindowEvent::Destroyed, false));
        assert!(!owner_event_invalidates(&WindowEvent::Focused(true), true));
    }
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn owner(handle: usize) -> SettingsOwner {
        SettingsOwner { handle, epoch: 0 }
    }
    fn observation(handle: usize) -> OwnerObservation {
        OwnerObservation {
            handle,
            visible: true,
            minimized: false,
            focused: true,
        }
    }
    #[test]
    fn owned_settings_requires_same_native_lifetime_visible_and_foreground() {
        let expected = SettingsOwner {
            handle: 7,
            epoch: 3,
        };
        assert!(owner_matches(expected, 3, 3, Some(observation(7))));
        for value in [
            None,
            Some(observation(8)),
            Some(OwnerObservation {
                visible: false,
                ..observation(7)
            }),
            Some(OwnerObservation {
                minimized: true,
                ..observation(7)
            }),
            Some(OwnerObservation {
                focused: false,
                ..observation(7)
            }),
        ] {
            assert!(!owner_matches(expected, 3, 3, value));
        }
        assert!(!owner_matches(owner(0), 0, 0, Some(observation(0))));
    }
    #[test]
    fn native_focus_observation_cannot_cross_blur_destroy_or_return_to_same_handle() {
        let expected = SettingsOwner {
            handle: 7,
            epoch: 3,
        };
        assert!(!owner_matches(expected, 3, 4, Some(observation(7))));
        assert!(!owner_matches(expected, 4, 4, Some(observation(7))));
        // New ownership can be admitted, but a late expiry retains the exact
        // old epoch and cannot consume it even if a native handle is reused.
        let mut leases = Editors::default();
        let now = Instant::now();
        let first = id();
        leases
            .begin("settings", expected, &first, now, 100, || Ok(()))
            .unwrap();
        let old = leases.active.as_ref().unwrap().clone();
        leases.invalidate(Some("settings"), now);
        let next = id();
        let replacement = SettingsOwner {
            epoch: 4,
            ..expected
        };
        leases
            .begin("settings", replacement, &next, now, 100, || Ok(()))
            .unwrap();
        assert!(leases.expire_exact(&old, now + LEASE_TTL).is_none());
        assert_eq!(leases.active.as_ref().unwrap().window, replacement);
        assert!(leases
            .begin("settings", expected, &next, now, 100, || Ok(()))
            .is_err());
    }
    #[test]
    fn native_observation_revoked_before_editor_lock_does_not_arm() {
        let expected = SettingsOwner {
            handle: 7,
            epoch: 3,
        };
        let epoch = AtomicU64::new(3);
        assert!(owner_matches(
            expected,
            3,
            epoch.load(Ordering::Acquire),
            Some(observation(7))
        ));
        // The real focus/destroy event increments this before acquiring the
        // lease mutex; begin must refuse the previously successful UI query.
        epoch.fetch_add(1, Ordering::AcqRel);
        let mut leases = Editors::default();
        assert!(leases
            .begin("settings", expected, &id(), Instant::now(), 100, || {
                if epoch.load(Ordering::Acquire) == expected.epoch {
                    Ok(())
                } else {
                    Err(CANCELED.into())
                }
            })
            .is_err());
        assert!(leases.active.is_none());
    }
    #[test]
    fn delayed_foreground_observation_cannot_record_or_cancel_same_owner_successor_lease() {
        let mut leases = Editors::default();
        let now = Instant::now();
        assert!(leases.matches_observation(None));
        let first = id();
        leases
            .begin("settings", owner(7), &first, now, 100, || Ok(()))
            .unwrap();
        let observed = leases.active.clone();
        assert!(leases.matches_observation(observed.as_ref()));
        assert!(!leases.matches_observation(None));
        // The UI query is in flight. End and immediately rearm in the same
        // native window/epoch; an address-only comparison would accept this.
        leases.end("settings", &first, now).unwrap();
        assert!(!leases.matches_observation(observed.as_ref()));
        let next = id();
        leases
            .begin("settings", owner(7), &next, now, 100, || Ok(()))
            .unwrap();
        assert!(!leases.matches_observation(observed.as_ref()));
        assert_eq!(leases.active.as_ref().unwrap().public.lease_id, next);
        assert!(leases
            .expire_exact(observed.as_ref().unwrap(), now + LEASE_TTL)
            .is_none());
        assert_eq!(leases.active.as_ref().unwrap().public.lease_id, next);
        assert!(leases.matches_observation(leases.active.as_ref()));
    }
    #[test]
    fn late_save_cannot_retire_replacement_settings_lease_but_same_owner_save_can() {
        let mut leases = Editors::default();
        let now = Instant::now();
        let old_owner = owner(7);
        let first = id();
        leases
            .begin("settings", old_owner, &first, now, 100, || Ok(()))
            .unwrap();
        let old_timer = leases.active.clone().unwrap();
        // Old Save has observed its window, but has not claimed the worker.
        // Focus/destruction retires it and a replacement arms before claim.
        leases.invalidate(Some("settings"), now).unwrap();
        let replacement = SettingsOwner {
            epoch: 1,
            ..old_owner
        };
        let next = id();
        leases
            .begin("settings", replacement, &next, now, 100, || Ok(()))
            .unwrap();
        let current = leases.active.clone().unwrap();
        assert!(leases
            .invalidate_owned("settings", old_owner, now)
            .is_none());
        assert!(leases.matches_observation(Some(&current)));
        assert!(leases.invalidate_owned("space", replacement, now).is_none());
        assert!(leases.matches_observation(Some(&current)));
        assert!(leases.expire_exact(&old_timer, now + LEASE_TTL).is_none());
        assert_eq!(leases.active.as_ref().unwrap().public.lease_id, next);
        let retired = leases
            .invalidate_owned("settings", replacement, now)
            .unwrap();
        assert!(retired.same_lease(&current));
        assert!(leases.active.is_none());
        assert!(leases.ended.contains_key(&first) && leases.ended.contains_key(&next));
    }
    #[test]
    fn early_end_cannot_arm_and_slow_old_end_cannot_clear_new_lease() {
        let now = Instant::now();
        let mut leases = Editors::default();
        let old = id();
        let next = id();
        leases.end("settings", &old, now).unwrap();
        assert!(leases
            .begin("settings", owner(1), &old, now, 100, || Ok(()))
            .is_err());
        leases
            .begin("settings", owner(1), &next, now, 100, || Ok(()))
            .unwrap();
        assert!(leases.end("settings", &old, now).unwrap().is_none());
        assert_eq!(leases.active.as_ref().unwrap().public.lease_id, next);
    }
    #[test]
    fn different_active_lease_is_busy_and_same_id_does_not_renew() {
        let now = Instant::now();
        let mut leases = Editors::default();
        let first = id();
        let original = leases
            .begin("settings", owner(1), &first, now, 100, || Ok(()))
            .unwrap();
        assert!(leases
            .begin("settings", owner(1), &id(), now, 100, || Ok(()))
            .is_err());
        assert!(leases
            .begin("settings", owner(2), &first, now, 100, || Ok(()))
            .is_err());
        assert_eq!(
            leases
                .begin(
                    "settings",
                    owner(1),
                    &first,
                    now + Duration::from_secs(20),
                    20_100,
                    || Ok(())
                )
                .unwrap(),
            original
        );
        assert!(leases.prune(now + LEASE_TTL).is_some());
        assert!(leases
            .begin(
                "settings",
                owner(1),
                &first,
                now + LEASE_TTL,
                30_100,
                || Ok(())
            )
            .is_err());
    }
    #[test]
    fn owner_admission_and_tombstone_budget_are_bounded() {
        let now = Instant::now();
        let mut leases = Editors::default();
        assert!(leases
            .begin("settings", owner(1), &id(), now, 100, || Err(
                "hidden".into()
            ))
            .is_err());
        assert!(leases.active.is_none());
        let current = id();
        leases
            .begin("settings", owner(1), &current, now, 100, || Ok(()))
            .unwrap();
        assert!(leases.end("space", &current, now).is_err());
        for _ in 0..MAX_TOMBSTONES - 1 {
            leases.end("settings", &id(), now).unwrap();
        }
        assert!(leases.end("settings", &id(), now).is_err());
        assert!(leases.end("settings", &current, now).unwrap().is_some());
        assert_eq!(leases.ended.len(), MAX_TOMBSTONES);
        leases.prune(now + TOMBSTONE_TTL);
        assert!(leases.ended.is_empty());
    }
    #[test]
    fn only_validated_virtual_letters_reach_native_parser() {
        let good = ShortcutChord::default_capture();
        assert_eq!(
            shortcut(&good).unwrap(),
            "Alt+Shift+S".parse::<Shortcut>().unwrap()
        );
        for code in ["KeyF8", "F8", "Escape", "Keya", "Control+X", "Key1"] {
            assert!(shortcut(&ShortcutChord {
                code: code.into(),
                ..good.clone()
            })
            .is_err());
        }
    }
    #[test]
    fn late_expiry_does_not_clear_a_new_lease() {
        let now = Instant::now();
        let mut leases = Editors::default();
        let old = id();
        leases
            .begin("settings", owner(1), &old, now, 100, || Ok(()))
            .unwrap();
        let timer = leases.active.as_ref().unwrap().clone();
        leases.end("settings", &old, now).unwrap();
        let next = id();
        leases
            .begin("settings", owner(1), &next, now + LEASE_TTL, 30_100, || {
                Ok(())
            })
            .unwrap();
        assert!(leases.expire_exact(&timer, now + LEASE_TTL).is_none());
        assert_eq!(leases.active.as_ref().unwrap().public.lease_id, next);
    }
    #[tokio::test]
    async fn end_before_timer_poll_wakes_cleanup_and_slots_stay_bounded() {
        let now = Instant::now();
        let mut leases = Editors::default();
        let lease_id = id();
        leases
            .begin("settings", owner(1), &lease_id, now, 100, || Ok(()))
            .unwrap();
        let lease = leases.active.as_ref().unwrap().clone();
        let slots = Arc::new(Semaphore::new(2));
        let canceled = slots.clone().try_acquire_owned().unwrap();
        leases.end("settings", &lease_id, now).unwrap();
        let current = slots.clone().try_acquire_owned().unwrap();
        assert!(slots.clone().try_acquire_owned().is_err());
        tokio::time::timeout(Duration::from_secs(1), lease.stop.notified())
            .await
            .unwrap();
        // Cancellation itself did not free a task permit; actual task exit does.
        drop(canceled);
        assert!(slots.clone().try_acquire_owned().is_ok());
        drop(current);
    }
    #[test]
    fn worker_owns_model_and_permit_until_exit_then_canceled_exit_can_recover() {
        let model = ShortcutModel::new(Ok(crate::capture_shortcut::LoadedPreferences::missing()));
        let runtime = HostRuntime(Arc::new(Inner {
            busy: AtomicBool::new(false),
            abort: AtomicBool::new(false),
            faulted: AtomicBool::new(false),
            owner_epoch: AtomicU64::new(0),
            view: Mutex::new(model.view()),
            model: Mutex::new(Some(model)),
            route: Mutex::new(Route::default()),
            editors: Mutex::new(Editors::default()),
            expiry_slots: Arc::new(Semaphore::new(2)),
            preferences: None,
        }));
        let work = runtime.claim().unwrap();
        let (done, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
            drop(work);
        });
        runtime.0.abort.store(true, Ordering::Release);
        assert!(runtime.claim().is_err());
        assert!(runtime.0.model.lock().unwrap().is_none());
        // Simulated invoke owner can end while the worker remains blocked.
        done.send(()).unwrap();
        worker.join().unwrap();
        assert!(!runtime.0.busy.load(Ordering::Acquire));
        assert!(runtime.0.model.lock().unwrap().is_some());
        let next = runtime.claim().unwrap();
        assert!(!runtime.0.abort.load(Ordering::Acquire));
        drop(next);
    }
}

#[tauri::command]
pub async fn configure_capture_shortcut_portal(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: u64,
    request_id: String,
) -> Result<CaptureShortcutState, String> {
    #[cfg(target_os = "linux")]
    {
        return crate::capture_shortcut_wayland_host::configure(
            app,
            window,
            expected_revision,
            request_id,
        )
        .await;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, window, expected_revision, request_id);
        Err("此平台不使用系统快捷键授权".into())
    }
}
#[tauri::command]
pub async fn cancel_capture_shortcut_portal(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        return crate::capture_shortcut_wayland_host::cancel(&app, &window, &request_id).await;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, window, request_id);
        Err("此平台不使用系统快捷键授权".into())
    }
}
#[tauri::command]
pub fn open_capture_shortcut_system_settings(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        return crate::capture_shortcut_wayland_host::open_system_settings(&app, &window);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, window);
        Err("此平台不使用系统快捷键授权".into())
    }
}
