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
        atomic::{AtomicBool, Ordering},
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
    window: usize,
    public: ShortcutEditLease,
    until: Instant,
    stop: Arc<Notify>,
}
#[derive(Default)]
struct Editors {
    active: Option<Editor>,
    // End-before-begin must be remembered without unbounded request-ID growth.
    ended: HashMap<String, (String, Instant)>,
}
impl Editors {
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
        window: usize,
        id: &str,
        now: Instant,
        unix_ms: u64,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<ShortcutEditLease, String> {
        valid_id(id)?;
        // Caller prunes while holding the same mutex before calling begin.
        // The native foreground/exit checks also run under this mutex, so a
        // concurrently invalidating owner cannot miss a new lease.
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
    fn expire_exact(&mut self, lease: &Editor, now: Instant) -> Option<Editor> {
        let exact = self.active.as_ref().is_some_and(|active| {
            active.owner == lease.owner
                && active.window == lease.window
                && active.public.lease_id == lease.public.lease_id
                && active.until == lease.until
        });
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
    #[cfg(not(windows))]
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
    #[cfg(not(windows))]
    {
        let _ = handle;
        false
    }
}
fn foreground(handle: usize) -> bool {
    #[cfg(windows)]
    unsafe {
        visible(handle)
            && windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as usize == handle
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        false
    }
}
fn admission(app: &AppHandle, inner: &Inner, owner: Option<usize>) -> bool {
    let Some(host) = app.try_state::<Host>() else {
        return false;
    };
    !inner.faulted.load(Ordering::Acquire)
        && host.exit.is_running()
        && !host.capturing.load(Ordering::Acquire)
        && owner.is_none_or(visible)
        && crate::recording::ensure_idle(app).is_ok()
}
fn gate(app: &AppHandle, inner: &Inner, owner: Option<usize>) -> bool {
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
struct NativeBackend {
    app: AppHandle,
    inner: Arc<Inner>,
    owner: Option<usize>,
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
    let (preferences, loaded) = match PreferencesFile::open(root) {
        Ok(file) => {
            let value = file.load();
            (Some(file), value)
        }
        Err(error) => (None, Err(error)),
    };
    let model = ShortcutModel::new(loaded);
    let runtime = HostRuntime(Arc::new(Inner {
        busy: AtomicBool::new(false),
        abort: AtomicBool::new(false),
        faulted: AtomicBool::new(false),
        view: Mutex::new(model.view()),
        model: Mutex::new(Some(model)),
        route: Mutex::new(Route::default()),
        editors: Mutex::new(Editors::default()),
        expiry_slots: Arc::new(Semaphore::new(2)),
        preferences,
    }));
    app.manage(runtime.clone());
    {
        let mut work = runtime.claim()?;
        let mut backend = NativeBackend {
            app: app.clone(),
            inner: runtime.0.clone(),
            owner: None,
        };
        // The Model is owned locally here; there is no mutex held across startup.
        work.model.as_mut().unwrap().startup(&mut backend);
    }
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
            editors.expire_exact(&lease, Instant::now())
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
    Ok(app.state::<HostRuntime>().view())
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
    let owner = window_handle(&window)?; // Never inside any shared mutex.
    let runtime = app.state::<HostRuntime>().inner().clone();
    if !admission(&app, &runtime.0, Some(owner)) {
        return Err("当前无法修改截图快捷键".into());
    }
    let mut work = runtime.claim()?;
    // Begin checks the busy flag under the editor mutex. Either it was already
    // admitted and is ended here, or it observes this permit and is rejected.
    invalidate_owner(&app, "settings");
    if !gate(&app, &runtime.0, Some(owner)) {
        return Err("当前无法修改截图快捷键".into());
    }
    let inner = runtime.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut backend = NativeBackend {
            app,
            inner,
            owner: Some(owner),
        };
        let result = work
            .model
            .as_mut()
            .unwrap()
            .set(expected_revision, shortcut, &mut backend)
            .map_err(|error| error.to_string());
        // work restores Model then returns the permit before JoinHandle resolves.
        drop(work);
        result
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
    let handle = window_handle(&window)?;
    let runtime = app.state::<HostRuntime>().inner().clone();
    crate::recording::ensure_idle(&app)?;
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
        let result = editors.begin("settings", handle, &lease_id, now, unix_ms(), || {
            app.state::<Host>().exit.ensure_running()?;
            if !runtime.view().editable || runtime.0.faulted.load(Ordering::Acquire) {
                return Err("快捷键设置当前不可编辑".into());
            }
            if runtime.0.busy.load(Ordering::Acquire) {
                return Err(BUSY.into());
            }
            if app.state::<Host>().capturing.load(Ordering::Acquire) || !foreground(handle) {
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
        (expired, editors.end("settings", &lease_id, now)?)
    };
    emit_ended(&app, expired);
    emit_ended(&app, ended);
    Ok(())
}

pub fn active_display(app: &AppHandle) -> String {
    app.try_state::<HostRuntime>()
        .and_then(|runtime| runtime.view().active.as_ref().map(display))
        .unwrap_or_default()
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
    let now = Instant::now();
    let (ended, recorded) = {
        let mut editors = runtime.0.editors.lock().unwrap_or_else(|e| e.into_inner());
        let mut ended = editors.prune(now);
        if editors.active.as_ref().is_some_and(|lease| {
            !foreground(lease.window)
                || !host.exit.is_running()
                || host.capturing.load(Ordering::Acquire)
        }) {
            ended = ended.or_else(|| editors.invalidate(None, now));
        }
        let recorded = editors.active.as_ref().map(|lease| {
            (
                lease.owner.clone(),
                Recorded {
                    lease_id: lease.public.lease_id.clone(),
                    shortcut: chord.clone(),
                },
            )
        });
        (ended, recorded)
    };
    emit_ended(app, ended);
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
    let Some(runtime) = app.try_state::<HostRuntime>() else {
        return;
    };
    let old = runtime
        .0
        .editors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .invalidate(Some(owner), Instant::now());
    emit_ended(app, old);
}
pub fn invalidate_all(app: &AppHandle) {
    let Some(runtime) = app.try_state::<HostRuntime>() else {
        return;
    };
    let old = runtime
        .0
        .editors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .invalidate(None, Instant::now());
    emit_ended(app, old);
}
pub async fn drain(app: &AppHandle, timeout: Duration) -> Result<(), String> {
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
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    #[test]
    fn early_end_cannot_arm_and_slow_old_end_cannot_clear_new_lease() {
        let now = Instant::now();
        let mut leases = Editors::default();
        let old = id();
        let next = id();
        leases.end("settings", &old, now).unwrap();
        assert!(leases
            .begin("settings", 1, &old, now, 100, || Ok(()))
            .is_err());
        leases
            .begin("settings", 1, &next, now, 100, || Ok(()))
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
            .begin("settings", 1, &first, now, 100, || Ok(()))
            .unwrap();
        assert!(leases
            .begin("settings", 1, &id(), now, 100, || Ok(()))
            .is_err());
        assert!(leases
            .begin("settings", 2, &first, now, 100, || Ok(()))
            .is_err());
        assert_eq!(
            leases
                .begin(
                    "settings",
                    1,
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
            .begin("settings", 1, &first, now + LEASE_TTL, 30_100, || Ok(()))
            .is_err());
    }
    #[test]
    fn owner_admission_and_tombstone_budget_are_bounded() {
        let now = Instant::now();
        let mut leases = Editors::default();
        assert!(leases
            .begin("settings", 1, &id(), now, 100, || Err("hidden".into()))
            .is_err());
        assert!(leases.active.is_none());
        let current = id();
        leases
            .begin("settings", 1, &current, now, 100, || Ok(()))
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
            .begin("settings", 1, &old, now, 100, || Ok(()))
            .unwrap();
        let timer = leases.active.as_ref().unwrap().clone();
        leases.end("settings", &old, now).unwrap();
        let next = id();
        leases
            .begin("settings", 1, &next, now + LEASE_TTL, 30_100, || Ok(()))
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
            .begin("settings", 1, &lease_id, now, 100, || Ok(()))
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
