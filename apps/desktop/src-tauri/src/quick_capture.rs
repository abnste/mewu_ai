// SPDX-License-Identifier: MPL-2.0
//! One temporary invocation. Generation and source ownership fence late release,
//! capture completion and renderer callbacks; ordinary capture remains separate.
use crate::Host;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager};
static ACTIVE: AtomicU64 = AtomicU64::new(0);
static TOUCHED: AtomicU64 = AtomicU64::new(0);
fn text_slots() -> &'static std::sync::Arc<tokio::sync::Semaphore> {
    static SLOTS: OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)))
}
fn image_slots() -> &'static std::sync::Arc<tokio::sync::Semaphore> {
    static SLOTS: OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)))
}
pub async fn wait_until_idle(timeout: std::time::Duration) -> bool {
    let began = tokio::time::Instant::now();
    loop {
        if text_slots().available_permits() == 1 && image_slots().available_permits() == 1 {
            return true;
        }
        if began.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
#[derive(Clone)]
struct Recognizer {
    plugin: String,
    revision: u64,
    contribution: String,
}
fn recognizer(app: &AppHandle) -> Result<Recognizer, String> {
    let host = app.state::<Host>();
    let runtime = host.plugins.lock().map_err(|_| "文字识别插件不可用")?;
    for plugin in runtime.store.snapshot()?.plugins {
        if plugin.state != crate::plugins::PluginState::Enabled || plugin.error.is_some() {
            continue;
        }
        for contribution in &plugin.manifest.contributions {
            if let crate::plugins::PluginContribution::Ocr { id, engine, .. } = contribution {
                if engine.supported() {
                    runtime
                        .store
                        .ocr(&plugin.manifest.id, plugin.revision, id)?;
                    return Ok(Recognizer {
                        plugin: plugin.manifest.id.clone(),
                        revision: plugin.revision,
                        contribution: id.clone(),
                    });
                }
            }
        }
    }
    Err("文字识别插件未启用".into())
}
fn check_recognizer(app: &AppHandle, source: &Recognizer) -> Result<(), String> {
    let host = app.state::<Host>();
    host.exit.ensure_background()?;
    let runtime = host.plugins.lock().map_err(|_| "文字识别插件不可用")?;
    runtime
        .store
        .ocr(&source.plugin, source.revision, &source.contribution)?;
    Ok(())
}
pub fn mouse_touch() {
    let generation = ACTIVE.load(Ordering::Acquire);
    if generation != 0 {
        TOUCHED.store(generation, Ordering::Release);
    }
}

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub generation: u64,
    pub active: bool,
    pub held: bool,
    pub interacted: bool,
    pub scene_id: Option<String>,
}
#[derive(Default)]
struct State {
    view: View,
    owner: String,
    recognition: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl State {
    fn release(&mut self, owner: &str, touched: u64) -> bool {
        if self.owner != owner || !self.view.held {
            return false;
        }
        self.view.held = false;
        if touched == self.view.generation {
            self.view.interacted = true;
        }
        true
    }
    fn close_requested(&self) -> bool {
        self.view.active && !self.view.held && !self.view.interacted
    }
}
#[cfg(test)]
mod release_tests {
    use super::*;
    fn active() -> State {
        State {
            view: View {
                generation: 8,
                active: true,
                held: true,
                interacted: false,
                scene_id: Some("scene".into()),
            },
            owner: "hold-rule".into(),
            recognition: None,
        }
    }
    #[test]
    fn release_is_source_bound_idempotent_and_only_untouched_invocations_close() {
        let mut state = active();
        assert!(!state.release("other-rule", 8));
        assert!(state.view.held);
        assert!(state.release("hold-rule", 7));
        assert!(state.close_requested());
        assert!(!state.release("hold-rule", 7));
        state.view.interacted = true;
        assert!(!state.close_requested());
        let mut touched = active();
        assert!(touched.release("hold-rule", 8));
        assert!(!touched.close_requested());
        let mut pending = active();
        pending.view.scene_id = None;
        assert!(pending.release("hold-rule", 0));
        assert!(!pending.view.held && !pending.view.interacted);
    }
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}
fn publish(app: &AppHandle, view: &View) {
    let _ = app.emit_to("space", "quick-interaction", view);
}
pub fn held(app: &AppHandle, owner: &str) {
    if let Ok(mut s) = state().lock() {
        if s.view.active {
            return;
        }
        s.view = View {
            generation: s.view.generation.wrapping_add(1).max(1),
            held: true,
            ..View::default()
        };
        s.owner = owner.into();
        publish(app, &s.view);
    }
}
pub fn released(app: &AppHandle, owner: &str) {
    let close = if let Ok(mut s) = state().lock() {
        if !s.release(owner, TOUCHED.load(Ordering::Acquire)) {
            return;
        }
        publish(app, &s.view);
        s.close_requested()
            .then(|| (s.view.generation, s.view.scene_id.clone()))
    } else {
        None
    };
    if let Some((generation, Some(scene))) = close {
        close_owned(app.clone(), generation, scene);
    }
}
fn close_owned(app: AppHandle, generation: u64, scene: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        if !owns(generation, &scene) {
            return;
        }
        // Normal close persists history and preserves pinned objects. Do not just
        // destroy a WebView or erase a frozen/minimized conversation.
        match crate::close_scene_window(app.clone(), scene).await {
            Ok(_) => clear(&app, generation),
            Err(error) => {
                let _ = app.emit_to("space", "host-error", error);
            }
        }
    });
}
fn owns(generation: u64, scene: &str) -> bool {
    state().lock().is_ok_and(|s| {
        s.view.generation == generation
            && s.close_requested()
            && s.view.scene_id.as_deref() == Some(scene)
    })
}
pub fn current(generation: u64) -> bool {
    state().lock().is_ok_and(|s| {
        s.view.generation == generation && s.view.active && (s.view.held || s.view.interacted)
    })
}
pub fn detach(app: &AppHandle) {
    let generation = state().lock().map(|s| s.view.generation).unwrap_or(0);
    clear(app, generation);
}
pub fn withdraw(app: &AppHandle, owner: &str) {
    let generation = state()
        .lock()
        .ok()
        .and_then(|s| (s.owner == owner).then_some(s.view.generation));
    if let Some(generation) = generation {
        clear(app, generation);
    }
}
pub fn scene_closed(app: &AppHandle, scene: &str) {
    let generation = state()
        .lock()
        .ok()
        .and_then(|s| (s.view.scene_id.as_deref() == Some(scene)).then_some(s.view.generation));
    if let Some(generation) = generation {
        clear(app, generation);
    }
}
pub fn reconcile(app: &AppHandle, snapshot: &mewu_core::Snapshot) {
    let stale = state().lock().ok().and_then(|s| {
        s.view.scene_id.as_ref().and_then(|scene| {
            (s.view.active
                && (snapshot.active_scene_id != *scene
                    || !snapshot
                        .scenes
                        .iter()
                        .any(|value| value.id == *scene && !value.closed && !value.frozen)))
            .then_some(s.view.generation)
        })
    });
    if let Some(generation) = stale {
        clear(app, generation);
    }
}
pub fn clear(app: &AppHandle, generation: u64) {
    if let Ok(mut s) = state().lock() {
        if s.view.generation != generation {
            return;
        }
        if let Some(token) = s.recognition.take() {
            token.store(true, Ordering::Release);
        }
        ACTIVE.store(0, Ordering::Release);
        s.view.active = false;
        s.view.held = false;
        s.view.scene_id = None;
        publish(app, &s.view);
    }
}
pub async fn start(app: &AppHandle, owner: Option<&str>) -> Result<View, String> {
    let generation = {
        let mut s = state().lock().map_err(|_| "快速交互状态不可用")?;
        if s.view.active {
            return Err("快速交互已打开".into());
        }
        if let Some(owner) = owner {
            if s.owner != owner || !s.view.held {
                return Err("长按已松开".into());
            }
        } else {
            s.view.generation = s.view.generation.wrapping_add(1).max(1);
            s.owner.clear();
            s.view.held = false;
            s.view.interacted = true;
        }
        s.view.active = true;
        ACTIVE.store(s.view.generation, Ordering::Release);
        publish(app, &s.view);
        s.view.generation
    };
    if app
        .state::<Host>()
        .capturing
        .load(std::sync::atomic::Ordering::Acquire)
    {
        clear(app, generation);
        return Err("截图正在切换".into());
    }
    let result = crate::capture_screen_inner(app.clone(), Some(generation)).await;
    match result {
        Ok(snapshot) => {
            let close = {
                let mut s = state().lock().map_err(|_| "快速交互状态不可用")?;
                if s.view.generation != generation || !s.view.active {
                    return Err("快速交互已切换".into());
                }
                s.view.scene_id = Some(snapshot.snapshot.active_scene_id.clone());
                publish(app, &s.view);
                (!s.view.held && !s.view.interacted)
                    .then(|| snapshot.snapshot.active_scene_id.clone())
            };
            if let Some(scene) = close {
                close_owned(app.clone(), generation, scene);
            }
            let latest = app.state::<Host>().lock()?.store.snapshot();
            reconcile(app, &latest);
            view(app)
        }
        Err(error) => {
            clear(app, generation);
            Err(error)
        }
    }
}
#[tauri::command]
pub fn quick_interaction_state(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<View, String> {
    if window.label() != "space" {
        return Err("此窗口不能访问快速交互".into());
    }
    view(&app)
}
#[tauri::command]
pub async fn quick_interaction_open(
    app: AppHandle,
    window: tauri::WebviewWindow,
    operation_id: Option<String>,
    operation_nonce: Option<String>,
) -> Result<View, String> {
    if window.label() != "space" {
        return Err("此窗口不能打开快速交互".into());
    }
    let origin = match (operation_id, operation_nonce) {
        (Some(id), Some(nonce)) => {
            crate::app_control_host::admit_feature(&app, &window, &id, &nonce, "capture.quick")?
        }
        (None, None) => None,
        _ => return Err("功能请求的来源证明不完整".into()),
    };
    let owner = origin.and_then(|value| value.hold_source);
    start(&app, owner.as_deref()).await
}
fn view(_app: &AppHandle) -> Result<View, String> {
    state()
        .lock()
        .map(|s| s.view.clone())
        .map_err(|_| "快速交互状态不可用".into())
}
#[tauri::command]
pub fn quick_interaction_touch(
    app: AppHandle,
    window: tauri::WebviewWindow,
    generation: u64,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能操作快速交互".into());
    }
    let mut s = state().lock().map_err(|_| "快速交互状态不可用")?;
    if s.view.generation != generation || !s.view.active {
        return Err("快速交互已切换".into());
    }
    s.view.interacted = true;
    publish(&app, &s.view);
    Ok(())
}
#[tauri::command]
pub async fn quick_interaction_recognize(
    app: AppHandle,
    window: tauri::WebviewWindow,
    generation: u64,
) -> Result<mewu_core::OcrDocument, String> {
    if window.label() != "space" {
        return Err("此窗口不能识别截图".into());
    }
    let provider = recognizer(&app)?;
    let (scene, cancel) = {
        let mut s = state().lock().map_err(|_| "快速交互状态不可用")?;
        if !s.view.active || s.view.generation != generation {
            return Err("快速交互已切换".into());
        }
        let scene = s.view.scene_id.clone().ok_or("截图尚未就绪")?;
        // Reconfiguration supersedes the previous recognizer. Its owned slot
        // remains held until the worker actually stops, so work cannot overlap.
        if let Some(token) = s.recognition.take() {
            token.store(true, Ordering::Release);
        }
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        s.recognition = Some(token.clone());
        (scene, token)
    };
    struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Cancel {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _cancel = Cancel(cancel.clone());
    let (asset, root) = {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        host.exit.ensure_new_operation()?;
        let source = engine.store.scene(&scene).map_err(|e| e.to_string())?;
        if source.closed {
            return Err("截图已关闭".into());
        }
        (
            source.background.clone().ok_or("截图尚未就绪")?,
            host.assets.clone(),
        )
    };
    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(16),
        text_slots().clone().acquire_owned(),
    )
    .await
    .map_err(|_| "等待文字识别器超时")?
    .map_err(|_| "文字识别器已停止")?;
    check_recognizer(&app, &provider)?;
    let token = cancel.clone();
    let mut worker = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if token.load(Ordering::Acquire) {
            return Err("文字识别已取消".into());
        }
        crate::ocr::recognize(crate::assets::image(&asset, &root)?.into_rgba8(), token)
    });
    let result = loop {
        tokio::select! {value=&mut worker=>break value.map_err(|_|"文字识别任务中断")?,_=tokio::time::sleep(std::time::Duration::from_millis(100))=>{if !current(generation){return Err("快速交互已切换".into());}check_recognizer(&app,&provider)?;}}
    };
    cancel.store(true, Ordering::Release);
    if !current(generation) {
        return Err("快速交互已切换".into());
    }
    check_recognizer(&app, &provider)?;
    result
}
#[tauri::command]
pub async fn quick_interaction_images(
    app: AppHandle,
    window: tauri::WebviewWindow,
    generation: u64,
) -> Result<Vec<crate::window_snap::SnapRect>, String> {
    if window.label() != "space" || !current(generation) {
        return Err("快速交互已切换".into());
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        return Ok(vec![]);
    }
    #[cfg(windows)]
    {
        let scene = state()
            .lock()
            .map_err(|_| "快速交互状态不可用")?
            .view
            .scene_id
            .clone()
            .ok_or("截图尚未就绪")?;
        let (map, windows, origin) = {
            let host = app.state::<Host>();
            let engine = host.lock()?;
            host.exit.ensure_new_operation()?;
            let asset = engine
                .store
                .scene(&scene)
                .map_err(|e| e.to_string())?
                .background
                .clone()
                .ok_or("截图尚未就绪")?;
            let Some(map) = host.window_snap.get(&asset) else {
                return Ok(vec![]);
            };
            let windows = map
                .roots
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    host.window_snap
                        .snapshot_target(&asset, node.id, node.bounds)
                        .map(|window| (index, window))
                })
                .collect::<Vec<_>>();
            (
                map,
                windows,
                (asset.origin_x.unwrap_or(0), asset.origin_y.unwrap_or(0)),
            )
        };
        let permit = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            image_slots().clone().acquire_owned(),
        )
        .await
        .map_err(|_| "等待图片识别器超时")?
        .map_err(|_| "图片识别器已停止")?;
        let result = tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            crate::quick_images_windows::discover(&map, &windows, origin, || {
                ACTIVE.load(Ordering::Acquire) == generation
            })
        })
        .await
        .map_err(|_| "图片区域识别中断")??;
        if !current(generation) {
            return Err("快速交互已切换".into());
        }
        Ok(result)
    }
}
