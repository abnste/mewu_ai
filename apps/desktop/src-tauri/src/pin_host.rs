// SPDX-License-Identifier: MPL-2.0
//! Runtime-only immutable image pins. Scene/plugin authority ends only after
//! the hidden first-party window acknowledges a decoded image and is shown.
use crate::{
    assets,
    import_pin_host::ImportedImageOrigin,
    pin_window,
    plugins::{PluginSnapshot, PluginState},
    Host,
};
use image::{ImageEncoder, RgbaImage};
use mewu_core::{Asset, AssetKind, OcrTarget, Region, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::sync::{oneshot, Notify, OwnedSemaphorePermit, Semaphore};
pub(crate) mod objects;

const MAX_PIXELS: u32 = 32 * 1024 * 1024;
const MAX_PNG: usize = 128 * 1024 * 1024;
const CANCELED: &str = "已取消贴图";

#[derive(Clone, Default)]
struct Cancel {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}
impl Cancel {
    fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }
    fn check(&self) -> Result<(), String> {
        if self.flag.load(Ordering::Acquire) {
            Err(CANCELED.into())
        } else {
            Ok(())
        }
    }
    async fn cancelled(&self) {
        let n = self.notify.notified();
        tokio::pin!(n);
        n.as_mut().enable();
        if self.flag.load(Ordering::Acquire) {
            return;
        }
        n.await;
    }
}

#[derive(Clone)]
struct Origin {
    owner: String,
    space_handle: usize,
    source: OriginSource,
}
#[derive(Clone)]
enum OriginSource {
    Region(RegionOrigin),
    Imported(ImportedImageOrigin),
}
#[derive(Clone)]
struct RegionOrigin {
    target: OcrTarget,
    background: Asset,
    region: Region,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
}
impl Origin {
    fn matches(&self, snapshot: &Snapshot) -> bool {
        match &self.source {
            OriginSource::Region(origin) => origin.matches(snapshot),
            OriginSource::Imported(origin) => origin.matches(snapshot),
        }
    }
    fn plugin_revoked(&self, snapshot: &PluginSnapshot) -> bool {
        match &self.source {
            OriginSource::Imported(_) => false,
            OriginSource::Region(origin) => !snapshot.plugins.iter().any(|p| {
                p.manifest.id == origin.plugin_id
                    && p.revision == origin.revision
                    && p.state == PluginState::Enabled
                    && p.error.is_none()
            }),
        }
    }
}
impl RegionOrigin {
    fn matches(&self, snapshot: &Snapshot) -> bool {
        snapshot.active_scene_id == self.target.scene_id
            && snapshot.scenes.iter().any(|s| {
                s.id == self.target.scene_id
                    && !s.closed
                    && !s.frozen
                    && s.background.as_ref() == Some(&self.background)
                    && s.regions.iter().find(|r| r.id == self.region.id) == Some(&self.region)
            })
    }
}
struct Request {
    origin: Origin,
    cancel: Cancel,
}
struct Recent {
    owner: String,
    at: Instant,
}
pub(crate) struct Entry {
    pub(crate) id: String,
    pub(crate) revision: u64,
    pub(crate) source: PinAssetLease,
    object_assets: HashMap<u8, Asset>,
    pub(crate) quarter_turns: u8,
    pub(crate) topmost: bool,
    pub(crate) opacity: f64,
    pub(crate) epoch: u64,
    pub(crate) original: pin_window::Rect,
    source_viewport: pin_window::Rect,
    pub(crate) live: bool,
    built: bool,
    request_id: String,
    cancel: Cancel,
    ready: Option<oneshot::Sender<Result<(), String>>>,
    _window_slot: Arc<OwnedSemaphorePermit>,
}
#[derive(Default)]
pub(crate) struct RegistryState {
    requests: HashMap<String, Request>,
    recent: HashMap<String, Recent>,
    pub(crate) entries: HashMap<String, Entry>,
    // Revoked labels no longer resolve content, but their native windows can
    // still be awaiting Destroyed. Do not return their WebView quota early.
    closing: HashMap<String, Arc<OwnedSemaphorePermit>>,
    pub(crate) epoch: u64,
}
impl RegistryState {
    fn finish(&mut self, id: &str) -> Vec<String> {
        let live = self.entries.values().any(|e| e.request_id == id && e.live);
        if let Some(r) = self.requests.remove(id) {
            if !live {
                r.cancel.cancel();
            }
            self.recent.insert(
                id.into(),
                Recent {
                    owner: r.origin.owner,
                    at: Instant::now(),
                },
            );
        }
        self.entries
            .iter()
            .filter(|(_, e)| e.request_id == id && !e.live)
            .map(|(l, _)| l.clone())
            .collect()
    }
    fn cancel_where(&self, matches: impl Fn(&Origin) -> bool) -> Vec<String> {
        let ids: Vec<_> = self
            .requests
            .iter()
            .filter(|(id, r)| {
                matches(&r.origin)
                    && !self
                        .entries
                        .values()
                        .any(|e| e.request_id == **id && e.live)
            })
            .map(|(id, r)| {
                r.cancel.cancel();
                id.as_str()
            })
            .collect();
        self.entries
            .iter()
            .filter(|(_, e)| !e.live && ids.contains(&e.request_id.as_str()))
            .map(|(l, _)| l.clone())
            .collect()
    }
}
pub struct PinRegistry {
    pub(crate) state: Mutex<RegistryState>,
    windows: Arc<Semaphore>,
    pixels: Arc<Semaphore>,
    decode: Arc<Semaphore>,
    pub(crate) arranging: AtomicBool,
    work: Arc<Work>,
}
impl Default for PinRegistry {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            windows: Arc::new(Semaphore::new(8)),
            pixels: Arc::new(Semaphore::new(MAX_PIXELS as usize)),
            decode: Arc::new(Semaphore::new(1)),
            arranging: AtomicBool::new(false),
            work: Arc::default(),
        }
    }
}
#[derive(Default)]
struct WorkState {
    active: usize,
    stopping: bool,
}
#[derive(Default)]
struct Work {
    state: Mutex<WorkState>,
    changed: Notify,
}
struct WorkLease(Arc<Work>);
impl Drop for WorkLease {
    fn drop(&mut self) {
        if let Ok(mut s) = self.0.state.lock() {
            s.active -= 1;
        }
        self.0.changed.notify_waiters();
    }
}
impl Work {
    fn begin(self: &Arc<Self>, app: &AppHandle) -> Result<WorkLease, String> {
        let mut s = self.state.lock().map_err(|_| "贴图工作状态不可用")?;
        // A failed application exit returns to Running. Reopen admission only
        // on a new explicit operation, never from a late canceled worker.
        app.state::<Host>().exit.ensure_running()?;
        s.stopping = false;
        s.active += 1;
        Ok(WorkLease(self.clone()))
    }
    fn commit(
        &self,
        app: &AppHandle,
        cancel: &Cancel,
        action: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let s = self.state.lock().map_err(|_| "贴图工作状态不可用")?;
        if s.stopping {
            return Err("应用正在退出".into());
        }
        app.state::<Host>().exit.ensure_running()?;
        cancel.check()?;
        // Drain takes this same short gate before waiting for workers. PNG
        // encoding and file dialogs occur outside it; only the final rename or
        // clipboard publication is inside. This is not a general app I/O lock.
        let result = action();
        drop(s);
        result
    }
}
pub async fn drain_pending(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    cancel_pending(app, "space");
    let Some(r) = app.try_state::<PinRegistry>() else {
        return Ok(());
    };
    let work = r.work.clone();
    tokio::time::timeout(timeout, async {
        loop {
            let changed = work.changed.notified(); tokio::pin!(changed); changed.as_mut().enable();
            match work.state.try_lock() {
                Ok(mut s) => { s.stopping = true; if s.active == 0 { return Ok(()); } }
                Err(std::sync::TryLockError::Poisoned(_)) => return Err("贴图工作状态不可用".into()),
                Err(std::sync::TryLockError::WouldBlock) => (),
            }
            // A synchronous native clipboard publication may currently own the
            // short commit gate. Do not block the async runtime waiting for it.
            tokio::select! { _ = changed => (), _ = tokio::time::sleep(Duration::from_millis(20)) => () }
        }
    }).await.map_err(|_| "贴图仍在处理，退出已取消，请稍后重试")?
}

struct Source {
    asset: Asset,
    root: PathBuf,
    // Hold directory handles as well as the pixel reservation through the last
    // HTTP/export reader. Files are removed before these handles are released.
    _directory: File,
    _root_directory: File,
    _pixels: OwnedSemaphorePermit,
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.asset.path);
    }
}
#[derive(Clone)]
pub struct PinAssetLease(Arc<Source>);
impl PinAssetLease {
    pub fn asset(&self) -> &Asset {
        &self.0.asset
    }
    pub fn root(&self) -> &Path {
        &self.0.root
    }
}
pub fn resolve_asset(app: &AppHandle, id: &str) -> Option<PinAssetLease> {
    let registry = app.try_state::<PinRegistry>()?;
    let state = registry.state.lock().ok()?;
    // Hidden windows need this lease to decode before their ready handshake.
    state
        .entries
        .values()
        .find(|e| e.source.asset().id == id && e.cancel.check().is_ok())
        .map(|e| e.source.clone())
}
pub(crate) fn storage_assets(app: &AppHandle) -> Result<Vec<Asset>, String> {
    let Some(registry) = app.try_state::<PinRegistry>() else { return Ok(Vec::new()); };
    let state = registry.state.lock().map_err(|_| "贴图状态不可用")?;
    // Source pixels live in their own pin-staging directory and hold leases.
    // Only durable orientation copies participate in the assets inventory.
    Ok(state.entries.values().flat_map(|entry| entry.object_assets.values()).cloned().collect())
}
pub fn is_pin_label(label: &str) -> bool {
    label.strip_prefix("pin-").is_some_and(valid_uuid)
}
fn valid_uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id)
}
fn space_owner(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "space" {
        Ok(())
    } else {
        Err("贴图只能由空间创建".into())
    }
}
fn gate(app: &AppHandle) -> Result<(), String> {
    locked_gate(app, space_handle(app)?)
}
fn space_handle(app: &AppHandle) -> Result<usize, String> {
    let window = app.get_webview_window("space").ok_or("空间窗口不可用")?;
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|h| h.0 as usize)
            .map_err(|_| "空间窗口不可用".into())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("当前贴图窗口仅支持 Windows".into())
    }
}
fn locked_gate(app: &AppHandle, space_handle: usize) -> Result<(), String> {
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("空间正在切换，请稍后贴图".into());
    }
    #[cfg(windows)]
    let visible = unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        IsWindow(space_handle as _) != 0 && IsWindowVisible(space_handle as _) != 0
    };
    #[cfg(not(windows))]
    let visible = {
        let _ = space_handle;
        false
    };
    if !visible {
        return Err("空间已收起".into());
    }
    crate::recording::ensure_idle(app)
}
fn check_origin(app: &AppHandle, origin: &Origin, cancel: &Cancel) -> Result<(), String> {
    with_origin(app, origin, cancel, || Ok(()))
}
fn with_origin<T>(
    app: &AppHandle,
    origin: &Origin,
    cancel: &Cancel,
    commit: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    cancel.check()?;
    locked_gate(app, origin.space_handle)?;
    let host = app.state::<Host>();
    match &origin.source {
        OriginSource::Region(source) => {
            // Preserve the original plugin -> Engine -> registry lock order.
            let plugins = host.plugins.lock().map_err(|_| "插件存储不可用")?;
            plugins
                .store
                .pin(&source.plugin_id, source.revision, &source.contribution_id)?;
            let engine = host.lock()?;
            locked_gate(app, origin.space_handle)?;
            cancel.check()?;
            let (background, region) = engine
                .store
                .region_for_pin(&source.target)
                .map_err(|e| e.to_string())?;
            if background != source.background || region != source.region {
                return Err("选区已更改，请重新贴图".into());
            }
            commit()
        }
        OriginSource::Imported(source) => {
            // Import is a user-authorized core action. Do not synthesize a
            // Region or route this already committed item through a plugin.
            let engine = host.lock()?;
            locked_gate(app, origin.space_handle)?;
            cancel.check()?;
            if !source.matches(&engine.store.snapshot()) {
                return Err("导入图片或会话已更改".into());
            }
            commit()
        }
    }
}
#[derive(Serialize)]
pub struct PinCreated {
    id: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinViewState {
    pub id: String,
    pub revision: u64,
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub quarter_turns: u8,
    pub topmost: bool,
    pub opacity: f64,
    pub scale_factor: f64,
    pub shadow_padding: u32,
    pub drag_threshold_x: f64,
    pub drag_threshold_y: f64,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PinAction {
    Close {},
    Restore {},
    Escape {},
    Zoom {
        direction: i8,
    },
    Rotate {
        #[serde(rename = "quarterTurns")]
        quarter_turns: i8,
    },
    SetTopmost {
        enabled: bool,
    },
    SetOpacity {
        opacity: f64,
    },
}

struct CreationGuard {
    app: AppHandle,
    id: String,
    label: Option<String>,
    window_slot: Arc<OwnedSemaphorePermit>,
    building: Option<WorkLease>,
}
impl Drop for CreationGuard {
    fn drop(&mut self) {
        finish_request(&self.app, &self.id);
        if let Some(label) = &self.label {
            let live = self
                .app
                .try_state::<PinRegistry>()
                .and_then(|r| {
                    r.state
                        .lock()
                        .ok()
                        .map(|s| s.entries.get(label).is_some_and(|e| e.live))
                })
                .unwrap_or(false);
            // Cancellation can unregister the entry while WebView construction
            // is in flight. The eventual window still belongs to this guard.
            if !live {
                if self.app.get_webview_window(label).is_some() {
                    if let Some(r) = self.app.try_state::<PinRegistry>() {
                        if let Ok(mut s) = r.state.lock() {
                            s.closing.insert(label.clone(), self.window_slot.clone());
                        }
                    }
                }
                close_entry(&self.app, label, true);
            }
        }
    }
}
fn finish_request(app: &AppHandle, id: &str) {
    let Some(registry) = app.try_state::<PinRegistry>() else {
        return;
    };
    let labels = if let Ok(mut s) = registry.state.lock() {
        s.finish(id)
    } else {
        Vec::new()
    };
    for label in labels {
        close_entry(app, &label, true);
    }
}
pub(crate) fn close_entry(app: &AppHandle, label: &str, force: bool) {
    objects::changed(app);
    // Unregister before asking the event loop to close; never hold this lock
    // across a Window API or a synchronous destruction callback.
    if let Some(registry) = app.try_state::<PinRegistry>() {
        let removed = registry.state.lock().ok().and_then(|mut s| {
            let entry = s.entries.remove(label);
            if let Some(e) = &entry {
                s.closing.insert(label.into(), e._window_slot.clone());
            }
            entry
        });
        if let Some(mut entry) = removed {
            entry.cancel.cancel();
            if let Some(tx) = entry.ready.take() {
                let _ = tx.send(Err(CANCELED.into()));
            }
            drop(entry);
        }
    }
    crate::image_host::cancel_owner(app, label);
    objects::changed(app);
    if let Some(w) = app.get_webview_window(label) {
        let _ = if force { w.destroy() } else { w.close() };
    } else if let Some(r) = app.try_state::<PinRegistry>() {
        // During construction CreationGuard still owns the same Arc permit.
        // After construction a missing runtime window is already destroyed.
        if let Ok(mut s) = r.state.lock() {
            s.closing.remove(label);
        }
    }
}
pub fn destroyed(app: &AppHandle, label: &str) {
    if !is_pin_label(label) {
        return;
    }
    if let Some(registry) = app.try_state::<PinRegistry>() {
        let removed = registry.state.lock().ok().and_then(|mut s| {
            s.closing.remove(label);
            s.entries.remove(label)
        });
        if let Some(mut e) = removed {
            e.cancel.cancel();
            if let Some(tx) = e.ready.take() {
                let _ = tx.send(Err(CANCELED.into()));
            }
        }
    }
    crate::image_host::cancel_owner(app, label);
    objects::changed(app);
}
fn cancel_matching(app: &AppHandle, matches: impl Fn(&Origin) -> bool) {
    let Some(registry) = app.try_state::<PinRegistry>() else {
        return;
    };
    let labels = if let Ok(s) = registry.state.lock() {
        s.cancel_where(matches)
    } else {
        Vec::new()
    };
    for label in labels {
        close_entry(app, &label, true);
    }
}
pub fn cancel_pending(app: &AppHandle, owner: &str) {
    cancel_matching(app, |o| o.owner == owner);
}
pub fn reconcile(app: &AppHandle, snapshot: &Snapshot) {
    cancel_matching(app, |o| !o.matches(snapshot));
}
pub fn revoke_plugins(app: &AppHandle, snapshot: &PluginSnapshot) {
    cancel_matching(app, |o| o.plugin_revoked(snapshot));
}
pub fn shutdown(app: &AppHandle) {
    cancel_pending(app, "space");
    let labels = app
        .try_state::<PinRegistry>()
        .and_then(|r| {
            r.state
                .lock()
                .ok()
                .map(|s| s.entries.keys().cloned().collect::<Vec<_>>())
        })
        .unwrap_or_default();
    for label in labels {
        close_entry(app, &label, true);
    }
}

#[tauri::command]
pub async fn run_plugin_pin(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
    target: OcrTarget,
    translation_id: Option<String>,
) -> Result<PinCreated, String> {
    space_owner(&window)?;
    if !valid_uuid(&request_id) {
        return Err("贴图请求无效".into());
    }
    #[cfg(not(windows))]
    {
        return Err("当前贴图窗口仅支持 Windows".into());
    }
    gate(&app)?;
    let space_handle = space_handle(&app)?;
    let registry = app.state::<PinRegistry>();
    let window_slot = Arc::new(
        registry
            .windows
            .clone()
            .try_acquire_owned()
            .map_err(|_| "最多同时打开 8 张贴图，请先关闭一些贴图")?,
    );
    let (origin, cancel, root, assets_root, pixel_slot) = {
        let host = app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件存储不可用")?;
        plugins.store.pin(&plugin_id, revision, &contribution_id)?;
        let engine = host.lock()?;
        locked_gate(&app, space_handle)?;
        let (background, region) = engine
            .store
            .region_for_pin(&target)
            .map_err(|e| e.to_string())?;
        if region.translation.as_ref().map(|t| &t.overlay.id) != translation_id.as_ref() {
            return Err("译文已更改，请重新贴图".into());
        }
        let (width, height) = source_dimensions(&region)?;
        let pixel_slot = registry
            .pixels
            .clone()
            .try_acquire_many_owned(width * height)
            .map_err(|_| "贴图总像素已达上限，请先关闭一些贴图")?;
        let origin = Origin {
            owner: "space".into(),
            space_handle,
            source: OriginSource::Region(RegionOrigin {
                target,
                background,
                region,
                plugin_id,
                revision,
                contribution_id,
            }),
        };
        let cancel = Cancel::default();
        let mut s = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        register_request(&mut s, &request_id, &origin, &cancel)?;
        (
            origin,
            cancel,
            host.root.clone(),
            host.assets.clone(),
            pixel_slot,
        )
    };
    let mut guard = CreationGuard {
        app: app.clone(),
        id: request_id.clone(),
        label: None,
        window_slot: window_slot.clone(),
        building: None,
    };
    let decode = tokio::select! { biased; _ = cancel.cancelled() => return Err(CANCELED.into()), slot = registry.decode.clone().acquire_owned() => slot.map_err(|_| "贴图处理已结束")? };
    cancel.check()?;
    let work = registry.work.begin(&app)?;
    let OriginSource::Region(region_origin) = &origin.source else {
        unreachable!()
    };
    let work_origin = region_origin.clone();
    let work_cancel = cancel.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        let _decode = decode;
        let _work = work;
        work_cancel.check()?;
        let image =
            assets::crop_from_parts(&work_origin.background, &work_origin.region, &assets_root)?
                .into_rgba8();
        if image.dimensions() != source_dimensions(&work_origin.region)? {
            return Err("选区像素尺寸已变更".into());
        }
        work_cancel.check()?;
        let source = stage(&root, &image, pixel_slot)?;
        work_cancel.check()?;
        Ok::<_, String>(source)
    });
    let source = tokio::select! { biased; _ = cancel.cancelled() => return Err(CANCELED.into()), result = worker => result.map_err(|_| "贴图处理已中断")?? };
    check_origin(&app, &origin, &cancel)?;
    let source_viewport = pin_window::current_space_rect(&app).await?;
    cancel.check()?;
    let original = pin_window::initial_rect(
        &region_origin.background,
        &region_origin.region,
        source.asset(),
        source_viewport,
    )?;
    show_source(
        &app,
        &request_id,
        &cancel,
        &mut guard,
        window_slot,
        source,
        source_viewport,
        original,
    )
    .await
}

fn register_request(
    state: &mut RegistryState,
    request_id: &str,
    origin: &Origin,
    cancel: &Cancel,
) -> Result<(), String> {
    state
        .recent
        .retain(|_, r| r.at.elapsed() < Duration::from_secs(300));
    if state.requests.contains_key(request_id) || state.recent.contains_key(request_id) {
        return Err("贴图请求已结束，请重试".into());
    }
    if state.requests.len() + state.recent.len() >= 512 {
        return Err("贴图请求过多，请稍后重试".into());
    }
    state.requests.insert(
        request_id.into(),
        Request {
            origin: origin.clone(),
            cancel: cancel.clone(),
        },
    );
    Ok(())
}

pub(crate) async fn pin_imported_image(
    app: AppHandle,
    imported: ImportedImageOrigin,
) -> Result<String, String> {
    gate(&app)?;
    let space_handle = space_handle(&app)?;
    let registry = app.state::<PinRegistry>();
    let window_slot = Arc::new(
        registry
            .windows
            .clone()
            .try_acquire_owned()
            .map_err(|_| "最多同时打开 8 张贴图，请先关闭一些贴图")?,
    );
    let request_id = uuid::Uuid::new_v4().to_string();
    let origin = Origin {
        owner: "space".into(),
        space_handle,
        source: OriginSource::Imported(imported.clone()),
    };
    let cancel = Cancel::default();
    let (root, assets_root, pixel_slot) = {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        locked_gate(&app, space_handle)?;
        if !imported.matches(&engine.store.snapshot()) {
            return Err("导入图片或会话已更改".into());
        }
        let (width, height) = image_dimensions(&imported.item.asset)?;
        let pixel_slot = registry
            .pixels
            .clone()
            .try_acquire_many_owned(width * height)
            .map_err(|_| "贴图总像素已达上限，请先关闭一些贴图")?;
        let mut state = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        register_request(&mut state, &request_id, &origin, &cancel)?;
        (host.root.clone(), host.assets.clone(), pixel_slot)
    };
    let mut guard = CreationGuard {
        app: app.clone(),
        id: request_id.clone(),
        label: None,
        window_slot: window_slot.clone(),
        building: None,
    };
    let decode = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(CANCELED.into()),
        slot = registry.decode.clone().acquire_owned() => slot.map_err(|_| "贴图处理已结束")?,
    };
    cancel.check()?;
    let work = registry.work.begin(&app)?;
    let work_cancel = cancel.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        let _decode = decode;
        let _work = work;
        work_cancel.check()?;
        let mut lease = crate::image_host::SourceLease::open(&imported.item.asset, &assets_root)?;
        let image = lease.decode_rgba(&imported.item.asset)?;
        work_cancel.check()?;
        let source = stage(&root, &image, pixel_slot)?;
        work_cancel.check()?;
        Ok::<_, String>(source)
    });
    let source = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(CANCELED.into()),
        result = worker => result.map_err(|_| "贴图处理已中断")??,
    };
    check_origin(&app, &origin, &cancel)?;
    let source_viewport = pin_window::current_space_rect(&app).await?;
    cancel.check()?;
    let original = crate::import_pin_host::original_rect(source.asset(), source_viewport)?;
    let created = show_source(
        &app,
        &request_id,
        &cancel,
        &mut guard,
        window_slot,
        source,
        source_viewport,
        original,
    )
    .await?;
    if let OriginSource::Imported(imported) = &origin.source {
        if let Ok(mut state) = registry.state.lock() {
            if let Some(entry) = state.entries.get_mut(&format!("pin-{}", created.id)) {
                entry.object_assets.insert(0, imported.item.asset.clone());
            }
        }
        objects::changed(&app);
    }
    Ok(created.id)
}

async fn show_source(
    app: &AppHandle,
    request_id: &str,
    cancel: &Cancel,
    guard: &mut CreationGuard,
    window_slot: Arc<OwnedSemaphorePermit>,
    source: PinAssetLease,
    source_viewport: pin_window::Rect,
    original: pin_window::Rect,
) -> Result<PinCreated, String> {
    let registry = app.state::<PinRegistry>();
    let id = uuid::Uuid::new_v4().to_string();
    let label = format!("pin-{id}");
    let (tx, rx) = oneshot::channel();
    {
        let mut s = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        cancel.check()?;
        let epoch = s.epoch;
        s.entries.insert(
            label.clone(),
            Entry {
                id: id.clone(),
                revision: 1,
                source,
                object_assets: HashMap::new(),
                quarter_turns: 0,
                topmost: true,
                opacity: 1.,
                epoch,
                original,
                source_viewport,
                live: false,
                built: false,
                request_id: request_id.into(),
                cancel: cancel.clone(),
                ready: Some(tx),
                _window_slot: window_slot,
            },
        );
    }
    // build is called from this async command, never while holding application
    // locks or synchronously inside a Win32/WebView event callback.
    guard.label = Some(label.clone());
    cancel.check()?;
    guard.building = Some(registry.work.begin(app)?);
    pin_window::build(app, &label, original).await?;
    guard.building = None;
    {
        let mut s = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        let e = s.entries.get_mut(&label).ok_or(CANCELED)?;
        e.built = true;
    }
    cancel.check()?;
    let result = tokio::select! { biased; _ = cancel.cancelled() => Err(CANCELED.into()), result = tokio::time::timeout(Duration::from_secs(15), rx) => match result { Ok(Ok(result)) => result, Ok(Err(_)) => Err("贴图窗口已关闭".into()), Err(_) => Err("贴图窗口加载超时，请重试".into()) } };
    result?;
    Ok(PinCreated { id })
}

#[tauri::command]
pub fn cancel_plugin_pin(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    space_owner(&window)?;
    if !valid_uuid(&request_id) {
        return Err("贴图请求无效".into());
    }
    let registry = app.state::<PinRegistry>();
    let mut s = registry.state.lock().map_err(|_| "贴图状态不可用")?;
    s.recent
        .retain(|_, r| r.at.elapsed() < Duration::from_secs(300));
    if let Some(r) = s.requests.get(&request_id) {
        if r.origin.owner != window.label() {
            return Err("贴图请求不属于此窗口".into());
        }
        // A completed pin is independent even if the original invoke response
        // has not reached the space yet.
        if !s
            .entries
            .values()
            .any(|e| e.request_id == request_id && e.live)
        {
            r.cancel.cancel();
        }
    } else if let Some(r) = s.recent.get(&request_id) {
        if r.owner != window.label() {
            return Err("贴图请求不属于此窗口".into());
        }
    } else {
        if s.recent.len() + s.requests.len() >= 512 {
            return Err("贴图请求过多".into());
        }
        s.recent.insert(
            request_id.clone(),
            Recent {
                owner: window.label().into(),
                at: Instant::now(),
            },
        );
    }
    let labels = s
        .entries
        .iter()
        .filter(|(_, e)| e.request_id == request_id && !e.live)
        .map(|(l, _)| l.clone())
        .collect::<Vec<_>>();
    drop(s);
    for label in labels {
        close_entry(&app, &label, true);
    }
    Ok(())
}

#[tauri::command]
pub fn get_pin_state(app: AppHandle, window: WebviewWindow) -> Result<PinViewState, String> {
    view_state(&app, &window)
}
pub(crate) fn view_state(app: &AppHandle, window: &WebviewWindow) -> Result<PinViewState, String> {
    if !is_pin_label(window.label()) {
        return Err("此窗口不是贴图".into());
    }
    let scale = window.scale_factor().map_err(|_| "无法读取贴图比例")?;
    let (dx, dy) = pin_window::drag_threshold(scale);
    let registry = app.state::<PinRegistry>();
    let s = registry.state.lock().map_err(|_| "贴图状态不可用")?;
    let e = s.entries.get(window.label()).ok_or("贴图已关闭")?;
    e.cancel.check()?;
    Ok(PinViewState {
        id: e.id.clone(),
        revision: e.revision,
        image_url: format!(
            "{}/{}",
            app.state::<Host>().content_origin,
            e.source.asset().id
        ),
        width: e.source.asset().width.unwrap(),
        height: e.source.asset().height.unwrap(),
        quarter_turns: e.quarter_turns,
        topmost: e.topmost,
        opacity: e.opacity,
        scale_factor: scale,
        shadow_padding: pin_window::PADDING,
        drag_threshold_x: dx,
        drag_threshold_y: dy,
    })
}
pub(crate) fn emit_state(app: &AppHandle, window: &WebviewWindow) {
    objects::changed(app);
    if let Ok(state) = view_state(app, window) {
        let _ = window.emit("pin-state", state);
    }
}
#[tauri::command]
pub async fn pin_ready(app: AppHandle, window: WebviewWindow, success: bool) -> Result<(), String> {
    if !is_pin_label(window.label()) {
        return Err("此窗口不是贴图".into());
    }
    if !success {
        {
            let r = app.state::<PinRegistry>();
            if let Ok(mut s) = r.state.lock() {
                if let Some(e) = s.entries.get_mut(window.label()) {
                    if let Some(tx) = e.ready.take() {
                        let _ = tx.send(Err("贴图图片加载失败".into()));
                    }
                }
            };
        }
        close_entry(&app, window.label(), true);
        return Err("贴图图片加载失败".into());
    }
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let built = {
                let r = app.state::<PinRegistry>();
                let s = r.state.lock().map_err(|_| "贴图状态不可用")?;
                let e = s.entries.get(window.label()).ok_or(CANCELED)?;
                e.cancel.check()?;
                e.built
            };
            if built {
                return Ok::<(), String>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "贴图窗口初始化超时")??;
    let (origin, cancel, source_viewport, original) = {
        let r = app.state::<PinRegistry>();
        let s = r.state.lock().map_err(|_| "贴图状态不可用")?;
        let e = s.entries.get(window.label()).ok_or("贴图已关闭")?;
        if e.live {
            return Ok(());
        }
        let request = s.requests.get(&e.request_id).ok_or(CANCELED)?;
        (
            request.origin.clone(),
            e.cancel.clone(),
            e.source_viewport,
            e.original,
        )
    };
    let result = async {
        check_origin(&app, &origin, &cancel)?;
        pin_window::show_without_activation(
            &window,
            cancel.flag.clone(),
            source_viewport,
            original,
        )
        .await?;
        // A source or permission change during the event-loop handshake still
        // destroys this new window; it has not become an independent pin yet.
        with_origin(&app, &origin, &cancel, || {
            let r = app.state::<PinRegistry>();
            let mut s = r.state.lock().map_err(|_| "贴图状态不可用")?;
            cancel.check()?;
            let e = s.entries.get_mut(window.label()).ok_or(CANCELED)?;
            e.live = true;
            if let Some(tx) = e.ready.take() {
                let _ = tx.send(Ok(()));
            }
            Ok(())
        })
    }
    .await;
    if result.is_err() {
        close_entry(&app, window.label(), true);
    } else {
        pin_window::arrange_for_space(&app);
        objects::changed(&app);
    }
    result
}

#[tauri::command]
pub async fn control_pin(
    app: AppHandle,
    window: WebviewWindow,
    action: PinAction,
) -> Result<(), String> {
    pin_window::control(&app, &window, action).await
}
#[tauri::command]
pub fn show_pin_menu(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    pin_window::menu(&app, &window)
}
#[tauri::command]
pub async fn start_pin_drag(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    view_state(&app, &window)?;
    app.state::<Host>().exit.ensure_running()?;
    pin_window::start_drag(&window).await
}

#[tauri::command]
pub async fn export_pin(
    app: AppHandle,
    window: WebviewWindow,
    clipboard: bool,
) -> Result<(), String> {
    export(&app, &window, clipboard).await
}
pub(crate) async fn export(
    app: &AppHandle,
    window: &WebviewWindow,
    clipboard: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    return export_windows(app, window, clipboard).await;
    #[cfg(not(windows))]
    return export_legacy(app, window, clipboard).await;
}

#[cfg(not(windows))]
async fn export_legacy(
    app: &AppHandle,
    window: &WebviewWindow,
    clipboard: bool,
) -> Result<(), String> {
    app.state::<Host>().exit.ensure_running()?;
    let (source, turns, cancel) = {
        let r = app.state::<PinRegistry>();
        let s = r.state.lock().map_err(|_| "贴图状态不可用")?;
        let e = s
            .entries
            .get(window.label())
            .filter(|e| e.live)
            .ok_or("贴图已关闭")?;
        (e.source.clone(), e.quarter_turns, e.cancel.clone())
    };
    let destination = if clipboard {
        None
    } else {
        let selected = rfd::AsyncFileDialog::new()
            .set_parent(window)
            .set_title("保存贴图")
            .add_filter("PNG 图片", &["png"])
            .set_file_name("Mewu-贴图.png")
            .save_file()
            .await;
        let Some(file) = selected else {
            return Ok(());
        };
        Some(file.path().to_owned())
    };
    cancel.check()?;
    app.state::<Host>().exit.ensure_running()?;
    let registry = app.state::<PinRegistry>();
    let permit = tokio::select! { biased; _ = cancel.cancelled() => return Err(CANCELED.into()), p = registry.decode.clone().acquire_owned() => p.map_err(|_| "贴图处理已结束")? };
    cancel.check()?;
    let work_lease = registry.work.begin(app)?;
    let work = registry.work.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let _work_lease = work_lease;
        cancel.check()?;
        let original = assets::image(source.asset(), source.root())?.into_rgba8();
        let image = rotate(original, turns);
        cancel.check()?;
        if let Some(path) = destination {
            export_destination(&path, source.root())?;
            save_png(&image, &path, |temporary, destination| {
                work.commit(&handle, &cancel, || {
                    std::fs::rename(temporary, destination)
                        .map_err(|_| "无法完成贴图保存，原文件未被覆盖".into())
                })
            })
        } else {
            work.commit(&handle, &cancel, || {
                let mut board = arboard::Clipboard::new().map_err(|_| "无法打开剪贴板")?;
                board
                    .set_image(arboard::ImageData {
                        width: image.width() as usize,
                        height: image.height() as usize,
                        bytes: std::borrow::Cow::Owned(image.into_raw()),
                    })
                    .map_err(|_| "无法复制贴图".into())
            })
        }
    })
    .await
    .map_err(|_| "贴图导出中断")?
}
fn rotate(image: RgbaImage, turns: u8) -> RgbaImage {
    match turns % 4 {
        1 => image::imageops::rotate90(&image),
        2 => image::imageops::rotate180(&image),
        3 => image::imageops::rotate270(&image),
        _ => image,
    }
}
#[cfg(windows)]
async fn export_windows(
    app: &AppHandle,
    window: &WebviewWindow,
    clipboard: bool,
) -> Result<(), String> {
    use crate::image_save_dialog::{ImageSaveName, SaveDialogOutcome};
    let handle = crate::image_host::window_handle(window)?;
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    let (source, turns, cancel) = {
        let registry = app.state::<PinRegistry>();
        let state = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        let entry = state
            .entries
            .get(window.label())
            .filter(|entry| entry.live)
            .ok_or("贴图已关闭")?;
        (
            entry.source.clone(),
            entry.quarter_turns,
            entry.cancel.clone(),
        )
    };
    let (image_work, export_cancel) =
        host.image_exports
            .begin_pin(window.label().to_owned(), || {
                host.exit.ensure_running()?;
                cancel.check()?;
                crate::image_host::ensure_visible(handle)
            })?;
    let _invoke = image_work.cancel_on_drop();
    let registry = app.state::<PinRegistry>();
    // Include the native picker in actual pin work. An aborted application
    // exit must preserve the pin itself while canceling just this operation.
    let work_lease = registry.work.begin(app)?;
    let permit = tokio::select! {
        biased;
        _ = cancel.cancelled() => return image_work.settle_result(Err(CANCELED.into())),
        _ = image_work.cancelled() => return image_work.settle_result(Err(crate::image_host::CANCELED.into())),
        permit = registry.decode.clone().acquire_owned() => permit.map_err(|_| "贴图处理已结束")?,
    };
    let work = registry.work.clone();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (_permit, _work_lease) = (permit, work_lease);
        let result = (|| {
            cancel.check()?;
            crate::image_host::check_cancel(&export_cancel)?;
            app.state::<Host>().exit.ensure_running()?;
            let destination = if clipboard {
                None
            } else {
                let picker_app = app.clone();
                let picker_entry_cancel = cancel.clone();
                let picker_operation_cancel = export_cancel.clone();
                let default_format = crate::image_save_dialog::preference_format(&app)?;
                let selected = crate::image_save_dialog::pick_image_destination_with_default(
                    handle as isize,
                    ImageSaveName::PinnedImage,
                    default_format,
                    export_cancel.clone(),
                    move || {
                        picker_app.state::<Host>().exit.ensure_running()?;
                        picker_entry_cancel.check()?;
                        crate::image_host::check_cancel(&picker_operation_cancel)?;
                        crate::image_host::ensure_visible(handle)
                    },
                )
                .map_err(|error| error.to_string())?;
                match selected {
                    SaveDialogOutcome::Selected { path, format } => Some(
                        crate::image_export::ImageDestination::prepare(&path, format)?,
                    ),
                    SaveDialogOutcome::UserCanceled | SaveDialogOutcome::RequestCanceled => {
                        return Ok(())
                    }
                }
            };
            cancel.check()?;
            crate::image_host::check_cancel(&export_cancel)?;
            let source_lease = crate::image_host::SourceLease::open(source.asset(), source.root())?;
            let sources = [source_lease];
            let managed = app.state::<Host>().root.clone();
            if let Some(destination) = &destination {
                crate::image_host::protect_destination(destination, &managed, &sources)?;
            }
            let original = assets::image(source.asset(), source.root())?.into_rgba8();
            let pixels = rotate(original, turns);
            cancel.check()?;
            crate::image_host::check_cancel(&export_cancel)?;
            if let Some(destination) = &destination {
                crate::image_export::save_image_atomic(
                    destination.format(),
                    &pixels,
                    destination,
                    &export_cancel,
                    |temporary, target| {
                        // Filesystem checks never hold the native work/registry locks.
                        crate::image_host::protect_destination(destination, &managed, &sources)?;
                        work.commit(&app, &cancel, || {
                            image_work.accept_publication(|| {
                                app.state::<Host>().exit.ensure_running()?;
                                cancel.check()?;
                                crate::image_host::ensure_visible(handle)
                            })?;
                            std::fs::rename(temporary, target)
                                .map_err(|_| "无法完成贴图保存，原文件未被覆盖".into())
                        })
                    },
                )
            } else {
                work.commit(&app, &cancel, || {
                    image_work.accept_publication(|| {
                        app.state::<Host>().exit.ensure_running()?;
                        cancel.check()?;
                        crate::image_host::ensure_visible(handle)
                    })?;
                    let mut board = arboard::Clipboard::new().map_err(|_| "无法打开剪贴板")?;
                    board
                        .set_image(arboard::ImageData {
                            width: pixels.width() as usize,
                            height: pixels.height() as usize,
                            bytes: std::borrow::Cow::Owned(pixels.into_raw()),
                        })
                        .map_err(|_| "无法复制贴图".into())
                })
            }
        })();
        // Source PinAssetLease and both real-work leases survive the whole
        // blocking operation, even if the invoking future disappears.
        image_work.settle_result(result)
    })
    .await
    .map_err(|_| "贴图导出中断")?
}

#[cfg(any(test, not(windows)))]
fn export_destination(path: &Path, protected: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("保存目录无效")?
        .canonicalize()
        .map_err(|_| "保存目录不可用")?;
    let protected = protected.canonicalize().map_err(|_| "贴图目录不可用")?;
    if parent.starts_with(&protected) {
        return Err("请选择贴图临时目录以外的保存位置".into());
    }
    Ok(())
}
fn source_dimensions(region: &Region) -> Result<(u32, u32), String> {
    let (w, h) = if let Some(asset) = &region.image_override {
        (
            asset.width.ok_or("替换图尺寸无效")?,
            asset.height.ok_or("替换图尺寸无效")?,
        )
    } else {
        (region.width.round() as u32, region.height.round() as u32)
    };
    if w == 0
        || h == 0
        || w > 16384
        || h > 16384
        || u64::from(w) * u64::from(h) > u64::from(MAX_PIXELS)
    {
        return Err("贴图超过 32 Mi 像素或 16384 像素边长，请缩小选区".into());
    }
    Ok((w, h))
}
fn image_dimensions(asset: &Asset) -> Result<(u32, u32), String> {
    if asset.kind != AssetKind::Image {
        return Err("图片来源无效".into());
    }
    let (width, height) = (
        asset.width.ok_or("图片宽度无效")?,
        asset.height.ok_or("图片高度无效")?,
    );
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > u64::from(MAX_PIXELS)
    {
        return Err("图片超过贴图像素限制".into());
    }
    Ok((width, height))
}
fn directory(path: &Path) -> Result<File, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "无法检查贴图目录")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("贴图目录不能是重解析点".into());
        }
    }
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("贴图目录无效".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(3)
            .custom_flags(0x0200_0000 | 0x0020_0000);
    }
    options.open(path).map_err(|_| "无法锁定贴图目录".into())
}
struct Temp(Option<PathBuf>);
impl Drop for Temp {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}
struct Limited<'a> {
    file: &'a mut File,
    remaining: usize,
}
impl Write for Limited<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other("PNG exceeds limit"));
        }
        let n = self.file.write(bytes)?;
        self.remaining -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
fn encode(image: &RgbaImage, file: &mut File) -> Result<(), String> {
    image::codecs::png::PngEncoder::new(Limited {
        file,
        remaining: MAX_PNG,
    })
    .write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|_| "贴图 PNG 编码失败或超过 128 MiB".into())
}
fn stage(
    root: &Path,
    image: &RgbaImage,
    pixels: OwnedSemaphorePermit,
) -> Result<PinAssetLease, String> {
    let root_lock = directory(root)?;
    let root = root.canonicalize().map_err(|_| "贴图目录不可用")?;
    let staging = root.join("pin-staging");
    match std::fs::create_dir(&staging) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("无法创建贴图目录".into()),
    }
    let staging_lock = directory(&staging)?;
    let staging = staging.canonicalize().map_err(|_| "贴图目录不可用")?;
    if staging.parent() != Some(root.as_path()) {
        return Err("贴图目录超出范围".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let path = staging.join(format!("{id}.png"));
    let mut temp = Temp(Some(path.clone()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|_| "无法创建贴图")?;
    encode(image, &mut file)?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法保存完整贴图")?;
    drop(file);
    let source = Source {
        asset: Asset {
            id,
            name: "贴图.png".into(),
            kind: AssetKind::Image,
            path: path.to_string_lossy().into_owned(),
            width: Some(image.width()),
            height: Some(image.height()),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        },
        root: staging,
        _directory: staging_lock,
        _root_directory: root_lock,
        _pixels: pixels,
    };
    temp.0 = None;
    Ok(PinAssetLease(Arc::new(source)))
}
#[cfg(any(test, not(windows)))]
fn save_png(
    image: &RgbaImage,
    path: &Path,
    commit: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("保存路径无效".into());
    }
    let parent = path
        .parent()
        .ok_or("保存目录无效")?
        .canonicalize()
        .map_err(|_| "保存目录不可用")?;
    let destination = parent.join(path.file_name().ok_or("保存文件名无效")?);
    let temporary = parent.join(format!(".mewu-pin-{}.tmp", uuid::Uuid::new_v4()));
    let temp = Temp(Some(temporary.clone()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| "无法创建贴图临时文件")?;
    encode(image, &mut file)?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法保存完整贴图")?;
    drop(file);
    commit(&temporary, &destination)?;
    drop(temp);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("mewu-pin-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn asset() -> Asset {
        Asset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "synthetic.png".into(),
            kind: AssetKind::Image,
            path: "synthetic.png".into(),
            width: Some(800),
            height: Some(600),
            origin_x: Some(-800),
            origin_y: Some(0),
            scale_factor: Some(1.5),
        }
    }
    fn region_origin() -> RegionOrigin {
        let background = asset();
        let region = Region {
            id: uuid::Uuid::new_v4().to_string(),
            x: 10.,
            y: 20.,
            width: 100.,
            height: 50.,
            ..Default::default()
        };
        RegionOrigin {
            target: OcrTarget {
                scene_id: "scene".into(),
                region_id: region.id.clone(),
                background_id: background.id.clone(),
                drawing_revision: 0,
                x: region.x,
                y: region.y,
                width: region.width,
                height: region.height,
            },
            background,
            region,
            plugin_id: "mewu.pin".into(),
            revision: 1,
            contribution_id: "pin".into(),
        }
    }
    fn origin() -> Origin {
        Origin {
            owner: "space".into(),
            space_handle: 0,
            source: OriginSource::Region(region_origin()),
        }
    }
    #[test]
    fn source_fence_includes_same_revision_translation_and_full_override() {
        let store = mewu_core::Store::open_in_memory().unwrap();
        let mut snapshot = store.snapshot();
        let mut o = region_origin();
        o.target.scene_id = snapshot.active_scene_id.clone();
        snapshot.scenes[0].background = Some(o.background.clone());
        snapshot.scenes[0].regions = vec![o.region.clone()];
        assert!(o.matches(&snapshot));
        snapshot.scenes[0].draft = "typing does not change source".into();
        assert!(o.matches(&snapshot));
        for case in 0..6 {
            let mut changed = snapshot.clone();
            let scene = &mut changed.scenes[0];
            match case {
                0 => scene.frozen = true,
                1 => scene.closed = true,
                2 => scene.background.as_mut().unwrap().path.push('x'),
                3 => scene.regions[0].image_override = Some(asset()),
                4 => {
                    scene.regions[0].translation = Some(mewu_core::SavedTranslation {
                        background_id: o.background.id.clone(),
                        drawing_revision: 0,
                        source_id: o.background.id.clone(),
                        document: mewu_core::TranslationDocument {
                            version: 1,
                            target_language: "en".into(),
                            width: 100,
                            height: 50,
                            text_angle: None,
                            lines: vec![],
                        },
                        overlay: asset(),
                        selection: mewu_core::OcrDocument {
                            engine: "mewu.translation.layout.v1".into(),
                            language: "en".into(),
                            width: 100,
                            height: 50,
                            text_angle: None,
                            lines: vec![],
                        },
                    })
                }
                _ => changed.active_scene_id = "other".into(),
            }
            assert!(!o.matches(&changed), "case {case}");
        }
    }
    #[test]
    fn lease_keeps_file_and_pixel_budget_until_last_reader() {
        let root = TestRoot::new();
        let pixels = Arc::new(Semaphore::new(4));
        let slot = pixels.clone().try_acquire_many_owned(4).unwrap();
        let source = stage(
            &root.0,
            &RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 255])),
            slot,
        )
        .unwrap();
        let path = PathBuf::from(&source.asset().path);
        let reader = source.clone();
        drop(source);
        assert!(path.exists());
        assert_eq!(pixels.available_permits(), 0);
        assert!(pixels.clone().try_acquire_owned().is_err());
        let decoded = assets::image(reader.asset(), reader.root())
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [10, 20, 30, 255]);
        drop(reader);
        assert!(!path.exists());
        assert_eq!(pixels.available_permits(), 4);
        // The unrelated source directory and any unknown user file survive.
        std::fs::write(root.0.join("keep.txt"), "keep").unwrap();
        assert!(root.0.join("keep.txt").exists());
    }
    #[test]
    fn imported_pin_uses_leased_pixels_and_owns_an_independent_staged_copy() {
        let root = TestRoot::new();
        let path = root.0.join("synthetic.png");
        let expected = RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 128]));
        expected.save(&path).unwrap();
        let mut asset = asset();
        asset.path = path.to_str().unwrap().into();
        asset.width = Some(2);
        asset.height = Some(2);
        let mut lease = crate::image_host::SourceLease::open(&asset, &root.0).unwrap();
        let decoded = lease.decode_rgba(&asset).unwrap();
        assert_eq!(decoded, expected);
        let mut wrong = asset.clone();
        wrong.width = Some(3);
        assert!(lease.decode_rgba(&wrong).is_err());
        #[cfg(windows)]
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        let pixels = Arc::new(Semaphore::new(4));
        let source = stage(
            &root.0,
            &decoded,
            pixels.clone().try_acquire_many_owned(4).unwrap(),
        )
        .unwrap();
        let copied_path = PathBuf::from(&source.asset().path);
        assert_ne!(copied_path, path);
        drop(lease);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(image::open(&copied_path).unwrap().into_rgba8(), expected);
        assert_eq!(pixels.available_permits(), 0);
        drop(source);
        assert!(!copied_path.exists());
        assert_eq!(pixels.available_permits(), 4);
    }
    #[test]
    fn imported_pin_has_core_authority_and_pending_source_revocation() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let image = asset();
        let snapshot = store.import_assets(&scene_id, vec![image.clone()]).unwrap();
        let imported = ImportedImageOrigin::capture(&snapshot, &scene_id, &image.id).unwrap();
        let origin = Origin {
            owner: "space".into(),
            space_handle: 0,
            source: OriginSource::Imported(imported),
        };
        assert!(origin.matches(&snapshot));
        let no_plugins = PluginSnapshot {
            revision: 9,
            plugins: vec![],
        };
        assert!(!origin.plugin_revoked(&no_plugins));
        assert!(self::origin().plugin_revoked(&no_plugins));
        let mut state = RegistryState::default();
        let cancel = Cancel::default();
        register_request(&mut state, "import", &origin, &cancel).unwrap();
        assert!(register_request(&mut state, "import", &origin, &cancel).is_err());
        state.cancel_where(|origin| origin.plugin_revoked(&no_plugins));
        assert!(cancel.check().is_ok());
        let mut replaced = snapshot.clone();
        replaced.scenes[0].items.clear();
        state.cancel_where(|origin| !origin.matches(&replaced));
        assert!(cancel.check().is_err());
        // The same RegistryState live exclusion used by screenshot pins is
        // also used by imports; no second window ownership path exists.
    }
    #[test]
    fn completed_pin_survives_pending_source_and_plugin_cancellation() {
        let root = TestRoot::new();
        let registry = PinRegistry::default();
        let pixels = registry.pixels.clone().try_acquire_many_owned(4).unwrap();
        let source = stage(&root.0, &RgbaImage::new(2, 2), pixels).unwrap();
        let token = Cancel::default();
        let id = uuid::Uuid::new_v4().to_string();
        let label = format!("pin-{id}");
        let mut s = registry.state.lock().unwrap();
        s.requests.insert(
            id.clone(),
            Request {
                origin: origin(),
                cancel: token.clone(),
            },
        );
        s.entries.insert(
            label.clone(),
            Entry {
                id: id.clone(),
                revision: 1,
                source,
                object_assets: HashMap::new(),
                quarter_turns: 0,
                topmost: true,
                opacity: 1.,
                epoch: 0,
                original: pin_window::Rect {
                    x: 0,
                    y: 0,
                    width: 26,
                    height: 26,
                },
                source_viewport: pin_window::Rect {
                    x: 0,
                    y: 0,
                    width: 26,
                    height: 26,
                },
                live: true,
                built: true,
                request_id: id.clone(),
                cancel: token.clone(),
                ready: None,
                _window_slot: Arc::new(registry.windows.clone().try_acquire_owned().unwrap()),
            },
        );
        assert!(s.cancel_where(|_| true).is_empty());
        assert!(token.check().is_ok());
        assert!(s.finish(&id).is_empty());
        assert!(token.check().is_ok());
        assert!(s.entries.contains_key(&label));
        let when = s.recent[&id].at;
        s.finish(&id);
        assert_eq!(s.recent[&id].at, when);
        let pending = uuid::Uuid::new_v4().to_string();
        let pending_token = Cancel::default();
        s.requests.insert(
            pending.clone(),
            Request {
                origin: origin(),
                cancel: pending_token.clone(),
            },
        );
        s.cancel_where(|o| o.owner == "unrelated");
        assert!(pending_token.check().is_ok());
        s.cancel_where(|_| true);
        assert!(pending_token.check().is_err());
        assert!(token.check().is_ok());
    }
    #[test]
    fn pixel_window_and_decode_capacity_are_independent_and_bounded() {
        let r = PinRegistry::default();
        let windows: Vec<_> = (0..8)
            .map(|_| r.windows.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(r.windows.clone().try_acquire_owned().is_err());
        drop(windows);
        let permit = Arc::new(r.windows.clone().try_acquire_owned().unwrap());
        let constructing = permit.clone();
        let mut closing = HashMap::new();
        closing.insert("window".to_string(), permit);
        assert_eq!(r.windows.available_permits(), 7);
        // Cancellation removes the registered entry, but the builder and then
        // Destroyed each retain the same one permit, never double-reserving it.
        drop(constructing);
        assert_eq!(r.windows.available_permits(), 7);
        closing.remove("window");
        assert_eq!(r.windows.available_permits(), 8);
        let pixels = r.pixels.clone().try_acquire_many_owned(MAX_PIXELS).unwrap();
        assert!(r.pixels.clone().try_acquire_owned().is_err());
        assert!(r.windows.clone().try_acquire_owned().is_ok());
        drop(pixels);
        let decode = r.decode.clone().try_acquire_owned().unwrap();
        assert!(r.decode.clone().try_acquire_owned().is_err());
        drop(decode);
        let mut region = region_origin().region;
        region.width = 16385.;
        assert!(source_dimensions(&region).is_err());
        region.width = 8192.;
        region.height = 8192.;
        assert!(source_dimensions(&region).is_err());
    }
    #[test]
    fn rotated_export_is_lossless_and_failed_atomic_commit_preserves_destination() {
        let root = TestRoot::new();
        let destination = root.0.join("saved.png");
        let protected = root.0.join("pin-staging");
        std::fs::create_dir(&protected).unwrap();
        assert!(export_destination(&protected.join("existing.png"), &protected).is_err());
        assert!(export_destination(&destination, &protected).is_ok());
        std::fs::write(&destination, b"existing").unwrap();
        let mut pixels = RgbaImage::new(2, 1);
        pixels.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        pixels.put_pixel(1, 0, Rgba([0, 0, 255, 128]));
        let rotated = rotate(pixels.clone(), 1);
        assert_eq!(rotated.dimensions(), (1, 2));
        assert_eq!(rotated.get_pixel(0, 1).0, [0, 0, 255, 128]);
        assert_eq!(rotate(rotated, 3), pixels);
        assert!(save_png(&pixels, &destination, |_, _| Err("cancel".into())).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
        assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 2);
        save_png(&pixels, &destination, |temp, dest| {
            std::fs::rename(temp, dest).map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(image::open(&destination).unwrap().into_rgba8(), pixels);
    }
    #[test]
    fn identities_and_actions_reject_untrusted_shape() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(is_pin_label(&format!("pin-{id}")));
        for label in ["space", "pin-../x", "pin-bad", "frozen-widget"] {
            assert!(!is_pin_label(label));
        }
        assert!(serde_json::from_str::<PinAction>(r#"{"type":"rotate","quarterTurns":1}"#).is_ok());
        for action in ["close", "restore", "escape"] {
            assert!(
                serde_json::from_value::<PinAction>(serde_json::json!({"type":action})).is_ok()
            );
            assert!(serde_json::from_value::<PinAction>(
                serde_json::json!({"type":action,"path":"untrusted"})
            )
            .is_err());
        }
        assert!(
            serde_json::from_str::<PinAction>(r#"{"type":"close","path":"C:/anything"}"#).is_err()
        );
        assert!(serde_json::from_str::<PinAction>(
            r#"{"type":"navigate","url":"https://example.com"}"#
        )
        .is_err());
        let root = TestRoot::new();
        std::fs::write(root.0.join("pin-staging"), b"not a directory").unwrap();
        let slot = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        assert!(stage(&root.0, &RgbaImage::new(1, 1), slot).is_err());
    }
    #[tokio::test]
    async fn cancellation_notification_cannot_be_lost_and_work_lease_tracks_real_lifetime() {
        let token = Cancel::default();
        token.cancel();
        tokio::time::timeout(Duration::from_millis(50), token.cancelled())
            .await
            .unwrap();
        let work = Arc::new(Work::default());
        work.state.lock().unwrap().active = 1;
        let lease = WorkLease(work.clone());
        assert_eq!(work.state.lock().unwrap().active, 1);
        drop(lease);
        assert_eq!(work.state.lock().unwrap().active, 0);
    }
}
