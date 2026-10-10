// SPDX-License-Identifier: MPL-2.0
//! Local code recognition. Pure result cache; no scene or Agent mutation.
//! Native URL opening uses the host's strictly scoped, explicitly invoked helper.
use crate::{
    assets,
    code_decode::{CodeFormat, CodeImage, DecodedCode},
    code_worker,
    plugins::{CodeEngine, PluginContribution, PluginSnapshot, PluginState},
    Host,
};
use mewu_core::{Asset, Drawing, OcrTarget, Region, RegionGeometry, SavedTranslation, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

const CANCELED: &str = "识别已取消";
const CACHE_COUNT: usize = 8;
const CACHE_BYTES: usize = 512 * 1024;
const RECENT_COUNT: usize = 512;
const RECENT_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Serialize)]
struct TranslationPixels {
    background_id: String,
    source_id: String,
    drawing_revision: u64,
    width: u32,
    height: u32,
    overlay: Asset,
}
impl TranslationPixels {
    fn from_saved(value: &SavedTranslation) -> Self {
        Self {
            background_id: value.background_id.clone(),
            source_id: value.source_id.clone(),
            drawing_revision: value.drawing_revision,
            width: value.document.width,
            height: value.document.height,
            overlay: value.overlay.clone(),
        }
    }
    fn matches(&self, value: &SavedTranslation) -> bool {
        self.background_id == value.background_id
            && self.source_id == value.source_id
            && self.drawing_revision == value.drawing_revision
            && self.width == value.document.width
            && self.height == value.document.height
            && self.overlay == value.overlay
    }
}
/// Contains every input that changes rendered pixels, excluding OCR/history/text
/// selection, conversation messages, refs and drafts. No full Snapshot clone.
#[derive(Clone, Debug, PartialEq, Serialize)]
struct PixelSource {
    scene_id: String,
    background: Asset,
    region_id: String,
    bounds: RegionGeometry,
    image_override: Option<Asset>,
    drawing_revision: u64,
    drawings: Vec<Drawing>,
    translation: Option<TranslationPixels>,
}
impl PixelSource {
    fn new(scene_id: &str, background: &Asset, region: &Region) -> Self {
        Self {
            scene_id: scene_id.into(),
            background: background.clone(),
            region_id: region.id.clone(),
            bounds: RegionGeometry::from(region),
            image_override: region.image_override.clone(),
            drawing_revision: region.drawing_revision,
            drawings: region.drawings.clone(),
            translation: region
                .translation
                .as_ref()
                .map(TranslationPixels::from_saved),
        }
    }
    fn matches_parts(&self, background: &Asset, region: &Region) -> bool {
        self.background == *background
            && self.region_id == region.id
            && self.bounds == RegionGeometry::from(region)
            && self.image_override == region.image_override
            && self.drawing_revision == region.drawing_revision
            && self.drawings == region.drawings
            && match (&self.translation, &region.translation) {
                (None, None) => true,
                (Some(a), Some(b)) => a.matches(b),
                _ => false,
            }
    }
    fn matches_snapshot(&self, snapshot: &Snapshot, active: bool) -> bool {
        snapshot
            .scenes
            .iter()
            .find(|s| {
                s.id == self.scene_id
                    && !s.closed
                    && (!active || (!s.frozen && s.id == snapshot.active_scene_id))
            })
            .is_some_and(|s| {
                s.background.as_ref().is_some_and(|b| {
                    s.regions
                        .iter()
                        .find(|r| r.id == self.region_id)
                        .is_some_and(|r| self.matches_parts(b, r))
                })
            })
    }
    fn target(&self) -> OcrTarget {
        OcrTarget {
            scene_id: self.scene_id.clone(),
            region_id: self.region_id.clone(),
            background_id: self.background.id.clone(),
            drawing_revision: self.drawing_revision,
            x: self.bounds.x,
            y: self.bounds.y,
            width: self.bounds.width,
            height: self.bounds.height,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Grant {
    plugin_id: String,
    revision: u64,
    contribution_id: String,
}
impl Grant {
    fn current(&self, snapshot: &PluginSnapshot) -> bool {
        snapshot.plugins.iter().any(|p| p.manifest.id == self.plugin_id && p.revision == self.revision
            && p.state == PluginState::Enabled && p.error.is_none()
            && p.manifest.contributions.iter().any(|c| matches!(c,
                PluginContribution::Codes {id,engine:CodeEngine::Rxing,..} if id == &self.contribution_id)))
    }
}
#[derive(Clone)]
struct Origin {
    owner: String,
    window: usize,
    source: Arc<PixelSource>,
    grant: Grant,
}
struct Cached {
    source: Arc<PixelSource>,
    grant: Grant,
    values: Arc<Vec<DecodedCode>>,
    bytes: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeValue {
    id: String,
    format: CodeFormat,
    text: String,
    can_open: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReply {
    request_id: String,
    scene_id: String,
    region_id: String,
    source_token: String,
    codes: Vec<CodeValue>,
}
#[derive(Clone)]
struct Authority {
    token: String,
    codes: Arc<Vec<CodeValue>>,
}
struct Request {
    id: String,
    origin: Origin,
    cancel: Arc<AtomicBool>,
    pending: bool,
    authority: Option<Authority>,
}
struct ActionJob {
    id: String,
    origin: Origin,
    request_id: String,
    source_token: String,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    current: Option<Request>,
    action: Option<ActionJob>,
    recent: HashMap<String, (String, Instant)>,
    cache: VecDeque<Cached>,
    cache_bytes: usize,
}
impl State {
    fn prune(&mut self) {
        self.recent.retain(|_, (_, at)| at.elapsed() < RECENT_TTL);
    }
    fn revoke_current(&mut self) -> Vec<String> {
        let Some(request) = self.current.as_mut() else {
            return vec![];
        };
        request.authority = None;
        let first = !request.cancel.swap(true, Ordering::AcqRel);
        if let Some(action) = self.action.as_ref().filter(|a| a.request_id == request.id) {
            action.cancel.store(true, Ordering::Release);
        }
        if first {
            vec![request.id.clone()]
        } else {
            vec![]
        }
    }
    fn cache_get(&mut self, source: &PixelSource, grant: &Grant) -> Option<Arc<Vec<DecodedCode>>> {
        let position = self
            .cache
            .iter()
            .position(|e| e.source.as_ref() == source && &e.grant == grant)?;
        let value = self.cache.remove(position).unwrap();
        let result = value.values.clone();
        self.cache.push_back(value);
        Some(result)
    }
    fn cache_put(
        &mut self,
        source: Arc<PixelSource>,
        grant: Grant,
        values: Arc<Vec<DecodedCode>>,
    ) -> Result<(), String> {
        // Include source metadata in the budget: drawings alone may legally be
        // several MiB. Such a result remains usable only as the current authority.
        let bytes = serde_json::to_vec(&(source.as_ref(), &grant, values.as_ref()))
            .map_err(|_| "无法记录识别结果")?
            .len();
        if bytes > CACHE_BYTES {
            return Ok(());
        }
        if let Some(index) = self
            .cache
            .iter()
            .position(|e| e.source == source && e.grant == grant)
        {
            self.cache_bytes -= self.cache.remove(index).unwrap().bytes;
        }
        while self.cache.len() >= CACHE_COUNT || self.cache_bytes + bytes > CACHE_BYTES {
            if let Some(old) = self.cache.pop_front() {
                self.cache_bytes -= old.bytes;
            } else {
                break;
            }
        }
        self.cache_bytes += bytes;
        self.cache.push_back(Cached {
            source,
            grant,
            values,
            bytes,
        });
        Ok(())
    }
    fn retain_cache(&mut self, keep: impl Fn(&Cached) -> bool) {
        self.cache.retain(keep);
        self.cache_bytes = self.cache.iter().map(|e| e.bytes).sum();
    }
}
struct Inner {
    state: Mutex<State>,
    work: Arc<Semaphore>,
    actions: Arc<Semaphore>,
}
#[derive(Clone)]
pub(crate) struct CodeRegistry(Arc<Inner>);
impl Default for CodeRegistry {
    fn default() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(State::default()),
            work: Arc::new(Semaphore::new(1)),
            actions: Arc::new(Semaphore::new(1)),
        }))
    }
}
impl CodeRegistry {
    fn start(
        &self,
        id: &str,
        origin: Origin,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(Arc<AtomicBool>, Option<Arc<Vec<DecodedCode>>>, Vec<String>), String> {
        valid_id(id)?;
        let mut state = self.0.state.lock().map_err(|_| "识别状态不可用")?;
        state.prune();
        if state.recent.contains_key(id) || state.current.as_ref().is_some_and(|r| r.id == id) {
            return Err(CANCELED.into());
        }
        if state.current.as_ref().is_some_and(|r| r.pending) || state.action.is_some() {
            return Err("识别任务尚未结束".into());
        }
        if state.recent.len() >= RECENT_COUNT - 1 {
            return Err("识别请求过多，请稍后重试".into());
        }
        // Transition raises its atomic gate before acquiring this registry. It
        // either rejects admission here or sees this exact token in cancellation.
        admit()?;
        let invalidated = state.revoke_current();
        if let Some(old) = state.current.take() {
            state
                .recent
                .entry(old.id)
                .or_insert((old.origin.owner, Instant::now()));
        }
        let values = state.cache_get(origin.source.as_ref(), &origin.grant);
        let cancel = Arc::new(AtomicBool::new(false));
        state.current = Some(Request {
            id: id.into(),
            origin,
            cancel: cancel.clone(),
            pending: true,
            authority: None,
        });
        Ok((cancel, values, invalidated))
    }
    fn finish(&self, id: &str, success: bool) {
        if let Ok(mut state) = self.0.state.lock() {
            let owner = if let Some(current) = state.current.as_mut().filter(|r| r.id == id) {
                current.pending = false;
                if !success {
                    current.cancel.store(true, Ordering::Release);
                    current.authority = None;
                }
                Some(current.origin.owner.clone())
            } else {
                None
            };
            if let Some(owner) = owner {
                state
                    .recent
                    .entry(id.into())
                    .or_insert((owner, Instant::now()));
            }
        }
    }
    fn authorize(&self, id: &str, cancel: &Arc<AtomicBool>) -> bool {
        !cancel.load(Ordering::Acquire)
            && self.0.state.lock().is_ok_and(|state| {
                state.current.as_ref().is_some_and(|r| {
                    r.id == id
                        && Arc::ptr_eq(&r.cancel, cancel)
                        && !r.cancel.load(Ordering::Acquire)
                })
            })
    }
    fn cancel(&self, id: &str, owner: &str) -> Result<Vec<String>, String> {
        valid_id(id)?;
        let mut state = self.0.state.lock().map_err(|_| "识别状态不可用")?;
        state.prune();
        if let Some(request) = state.current.as_ref().filter(|r| r.id == id) {
            if request.origin.owner != owner {
                return Err("识别请求不属于此窗口".into());
            }
            return Ok(state.revoke_current());
        }
        if let Some((previous, _)) = state.recent.get(id) {
            if previous != owner {
                return Err("识别请求不属于此窗口".into());
            }
        } else {
            let limit = RECENT_COUNT
                - usize::from(
                    state
                        .current
                        .as_ref()
                        .is_some_and(|r| !state.recent.contains_key(&r.id)),
                );
            if state.recent.len() >= limit {
                return Err("识别请求过多，请稍后重试".into());
            }
            state
                .recent
                .insert(id.into(), (owner.into(), Instant::now()));
        }
        Ok(vec![])
    }
    fn invalidate(
        &self,
        invalid: impl Fn(&Origin) -> bool,
        cache_keep: impl Fn(&Cached) -> bool,
    ) -> Vec<String> {
        let Ok(mut state) = self.0.state.lock() else {
            code_worker::cancel_all();
            return vec![];
        };
        let ids = if state.current.as_ref().is_some_and(|r| invalid(&r.origin)) {
            state.revoke_current()
        } else {
            vec![]
        };
        if let Some(action) = state.action.as_ref().filter(|a| invalid(&a.origin)) {
            action.cancel.store(true, Ordering::Release);
        }
        state.retain_cache(cache_keep);
        ids
    }
    fn pending(&self, id: &str) -> bool {
        self.0.state.lock().is_ok_and(|s| {
            s.current.as_ref().is_some_and(|r| r.id == id && r.pending)
                || s.action.as_ref().is_some_and(|a| a.request_id == id)
        })
    }
}

fn valid_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id) {
        Ok(())
    } else {
        Err("识别标识无效".into())
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
fn window_handle(window: &WebviewWindow) -> Result<usize, String> {
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
        Err("此平台暂不支持本机扫码".into())
    }
}
fn atomic_gate(host: &Host, handle: usize) -> Result<(), String> {
    host.exit.ensure_running()?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("截图正在切换".into());
    }
    if !visible(handle) {
        return Err("空间已收起".into());
    }
    Ok(())
}
fn check_origin(
    app: &AppHandle,
    origin: &Origin,
    id: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    crate::recording::ensure_idle(app)?;
    let host = app.state::<Host>();
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    plugins.store.codes(
        &origin.grant.plugin_id,
        origin.grant.revision,
        &origin.grant.contribution_id,
    )?;
    let engine = host.lock()?;
    atomic_gate(&host, origin.window)?;
    crate::recording::ensure_idle(app)?;
    let (background, region) = engine
        .store
        .region_for_pin(&origin.source.target())
        .map_err(|e| e.to_string())?;
    if !origin.source.matches_parts(&background, &region) || !host.codes.authorize(id, cancel) {
        return Err(CANCELED.into());
    }
    Ok(())
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Invalidated {
    request_id: String,
}
fn emit_invalidated(app: &AppHandle, ids: Vec<String>) {
    for request_id in ids {
        let _ = app.emit_to("space", "code-scan-invalidated", Invalidated { request_id });
    }
}
pub(crate) fn cancel_owner(app: &AppHandle, owner: &str) {
    if let Some(host) = app.try_state::<Host>() {
        emit_invalidated(app, host.codes.invalidate(|o| o.owner == owner, |_| true));
    }
}
pub(crate) fn cancel_all(app: &AppHandle) {
    if let Some(host) = app.try_state::<Host>() {
        emit_invalidated(app, host.codes.invalidate(|_| true, |_| false));
    }
    code_worker::cancel_all();
}
/// Called with an already-available snapshot, sometimes under Engine: never
/// reacquire Engine, obtain a window handle, perform file I/O or invoke OS APIs.
pub(crate) fn reconcile(app: &AppHandle, snapshot: &Snapshot) {
    if let Some(host) = app.try_state::<Host>() {
        emit_invalidated(
            app,
            host.codes.invalidate(
                |o| !o.source.matches_snapshot(snapshot, true),
                |e| e.source.matches_snapshot(snapshot, false),
            ),
        );
    }
}
pub(crate) fn revoke_plugins(app: &AppHandle, snapshot: &PluginSnapshot) {
    if let Some(host) = app.try_state::<Host>() {
        emit_invalidated(
            app,
            host.codes.invalidate(
                |o| !o.grant.current(snapshot),
                |e| e.grant.current(snapshot),
            ),
        );
    }
}
pub(crate) async fn drain(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    cancel_all(app);
    let Some(host) = app.try_state::<Host>() else {
        return Ok(());
    };
    let registry = host.codes.clone();
    drop(host);
    let started = Instant::now();
    while registry.0.work.available_permits() == 0
        || registry.0.actions.available_permits() == 0
        || code_worker::busy()
    {
        if started.elapsed() >= timeout {
            return Err("识别任务尚未停止，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}
struct Work {
    registry: CodeRegistry,
    id: String,
    success: bool,
    // A blocking crop clone retains the exact slot until it really exits.
    _slot: Arc<OwnedSemaphorePermit>,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.registry.finish(&self.id, self.success);
    }
}
struct InvokeCancel {
    cancel: Arc<AtomicBool>,
    armed: bool,
}
impl Drop for InvokeCancel {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.store(true, Ordering::Release);
        }
    }
}

#[tauri::command]
pub async fn scan_plugin_codes(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
    target: OcrTarget,
    translation_id: Option<String>,
) -> Result<ScanReply, String> {
    if window.label() != "space" {
        return Err("此窗口不能识别选区".into());
    }
    let native_window = window_handle(&window)?;
    crate::recording::ensure_idle(&app)?;
    let host = app.state::<Host>();
    let registry = host.codes.clone();
    let slot = Arc::new(
        registry
            .0
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| "识别任务尚未结束")?,
    );
    let grant = Grant {
        plugin_id,
        revision,
        contribution_id,
    };
    let (origin, background, region, cancel, cached, invalidated) = {
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins
            .store
            .codes(&grant.plugin_id, grant.revision, &grant.contribution_id)?;
        let engine = host.lock()?;
        atomic_gate(&host, native_window)?;
        crate::recording::ensure_idle(&app)?;
        let (background, mut region) = engine
            .store
            .region_for_pin(&target)
            .map_err(|e| e.to_string())?;
        if region.translation.as_ref().map(|v| v.overlay.id.as_str()) != translation_id.as_deref() {
            return Err("选区内容已更改".into());
        }
        let source = Arc::new(PixelSource::new(&target.scene_id, &background, &region));
        let origin = Origin {
            owner: "space".into(),
            window: native_window,
            source,
            grant,
        };
        let (cancel, cached, invalidated) = registry.start(&request_id, origin.clone(), || {
            atomic_gate(&host, native_window)
        })?;
        region.ocr = None;
        region.drawing_history = Default::default();
        (origin, background, region, cancel, cached, invalidated)
    };
    let root = host.assets.clone();
    drop(host);
    emit_invalidated(&app, invalidated);
    let mut guard = InvokeCancel {
        cancel: cancel.clone(),
        armed: true,
    };
    let (send, receive) = oneshot::channel();
    let task_app = app.clone();
    let task_id = request_id.clone();
    let task_cancel = cancel.clone();
    tauri::async_runtime::spawn(async move {
        let mut work = Work {
            registry: registry.clone(),
            id: task_id.clone(),
            success: false,
            _slot: slot.clone(),
        };
        let outcome = async {
            check_origin(&task_app, &origin, &task_id, &task_cancel)?;
            let values = if let Some(values) = cached {
                values
            } else {
                let blocking_cancel = task_cancel.clone();
                let blocking_slot = slot.clone();
                let image = tauri::async_runtime::spawn_blocking(move || {
                    let _slot = blocking_slot;
                    render_luma(&background, &region, &root, &blocking_cancel)
                })
                .await
                .map_err(|_| "图片准备任务中断".to_string())??;
                check_origin(&task_app, &origin, &task_id, &task_cancel)?;
                let result = code_worker::run(image, task_cancel.clone())
                    .await
                    .map_err(|e| e.to_string());
                // Even CleanupTimedOut must not free the accepted host work slot
                // while the supervised child/reaper still owns actual resources.
                while code_worker::busy() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Arc::new(result?)
            };
            register_result(&task_app, &origin, &task_id, &task_cancel, values)
        }
        .await;
        work.success = outcome.is_ok();
        drop(work);
        drop(slot);
        if send.send(outcome).is_err() {
            task_cancel.store(true, Ordering::Release);
            emit_invalidated(
                &task_app,
                registry.cancel(&task_id, "space").unwrap_or_default(),
            );
        }
    });
    let reply = receive.await.map_err(|_| "识别任务中断".to_string())?;
    if reply.is_ok() && cancel.load(Ordering::Acquire) {
        return Err(CANCELED.into());
    }
    guard.armed = false;
    reply
}
fn register_result(
    app: &AppHandle,
    origin: &Origin,
    id: &str,
    cancel: &Arc<AtomicBool>,
    values: Arc<Vec<DecodedCode>>,
) -> Result<ScanReply, String> {
    crate::code_decode::validate_results(values.as_ref()).map_err(|e| e.to_string())?;
    let host = app.state::<Host>();
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    plugins.store.codes(
        &origin.grant.plugin_id,
        origin.grant.revision,
        &origin.grant.contribution_id,
    )?;
    let engine = host.lock()?;
    atomic_gate(&host, origin.window)?;
    crate::recording::ensure_idle(app)?;
    let (background, region) = engine
        .store
        .region_for_pin(&origin.source.target())
        .map_err(|e| e.to_string())?;
    if !origin.source.matches_parts(&background, &region) {
        return Err(CANCELED.into());
    }
    let mut state = host.codes.0.state.lock().map_err(|_| "识别状态不可用")?;
    atomic_gate(&host, origin.window)?;
    if cancel.load(Ordering::Acquire)
        || !state
            .current
            .as_ref()
            .is_some_and(|r| r.id == id && Arc::ptr_eq(&r.cancel, cancel))
    {
        return Err(CANCELED.into());
    }
    state.cache_put(origin.source.clone(), origin.grant.clone(), values.clone())?;
    let codes: Vec<_> = values
        .iter()
        .map(|v| CodeValue {
            id: uuid::Uuid::new_v4().to_string(),
            format: v.format,
            text: v.text.clone(),
            can_open: code_url(&v.text).is_ok(),
        })
        .collect();
    let source_token = uuid::Uuid::new_v4().to_string();
    state.current.as_mut().unwrap().authority = Some(Authority {
        token: source_token.clone(),
        codes: Arc::new(codes.clone()),
    });
    Ok(ScanReply {
        request_id: id.into(),
        scene_id: origin.source.scene_id.clone(),
        region_id: origin.source.region_id.clone(),
        source_token,
        codes,
    })
}
fn render_luma(
    background: &Asset,
    region: &Region,
    root: &PathBuf,
    cancel: &AtomicBool,
) -> Result<CodeImage, String> {
    let check = || {
        if cancel.load(Ordering::Acquire) {
            Err(CANCELED.to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let mut image = assets::crop_from_parts(background, region, root)?;
    check()?;
    let (width, height) = (image.width(), image.height());
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || width as u64 * height as u64 > 32 * 1024 * 1024
    {
        return Err("图片超出识别范围".into());
    }
    let ratio = (8192.0 / width as f64)
        .min(8192.0 / height as f64)
        .min((crate::code_decode::MAX_PIXELS as f64 / (width as f64 * height as f64)).sqrt())
        .min(1.0);
    if ratio < 1.0 {
        image = image.resize_exact(
            (width as f64 * ratio).floor().max(1.0) as u32,
            (height as f64 * ratio).floor().max(1.0) as u32,
            image::imageops::FilterType::Triangle,
        );
    }
    check()?;
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut luma = Vec::with_capacity(width as usize * height as usize);
    for (index, pixel) in rgba.pixels().enumerate() {
        if index % 4096 == 0 {
            check()?;
        }
        let alpha = pixel[3] as u32;
        let white = |channel: u8| (channel as u32 * alpha + 255 * (255 - alpha) + 127) / 255;
        luma.push(
            ((77 * white(pixel[0]) + 150 * white(pixel[1]) + 29 * white(pixel[2]) + 128) >> 8)
                as u8,
        );
    }
    drop(rgba);
    check()?;
    let output = CodeImage {
        width,
        height,
        luma,
    };
    output.validate().map_err(|e| e.to_string())?;
    Ok(output)
}

#[tauri::command]
pub async fn cancel_plugin_code_scan(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能取消选区识别".into());
    }
    let registry = app.state::<Host>().codes.clone();
    emit_invalidated(&app, registry.cancel(&request_id, "space")?);
    let started = Instant::now();
    while registry.pending(&request_id) {
        if started.elapsed() >= Duration::from_secs(5) {
            return Err("识别任务尚未停止".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "String")]
pub enum CodeAction {
    Copy,
    Open,
}
impl TryFrom<String> for CodeAction {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "copy" => Ok(Self::Copy),
            "open" => Ok(Self::Open),
            _ => Err("不支持此码操作"),
        }
    }
}
fn code_url(value: &str) -> Result<url::Url, String> {
    crate::code_open::http_url(value).ok_or_else(|| "此内容不是可打开的链接".into())
}
struct ActionWork {
    registry: CodeRegistry,
    id: String,
    cancel: Arc<AtomicBool>,
    _slot: Arc<OwnedSemaphorePermit>,
}
impl Drop for ActionWork {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Ok(mut state) = self.registry.0.state.lock() {
            if state.action.as_ref().is_some_and(|a| a.id == self.id) {
                state.action = None;
            }
        }
    }
}
fn action_allowed(app: &AppHandle, id: &str, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let host = app.state::<Host>();
    let (origin, request_id, source_token) = {
        let state = host.codes.0.state.lock().map_err(|_| "识别状态不可用")?;
        let job = state
            .action
            .as_ref()
            .filter(|a| a.id == id && Arc::ptr_eq(&a.cancel, cancel))
            .ok_or(CANCELED)?;
        (
            job.origin.clone(),
            job.request_id.clone(),
            job.source_token.clone(),
        )
    };
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELED.into());
    }
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    plugins.store.codes(
        &origin.grant.plugin_id,
        origin.grant.revision,
        &origin.grant.contribution_id,
    )?;
    let engine = host.lock()?;
    atomic_gate(&host, origin.window)?;
    crate::recording::ensure_idle(app)?;
    let (background, region) = engine
        .store
        .region_for_pin(&origin.source.target())
        .map_err(|e| e.to_string())?;
    if !origin.source.matches_parts(&background, &region) {
        return Err(CANCELED.into());
    }
    let state = host.codes.0.state.lock().map_err(|_| "识别状态不可用")?;
    atomic_gate(&host, origin.window)?;
    if cancel.load(Ordering::Acquire)
        || !state.current.as_ref().is_some_and(|r| {
            r.id == request_id
                && !r.cancel.load(Ordering::Acquire)
                && r.authority
                    .as_ref()
                    .is_some_and(|a| a.token == source_token)
        })
    {
        return Err(CANCELED.into());
    }
    Ok(())
}
#[tauri::command]
pub async fn act_on_code(
    app: AppHandle,
    window: WebviewWindow,
    source_token: String,
    code_id: String,
    action: CodeAction,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能操作识别结果".into());
    }
    valid_id(&source_token)?;
    valid_id(&code_id)?;
    let handle = window_handle(&window)?;
    let host = app.state::<Host>();
    let registry = host.codes.clone();
    let slot = Arc::new(
        registry
            .0
            .actions
            .clone()
            .try_acquire_owned()
            .map_err(|_| "上次操作尚未结束")?,
    );
    let (id, cancel, text, source) = {
        let mut state = registry.0.state.lock().map_err(|_| "识别状态不可用")?;
        atomic_gate(&host, handle)?;
        let current = state
            .current
            .as_ref()
            .filter(|r| {
                r.origin.owner == "space"
                    && r.origin.window == handle
                    && !r.cancel.load(Ordering::Acquire)
                    && !r.pending
            })
            .ok_or(CANCELED)?;
        let authority = current
            .authority
            .as_ref()
            .filter(|a| a.token == source_token)
            .ok_or(CANCELED)?;
        let value = authority
            .codes
            .iter()
            .find(|v| v.id == code_id)
            .ok_or("识别内容不存在")?;
        if matches!(action, CodeAction::Open) {
            code_url(&value.text)?;
        }
        let text = value.text.clone();
        let source = current.origin.source.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        state.action = Some(ActionJob {
            id: id.clone(),
            origin: current.origin.clone(),
            request_id: current.id.clone(),
            source_token,
            cancel: cancel.clone(),
        });
        (id, cancel, text, source)
    };
    drop(host);
    let mut invocation = InvokeCancel {
        cancel: cancel.clone(),
        armed: true,
    };
    let (send, receive) = oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let work = ActionWork {
            registry,
            id: id.clone(),
            cancel: cancel.clone(),
            _slot: slot.clone(),
        };
        let worker_app = app.clone();
        let worker_cancel = cancel.clone();
        let worker_id = id.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            let _slot = slot;
            action_allowed(&worker_app, &worker_id, &worker_cancel)?;
            // No application mutex lives across an OS clipboard/browser call.
            // Cancellation after publication cannot roll back that side effect.
            match action {
                CodeAction::Copy => arboard::Clipboard::new()
                    .map_err(|_| "剪贴板不可用".to_string())?
                    .set_text(text)
                    .map_err(|_| "无法复制识别内容".to_string()),
                CodeAction::Open => crate::code_open::open(code_url(&text)?, move || {
                    action_allowed(&worker_app, &worker_id, &worker_cancel)
                }),
            }
        })
        .await
        .map_err(|_| "识别内容操作中断".to_string())
        .and_then(|r| r);
        let outcome = if outcome.is_ok() && matches!(action, CodeAction::Open) {
            // The browser taking focus revokes this action's token itself. Once
            // opening succeeded, use the original pixels under a transition to
            // decide whether to hide, not the now-revoked pre-action authority.
            // A later exit/transition may win; it does not undo a successful open.
            let _ = crate::hide_space_checked(app.clone(), move |snapshot| {
                source.matches_snapshot(snapshot, true)
            })
            .await;
            Ok(())
        } else {
            outcome
        };
        drop(work);
        let _ = send.send(outcome);
    });
    let outcome = receive.await.map_err(|_| "识别内容操作中断".to_string())?;
    invocation.armed = false;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{AssetKind, DrawingEdit, OcrDocument, RegionOcr, TranslationDocument};

    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn asset(width: u32, height: u32) -> Asset {
        let id = id();
        Asset {
            path: format!("{id}.png"),
            id,
            name: "synthetic.png".into(),
            kind: AssetKind::Image,
            width: Some(width),
            height: Some(height),
            origin_x: Some(0),
            origin_y: Some(0),
            scale_factor: Some(1.0),
        }
    }
    fn fixture() -> (Snapshot, Origin, Region) {
        let mut snapshot = mewu_core::Store::open_in_memory().unwrap().snapshot();
        let background = asset(10, 10);
        let region = Region {
            id: id(),
            width: 10.0,
            height: 10.0,
            ..Default::default()
        };
        snapshot.scenes[0].background = Some(background.clone());
        snapshot.scenes[0].regions.push(region.clone());
        let origin = Origin {
            owner: "space".into(),
            window: 1,
            source: Arc::new(PixelSource::new(
                &snapshot.active_scene_id,
                &background,
                &region,
            )),
            grant: Grant {
                plugin_id: "mewu.codes".into(),
                revision: 1,
                contribution_id: "recognize".into(),
            },
        };
        (snapshot, origin, region)
    }
    fn selection() -> OcrDocument {
        OcrDocument {
            engine: "synthetic".into(),
            language: "en".into(),
            width: 10,
            height: 10,
            text_angle: None,
            lines: vec![],
        }
    }
    #[test]
    fn pixel_key_ignores_ocr_history_and_conversation_but_not_translation_or_source() {
        let (mut snapshot, origin, _) = fixture();
        snapshot.scenes[0].draft = "not pixels".into();
        snapshot.scenes[0].regions[0].ocr = Some(RegionOcr {
            background_id: origin.source.background.id.clone(),
            drawing_revision: 0,
            document: selection(),
        });
        snapshot.scenes[0].regions[0].drawing_history.undo.push(
            DrawingEdit {
                index: 0,
                before: None,
                after: None,
            }
            .into(),
        );
        assert!(origin.source.matches_snapshot(&snapshot, true));
        snapshot.scenes[0].regions[0].translation = Some(SavedTranslation {
            background_id: origin.source.background.id.clone(),
            source_id: origin.source.background.id.clone(),
            drawing_revision: 0,
            document: TranslationDocument {
                version: 1,
                target_language: "en".into(),
                width: 10,
                height: 10,
                text_angle: None,
                lines: vec![],
            },
            overlay: asset(10, 10),
            selection: selection(),
        });
        assert!(!origin.source.matches_snapshot(&snapshot, true));
        let translated = PixelSource::new(
            &snapshot.active_scene_id,
            snapshot.scenes[0].background.as_ref().unwrap(),
            &snapshot.scenes[0].regions[0],
        );
        snapshot.scenes[0].regions[0]
            .translation
            .as_mut()
            .unwrap()
            .overlay
            .path = "other.png".into();
        assert!(!translated.matches_snapshot(&snapshot, true));
        snapshot.scenes[0].regions[0].translation = None;
        snapshot.scenes[0].regions[0].image_override = Some(asset(10, 10));
        assert!(!origin.source.matches_snapshot(&snapshot, true));
        snapshot.scenes[0].regions[0].image_override = None;
        snapshot.scenes[0].frozen = true;
        assert!(!origin.source.matches_snapshot(&snapshot, true));
        assert!(origin.source.matches_snapshot(&snapshot, false));
        snapshot.scenes[0].closed = true;
        assert!(!origin.source.matches_snapshot(&snapshot, false));
    }
    #[test]
    fn cancel_finished_request_revokes_actions_and_old_cancel_never_hits_new_request() {
        let (_, origin, _) = fixture();
        let registry = CodeRegistry::default();
        let first = id();
        let (cancel, _, _) = registry.start(&first, origin.clone(), || Ok(())).unwrap();
        {
            let mut state = registry.0.state.lock().unwrap();
            state.current.as_mut().unwrap().authority = Some(Authority {
                token: id(),
                codes: Arc::new(vec![]),
            });
        }
        registry.finish(&first, true);
        assert!(registry.authorize(&first, &cancel));
        assert_eq!(
            registry.cancel(&first, "space").unwrap(),
            vec![first.clone()]
        );
        assert!(registry
            .0
            .state
            .lock()
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .authority
            .is_none());
        let next = id();
        let (next_cancel, _, _) = registry.start(&next, origin, || Ok(())).unwrap();
        registry.cancel(&first, "space").unwrap();
        assert!(registry.authorize(&next, &next_cancel));
        assert!(!registry.authorize(&first, &cancel));
    }
    #[test]
    fn early_cancel_and_admission_gate_cannot_arm_a_late_scan() {
        let (_, origin, _) = fixture();
        let registry = CodeRegistry::default();
        let first = id();
        registry.cancel(&first, "space").unwrap();
        assert!(registry.start(&first, origin.clone(), || Ok(())).is_err());
        let next = id();
        assert!(registry
            .start(&next, origin.clone(), || Err("transition".into()))
            .is_err());
        assert!(registry.0.state.lock().unwrap().current.is_none());
        let (cancel, _, _) = registry.start(&next, origin, || Ok(())).unwrap();
        registry.invalidate(|_| true, |_| false);
        assert!(!registry.authorize(&next, &cancel));
        assert!(registry.pending(&next)); // cancellation never pretends real work exited
        registry.finish(&next, false);
        assert!(!registry.pending(&next));
    }
    #[test]
    fn cache_is_byte_and_count_bounded_keeps_negative_results_and_never_caches_huge_sources() {
        let (_, origin, _) = fixture();
        let mut state = State::default();
        for number in 0..12 {
            let mut source = origin.source.as_ref().clone();
            source.region_id = format!("region-{number}");
            state
                .cache_put(Arc::new(source), origin.grant.clone(), Arc::new(vec![]))
                .unwrap();
        }
        assert_eq!(state.cache.len(), CACHE_COUNT);
        assert!(state.cache_bytes <= CACHE_BYTES);
        let key = state.cache.back().unwrap().source.clone();
        assert!(state.cache_get(&key, &origin.grant).unwrap().is_empty());
        let mut huge = origin.source.as_ref().clone();
        huge.background.name = "x".repeat(CACHE_BYTES + 1);
        state
            .cache_put(
                Arc::new(huge.clone()),
                origin.grant.clone(),
                Arc::new(vec![]),
            )
            .unwrap();
        assert!(state.cache_get(&huge, &origin.grant).is_none());
        let values: Arc<Vec<DecodedCode>> = Arc::new(
            (0..12)
                .map(|i| DecodedCode {
                    format: CodeFormat::QrCode,
                    text: format!("{i}{}", "a".repeat(5000)),
                })
                .collect(),
        );
        for number in 0..12 {
            let mut source = origin.source.as_ref().clone();
            source.region_id = format!("full-{number}");
            state
                .cache_put(Arc::new(source), origin.grant.clone(), values.clone())
                .unwrap();
        }
        assert!(state.cache.len() <= CACHE_COUNT);
        assert!(state.cache_bytes <= CACHE_BYTES);
        assert_eq!(
            state.cache_bytes,
            state.cache.iter().map(|c| c.bytes).sum::<usize>()
        );
    }
    #[test]
    fn cache_hit_is_only_values_and_does_not_reuse_action_authority() {
        let (_, origin, _) = fixture();
        let registry = CodeRegistry::default();
        let first = id();
        let (cancel, _, _) = registry.start(&first, origin.clone(), || Ok(())).unwrap();
        {
            let mut state = registry.0.state.lock().unwrap();
            state
                .cache_put(
                    origin.source.clone(),
                    origin.grant.clone(),
                    Arc::new(vec![]),
                )
                .unwrap();
            state.current.as_mut().unwrap().authority = Some(Authority {
                token: id(),
                codes: Arc::new(vec![]),
            });
        }
        registry.finish(&first, true);
        let second = id();
        let (next, hit, invalidated) = registry.start(&second, origin, || Ok(())).unwrap();
        assert!(hit.is_some());
        assert_eq!(invalidated, vec![first]);
        assert!(cancel.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&next, &cancel));
        assert!(registry
            .0
            .state
            .lock()
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .authority
            .is_none());
    }
    #[test]
    fn canceled_crop_keeps_actual_work_permit_until_the_real_thread_exits() {
        let (_, origin, _) = fixture();
        let registry = CodeRegistry::default();
        let request = id();
        let (cancel, _, _) = registry.start(&request, origin, || Ok(())).unwrap();
        let slot = Arc::new(registry.0.work.clone().try_acquire_owned().unwrap());
        let work = Work {
            registry: registry.clone(),
            id: request.clone(),
            success: false,
            _slot: slot.clone(),
        };
        let (send, receive) = std::sync::mpsc::channel();
        let held = slot.clone();
        let worker = std::thread::spawn(move || {
            let _held = held;
            receive.recv().unwrap();
        });
        registry.cancel(&request, "space").unwrap();
        drop(work);
        drop(slot);
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry.0.work.clone().try_acquire_owned().is_err());
        send.send(()).unwrap();
        worker.join().unwrap();
        assert!(registry.0.work.clone().try_acquire_owned().is_ok());
    }
    #[test]
    fn url_actions_are_strict_and_cannot_be_enum_objects() {
        for text in [
            "http:example.com",
            "javascript:alert(1)",
            "file:///C:/secret",
            "https://user:pass@example.test",
            " https://example.test",
            "https://example.test\n",
            "tel:123",
            "plain text",
        ] {
            assert!(code_url(text).is_err(), "{text}");
        }
        assert!(code_url("https://example.test/path?q=a").is_ok());
        assert!(serde_json::from_str::<CodeAction>(r#""copy""#).is_ok());
        assert!(serde_json::from_str::<CodeAction>(r#"{"Copy":null}"#).is_err());
        assert!(serde_json::from_str::<CodeAction>(r#""delete""#).is_err());
    }
    #[test]
    fn composed_crop_uses_white_for_transparency_and_cancellation_reads_nothing() {
        struct Temp(PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = Temp(std::env::temp_dir().join(format!("mewu-code-host-{}", id())));
        std::fs::create_dir(&dir.0).unwrap();
        let mut image = asset(2, 1);
        image.path = dir.0.join(&image.path).to_string_lossy().into_owned();
        let mut pixels = image::RgbaImage::new(2, 1);
        pixels.put_pixel(0, 0, image::Rgba([255, 0, 0, 0]));
        pixels.put_pixel(1, 0, image::Rgba([0, 0, 0, 255]));
        pixels.save(&image.path).unwrap();
        let region = Region {
            id: id(),
            width: 2.0,
            height: 1.0,
            ..Default::default()
        };
        let output = render_luma(&image, &region, &dir.0, &AtomicBool::new(false)).unwrap();
        assert_eq!((output.width, output.height), (2, 1));
        assert_eq!(output.luma, vec![255, 0]);
        std::fs::remove_file(&image.path).unwrap();
        assert_eq!(
            render_luma(&image, &region, &dir.0, &AtomicBool::new(true))
                .err()
                .unwrap(),
            CANCELED
        );
    }
}
