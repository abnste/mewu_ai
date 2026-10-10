// SPDX-License-Identifier: MPL-2.0
//! Native video metadata, range edits and exports. Renderers supply identities,
//! never paths or media durations. Accepted publication owns its real I/O lifetime.
use crate::{
    assets,
    export_variant_dialog::{self, ExportVariant, Media, Outcome as VariantOutcome},
    plugins::{self, GifGrant, PluginSnapshot},
    video_clip_contract as backend, video_clip_worker as worker,
    video_save_dialog::{self, SaveDialogOutcome, VideoSaveFormat},
    Host, HostSnapshot,
};
use crate::{clipboard_video_storage, table_clipboard};
use mewu_core::{Asset, Snapshot, VerifiedVideoSource, VideoExportOrigin, VideoRange, VideoTarget};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

const CANCELED: &str = "视频操作已取消";

#[derive(Clone, Debug, PartialEq)]
struct Stamp {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}
pub(crate) struct SourceLease {
    file: File,
    stamp: Stamp,
}
impl SourceLease {
    pub(crate) fn path(&self) -> &Path {
        &self.stamp.path
    }
    pub(crate) fn source_sha256(&self, cancel: &AtomicBool) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        let mut file = self.file.try_clone().map_err(|_| "无法读取视频")?;
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(0)).map_err(|_| "无法读取视频")?;
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 64 * 1024];
        loop {
            check_cancel(cancel)?;
            let count = file.read(&mut bytes).map_err(|_| "无法读取视频")?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    pub(crate) fn open(asset: &Asset, root: &Path) -> Result<Self, String> {
        let path = assets::verified_path(asset, root)?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Refuse replacement/writes for the entire inspect/render/copy lease.
            options.share_mode(1);
        }
        let file = options.open(&path).map_err(|_| "视频正在使用或无法读取")?;
        let (checked, length) = assets::open_video(asset, root)?;
        if same_file::Handle::from_file(file.try_clone().map_err(|_| "无法读取视频")?)
            .map_err(|_| "无法检查视频")?
            != same_file::Handle::from_file(checked).map_err(|_| "无法检查视频")?
        {
            return Err("视频来源已更改".into());
        }
        let metadata = file.metadata().map_err(|_| "无法读取视频")?;
        if metadata.len() != length {
            return Err("视频来源已更改".into());
        }
        Ok(Self {
            file,
            stamp: Stamp {
                path,
                length,
                modified: metadata.modified().ok(),
                created: metadata.created().ok(),
            },
        })
    }
}

#[derive(Clone)]
struct Cached {
    asset: Asset,
    stamp: Stamp,
    metadata: backend::VideoMetadata,
}
struct Pending {
    id: String,
    origin: VideoExportOrigin,
    cancel: Arc<AtomicBool>,
    publishing: bool,
    gif: Option<GifGrant>,
    drawing: Option<(plugins::VideoDrawingGrant, plugins::VideoDrawingTool)>,
}
#[derive(Default)]
struct Data {
    pending: Option<Pending>,
    recent: VecDeque<(String, Instant)>,
    cache: VecDeque<Cached>,
}
struct Inner {
    data: Mutex<Data>,
    work: Arc<Semaphore>,
    edits: Arc<Semaphore>,
}
#[derive(Clone)]
pub struct VideoRegistry(Arc<Inner>);
impl Default for VideoRegistry {
    fn default() -> Self {
        Self(Arc::new(Inner {
            data: Mutex::new(Data::default()),
            work: Arc::new(Semaphore::new(1)),
            edits: Arc::new(Semaphore::new(1)),
        }))
    }
}
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|v| v.hyphenated().to_string() == id)
}
fn remembers(data: &mut Data, id: &str) -> bool {
    while data
        .recent
        .front()
        .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(300))
    {
        data.recent.pop_front();
    }
    data.recent.iter().any(|(v, _)| v == id)
}
fn remember(data: &mut Data, id: &str) {
    if !remembers(data, id) {
        data.recent.push_back((id.into(), Instant::now()));
    }
    while data.recent.len() > 512 {
        data.recent.pop_front();
    }
}
impl VideoRegistry {
    pub(crate) fn begin(
        &self,
        id: &str,
        origin: VideoExportOrigin,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(Work, Arc<AtomicBool>), String> {
        if !valid_id(id) {
            return Err("视频请求无效".into());
        }
        let slot = self
            .0
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| "视频操作尚未结束")?;
        let mut data = self.0.data.lock().map_err(|_| "视频状态不可用")?;
        admit()?;
        if remembers(&mut data, id) {
            return Err(CANCELED.into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        data.pending = Some(Pending {
            id: id.into(),
            origin,
            cancel: cancel.clone(),
            publishing: false,
            gif: None,
            drawing: None,
        });
        Ok((
            Work {
                registry: self.clone(),
                id: id.into(),
                _slot: slot,
            },
            cancel,
        ))
    }
    fn cancel(&self, id: Option<&str>) -> Result<(), String> {
        let mut data = self.0.data.lock().map_err(|_| "视频状态不可用")?;
        if let Some(id) = id {
            remember(&mut data, id);
        }
        if let Some(pending) = &data.pending {
            if id.is_none_or(|id| id == pending.id) && !pending.publishing {
                pending.cancel.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
    fn active(&self, id: &str) -> bool {
        self.0
            .data
            .lock()
            .map(|data| data.pending.as_ref().is_some_and(|v| v.id == id))
            .unwrap_or(true)
    }
    fn cached(&self, asset: &Asset, stamp: &Stamp) -> Option<backend::VideoMetadata> {
        self.0
            .data
            .lock()
            .ok()?
            .cache
            .iter()
            .find(|v| &v.asset == asset && &v.stamp == stamp)
            .map(|v| v.metadata.clone())
    }
    fn cache(
        &self,
        asset: Asset,
        stamp: Stamp,
        metadata: backend::VideoMetadata,
    ) -> Result<(), String> {
        let mut data = self.0.data.lock().map_err(|_| "视频状态不可用")?;
        data.cache.retain(|v| v.asset.id != asset.id);
        data.cache.push_back(Cached {
            asset,
            stamp,
            metadata,
        });
        while data.cache.len() > 8 {
            data.cache.pop_front();
        }
        Ok(())
    }
    // Caller holds plugins -> Engine and has verified the exact package grant.
    fn bind_gif(&self, id: &str, grant: GifGrant) -> Result<(), String> {
        let mut data = self.0.data.lock().map_err(|_| "视频状态不可用")?;
        let pending = data
            .pending
            .as_mut()
            .filter(|v| v.id == id)
            .ok_or(CANCELED)?;
        check_cancel(&pending.cancel)?;
        if pending.publishing || pending.gif.is_some() {
            return Err("视频保存状态已更改".into());
        }
        pending.gif = Some(grant);
        Ok(())
    }
    // Exact authoring grant; document read/move/delete/export never bind it.
    pub(crate) fn bind_drawing(
        &self,
        id: &str,
        grant: plugins::VideoDrawingGrant,
        tool: plugins::VideoDrawingTool,
    ) -> Result<(), String> {
        let mut data = self.0.data.lock().map_err(|_| "视频状态不可用")?;
        let pending = data
            .pending
            .as_mut()
            .filter(|v| v.id == id)
            .ok_or(CANCELED)?;
        check_cancel(&pending.cancel)?;
        if pending.publishing || pending.gif.is_some() || pending.drawing.is_some() {
            return Err("视频绘制状态已更改".into());
        }
        pending.drawing = Some((grant, tool));
        Ok(())
    }
    fn revoke_gif(&self, snapshot: &PluginSnapshot) {
        if let Ok(data) = self.0.data.lock() {
            if let Some(pending) = &data.pending {
                if !pending.publishing
                    && (pending
                        .gif
                        .as_ref()
                        .is_some_and(|grant| !plugins::gif_grant_is_current(grant, snapshot))
                        || pending.drawing.as_ref().is_some_and(|(grant, tool)| {
                            !plugins::video_drawing_grant_is_current(grant, *tool, snapshot)
                        }))
                {
                    pending.cancel.store(true, Ordering::Release);
                }
            }
        }
    }
    /// Called with plugins -> Engine held; no I/O or native window getter here.
    fn accept_publication(&self, id: &str) -> Result<(), String> {
        let mut data = self.0.data.lock().map_err(|_| "视频状态不可用")?;
        let pending = data
            .pending
            .as_mut()
            .filter(|v| v.id == id)
            .ok_or(CANCELED)?;
        check_cancel(&pending.cancel)?;
        pending.publishing = true;
        Ok(())
    }
}
pub fn revoke_plugins(app: &AppHandle, snapshot: &PluginSnapshot) {
    app.state::<Host>().video.revoke_gif(snapshot);
}
pub(crate) struct Work {
    registry: VideoRegistry,
    id: String,
    _slot: OwnedSemaphorePermit,
}
impl Drop for Work {
    fn drop(&mut self) {
        if let Ok(mut data) = self.registry.0.data.lock() {
            if data.pending.as_ref().is_some_and(|v| v.id == self.id) {
                data.pending = None;
            }
            remember(&mut data, &self.id);
        }
    }
}
pub(crate) struct InvokeCancel(pub(crate) Arc<AtomicBool>);
impl Drop for InvokeCancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELED.into())
    } else {
        Ok(())
    }
}
fn ensure_space(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "space" {
        Ok(())
    } else {
        Err("此窗口不能操作视频".into())
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
        Err("当前系统不支持视频裁剪".into())
    }
}
pub(crate) fn atomic_gate(host: &Host, handle: usize, edit: bool) -> Result<(), String> {
    if edit {
        host.exit.ensure_background()?;
    } else {
        host.exit.ensure_running()?;
    }
    if host.capturing.load(Ordering::Acquire) {
        return Err("请先完成当前采集或空间切换".into());
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        if !unsafe { IsWindow(handle as _) != 0 && IsWindowVisible(handle as _) != 0 } {
            return Err("空间已收起".into());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
    }
    Ok(())
}
fn gate(app: &AppHandle, edit: bool) -> Result<(), String> {
    let host = app.state::<Host>();
    if edit {
        host.exit.ensure_background()?;
    } else {
        host.exit.ensure_running()?;
    }
    if host.capturing.load(Ordering::Acquire) {
        return Err("请先完成当前采集或空间切换".into());
    }
    crate::recording::ensure_idle(app)
}
fn origin(host: &Host, target: &VideoTarget) -> Result<VideoExportOrigin, String> {
    let engine = host.lock()?;
    let snapshot = engine.store.snapshot();
    let scene = snapshot
        .scenes
        .iter()
        .find(|v| v.id == target.scene_id)
        .ok_or("会话不存在")?;
    if snapshot.active_scene_id != target.scene_id || scene.closed || scene.frozen {
        return Err("视频所在会话已更改".into());
    }
    let source = engine
        .store
        .video_source(&target.scene_id, &target.item_id)
        .map_err(|e| e.to_string())?;
    if source.asset.id != target.source_id
        || source.edit.as_ref().map_or(0, |v| v.revision) != target.expected_revision
    {
        return Err("视频范围或来源已更改，请重新操作".into());
    }
    Ok(VideoExportOrigin {
        scene_id: target.scene_id.clone(),
        item_id: target.item_id.clone(),
        source,
    })
}
pub(crate) fn matches(snapshot: &Snapshot, origin: &VideoExportOrigin) -> bool {
    snapshot.active_scene_id == origin.scene_id
        && snapshot.scenes.iter().any(|s| {
            s.id == origin.scene_id
                && !s.frozen
                && !s.closed
                && s.items.iter().any(|i| {
                    i.id == origin.item_id
                        && i.asset == origin.source.asset
                        && i.video_edit == origin.source.edit
                        && i.video_annotations == origin.source.annotations
                })
        })
}
fn recheck(
    app: &AppHandle,
    handle: usize,
    origin: &VideoExportOrigin,
    cancel: &AtomicBool,
) -> Result<(), String> {
    check_cancel(cancel)?;
    gate(app, false)?;
    let host = app.state::<Host>();
    let engine = host.lock()?;
    atomic_gate(&host, handle, false)?;
    if !matches(&engine.store.snapshot(), origin) {
        return Err("视频范围或来源已更改，请重新操作".into());
    }
    check_cancel(cancel)
}
pub fn reconcile(app: &AppHandle, snapshot: &Snapshot) {
    let Some(host) = app.try_state::<Host>() else {
        return;
    };
    if let Ok(data) = host.video.0.data.lock() {
        if let Some(pending) = &data.pending {
            if !pending.publishing && !matches(snapshot, &pending.origin) {
                pending.cancel.store(true, Ordering::Release);
            }
        }
    };
}
pub fn cancel_all(app: &AppHandle) {
    if let Some(host) = app.try_state::<Host>() {
        let _ = host.video.cancel(None);
    }
}
pub(crate) fn cancel_scene(app: &AppHandle, scene_id: &str) {
    if let Some(host) = app.try_state::<Host>() {
        if let Ok(data) = host.video.0.data.lock() {
            if let Some(pending) = &data.pending {
                if pending.origin.scene_id == scene_id && !pending.publishing {
                    pending.cancel.store(true, Ordering::Release);
                }
            }
        }
    }
}
pub async fn drain(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    let registry = app.state::<Host>().video.clone();
    let started = Instant::now();
    while registry.0.work.available_permits() == 0
        || registry.0.edits.available_permits() == 0
        || worker::busy()
    {
        if started.elapsed() >= timeout {
            return Err("视频操作尚未结束，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}
#[tauri::command]
pub async fn cancel_video_request(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    ensure_space(&window)?;
    if !valid_id(&request_id) {
        return Err("视频请求无效".into());
    }
    let registry = app.state::<Host>().video.clone();
    registry.cancel(Some(&request_id))?;
    let started = Instant::now();
    while registry.active(&request_id) {
        if started.elapsed() >= Duration::from_secs(5) {
            return Err("视频操作尚未结束".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    request_id: String,
    target: VideoTarget,
    metadata: backend::VideoMetadata,
}
pub(crate) async fn native_job(
    job: backend::ClipJob,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(u8) + Send + Sync>,
) -> Result<backend::ClipJobResult, String> {
    let result = worker::run(job, cancel, progress).await;
    // A worker cleanup timeout must not free Host authority/IO leases early.
    while worker::busy() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    result.map_err(|e| match e {
        backend::ClipError::Cancelled => CANCELED.into(),
        backend::ClipError::InvalidRange => "视频范围无效".into(),
        _ => "无法处理视频，请重试".into(),
    })
}
pub(crate) async fn metadata(
    registry: &VideoRegistry,
    source: &Asset,
    lease: &SourceLease,
    cancel: Arc<AtomicBool>,
) -> Result<backend::VideoMetadata, String> {
    if let Some(value) = registry.cached(source, &lease.stamp) {
        check_cancel(&cancel)?;
        return Ok(value);
    }
    let value = match native_job(
        backend::ClipJob::Inspect {
            source: lease.stamp.path.clone(),
        },
        cancel.clone(),
        Arc::new(|_| {}),
    )
    .await?
    {
        backend::ClipJobResult::Inspected(value) => value,
        _ => return Err("视频信息无效".into()),
    };
    value.validate().map_err(|_| "视频信息无效")?;
    if source.width != Some(value.width) || source.height != Some(value.height) {
        return Err("视频尺寸已更改".into());
    }
    check_cancel(&cancel)?;
    registry.cache(source.clone(), lease.stamp.clone(), value.clone())?;
    Ok(value)
}
#[tauri::command]
pub async fn get_video_info(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    target: VideoTarget,
) -> Result<VideoInfo, String> {
    ensure_space(&window)?;
    gate(&app, false)?;
    let handle = window_handle(&window)?;
    let host = app.state::<Host>();
    let registry = host.video.clone();
    let origin = origin(&host, &target)?;
    let (work, cancel) = registry.begin(&request_id, origin.clone(), || {
        atomic_gate(&host, handle, false)
    })?;
    let guard = InvokeCancel(cancel.clone());
    let root = host.assets.clone();
    let (send, receive) = oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let result = async {
            recheck(&app, handle, &origin, &cancel)?;
            let asset = origin.source.asset.clone();
            let lease =
                tauri::async_runtime::spawn_blocking(move || SourceLease::open(&asset, &root))
                    .await
                    .map_err(|_| "视频读取中断")??;
            let metadata =
                metadata(&registry, &origin.source.asset, &lease, cancel.clone()).await?;
            recheck(&app, handle, &origin, &cancel)?;
            Ok(VideoInfo {
                request_id,
                target,
                metadata,
            })
        }
        .await;
        drop(work);
        let _ = send.send(result);
    });
    let result = receive.await.map_err(|_| "视频读取中断")?;
    drop(guard);
    result
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VideoEditAction {
    Set {
        #[serde(deserialize_with = "required_range")]
        from: Option<VideoRange>,
        #[serde(deserialize_with = "required_range")]
        to: Option<VideoRange>,
    },
    Undo {
        #[serde(deserialize_with = "required_range")]
        from: Option<VideoRange>,
        #[serde(rename = "operationId")]
        operation_id: String,
    },
    Redo {
        #[serde(deserialize_with = "required_range")]
        from: Option<VideoRange>,
        #[serde(rename = "operationId")]
        operation_id: String,
    },
}
fn required_range<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<VideoRange>, D::Error> {
    Option::deserialize(d)
}
#[tauri::command]
pub async fn apply_video_edit(
    app: AppHandle,
    window: WebviewWindow,
    plugin_id: String,
    revision: u64,
    contribution_id: String,
    target: VideoTarget,
    action: VideoEditAction,
) -> Result<HostSnapshot, String> {
    ensure_space(&window)?;
    gate(&app, true)?;
    let handle = window_handle(&window)?;
    let registry = app.state::<Host>().video.clone();
    let slot = registry
        .0
        .edits
        .clone()
        .try_acquire_owned()
        .map_err(|_| "视频范围正在保存")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _slot = slot;
        gate(&app, true)?;
        let host = app.state::<Host>();
        let origin = origin(&host, &target)?;
        let lease = SourceLease::open(&origin.source.asset, &host.assets)?;
        let meta = registry
            .cached(&origin.source.asset, &lease.stamp)
            .ok_or("请先读取视频信息")?;
        if origin
            .source
            .edit
            .as_ref()
            .is_some_and(|v| v.source_duration_ticks != meta.duration_ticks)
        {
            return Err("视频时长已更改".into());
        }
        let snapshot = {
            let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
            plugins
                .store
                .video_trim(&plugin_id, revision, &contribution_id)?;
            let mut engine = host.lock()?;
            gate(&app, true)?;
            atomic_gate(&host, handle, true)?;
            if !engine.store.video_export_matches(&origin) {
                return Err("视频范围或来源已更改，请重新操作".into());
            }
            match action {
                VideoEditAction::Set { from, to } => {
                    let verified = VerifiedVideoSource {
                        asset: origin.source.asset.clone(),
                        duration_ticks: meta.duration_ticks,
                        width: meta.width,
                        height: meta.height,
                    };
                    engine.store.set_video_range(&target, &verified, from, to)
                }
                VideoEditAction::Undo { from, operation_id } => {
                    engine.store.undo_video_range(&target, &operation_id, from)
                }
                VideoEditAction::Redo { from, operation_id } => {
                    engine.store.redo_video_range(&target, &operation_id, from)
                }
            }
            .map_err(|e| e.to_string())?
        };
        Ok(crate::publish(&app, &snapshot))
    })
    .await
    .map_err(|_| "视频范围保存中断")?
}

struct Staging(PathBuf);
impl Staging {
    fn create(root: &Path, format: VideoSaveFormat) -> Result<Self, String> {
        let extension = match format {
            VideoSaveFormat::Mp4 => "mp4",
            VideoSaveFormat::Gif => "gif",
        };
        let path =
            root.join("video-staging")
                .join(format!("{}.{}", uuid::Uuid::new_v4(), extension));
        // Backend creates this exact new file. Startup only cleans direct,
        // ordinary UUID.mp4/UUID.gif leftovers in this separate staging directory.
        if path.exists() {
            return Err("视频临时文件冲突".into());
        }
        Ok(Self(path))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    request_id: &'a str,
    percent: u8,
}
fn publish_file(
    mut input: File,
    length: u64,
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    if destination == source
        || (destination.exists()
            && same_file::is_same_file(destination, source).map_err(|_| "无法检查保存位置")?)
    {
        return Err("请选择其他保存位置".into());
    }
    let temporary = destination.with_file_name(format!(".mewu-{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "无法保存视频")?;
        let copied = std::io::copy(&mut Read::by_ref(&mut input).take(length), &mut output)
            .map_err(|_| "无法保存视频")?;
        if copied != length {
            return Err("视频读取不完整".into());
        }
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|_| "无法完成视频保存")?;
        drop(output);
        std::fs::rename(&temporary, destination).map_err(|_| "无法完成视频保存".into())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

// One precise MP4 encoder route shared by save and file-copy. Callers retain
// their existing metadata/source-duration admission and their own output guard.
// This intentionally does not touch the GIF encoder, grant or validation path.
async fn render_precise_mp4(
    source: PathBuf,
    output: PathBuf,
    expected: backend::VideoMetadata,
    range: backend::VideoRange,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(u8) + Send + Sync>,
) -> Result<(), String> {
    match native_job(
        backend::ClipJob::Render {
            source,
            output,
            expected,
            range,
        },
        cancel,
        progress,
    )
    .await?
    {
        backend::ClipJobResult::Rendered(_) => Ok(()),
        _ => Err("视频保存结果无效".into()),
    }
}

async fn prepare_overlays(
    app: &AppHandle,
    handle: usize,
    origin: &VideoExportOrigin,
    cancel: Arc<AtomicBool>,
) -> Result<Option<crate::video_overlay_host::StagedOverlays>, String> {
    if !crate::video_overlay_host::has_overlays(origin) {
        return Ok(None);
    }
    let worker = crate::visual_annotation_host::TABLE_WORKERS
        .acquire()
        .await
        .map_err(|_| "视频标注处理不可用")?;
    let root = app.state::<Host>().root.clone();
    let app = app.clone();
    let origin = origin.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _worker = worker;
        crate::video_overlay_host::StagedOverlays::create(&root, &origin, || {
            recheck(&app, handle, &origin, &cancel)
        })
        .map(Some)
    })
    .await
    .map_err(|_| "视频标注处理中断")?
}

async fn render_materialized_mp4(
    source: PathBuf,
    output: PathBuf,
    expected: backend::VideoMetadata,
    range: Option<backend::VideoRange>,
    overlays: Option<&crate::video_overlay_host::StagedOverlays>,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(u8) + Send + Sync>,
) -> Result<(), String> {
    if let Some(overlays) = overlays {
        match native_job(
            backend::ClipJob::RenderAnnotated {
                source,
                output: output.clone(),
                expected: expected.clone(),
                range: range.clone(),
                overlays: overlays.overlays.clone(),
            },
            cancel.clone(),
            progress,
        )
        .await?
        {
            backend::ClipJobResult::Rendered(_) => (),
            _ => return Err("视频保存结果无效".into()),
        }
        let actual = match native_job(
            backend::ClipJob::Inspect { source: output },
            cancel,
            Arc::new(|_| {}),
        )
        .await?
        {
            backend::ClipJobResult::Inspected(actual) => actual,
            _ => return Err("导出视频信息无效".into()),
        };
        let duration = range
            .as_ref()
            .map_or(expected.duration_ticks, |r| r.end_ticks - r.start_ticks);
        if actual.width != expected.width
            || actual.height != expected.height
            || actual.has_audio != expected.has_audio
            || actual.duration_ticks.abs_diff(duration) > 1_000_000
        {
            return Err("导出视频信息不一致".into());
        }
        Ok(())
    } else {
        render_precise_mp4(
            source,
            output,
            expected,
            range.ok_or("视频范围无效")?,
            cancel,
            progress,
        )
        .await
    }
}

fn publish_stable_copy(
    stable: clipboard_video_storage::StableVideo,
    publish: impl FnOnce(&Path) -> table_clipboard::FileDropPublication,
) -> Result<bool, String> {
    match publish(stable.path()) {
        table_clipboard::FileDropPublication::Published { warning: _ } => {
            // The primary format is committed. Optional/close warnings and a
            // late cancellation cannot remove its file or change this result.
            Ok(true)
        }
        table_clipboard::FileDropPublication::Unpublished { error } => {
            // Only this explicit outcome permits deletion. A panic unwinds
            // through StableVideo's non-deleting Drop and preserves the file.
            if let Err(cleanup) = stable.discard_unpublished() {
                return Err(format!("{error}；{cleanup}"));
            }
            if error == CANCELED {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}
#[tauri::command]
pub async fn export_video(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    target: VideoTarget,
) -> Result<(), String> {
    ensure_space(&window)?;
    gate(&app, false)?;
    let handle = window_handle(&window)?;
    let host = app.state::<Host>();
    let registry = host.video.clone();
    let origin = origin(&host, &target)?;
    let (work, cancel) = registry.begin(&request_id, origin.clone(), || {
        atomic_gate(&host, handle, false)
    })?;
    let guard = InvokeCancel(cancel.clone());
    let root = host.assets.clone();
    let staging_root = host.root.clone();
    let (send, receive) = oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let result = async {
            recheck(&app, handle, &origin, &cancel)?;
            let asset = origin.source.asset.clone();
            let picker_app = app.clone();
            let picker_origin = origin.clone();
            let picker_cancel = cancel.clone();
            // Candidate availability affects only the picker. A candidate is
            // bound to the request only after GIF is explicitly selected.
            let candidate = {
                let host = app.state::<Host>();
                // An unavailable plugin store must not disable basic MP4 save.
                let candidate = host
                    .plugins
                    .lock()
                    .ok()
                    .and_then(|plugins| plugins.store.gif_candidate().ok().flatten());
                candidate
            };
            let allow_gif = candidate.is_some();
            let prepared = tauri::async_runtime::spawn_blocking(move || {
                let lease = SourceLease::open(&asset, &root)?;
                recheck(&picker_app, handle, &picker_origin, &picker_cancel)?;
                let allowed_cancel = picker_cancel.clone();
                let destination = video_save_dialog::pick_video_destination(
                    handle as isize,
                    allow_gif,
                    picker_cancel,
                    move || recheck(&picker_app, handle, &picker_origin, &allowed_cancel),
                )
                .map_err(|e| e.to_string())?;
                Ok::<_, String>((lease, destination))
            })
            .await
            .map_err(|_| "视频保存中断")??;
            let (lease, chosen) = prepared;
            let (destination, format) = match chosen {
                SaveDialogOutcome::Selected { path, format } => (path, format),
                SaveDialogOutcome::UserCanceled => return Ok(()),
                SaveDialogOutcome::RequestCanceled => return Err(CANCELED.into()),
            };
            recheck(&app, handle, &origin, &cancel)?;
            let gif = if format == VideoSaveFormat::Gif {
                let grant = candidate.ok_or("GIF 导出插件未启用")?;
                let host = app.state::<Host>();
                let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
                plugins.store.video_gif(
                    &grant.plugin_id,
                    grant.revision,
                    &grant.contribution_id,
                )?;
                let engine = host.lock()?;
                atomic_gate(&host, handle, false)?;
                if !matches(&engine.store.snapshot(), &origin) {
                    return Err("视频范围或来源已更改，请重新操作".into());
                }
                registry.bind_gif(&request_id, grant.clone())?;
                Some(grant)
            } else {
                None
            };
            let choice_app = app.clone();
            let choice_origin = origin.clone();
            let choice_cancel = cancel.clone();
            let variant = tauri::async_runtime::spawn_blocking(move || {
                let allowed_cancel = choice_cancel.clone();
                export_variant_dialog::pick_variant(
                    handle as isize,
                    Media::Video,
                    choice_origin
                        .source
                        .annotations
                        .as_ref()
                        .is_some_and(|document| !document.objects.is_empty()),
                    choice_cancel,
                    move || recheck(&choice_app, handle, &choice_origin, &allowed_cancel),
                )
            })
            .await
            .map_err(|_| "视频导出选项已中断")??;
            let variant = match variant {
                VariantOutcome::Selected(variant) => variant,
                VariantOutcome::UserCanceled => return Ok(()),
                VariantOutcome::RequestCanceled => return Err(CANCELED.into()),
            };
            recheck(&app, handle, &origin, &cancel)?;
            let destination_parent = destination
                .parent()
                .ok_or("保存位置无效")?
                .canonicalize()
                .map_err(|_| "保存目录不存在")?;
            let protected_root = staging_root.canonicalize().map_err(|_| "应用目录不可用")?;
            if destination_parent.starts_with(&protected_root) {
                return Err("请选择应用数据目录以外的保存位置".into());
            }
            let source_path = lease.stamp.path.clone();
            let mut staging = None;
            let range = origin.source.edit.as_ref().and_then(|v| v.range);
            let overlays = if variant == ExportVariant::Annotated {
                prepare_overlays(&app, handle, &origin, cancel.clone()).await?
            } else {
                None
            };
            recheck(&app, handle, &origin, &cancel)?;
            let (input, length) =
                if format == VideoSaveFormat::Gif || range.is_some() || overlays.is_some() {
                    let meta =
                        metadata(&registry, &origin.source.asset, &lease, cancel.clone()).await?;
                    recheck(&app, handle, &origin, &cancel)?;
                    if origin
                        .source
                        .edit
                        .as_ref()
                        .is_some_and(|v| v.source_duration_ticks != meta.duration_ticks)
                    {
                        return Err("视频时长已更改".into());
                    }
                    let temp = Staging::create(&staging_root, format)?;
                    let output = temp.0.clone();
                    let progress_app = app.clone();
                    let progress_id = request_id.clone();
                    let progress = Arc::new(move |percent| {
                        let _ = progress_app.emit_to(
                            "space",
                            "video-export-progress",
                            Progress {
                                request_id: &progress_id,
                                percent,
                            },
                        );
                    });
                    let range = range.map(|r| backend::VideoRange {
                        start_ticks: r.start_ticks,
                        end_ticks: r.end_ticks,
                    });
                    match format {
                        VideoSaveFormat::Gif => {
                            let job = if let Some(overlays) = &overlays {
                                backend::ClipJob::RenderGifAnnotated {
                                    source: source_path.clone(),
                                    output: output.clone(),
                                    expected: meta,
                                    range,
                                    overlays: overlays.overlays.clone(),
                                }
                            } else {
                                backend::ClipJob::RenderGif {
                                    source: source_path.clone(),
                                    output: output.clone(),
                                    expected: meta,
                                    range,
                                }
                            };
                            match native_job(job, cancel.clone(), progress).await? {
                                backend::ClipJobResult::GifRendered(_) => (),
                                _ => return Err("视频保存结果无效".into()),
                            }
                        }
                        VideoSaveFormat::Mp4 => {
                            render_materialized_mp4(
                                source_path.clone(),
                                output.clone(),
                                meta,
                                range,
                                overlays.as_ref(),
                                cancel.clone(),
                                progress,
                            )
                            .await?
                        }
                    }
                    recheck(&app, handle, &origin, &cancel)?;
                    let input = File::open(&output).map_err(|_| "无法读取导出文件")?;
                    let length = input.metadata().map_err(|_| "无法读取导出文件")?.len();
                    let limit = if format == VideoSaveFormat::Gif {
                        backend::MAX_GIF_BYTES
                    } else {
                        assets::MAX_VIDEO_BYTES
                    };
                    if !(24..=limit).contains(&length) {
                        return Err("导出文件大小无效".into());
                    }
                    staging = Some(temp);
                    (input, length)
                } else {
                    (
                        lease.file.try_clone().map_err(|_| "无法读取视频")?,
                        lease.stamp.length,
                    )
                };
            // Capture acceptance under plugins -> Engine -> registry, then perform the actual
            // file publication without either lock. Edits after this point do not
            // revoke an export whose concrete snapshot has already been accepted.
            {
                let host = app.state::<Host>();
                gate(&app, false)?;
                let plugins = if gif.is_some() {
                    Some(host.plugins.lock().map_err(|_| "插件状态不可用")?)
                } else {
                    None
                };
                if let (Some(plugins), Some(grant)) = (&plugins, &gif) {
                    plugins.store.video_gif(
                        &grant.plugin_id,
                        grant.revision,
                        &grant.contribution_id,
                    )?;
                }
                let engine = host.lock()?;
                atomic_gate(&host, handle, false)?;
                if !matches(&engine.store.snapshot(), &origin) {
                    return Err("视频范围或来源已更改，请重新操作".into());
                }
                registry.accept_publication(&request_id)?;
            }
            tauri::async_runtime::spawn_blocking(move || {
                let _source_lease = lease;
                let _staging = staging;
                publish_file(input, length, &source_path, &destination)
            })
            .await
            .map_err(|_| "视频保存中断")??;
            Ok(())
        }
        .await;
        drop(work);
        let _ = send.send(result);
    });
    let result = receive.await.map_err(|_| "视频保存中断")?;
    drop(guard);
    result
}

/// Copy a materialized MP4 file; renderer supplies identities only. `true`
/// means the primary CF_HDROP format was accepted. `false` means canceled
/// before publication. No return value contains the source or cache path.
#[tauri::command]
pub async fn copy_video(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    target: VideoTarget,
) -> Result<bool, String> {
    ensure_space(&window)?;
    gate(&app, false)?;
    let handle = window_handle(&window)?;
    let host = app.state::<Host>();
    let registry = host.video.clone();
    let origin = origin(&host, &target)?;
    let admitted = registry.begin(&request_id, origin.clone(), || {
        atomic_gate(&host, handle, false)
    });
    let (work, cancel) = match admitted {
        Ok(value) => value,
        Err(error) if error == CANCELED => return Ok(false),
        Err(error) => return Err(error),
    };
    let guard = InvokeCancel(cancel.clone());
    let assets_root = host.assets.clone();
    let data_root = host.root.clone();
    let (send, receive) = oneshot::channel();
    // Dropping the IPC waiter only cancels its token. This task retains Work
    // through actual child exit and the complete blocking file/clipboard work.
    tauri::async_runtime::spawn(async move {
        let result = async {
            recheck(&app, handle, &origin, &cancel)?;
            let asset = origin.source.asset.clone();
            let lease = tauri::async_runtime::spawn_blocking(move || {
                SourceLease::open(&asset, &assets_root)
            })
            .await
            .map_err(|_| "视频复制中断")??;
            recheck(&app, handle, &origin, &cancel)?;
            let mut staging = None;
            let saved_range = origin.source.edit.as_ref().and_then(|edit| edit.range);
            let overlays = prepare_overlays(&app, handle, &origin, cancel.clone()).await?;
            let (mut input, length) = if saved_range.is_some() || overlays.is_some() {
                let meta =
                    metadata(&registry, &origin.source.asset, &lease, cancel.clone()).await?;
                if origin
                    .source
                    .edit
                    .as_ref()
                    .is_some_and(|edit| edit.source_duration_ticks != meta.duration_ticks)
                {
                    return Err("视频时长已更改".into());
                }
                let temporary = Staging::create(&data_root, VideoSaveFormat::Mp4)?;
                let output = temporary.0.clone();
                let progress_app = app.clone();
                let progress_id = request_id.clone();
                let progress = Arc::new(move |percent| {
                    let _ = progress_app.emit_to(
                        "space",
                        "video-export-progress",
                        Progress {
                            request_id: &progress_id,
                            percent,
                        },
                    );
                });
                render_materialized_mp4(
                    lease.stamp.path.clone(),
                    output.clone(),
                    meta,
                    saved_range.map(|range| backend::VideoRange {
                        start_ticks: range.start_ticks,
                        end_ticks: range.end_ticks,
                    }),
                    overlays.as_ref(),
                    cancel.clone(),
                    progress,
                )
                .await?;
                // The existing backend validated the precise result and native_job
                // waited for child/pipe exit. No second encoder is involved.
                recheck(&app, handle, &origin, &cancel)?;
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.share_mode(1); // keep the completed candidate immutable while copied
                }
                let input = options.open(&output).map_err(|_| "无法读取复制文件")?;
                let length = input.metadata().map_err(|_| "无法读取复制文件")?.len();
                if !(24..=assets::MAX_VIDEO_BYTES).contains(&length) {
                    return Err("复制文件大小无效".into());
                }
                staging = Some(temporary);
                (input, length)
            } else {
                // Full-source copy bypasses metadata decoding and encoding.
                (
                    lease.file.try_clone().map_err(|_| "无法读取视频")?,
                    lease.stamp.length,
                )
            };
            recheck(&app, handle, &origin, &cancel)?;
            tauri::async_runtime::spawn_blocking(move || {
                let _source_lease = lease;
                let _staging = staging;
                // reserve(length) enforces the stable-file budget before this
                // one physical copy. StableVideo Drop never deletes the file.
                let copied = (|| {
                    let storage = clipboard_video_storage::ClipboardVideoStorage::open(&data_root)?;
                    let reservation = storage.reserve(length)?;
                    reservation.copy_from(&mut input, &cancel)
                })();
                // The trimmed input denies DELETE sharing. Close it before any
                // error can drop Staging and attempt to remove that candidate.
                drop(input);
                let stable = copied?;
                publish_stable_copy(stable, |path| {
                    table_clipboard::publish_file_drop(path, || {
                        // Called while WRITER and the OS clipboard are held. No
                        // file I/O, window getter, gate/recheck, or await here.
                        let host = app.state::<Host>();
                        let engine = host.lock()?;
                        atomic_gate(&host, handle, false)?;
                        if !matches(&engine.store.snapshot(), &origin) {
                            return Err("视频范围或来源已更改，请重新操作".into());
                        }
                        registry.accept_publication(&request_id)
                    })
                })
            })
            .await
            .map_err(|_| "视频复制中断")?
        }
        .await;
        // Cancellation of copy/render before the sink has published nothing.
        // Never infer canceled from the token after the sink accepted CF_HDROP.
        let result = match result {
            Err(error) if error == CANCELED => Ok(false),
            result => result,
        };
        drop(work);
        let _ = send.send(result);
    });
    let result = receive.await.map_err(|_| "视频复制中断")?;
    drop(guard);
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Admission/revocation proof used by each same-build real media gate.
    /// The caller supplies only its fresh synthetic media output directory.
    pub(crate) fn core_capabilities_native_admission(root: &Path) {
        let registry_root = root.join("core-capability-registry");
        std::fs::create_dir(&registry_root).unwrap();
        let store = plugins::PluginStore::open(&registry_root).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert!(snapshot.plugins.iter().all(|record| !matches!(
            record.manifest.id.as_str(),
            "mewu.drawing" | "mewu.recording"
        )));
        assert_eq!(
            store
                .drawing_tools(plugins::CORE_DRAWING, 1, "drawing-tools")
                .unwrap()
                .len(),
            9
        );
        assert_eq!(
            store
                .video_drawing_tools(plugins::CORE_DRAWING, 1, "video-drawing-tools")
                .unwrap()
                .len(),
            7
        );
        assert!(store
            .recording(plugins::CORE_RECORDING, 1, "record")
            .is_ok());
        assert!(store
            .recording_audio(plugins::CORE_RECORDING, 1, "audio")
            .is_ok());
        assert!(store.video_trim(plugins::CORE_RECORDING, 1, "trim").is_ok());
        assert!(store.video_gif(plugins::CORE_RECORDING, 1, "gif").is_ok());
        assert!(store
            .video_drawing_tools(plugins::CORE_DRAWING, 2, "video-drawing-tools")
            .is_err());
        assert!(store
            .video_drawing_tools(plugins::CORE_DRAWING, 1, "drawing-tools")
            .is_err());
        assert!(store
            .recording(plugins::CORE_RECORDING, 2, "record")
            .is_err());
        assert!(store
            .recording_audio(plugins::CORE_RECORDING, 1, "record")
            .is_err());
        assert!(store.video_trim(plugins::CORE_RECORDING, 1, "gif").is_err());
        assert!(store.video_gif(plugins::CORE_RECORDING, 1, "trim").is_err());
        let legacy = PluginSnapshot {
            revision: 2,
            plugins: [
                include_bytes!("../../plugins/migrations/official-drawing-1.2.0.json").as_slice(),
                include_bytes!("../../plugins/migrations/official-recording-1.0.0.json").as_slice(),
            ]
            .into_iter()
            .map(|bytes| plugins::PluginRecord {
                manifest: plugins::parse_manifest(bytes).unwrap(),
                source: plugins::PluginSource::Official,
                revision: 2,
                state: plugins::PluginState::Disabled,
                has_rollback: false,
                error: None,
            })
            .collect(),
        };
        for drawing in [false, true] {
            let registry = VideoRegistry::default();
            let id = uuid::Uuid::new_v4().to_string();
            let (work, cancel) = registry.begin(&id, fixture_origin(), || Ok(())).unwrap();
            if drawing {
                registry
                    .bind_drawing(
                        &id,
                        plugins::VideoDrawingGrant {
                            plugin_id: plugins::CORE_DRAWING.into(),
                            revision: 1,
                            contribution_id: "video-drawing-tools".into(),
                        },
                        plugins::VideoDrawingTool::Pen,
                    )
                    .unwrap();
            } else {
                registry
                    .bind_gif(&id, store.gif_candidate().unwrap().unwrap())
                    .unwrap();
            }
            for plugins in [&snapshot, &empty_plugins(), &legacy] {
                registry.revoke_gif(plugins);
                assert!(!cancel.load(Ordering::Acquire));
            }
            assert!(registry.active(&id));
            assert_eq!(registry.0.work.available_permits(), 0);
            registry.cancel(Some(&id)).unwrap();
            assert!(cancel.load(Ordering::Acquire));
            assert!(registry
                .begin(&uuid::Uuid::new_v4().to_string(), fixture_origin(), || Ok(
                    ()
                ))
                .is_err());
            drop(work);
            assert_eq!(registry.0.work.available_permits(), 1);
            let (next, _) = registry
                .begin(&uuid::Uuid::new_v4().to_string(), fixture_origin(), || {
                    Ok(())
                })
                .unwrap();
            drop(next);
        }
        assert!(!registry_root.join("recording-preferences.json").exists());
    }

    #[test]
    fn core_capabilities_keep_real_video_registry_ownership_after_legacy_revocation() {
        let root = std::env::temp_dir().join(format!(
            "mewu-core-capture-registry-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        core_capabilities_native_admission(&root);
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mewu-core-capture-registry-"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    struct CopyFixture {
        root: PathBuf,
        origin: VideoExportOrigin,
    }
    #[cfg(windows)]
    impl CopyFixture {
        fn new() -> Self {
            let parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
            let root = parent.join(format!("mewu-video-copy-host-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let mut origin = fixture_origin();
            let source = root.join(format!("{}.mp4", origin.source.asset.id));
            // Container signature fixture only; these host tests run no codec.
            std::fs::write(&source, b"\x00\x00\x00\x18ftypisom\0\0\0\0isomiso2").unwrap();
            origin.source.asset.path = source.to_string_lossy().into_owned();
            Self { root, origin }
        }
        fn copy(&self) -> (SourceLease, clipboard_video_storage::StableVideo) {
            let mut lease = SourceLease::open(&self.origin.source.asset, &self.root).unwrap();
            let storage = clipboard_video_storage::ClipboardVideoStorage::open(&self.root).unwrap();
            let stable = storage
                .reserve(lease.stamp.length)
                .unwrap()
                .copy_from(&mut lease.file, &AtomicBool::new(false))
                .unwrap();
            (lease, stable)
        }
    }
    #[cfg(windows)]
    impl Drop for CopyFixture {
        fn drop(&mut self) {
            // Only this fixture's direct known directories/files; no recursive
            // deletion or traversal of any link supplied outside the fixture.
            for name in ["clipboard-videos", "video-staging"] {
                let directory = self.root.join(name);
                if let Ok(entries) = std::fs::read_dir(&directory) {
                    for entry in entries.flatten() {
                        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                    let _ = std::fs::remove_dir(directory);
                }
            }
            let _ = std::fs::remove_file(&self.origin.source.asset.path);
            let _ = std::fs::remove_dir(&self.root);
        }
    }

    #[cfg(windows)]
    #[test]
    fn unpublished_copy_cancel_removes_only_candidate_and_preserves_original() {
        let _serial = clipboard_video_storage::test_serial_guard();
        let fixture = CopyFixture::new();
        let before = std::fs::read(&fixture.origin.source.asset.path).unwrap();
        let (_lease, stable) = fixture.copy();
        let path = stable.path().to_owned();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!same_file::is_same_file(&path, &fixture.origin.source.asset.path).unwrap());
        assert!(!publish_stable_copy(stable, |_| {
            table_clipboard::FileDropPublication::Unpublished {
                error: CANCELED.into(),
            }
        })
        .unwrap());
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(&fixture.origin.source.asset.path).unwrap(),
            before
        );
    }

    #[cfg(windows)]
    #[test]
    fn confirmed_copy_survives_waiter_cancel_and_holds_source_and_slot_until_sink_exits() {
        let _serial = clipboard_video_storage::test_serial_guard();
        let fixture = CopyFixture::new();
        let (lease, stable) = fixture.copy();
        let path = stable.path().to_owned();
        let registry = VideoRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (work, cancel) = registry
            .begin(&id, fixture.origin.clone(), || Ok(()))
            .unwrap();
        let waiter = InvokeCancel(cancel.clone());
        let accepted = registry.clone();
        let accepted_id = id.clone();
        let (ready, started) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _work = work;
            let _source_lease = lease;
            publish_stable_copy(stable, |_| {
                accepted.accept_publication(&accepted_id).unwrap();
                ready.send(()).unwrap();
                blocked.recv_timeout(Duration::from_secs(5)).unwrap();
                table_clipboard::FileDropPublication::Published {
                    warning: Some("合成辅助格式失败".into()),
                }
            })
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(waiter); // This direct token cancellation also occurs after Publishing.
        registry.cancel(Some(&id)).unwrap();
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry.active(&id));
        assert_eq!(registry.0.work.available_permits(), 0);
        assert!(registry
            .begin(
                &uuid::Uuid::new_v4().to_string(),
                fixture.origin.clone(),
                || Ok(())
            )
            .is_err());
        #[cfg(windows)]
        assert!(std::fs::OpenOptions::new()
            .write(true)
            .open(&fixture.origin.source.asset.path)
            .is_err());
        release.send(()).unwrap();
        assert!(thread.join().unwrap().unwrap());
        assert!(path.exists());
        assert_eq!(registry.0.work.available_permits(), 1);
        assert!(!registry.active(&id));
    }

    #[cfg(windows)]
    #[test]
    fn unknown_copy_publication_keeps_stable_file_after_thread_panic() {
        let _serial = clipboard_video_storage::test_serial_guard();
        let fixture = CopyFixture::new();
        let (_lease, stable) = fixture.copy();
        let path = stable.path().to_owned();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            publish_stable_copy(stable, |_| panic!("synthetic uncertain publication"))
        }));
        assert!(result.is_err());
        assert!(path.exists());
        assert_eq!(
            std::fs::read(path).unwrap(),
            std::fs::read(&fixture.origin.source.asset.path).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "explicit synthetic audio MP4 backend plus file-copy; no clipboard or devices"]
    fn synthetic_audio_mp4_full_and_precise_range_copy_keep_bytes_audio_and_lifetime() {
        use sha2::{Digest, Sha256};
        let _serial = clipboard_video_storage::test_serial_guard();
        let fixtures = PathBuf::from(
            std::env::var_os("MEWU_SYNTHETIC_MEDIA_ROOT")
                .expect("set MEWU_SYNTHETIC_MEDIA_ROOT to the registered synthetic fixtures"),
        )
        .canonicalize()
        .unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixtures.join("inputs.json")).unwrap()).unwrap();
        assert_eq!(manifest["syntheticOnly"], true);
        let registered_source = fixtures.join("synthetic-audio.mp4");
        let original_bytes = std::fs::read(&registered_source).unwrap();
        let original_hash = Sha256::digest(&original_bytes);
        let fixture = CopyFixture::new();
        std::fs::write(&fixture.origin.source.asset.path, &original_bytes).unwrap();
        crate::recording_storage::prepare_video(&fixture.root).unwrap();
        let mut lease = SourceLease::open(&fixture.origin.source.asset, &fixture.root).unwrap();
        let cancel = AtomicBool::new(false);
        let storage = clipboard_video_storage::ClipboardVideoStorage::open(&fixture.root).unwrap();

        // This opt-in test follows the existing direct-codec probe pattern.
        // It does not exercise a child process: libtest is not a worker EXE.
        let backend::ClipJobResult::Inspected(expected) = crate::video_clip_backend::execute(
            &backend::ClipJob::Inspect {
                source: lease.stamp.path.clone(),
            },
            &cancel,
            &|_| {},
        )
        .unwrap() else {
            panic!("inspect result")
        };
        assert!(expected.has_audio);
        let full = storage
            .reserve(lease.stamp.length)
            .unwrap()
            .copy_from(&mut lease.file, &cancel)
            .unwrap();
        let full_path = full.path().to_owned();
        assert!(!same_file::is_same_file(&full_path, &lease.stamp.path).unwrap());
        let copied = std::fs::read(&full_path).unwrap();
        assert_eq!(copied, original_bytes);
        assert_eq!(Sha256::digest(&copied), original_hash);
        assert!(publish_stable_copy(full, |_| {
            table_clipboard::FileDropPublication::Published { warning: None }
        })
        .unwrap());
        assert!(full_path.is_file());

        let range = backend::VideoRange {
            start_ticks: 12_500_000,
            end_ticks: 42_500_000,
        };
        let staging = Staging::create(&fixture.root, VideoSaveFormat::Mp4).unwrap();
        let backend::ClipJobResult::Rendered(rendered) = crate::video_clip_backend::execute(
            &backend::ClipJob::Render {
                source: lease.stamp.path.clone(),
                output: staging.0.clone(),
                expected: expected.clone(),
                range: range.clone(),
            },
            &cancel,
            &|_| {},
        )
        .unwrap() else {
            panic!("precise MP4 result")
        };
        backend::validate_rendered(&expected, &range, &rendered).unwrap();
        assert!(rendered.has_audio);
        let mut input = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&staging.0)
                .unwrap()
        };
        let trimmed_bytes = std::fs::read(&staging.0).unwrap();
        let trimmed = storage
            .reserve(input.metadata().unwrap().len())
            .unwrap()
            .copy_from(&mut input, &cancel)
            .unwrap();
        drop(input);
        let trimmed_path = trimmed.path().to_owned();
        assert_eq!(
            Sha256::digest(std::fs::read(&trimmed_path).unwrap()),
            Sha256::digest(&trimmed_bytes)
        );
        let backend::ClipJobResult::Inspected(cached_metadata) =
            crate::video_clip_backend::execute(
                &backend::ClipJob::Inspect {
                    source: trimmed_path.clone(),
                },
                &cancel,
                &|_| {},
            )
            .unwrap()
        else {
            panic!("cached MP4 inspect result")
        };
        backend::validate_rendered(&expected, &range, &cached_metadata).unwrap();
        assert!(cached_metadata.has_audio);
        assert!(publish_stable_copy(trimmed, |_| {
            table_clipboard::FileDropPublication::Published {
                warning: Some("合成辅助格式失败".into()),
            }
        })
        .unwrap());
        let temporary_path = staging.0.clone();
        drop(staging);
        assert!(!temporary_path.exists());
        assert!(trimmed_path.is_file());
        assert!(full_path.is_file());
        assert_eq!(
            Sha256::digest(std::fs::read(&lease.stamp.path).unwrap()),
            original_hash
        );
        assert_eq!(
            Sha256::digest(std::fs::read(&registered_source).unwrap()),
            original_hash
        );
    }
    fn gif_grant() -> GifGrant {
        GifGrant {
            plugin_id: "mewu.gif".into(),
            revision: 1,
            contribution_id: "gif".into(),
        }
    }
    fn empty_plugins() -> PluginSnapshot {
        PluginSnapshot {
            revision: 2,
            plugins: vec![],
        }
    }
    #[test]
    fn unbound_picker_and_mp4_export_ignore_gif_revocation() {
        let registry = VideoRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (_work, cancel) = registry.begin(&id, fixture_origin(), || Ok(())).unwrap();
        registry.revoke_gif(&empty_plugins());
        assert!(!cancel.load(Ordering::Acquire));
        registry.accept_publication(&id).unwrap();
    }
    #[test]
    fn bound_gif_revocation_cancels_before_publication_but_keeps_real_slot() {
        let registry = VideoRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (work, cancel) = registry.begin(&id, fixture_origin(), || Ok(())).unwrap();
        registry.bind_gif(&id, gif_grant()).unwrap();
        registry.revoke_gif(&empty_plugins());
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry.accept_publication(&id).is_err());
        assert!(registry.bind_gif(&id, gif_grant()).is_err());
        assert_eq!(registry.0.work.available_permits(), 0);
        drop(work);
        assert_eq!(registry.0.work.available_permits(), 1);
    }
    #[test]
    fn accepted_gif_publication_survives_revocation_and_refuses_rebinding() {
        let registry = VideoRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (_work, cancel) = registry.begin(&id, fixture_origin(), || Ok(())).unwrap();
        registry.bind_gif(&id, gif_grant()).unwrap();
        assert!(registry.bind_gif(&id, gif_grant()).is_err());
        registry.accept_publication(&id).unwrap();
        registry.revoke_gif(&empty_plugins());
        assert!(!cancel.load(Ordering::Acquire));
        assert_eq!(registry.0.work.available_permits(), 0);
    }
    fn fixture_origin() -> VideoExportOrigin {
        let id = uuid::Uuid::new_v4().to_string();
        VideoExportOrigin {
            scene_id: "scene".into(),
            item_id: "item".into(),
            source: mewu_core::VideoSourceView {
                asset: Asset {
                    id: id.clone(),
                    name: "synthetic.mp4".into(),
                    kind: mewu_core::AssetKind::Video,
                    path: format!("{id}.mp4"),
                    width: Some(320),
                    height: Some(180),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                },
                edit: None,
                annotations: None,
            },
        }
    }
    #[test]
    fn admission_is_checked_under_cancel_lock_and_failed_admission_releases_slot() {
        let registry = VideoRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let result = registry.begin(&id, fixture_origin(), || {
            assert!(matches!(
                registry.0.data.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Err("space already hidden".into())
        });
        assert!(result.is_err());
        assert!(!registry.active(&id));
        assert_eq!(registry.0.work.available_permits(), 1);
    }

    #[test]
    fn early_cancel_never_revives_and_old_cancel_cannot_stop_next_request() {
        let registry = VideoRegistry::default();
        let early = uuid::Uuid::new_v4().to_string();
        registry.cancel(Some(&early)).unwrap();
        assert!(registry.begin(&early, fixture_origin(), || Ok(())).is_err());
        let current = uuid::Uuid::new_v4().to_string();
        let (work, cancel) = registry
            .begin(&current, fixture_origin(), || Ok(()))
            .unwrap();
        registry.cancel(Some(&early)).unwrap();
        assert!(!cancel.load(Ordering::Acquire));
        assert!(registry
            .begin(&uuid::Uuid::new_v4().to_string(), fixture_origin(), || Ok(
                ()
            ))
            .is_err());
        registry.cancel(Some(&current)).unwrap();
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry.active(&current));
        drop(work);
        assert!(!registry.active(&current));
        assert!(registry
            .begin(&current, fixture_origin(), || Ok(()))
            .is_err());
    }
    #[test]
    fn accepted_publication_survives_later_cancel_but_keeps_real_slot() {
        let registry = VideoRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (work, cancel) = registry.begin(&id, fixture_origin(), || Ok(())).unwrap();
        registry.accept_publication(&id).unwrap();
        registry.cancel(Some(&id)).unwrap();
        registry.cancel(None).unwrap();
        assert!(!cancel.load(Ordering::Acquire));
        let clone = registry.clone();
        let thread = std::thread::spawn(move || {
            assert!(clone.active(&id));
            assert_eq!(clone.0.work.available_permits(), 0);
            drop(work);
        });
        thread.join().unwrap();
        assert_eq!(registry.0.work.available_permits(), 1);
    }
    #[test]
    fn cancellation_before_publication_rejects_acceptance() {
        let registry = VideoRegistry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (_work, _) = registry.begin(&id, fixture_origin(), || Ok(())).unwrap();
        registry.cancel(None).unwrap();
        assert!(registry.accept_publication(&id).is_err());
    }
    #[test]
    fn action_wire_requires_explicit_nullable_ranges_and_rejects_unknown_fields() {
        assert!(
            serde_json::from_str::<VideoEditAction>(r#"{"type":"set","from":null,"to":null}"#)
                .is_ok()
        );
        for value in [
            r#"{"type":"set","to":null}"#,
            r#"{"type":"undo","operationId":"a"}"#,
            r#"{"type":"set","from":null,"to":null,"path":"x"}"#,
        ] {
            assert!(serde_json::from_str::<VideoEditAction>(value).is_err());
        }
    }
    #[test]
    fn file_publication_copies_exact_bytes_and_refuses_same_source() {
        let root =
            std::env::temp_dir().join(format!("mewu-video-publish-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.mp4");
        let output = root.join("output.mp4");
        std::fs::write(&source, b"owned synthetic bytes").unwrap();
        let file = File::open(&source).unwrap();
        assert!(publish_file(file, 21, &source, &source).is_err());
        publish_file(File::open(&source).unwrap(), 21, &source, &output).unwrap();
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&output).unwrap()
        );
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn incomplete_copy_preserves_existing_destination_and_removes_only_own_temp() {
        let root = std::env::temp_dir().join(format!("mewu-video-short-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.mp4");
        let output = root.join("output.mp4");
        std::fs::write(&source, b"short").unwrap();
        std::fs::write(&output, b"keep").unwrap();
        assert!(publish_file(File::open(&source).unwrap(), 100, &source, &output).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
