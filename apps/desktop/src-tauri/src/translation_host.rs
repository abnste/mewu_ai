// SPDX-License-Identifier: MPL-2.0
//! Translation owns no conversation run: local masked-source OCR, bounded
//! text-only requests, native layout, then one complete document transaction.
use crate::{
    plugins::{PluginSnapshot, PluginState},
    provider_transport,
    translation_document::{self, Plan},
    Host, HostSnapshot, RunPreflight,
};
use image::ImageEncoder;
use mewu_core::{Asset, AssetKind, ConnectionProfile, OcrTarget, Snapshot, TranslationDocument};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

const CANCELED: &str = "已取消原位翻译";
const OVERALL_TIMEOUT: Duration = Duration::from_secs(6 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OVERLAY_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Default)]
struct Cancellation {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}
impl Cancellation {
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
        // Register before checking the flag, so cancellation between the check
        // and await cannot be lost. Native OCR also observes the same atomic.
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.flag.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}
struct CancelOnDrop(Cancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[derive(Clone)]
struct Origin {
    owner: String,
    target: OcrTarget,
    background: Asset,
    source: Asset,
    connection: ConnectionProfile,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
}
struct Job {
    origin: Origin,
    cancel: Cancellation,
}
struct Tombstone {
    owner: String,
    at: Instant,
}
#[derive(Default)]
pub(crate) struct TranslationRegistry {
    jobs: HashMap<String, Job>,
    recent: HashMap<String, Tombstone>,
}
impl TranslationRegistry {
    fn prune(&mut self) {
        self.recent
            .retain(|_, item| item.at.elapsed() < Duration::from_secs(300));
    }
    fn start(&mut self, id: &str, origin: Origin) -> Result<Cancellation, String> {
        valid_id(id)?;
        self.prune();
        if self.jobs.contains_key(id) || self.recent.contains_key(id) {
            return Err("翻译请求已结束，请重新翻译".into());
        }
        // Canceled workers retain their slot until the last worker lease exits.
        if self.jobs.len() >= 2 || self.jobs.len() + self.recent.len() >= 512 {
            return Err("翻译正在处理，请稍后重试".into());
        }
        self.cancel_scene(&origin.owner, &origin.target.scene_id);
        let cancel = Cancellation::default();
        self.jobs.insert(
            id.into(),
            Job {
                origin,
                cancel: cancel.clone(),
            },
        );
        Ok(cancel)
    }
    fn cancel(&mut self, id: &str, owner: &str) -> Result<(), String> {
        valid_id(id)?;
        self.prune();
        if let Some(job) = self.jobs.get(id) {
            if job.origin.owner != owner {
                return Err("翻译请求不属于此窗口".into());
            }
            job.cancel.cancel();
        } else if let Some(item) = self.recent.get(id) {
            if item.owner != owner {
                return Err("翻译请求不属于此窗口".into());
            }
        } else {
            if self.jobs.len() + self.recent.len() >= 512 {
                return Err("翻译请求过多，请稍后重试".into());
            }
            // Cancel-before-start remains canceled; IPC delivery order is not authority.
            self.recent.insert(
                id.into(),
                Tombstone {
                    owner: owner.into(),
                    at: Instant::now(),
                },
            );
        }
        Ok(())
    }
    fn finish(&mut self, id: &str) {
        if let Some(job) = self.jobs.remove(id) {
            self.recent.insert(
                id.into(),
                Tombstone {
                    owner: job.origin.owner,
                    at: Instant::now(),
                },
            );
        }
    }
    fn authorized(&self, id: &str, token: &Cancellation) -> bool {
        token.check().is_ok()
            && self
                .jobs
                .get(id)
                .is_some_and(|job| Arc::ptr_eq(&job.cancel.flag, &token.flag))
    }
    pub(crate) fn cancel_owner(&self, owner: &str) {
        for job in self.jobs.values().filter(|job| job.origin.owner == owner) {
            job.cancel.cancel();
        }
    }
    pub(crate) fn cancel_scene(&self, owner: &str, scene_id: &str) {
        for job in self
            .jobs
            .values()
            .filter(|job| job.origin.owner == owner && job.origin.target.scene_id == scene_id)
        {
            job.cancel.cancel();
        }
    }
    pub(crate) fn cancel_connection(&self, id: &str) {
        for job in self
            .jobs
            .values()
            .filter(|job| job.origin.connection.id == id)
        {
            job.cancel.cancel();
        }
    }
    pub(crate) fn reconcile(&self, snapshot: &Snapshot) {
        for job in self.jobs.values() {
            if !matches_snapshot(snapshot, &job.origin) {
                job.cancel.cancel();
            }
        }
    }
    pub(crate) fn revoke_plugins(&self, snapshot: &PluginSnapshot) {
        for job in self.jobs.values() {
            if !snapshot.plugins.iter().any(|record| {
                record.manifest.id == job.origin.plugin_id
                    && record.revision == job.origin.plugin_revision
                    && record.state == PluginState::Enabled
                    && record.error.is_none()
            }) {
                job.cancel.cancel();
            }
        }
    }
}
fn valid_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id) {
        Ok(())
    } else {
        Err("翻译请求标识无效".into())
    }
}
fn matches_snapshot(snapshot: &Snapshot, origin: &Origin) -> bool {
    let Some(scene) = snapshot
        .scenes
        .iter()
        .find(|scene| scene.id == origin.target.scene_id && !scene.closed)
    else {
        return false;
    };
    let Some(region) = scene
        .regions
        .iter()
        .find(|region| region.id == origin.target.region_id)
    else {
        return false;
    };
    scene.background.as_ref() == Some(&origin.background)
        && scene.connection_id.as_deref() == Some(origin.connection.id.as_str())
        && snapshot
            .connections
            .iter()
            .any(|profile| profile == &origin.connection)
        && region.drawing_revision == origin.target.drawing_revision
        && (region.x, region.y, region.width, region.height)
            == (
                origin.target.x,
                origin.target.y,
                origin.target.width,
                origin.target.height,
            )
        && region.image_override.as_ref().unwrap_or(&origin.background) == &origin.source
}

struct Lease {
    app: AppHandle,
    request_id: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut engine) = self.app.state::<Host>().lock() {
            engine.translation_jobs.finish(&self.request_id);
        };
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    request_id: &'a str,
    scene_id: &'a str,
    region_id: &'a str,
    phase: &'a str,
    completed: usize,
    total: usize,
}
fn progress(
    app: &AppHandle,
    id: &str,
    origin: &Origin,
    phase: &str,
    completed: usize,
    total: usize,
) {
    let _ = app.emit_to(
        "space",
        "translation-progress",
        Progress {
            request_id: id,
            scene_id: &origin.target.scene_id,
            region_id: &origin.target.region_id,
            phase,
            completed,
            total,
        },
    );
}

fn check_origin(
    engine: &crate::Engine,
    origin: &Origin,
    id: &str,
    cancel: &Cancellation,
) -> Result<(), String> {
    if !engine.translation_jobs.authorized(id, cancel) {
        return Err(CANCELED.into());
    }
    let (background, source, _) = region_source(&engine.store, &origin.target)?;
    if background != origin.background
        || source != origin.source
        || engine
            .store
            .connection_for_scene(&origin.target.scene_id)
            .map_err(|error| error.to_string())?
            != origin.connection
    {
        return Err("选区或连接已更改，请重新翻译".into());
    }
    Ok(())
}

/// Store returns a virtual 0,0 region with the replacement pixels for a long
/// image. Keep its actual source distinct from the scene's desktop background.
fn region_source(
    store: &mewu_core::Store,
    target: &OcrTarget,
) -> Result<(Asset, Asset, mewu_core::Region), String> {
    let (source, region) = store
        .region_for_translation(target)
        .map_err(|error| error.to_string())?;
    let background = store
        .background_for_preview(&target.scene_id, &target.background_id)
        .map_err(|error| error.to_string())?;
    Ok((background, source, region))
}
fn fence(app: &AppHandle, origin: &Origin, id: &str, cancel: &Cancellation) -> Result<(), String> {
    let host = app.state::<Host>();
    let runtime = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?;
    runtime.store.translation(
        &origin.plugin_id,
        origin.plugin_revision,
        &origin.contribution_id,
    )?;
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    check_origin(&engine, origin, id, cancel)
}

#[tauri::command]
pub async fn run_plugin_translation(
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
    target: OcrTarget,
    language: String,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("请在截图选区中翻译".into());
    }
    translation_document::language_name(&language)?;
    valid_id(&request_id)?;
    crate::recording::ensure_idle(&app)?;
    let (origin, region, cancel, root, preflight) = {
        let host = app.state::<Host>();
        let runtime = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?;
        runtime
            .store
            .translation(&plugin_id, revision, &contribution_id)?;
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        // Scroll capture registers its slot under Engine without setting the
        // window-transition atomic. Recheck under the same registration lock.
        crate::recording::ensure_idle(&app)?;
        if host.capturing.load(Ordering::Acquire)
            || !window.is_visible().map_err(|_| "截图窗口不可用")?
        {
            return Err("截图正在切换，请稍后翻译".into());
        }
        let (background, source, region) = region_source(&engine.store, &target)?;
        let preflight = crate::preflight_run(&host, &engine, &target.scene_id)?;
        let origin = Origin {
            owner: window.label().into(),
            source,
            background,
            target,
            connection: preflight.connection.clone(),
            plugin_id,
            plugin_revision: revision,
            contribution_id,
        };
        let cancel = engine.translation_jobs.start(&request_id, origin.clone())?;
        engine
            .ocr_jobs
            .cancel_scene(window.label(), &origin.target.scene_id);
        (origin, region, cancel, host.assets.clone(), preflight)
    };
    let _cancel_on_drop = CancelOnDrop(cancel.clone());
    let lease = Arc::new(Lease {
        app: app.clone(),
        request_id: request_id.clone(),
    });
    let work = async {
        progress(&app, &request_id, &origin, "recognizing", 0, 0);
        let source = origin.source.clone();
        let source_root = root.clone();
        let worker_cancel = cancel.clone();
        let worker_lease = lease.clone();
        let (clean, ocr) = tauri::async_runtime::spawn_blocking(move || {
            let _lease = worker_lease;
            worker_cancel.check()?;
            // This clean path retains mosaic privacy masks, but excludes text,
            // drawing strokes and the previous translation. It never captures a screen.
            let clean = crate::assets::clean_crop_from_parts(&source, &region, &source_root)?;
            worker_cancel.check()?;
            let ocr = crate::ocr::recognize(clean.to_rgba8(), worker_cancel.flag.clone())?;
            worker_cancel.check()?;
            Ok::<_, String>((clean, ocr))
        })
        .await
        .map_err(|_| "翻译文字识别中断")??;
        fence(&app, &origin, &request_id, &cancel)?;
        let plan = Plan::new(&ocr, &language)?;
        let total = plan.line_count();
        progress(&app, &request_id, &origin, "translating", 0, total);
        let document = translate_plan(
            plan,
            &preflight,
            &cancel,
            || fence(&app, &origin, &request_id, &cancel),
            |completed| progress(&app, &request_id, &origin, "translating", completed, total),
        )
        .await?;
        fence(&app, &origin, &request_id, &cancel)?;
        progress(&app, &request_id, &origin, "rendering", total, total);
        let output_root = root.clone();
        let worker_cancel = cancel.clone();
        let worker_lease = lease.clone();
        let (document, rendered, mut output) = tauri::async_runtime::spawn_blocking(move || {
            let _lease = worker_lease;
            worker_cancel.check()?;
            let rendered = crate::translation_render::render_cancellable(
                &clean,
                &document,
                worker_cancel.flag.as_ref(),
            )?;
            worker_cancel.check()?;
            let output = save_overlay(&output_root, &rendered.overlay)?;
            worker_cancel.check()?;
            Ok::<_, String>((document, rendered, output))
        })
        .await
        .map_err(|_| "译文排版中断")??;
        let host = app.state::<Host>();
        // Same lock order as plugin revocation. Keep this final validation and
        // the one complete SQLite commit together; no request holds these locks.
        let runtime = host.plugins.lock().map_err(|_| "插件存储需要重新启动")?;
        runtime.store.translation(
            &origin.plugin_id,
            origin.plugin_revision,
            &origin.contribution_id,
        )?;
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        check_origin(&engine, &origin, &request_id, &cancel)?;
        let snapshot = engine
            .store
            .set_region_translation(
                &origin.target,
                document,
                output.asset.clone(),
                rendered.selection,
            )
            .map_err(|error| error.to_string())?;
        output.retained = true;
        Ok(crate::publish(&app, &snapshot))
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(CANCELED.into()),
        result = tokio::time::timeout(OVERALL_TIMEOUT, work) => result.map_err(|_| "原位翻译超时，请缩小选区或稍后重试".to_string())?,
    }
}

#[tauri::command]
pub fn cancel_plugin_translation(
    host: tauri::State<'_, Host>,
    window: tauri::WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    if window.label() != "space" {
        return Err("请在截图选区中操作".into());
    }
    host.lock()?
        .translation_jobs
        .cancel(&request_id, window.label())
}

/// Removing one's saved document remains available after its generating plugin
/// is disabled/uninstalled. This grants no new OCR or network capability.
#[tauri::command]
pub fn clear_region_translation(
    app: AppHandle,
    window: tauri::WebviewWindow,
    target: OcrTarget,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("请在截图选区中移除译文".into());
    }
    let host = app.state::<Host>();
    let _transition = crate::begin_space_transition(&app, &host)?;
    crate::recording::ensure_idle(&app)?;
    let mut engine = host.lock()?;
    host.exit.ensure_running()?;
    engine
        .store
        .region_for_translation(&target)
        .map_err(|error| error.to_string())?;
    engine
        .translation_jobs
        .cancel_scene(window.label(), &target.scene_id);
    // Clearing a translated bitmap does not advance drawingRevision. In-flight
    // OCR may have read those old pixels, so its token must also be revoked.
    engine
        .ocr_jobs
        .cancel_scene(window.label(), &target.scene_id);
    let snapshot = engine
        .store
        .clear_region_translation(&target)
        .map_err(|error| error.to_string())?;
    Ok(crate::publish(&app, &snapshot))
}

async fn translate_plan(
    plan: Plan,
    preflight: &RunPreflight,
    cancel: &Cancellation,
    mut before_request: impl FnMut() -> Result<(), String>,
    mut on_progress: impl FnMut(usize),
) -> Result<TranslationDocument, String> {
    let mut results = Vec::with_capacity(plan.batches.len());
    let mut completed = 0;
    for batch in &plan.batches {
        cancel.check()?;
        before_request()?;
        let mut body = serde_json::json!({"model":preflight.connection.model,"messages":plan.messages(batch)?,"stream":true});
        crate::connection_host::apply_parameters(&mut body, &preflight.connection)?;
        let body = provider_transport::request_body(preflight.transport.protocol, body)?;
        let response =
            request_text(&preflight.transport, preflight.key.as_str(), body, cancel).await?;
        cancel.check()?;
        let translations = translation_document::parse_response(&response, batch)?;
        completed += translations.len();
        results.push(translations);
        on_progress(completed);
    }
    cancel.check()?;
    plan.complete(results)
}

async fn request_text(
    transport: &provider_transport::Transport,
    key: &str,
    body: serde_json::Value,
    cancel: &Cancellation,
) -> Result<String, String> {
    let overflow = AtomicBool::new(false);
    let limit = Notify::new();
    let mut bytes = 0usize;
    let request = provider_transport::stream_turn(transport, key, body, |delta| {
        bytes = bytes.saturating_add(delta.len());
        if bytes > translation_document::MAX_RESPONSE_BYTES {
            overflow.store(true, Ordering::Release);
            limit.notify_one();
        }
    });
    let turn = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(CANCELED.into()),
        _ = limit.notified() => return Err("译文返回过长，请缩小选区".into()),
        result = tokio::time::timeout(REQUEST_TIMEOUT, request) => result.map_err(|_| "翻译请求超时，请稍后重试".to_string())??,
    };
    cancel.check()?;
    if overflow.load(Ordering::Acquire)
        || turn.content.len() > translation_document::MAX_RESPONSE_BYTES
    {
        return Err("译文返回过长，请缩小选区".into());
    }
    if !turn.tool_calls.is_empty() {
        return Err("原位翻译不允许调用工具".into());
    }
    Ok(turn.content)
}

struct PendingOverlay {
    asset: Asset,
    path: PathBuf,
    retained: bool,
}
impl Drop for PendingOverlay {
    fn drop(&mut self) {
        if !self.retained {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
struct BoundedFile<'a> {
    file: &'a mut File,
    bytes: usize,
}
impl Write for BoundedFile<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_OVERLAY_BYTES.saturating_sub(self.bytes) {
            return Err(io::Error::other("translation overlay exceeds limit"));
        }
        let written = self.file.write(bytes)?;
        self.bytes += written;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
fn save_overlay(root: &Path, image: &image::RgbaImage) -> Result<PendingOverlay, String> {
    if image.width() == 0
        || image.height() == 0
        || image.width() > 16384
        || image.height() > 16384
        || u64::from(image.width()) * u64::from(image.height()) > 32 * 1024 * 1024
    {
        return Err("译文图层尺寸超过限制".into());
    }
    let root = root.canonicalize().map_err(|_| "译文素材目录不可用")?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.png"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "无法创建译文图层")?;
    let output = PendingOverlay {
        asset: Asset {
            id,
            name: "原位译文.png".into(),
            kind: AssetKind::Image,
            path: path.to_string_lossy().into_owned(),
            width: Some(image.width()),
            height: Some(image.height()),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        },
        path,
        retained: false,
    };
    let encoded = (|| {
        let mut writer = BoundedFile {
            file: &mut file,
            bytes: 0,
        };
        image::codecs::png::PngEncoder::new(&mut writer)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|_| "无法编码完整译文图层")?;
        writer.flush().map_err(|_| "无法写入完整译文图层")?;
        file.sync_all().map_err(|_| "无法保存完整译文图层")?;
        Ok::<_, String>(())
    })();
    drop(file);
    encoded?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{
        ConnectionAdvanced, ConnectionAuthMode, ConnectionProtocol, OcrDocument, OcrLine, OcrWord,
        Region, SceneCommand, Store,
    };
    use serde_json::{json, Value};
    use std::{sync::Mutex, thread};
    use tiny_http::{Header, Response, Server};

    fn profile(protocol: ConnectionProtocol, url: &str) -> ConnectionProfile {
        ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "synthetic connection".into(),
            provider_id: "custom".into(),
            base_url: url.into(),
            model: "synthetic".into(),
            has_key: false,
            credential_id: None,
            revision: 1,
            advanced: ConnectionAdvanced {
                protocol,
                auth_mode: ConnectionAuthMode::None,
                ..Default::default()
            },
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
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        }
    }
    fn origin() -> Origin {
        let background = asset();
        Origin {
            owner: "space".into(),
            target: OcrTarget {
                scene_id: "scene".into(),
                region_id: "region".into(),
                background_id: background.id.clone(),
                drawing_revision: 0,
                x: 0.,
                y: 0.,
                width: 200.,
                height: 100.,
            },
            background: background.clone(),
            source: background,
            connection: profile(ConnectionProtocol::ChatCompletions, "http://127.0.0.1:1/v1"),
            plugin_id: "mewu.translation".into(),
            plugin_revision: 1,
            contribution_id: "translate".into(),
        }
    }
    fn ocr(count: usize) -> OcrDocument {
        OcrDocument {
            engine: "synthetic".into(),
            language: "en".into(),
            width: 800,
            height: 600,
            text_angle: None,
            lines: (0..count)
                .map(|index| OcrLine {
                    text: format!("Original line {}", index + 1),
                    words: vec![OcrWord {
                        text: "word".into(),
                        x: 20.,
                        y: 20. + index as f64 * 20.,
                        width: 100.,
                        height: 16.,
                    }],
                })
                .collect(),
        }
    }
    fn translated(first: usize, count: usize) -> String {
        json!({"translations":(first..first+count).map(|index|json!({"id":format!("line-{index:04}"),"text":format!("译文 {index}")})).collect::<Vec<_>>()}).to_string()
    }
    fn chat(text: &str) -> String {
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
        )
    }
    struct Fixture {
        url: String,
        requests: Arc<Mutex<Vec<Value>>>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Fixture {
        fn new(responses: Vec<Option<String>>) -> Self {
            let server = Server::http((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            let url = format!("http://{}/v1/chat/completions", server.server_addr());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = requests.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let worker = thread::spawn(move || {
                let mut replies = responses.into_iter();
                while !stopped.load(Ordering::Acquire) {
                    let Some(mut request) = server.recv_timeout(Duration::from_millis(10)).unwrap()
                    else {
                        continue;
                    };
                    let mut bytes = String::new();
                    request.as_reader().read_to_string(&mut bytes).unwrap();
                    captured
                        .lock()
                        .unwrap()
                        .push(serde_json::from_str(&bytes).unwrap());
                    let Some(body) = replies.next().unwrap_or(Some(String::new())) else {
                        while !stopped.load(Ordering::Acquire) {
                            thread::sleep(Duration::from_millis(5));
                        }
                        break;
                    };
                    let _ = request.respond(Response::from_string(body).with_header(
                        Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                    ));
                }
            });
            Self {
                url,
                requests,
                stop,
                worker: Some(worker),
            }
        }
        fn preflight(&self, protocol: ConnectionProtocol) -> RunPreflight {
            let connection = profile(protocol, &self.url);
            RunPreflight {
                transport: crate::connection_host::transport_for(&connection).unwrap(),
                key: zeroize::Zeroizing::new(String::new()),
                connection,
            }
        }
        fn count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    #[test]
    fn different_scenes_run_independently_and_canceled_workers_keep_their_slot() {
        let mut registry = TranslationRegistry::default();
        let mut a = origin();
        a.target.scene_id = "A".into();
        let mut b = origin();
        b.target.scene_id = "B".into();
        let a_id = uuid::Uuid::new_v4().to_string();
        let b_id = uuid::Uuid::new_v4().to_string();
        let a_token = registry.start(&a_id, a.clone()).unwrap();
        let b_token = registry.start(&b_id, b.clone()).unwrap();
        assert!(registry.authorized(&a_id, &a_token));
        assert!(registry.authorized(&b_id, &b_token));
        registry.cancel_scene("settings", "A");
        assert!(registry.authorized(&a_id, &a_token));
        registry.cancel_scene("space", "A");
        assert!(!registry.authorized(&a_id, &a_token));
        assert!(registry.authorized(&b_id, &b_token));
        assert!(registry
            .start(&uuid::Uuid::new_v4().to_string(), a.clone())
            .is_err());
        registry.finish(&a_id);
        let a2_id = uuid::Uuid::new_v4().to_string();
        let a2_token = registry.start(&a2_id, a).unwrap();
        assert!(registry.authorized(&a2_id, &a2_token));
        assert!(registry.authorized(&b_id, &b_token));
    }

    #[test]
    fn same_scene_replacement_revokes_only_its_previous_job_and_full_capacity_is_non_destructive() {
        let mut registry = TranslationRegistry::default();
        let mut a = origin();
        a.target.scene_id = "A".into();
        let a_id = uuid::Uuid::new_v4().to_string();
        let a_token = registry.start(&a_id, a.clone()).unwrap();
        let a2_id = uuid::Uuid::new_v4().to_string();
        let a2_token = registry.start(&a2_id, a.clone()).unwrap();
        assert!(!registry.authorized(&a_id, &a_token));
        assert!(registry.authorized(&a2_id, &a2_token));
        let mut b = origin();
        b.target.scene_id = "B".into();
        assert!(registry
            .start(&uuid::Uuid::new_v4().to_string(), b.clone())
            .is_err());
        registry.finish(&a_id);
        let b_id = uuid::Uuid::new_v4().to_string();
        let b_token = registry.start(&b_id, b).unwrap();
        assert!(registry
            .start(&uuid::Uuid::new_v4().to_string(), a)
            .is_err());
        assert!(registry.authorized(&a2_id, &a2_token));
        assert!(registry.authorized(&b_id, &b_token));
        registry.cancel_owner("space");
        assert!(!registry.authorized(&a2_id, &a2_token));
        assert!(!registry.authorized(&b_id, &b_token));
    }

    #[test]
    fn registry_tombstones_owner_revision_source_and_connection_are_fenced() {
        let mut registry = TranslationRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        registry.cancel(&id, "space").unwrap();
        assert!(registry.start(&id, origin()).is_err());
        let first = uuid::Uuid::new_v4().to_string();
        let first_origin = origin();
        let token = registry.start(&first, first_origin.clone()).unwrap();
        assert!(registry.cancel(&first, "settings").is_err());
        registry.cancel_connection("unrelated");
        assert!(registry.authorized(&first, &token));
        registry.cancel_connection(&first_origin.connection.id);
        assert!(!registry.authorized(&first, &token));
        let second = uuid::Uuid::new_v4().to_string();
        let second_token = registry.start(&second, origin()).unwrap();
        assert!(registry
            .start(&uuid::Uuid::new_v4().to_string(), origin())
            .is_err());
        registry.finish(&first);
        let when = registry.recent[&first].at;
        registry.cancel_owner("space");
        registry.cancel(&first, "space").unwrap();
        assert_eq!(registry.recent[&first].at, when);
        assert!(!registry.authorized(&second, &second_token));
        registry.finish(&second);
        let third = uuid::Uuid::new_v4().to_string();
        let token = registry.start(&third, origin()).unwrap();
        registry.revoke_plugins(&PluginSnapshot {
            revision: 99,
            plugins: vec![],
        });
        assert!(!registry.authorized(&third, &token));
    }

    #[test]
    fn frozen_scene_stays_authorized_but_source_geometry_connection_and_close_revoke() {
        let mut store = Store::open_in_memory().unwrap();
        let mut snapshot = store.snapshot();
        let mut origin = origin();
        let scene = snapshot
            .scenes
            .iter_mut()
            .find(|scene| scene.id == snapshot.active_scene_id)
            .unwrap();
        origin.target.scene_id = scene.id.clone();
        scene.background = Some(origin.background.clone());
        scene.connection_id = Some(origin.connection.id.clone());
        scene.regions.push(Region {
            id: origin.target.region_id.clone(),
            x: 0.,
            y: 0.,
            width: 200.,
            height: 100.,
            ..Default::default()
        });
        snapshot.connections = vec![origin.connection.clone()];
        assert!(matches_snapshot(&snapshot, &origin));
        snapshot.scenes[0].frozen = true;
        assert!(matches_snapshot(&snapshot, &origin));
        for changed in 0..5 {
            let mut value = snapshot.clone();
            match changed {
                0 => value.scenes[0].regions[0].drawing_revision += 1,
                1 => value.connections[0].revision += 1,
                2 => value.scenes[0].closed = true,
                3 => value.scenes[0].regions[0].image_override = Some(asset()),
                _ => value.scenes[0].regions[0].width += 1.,
            }
            assert!(!matches_snapshot(&value, &origin));
        }
        // No registry operation writes or consumes conversation drafts/history.
        let id = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "keep me".into(),
            })
            .unwrap();
        let mut registry = TranslationRegistry::default();
        let token = registry
            .start(&uuid::Uuid::new_v4().to_string(), origin)
            .unwrap();
        registry.reconcile(&store.snapshot());
        assert!(token.check().is_err());
        assert_eq!(
            store
                .snapshot()
                .scenes
                .iter()
                .find(|scene| scene.id == id)
                .unwrap()
                .draft,
            "keep me"
        );
    }

    #[test]
    fn long_image_keeps_desktop_and_actual_source_distinct_across_unrelated_scene_changes() {
        let mut store = Store::open_in_memory().unwrap();
        let mut origin = origin();
        origin.target.scene_id = store.snapshot().active_scene_id;
        origin.target.region_id = uuid::Uuid::new_v4().to_string();
        store
            .set_background(&origin.target.scene_id, origin.background.clone())
            .unwrap();
        store
            .apply(SceneCommand::AddRegion {
                scene_id: origin.target.scene_id.clone(),
                region: Region {
                    id: origin.target.region_id.clone(),
                    x: 20.,
                    y: 30.,
                    width: 200.,
                    height: 100.,
                    ..Default::default()
                },
            })
            .unwrap();
        origin.target.x = 20.;
        origin.target.y = 30.;
        let mut replacement = asset();
        replacement.width = Some(200);
        replacement.height = Some(2000);
        let state = store
            .set_region_image(&origin.target, replacement.clone())
            .unwrap();
        let region = state
            .scenes
            .iter()
            .find(|scene| scene.id == origin.target.scene_id)
            .unwrap()
            .regions[0]
            .clone();
        origin.target.x = region.x;
        origin.target.y = region.y;
        origin.target.width = region.width;
        origin.target.height = region.height;
        origin.target.drawing_revision = region.drawing_revision;
        let (background, source, virtual_region) = region_source(&store, &origin.target).unwrap();
        assert_eq!(background, origin.background);
        assert_eq!(source, replacement);
        assert_ne!(background.id, source.id);
        assert_eq!(
            (
                virtual_region.x,
                virtual_region.y,
                virtual_region.width,
                virtual_region.height
            ),
            (0., 0., 200., 2000.)
        );
        assert!(virtual_region.image_override.is_none());
        origin.background = background;
        origin.source = source;
        let mut registry = TranslationRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let token = registry.start(&id, origin.clone()).unwrap();
        let project = |mut state: Snapshot| {
            state.connections = vec![origin.connection.clone()];
            state
                .scenes
                .iter_mut()
                .find(|scene| scene.id == origin.target.scene_id)
                .unwrap()
                .connection_id = Some(origin.connection.id.clone());
            state
        };
        for command in [
            SceneCommand::SetDraft {
                scene_id: origin.target.scene_id.clone(),
                draft: "继续输入".into(),
            },
            SceneCommand::SetRefs {
                scene_id: origin.target.scene_id.clone(),
                refs: vec![mewu_core::Reference {
                    kind: mewu_core::ReferenceKind::Region,
                    id: origin.target.region_id.clone(),
                }],
            },
            SceneCommand::FreezeScene {
                scene_id: origin.target.scene_id.clone(),
            },
        ] {
            let state = project(store.apply(command).unwrap());
            registry.reconcile(&state);
            assert!(registry.authorized(&id, &token));
            assert!(matches_snapshot(&state, &origin));
        }
        let mut state = project(store.snapshot());
        let region = &mut state
            .scenes
            .iter_mut()
            .find(|scene| scene.id == origin.target.scene_id)
            .unwrap()
            .regions[0];
        region.image_override.as_mut().unwrap().id = uuid::Uuid::new_v4().to_string();
        registry.reconcile(&state);
        assert!(!registry.authorized(&id, &token));
    }

    #[tokio::test]
    async fn three_protocols_translate_text_without_history_images_or_tool_authority() {
        for protocol in [
            ConnectionProtocol::ChatCompletions,
            ConnectionProtocol::AnthropicMessages,
            ConnectionProtocol::OpenAiResponses,
        ] {
            let answer = translated(1, 2);
            let stream = match protocol {
                ConnectionProtocol::ChatCompletions => chat(&answer),
                ConnectionProtocol::AnthropicMessages => {
                    crate::provider_transport::tests::anthropic(&answer, None)
                }
                ConnectionProtocol::OpenAiResponses => {
                    crate::provider_transport::tests::responses(&answer, None)
                }
            };
            let fixture = Fixture::new(vec![Some(stream)]);
            let plan = Plan::new(&ocr(2), "zh-Hans").unwrap();
            let result = translate_plan(
                plan,
                &fixture.preflight(protocol),
                &Cancellation::default(),
                || Ok(()),
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(result.lines[0].text, "译文 1");
            assert_eq!(fixture.count(), 1);
            let requests = fixture.requests.lock().unwrap();
            let body = &requests[0];
            assert!(body.get("tools").is_none());
            assert!(body.get("tool_choice").is_none());
            assert!(!body.to_string().contains("image_url"));
            assert!(!body.to_string().contains("memory"));
            if protocol == ConnectionProtocol::OpenAiResponses {
                assert_eq!(body["store"], false);
            }
        }
    }

    #[tokio::test]
    async fn malformed_or_incomplete_first_batch_does_not_retry_or_send_remaining_batches() {
        for stream in [
            chat(r#"{"translations":[]}"#),
            format!(
                "data: {}\n\n",
                json!({"choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}]})
            ),
            crate::provider_transport::tests::responses("", Some("unauthorized-tool")),
        ] {
            let protocol = if stream.contains("response.created") {
                ConnectionProtocol::OpenAiResponses
            } else {
                ConnectionProtocol::ChatCompletions
            };
            let fixture = Fixture::new(vec![Some(stream)]);
            let plan = Plan::new(&ocr(13), "en").unwrap();
            assert!(translate_plan(
                plan,
                &fixture.preflight(protocol),
                &Cancellation::default(),
                || Ok(()),
                |_| {}
            )
            .await
            .is_err());
            assert_eq!(fixture.count(), 1);
        }
    }

    #[tokio::test]
    async fn cancellation_drops_live_http_and_blocks_later_batches_and_completed_result() {
        let fixture = Fixture::new(vec![None]);
        let preflight = fixture.preflight(ConnectionProtocol::ChatCompletions);
        let token = Cancellation::default();
        let work = translate_plan(
            Plan::new(&ocr(1), "en").unwrap(),
            &preflight,
            &token,
            || Ok(()),
            |_| {},
        );
        tokio::pin!(work);
        tokio::select! {
            result=&mut work=>panic!("unexpected response: {}",result.is_ok()),
            _=async {while fixture.count()==0 {tokio::time::sleep(Duration::from_millis(5)).await;}}=>{},
        }
        token.cancel();
        assert!(tokio::time::timeout(Duration::from_millis(250), &mut work)
            .await
            .unwrap()
            .is_err());
        assert_eq!(fixture.count(), 1);
        let fixture = Fixture::new(vec![
            Some(chat(&translated(1, 12))),
            Some(chat(&translated(13, 1))),
        ]);
        let token = Cancellation::default();
        let result = translate_plan(
            Plan::new(&ocr(13), "en").unwrap(),
            &fixture.preflight(ConnectionProtocol::ChatCompletions),
            &token,
            || Ok(()),
            |_| token.cancel(),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fixture.count(), 1);
    }

    #[tokio::test]
    async fn oversized_stream_cancels_before_accepting_a_document() {
        let fixture = Fixture::new(vec![Some(chat(
            &"x".repeat(translation_document::MAX_RESPONSE_BYTES + 1),
        ))]);
        let result = translate_plan(
            Plan::new(&ocr(1), "en").unwrap(),
            &fixture.preflight(ConnectionProtocol::ChatCompletions),
            &Cancellation::default(),
            || Ok(()),
            |_| {},
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fixture.count(), 1);
    }

    #[test]
    fn uncommitted_overlay_is_removed_while_committed_pixels_remain_exact() {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let root = parent.join(format!(
            "mewu-translation-overlay-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 77]));
        let output = save_overlay(&root, &image).unwrap();
        let path = output.path.clone();
        assert_eq!(image::open(&path).unwrap().to_rgba8(), image);
        drop(output);
        assert!(!path.exists());
        let mut output = save_overlay(&root, &image).unwrap();
        output.retained = true;
        let path = output.path.clone();
        drop(output);
        assert!(path.exists());
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            root.canonicalize().unwrap().parent(),
            Some(parent.as_path())
        );
        std::fs::remove_dir(root).unwrap();
    }
}
