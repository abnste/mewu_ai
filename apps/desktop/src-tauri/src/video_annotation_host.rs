// SPDX-License-Identifier: MPL-2.0
//! Persisted video documents remain usable without their authoring extension.
use crate::{video_annotation_raster as raster, Host, HostSnapshot};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use mewu_core::{
    Store, TimedVideoAnnotation, VerifiedVideoTextLayout, VideoAnnotationDocument,
    VideoAnnotationDraft, VideoAnnotationMutation, VideoAnnotationPrimitive, VideoAnnotationTarget,
    VideoExportOrigin, VideoPixelPoint, VideoPixelRect, VideoRange, VideoSourceClock,
    VideoTextContent, VideoTextLayoutRef,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;

static REQUESTS: Semaphore = Semaphore::const_new(16);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub annotation_id: String,
    pub interval: VideoRange,
    pub bounds: VideoPixelRect,
    pub reference: raster::RasterRef,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub target: VideoAnnotationTarget,
    pub document_sha256: String,
    pub clock: VideoSourceClock,
    pub source_duration_ticks: u64,
    pub source_width: u32,
    pub source_height: u32,
    pub range: VideoRange,
    pub entries: Vec<PlanEntry>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewTarget {
    pub target: VideoAnnotationTarget,
    pub document_sha256: String,
    pub annotation_id: String,
    pub reference: raster::RasterRef,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    #[serde(flatten)]
    pub target: PreviewTarget,
    pub data_url: String,
}
#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DocumentAction {
    Move {
        annotation_id: String,
        from_top_left: VideoPixelPoint,
        to_top_left: VideoPixelPoint,
    },
    Remove {
        annotation_id: String,
    },
    Undo {
        operation_id: String,
    },
    Redo {
        operation_id: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
    pub request_id: String,
    pub target: VideoAnnotationTarget,
    pub annotation_id: String,
    pub reference: VideoTextLayoutRef,
    pub content: VideoTextContent,
}

/// Both asynchronous commands share the existing video request/cancel/drain
/// registry. A dropped invoke revokes submission, while its real blocking job
/// retains Work and TABLE_WORKERS until the file read/render has actually ended.
struct TextJob {
    _work: crate::video_host::Work,
    cancel: Arc<AtomicBool>,
    handle: usize,
    origin: VideoExportOrigin,
}

fn window_handle(window: &WebviewWindow) -> Result<usize, String> {
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|v| v.0 as usize)
            .map_err(|_| "空间窗口不可用".into())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Ok(0)
    }
}

fn checked_origin(
    store: &Store,
    target: &VideoAnnotationTarget,
    editing: bool,
) -> Result<VideoExportOrigin, String> {
    Ok(VideoExportOrigin {
        scene_id: target.scene_id.clone(),
        item_id: target.item_id.clone(),
        source: store
            .video_annotation_source(target, editing)
            .map_err(|e| e.to_string())?,
    })
}

fn same_origin(
    store: &Store,
    target: &VideoAnnotationTarget,
    origin: &VideoExportOrigin,
    editing: bool,
) -> Result<(), String> {
    if checked_origin(store, target, editing)? != *origin {
        return Err("视频范围或批注已更改，请重试".into());
    }
    Ok(())
}

fn object<'a>(
    doc: &'a VideoAnnotationDocument,
    annotation_id: &str,
) -> Result<&'a TimedVideoAnnotation, String> {
    doc.objects
        .iter()
        .find(|o| o.id == annotation_id)
        .ok_or_else(|| "视频批注已不存在".into())
}

fn exact_text<'a>(
    doc: &'a VideoAnnotationDocument,
    annotation_id: &str,
    reference: &VideoTextLayoutRef,
) -> Result<&'a TimedVideoAnnotation, String> {
    let object = object(doc, annotation_id)?;
    if !matches!(&object.primitive, VideoAnnotationPrimitive::Text { layout, .. } if layout == reference)
    {
        return Err("视频文字图层已更改".into());
    }
    Ok(object)
}

fn move_mutation(
    doc: &VideoAnnotationDocument,
    annotation_id: &str,
    from: VideoPixelPoint,
    to: VideoPixelPoint,
) -> Result<VideoAnnotationMutation, String> {
    let object = object(doc, annotation_id)?;
    // Manual vector ink may extend outside source; its control geometry may not.
    if let VideoAnnotationPrimitive::Vector { top_left, layout } = &object.primitive {
        let g = layout.geometry_bounds;
        if *top_left != from {
            return Err("视频批注位置已更改".into());
        }
        if !to.x.is_finite()
            || !to.y.is_finite()
            || to.x.fract() != 0.
            || to.y.fract() != 0.
            || to.x + g.x < 0.
            || to.y + g.y < 0.
            || to.x + g.x + g.width > f64::from(doc.source_width)
            || to.y + g.y + g.height > f64::from(doc.source_height)
        {
            return Err("视频绘制位置超出原视频".into());
        }
        return Ok(VideoAnnotationMutation::Update {
            id: object.id.clone(),
            replacement: VideoAnnotationDraft {
                interval: object.interval,
                primitive: VideoAnnotationPrimitive::Vector {
                    top_left: to,
                    layout: layout.clone(),
                },
            },
        });
    }
    let (old, width, height) = match &object.primitive {
        VideoAnnotationPrimitive::Rect { bounds, .. } => (
            VideoPixelPoint {
                x: bounds.x,
                y: bounds.y,
            },
            bounds.width,
            bounds.height,
        ),
        VideoAnnotationPrimitive::Text { top_left, layout } => {
            (*top_left, f64::from(layout.width), f64::from(layout.height))
        }
        VideoAnnotationPrimitive::Vector { .. } => unreachable!("handled above"),
    };
    if old != from {
        return Err("视频批注位置已更改".into());
    }
    if !to.x.is_finite()
        || !to.y.is_finite()
        || to.x < 0.
        || to.y < 0.
        || to.x + width > f64::from(doc.source_width)
        || to.y + height > f64::from(doc.source_height)
    {
        return Err("视频批注位置超出原视频".into());
    }
    let mut primitive = object.primitive.clone();
    match &mut primitive {
        VideoAnnotationPrimitive::Rect { bounds, .. } => {
            bounds.x = to.x;
            bounds.y = to.y;
        }
        VideoAnnotationPrimitive::Text { top_left, .. } => *top_left = to,
        VideoAnnotationPrimitive::Vector { .. } => unreachable!("handled above"),
    }
    Ok(VideoAnnotationMutation::Update {
        id: object.id.clone(),
        replacement: VideoAnnotationDraft {
            interval: object.interval,
            primitive,
        },
    })
}

/// Cheap, literal-text admission precedes the renderer and its shared slot.
/// These are render_text's existing public content limits, not a second layout.
pub(crate) fn validate_content(content: &VideoTextContent) -> Result<(), String> {
    if content.version != 1
        || content.text.trim().is_empty()
        || content.text.len() > 4096
        || content.text.chars().count() > 500
        || content
            .text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        || !content.font_size.is_finite()
        || !(8. ..=128.).contains(&content.font_size)
        || content.color.len() != 7
        || !content.color.starts_with('#')
        || !content.color.as_bytes()[1..]
            .iter()
            .all(u8::is_ascii_hexdigit)
    {
        return Err("视频批注文字无效".into());
    }
    Ok(())
}

fn prepare_text(
    doc: &VideoAnnotationDocument,
    annotation_id: &str,
    reference: &VideoTextLayoutRef,
    previous: &VerifiedVideoTextLayout,
    content: VideoTextContent,
    canceled: &AtomicBool,
) -> Result<(VideoAnnotationMutation, Vec<VerifiedVideoTextLayout>), String> {
    crate::video_host::check_cancel(canceled)?;
    validate_content(&content)?;
    let old = exact_text(doc, annotation_id, reference)?;
    if previous.reference() != reference {
        return Err("视频文字图层已更改".into());
    }
    let (new_ref, layouts) = if previous.content() == &content {
        (reference.clone(), vec![])
    } else {
        let rendered = raster::render_text(content)?;
        crate::video_host::check_cancel(canceled)?;
        (rendered.reference().clone(), vec![rendered])
    };
    let VideoAnnotationPrimitive::Text { top_left, .. } = &old.primitive else {
        return Err("视频批注不是文字".into());
    };
    if top_left.x + f64::from(new_ref.width) > f64::from(doc.source_width)
        || top_left.y + f64::from(new_ref.height) > f64::from(doc.source_height)
    {
        return Err("视频文字布局超出原视频".into());
    }
    Ok((
        VideoAnnotationMutation::Update {
            id: old.id.clone(),
            replacement: VideoAnnotationDraft {
                interval: old.interval,
                primitive: VideoAnnotationPrimitive::Text {
                    top_left: *top_left,
                    layout: new_ref,
                },
            },
        },
        layouts,
    ))
}

fn commit_text(
    store: &mut Store,
    target: &VideoAnnotationTarget,
    origin: &VideoExportOrigin,
    mutation: VideoAnnotationMutation,
    layouts: Vec<VerifiedVideoTextLayout>,
    canceled: &AtomicBool,
) -> Result<mewu_core::Snapshot, String> {
    same_origin(store, target, origin, true)?;
    crate::video_host::check_cancel(canceled)?;
    // Layout registration and object/history replacement are one Store SQLite
    // transaction. Never persist a rendered layout separately on this path.
    store
        .edit_video_annotation_with_layouts(target, mutation, layouts)
        .map_err(|e| e.to_string())
}

fn admit_text_job(
    app: &AppHandle,
    window: &WebviewWindow,
    request_id: &str,
    target: &VideoAnnotationTarget,
    annotation_id: &str,
    reference: &VideoTextLayoutRef,
    editing: bool,
) -> Result<TextJob, String> {
    ensure_owner(window.label())?;
    let handle = window_handle(window)?; // no Tauri getter inside Engine/registry locks
    crate::recording::ensure_idle(app)?;
    let host = app.state::<Host>();
    let engine = host.lock()?;
    crate::recording::ensure_idle(app)?;
    crate::video_host::atomic_gate(&host, handle, editing)?;
    let origin = checked_origin(&engine.store, target, editing)?;
    exact_text(
        origin.source.annotations.as_ref().ok_or("视频尚无批注")?,
        annotation_id,
        reference,
    )?;
    let (work, cancel) = host.video.begin(request_id, origin.clone(), || {
        crate::video_host::atomic_gate(&host, handle, editing)
    })?;
    Ok(TextJob {
        _work: work,
        cancel,
        handle,
        origin,
    })
}

fn check_text_job(
    app: &AppHandle,
    job: &TextJob,
    target: &VideoAnnotationTarget,
    editing: bool,
) -> Result<(), String> {
    crate::video_host::check_cancel(&job.cancel)?;
    crate::recording::ensure_idle(app)?;
    let host = app.state::<Host>();
    let engine = host.lock()?;
    crate::recording::ensure_idle(app)?;
    crate::video_host::atomic_gate(&host, job.handle, editing)?;
    same_origin(&engine.store, target, &job.origin, editing)?;
    crate::video_host::check_cancel(&job.cancel)
}

#[tauri::command]
pub async fn get_video_annotation_text(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    target: VideoAnnotationTarget,
    annotation_id: String,
    reference: VideoTextLayoutRef,
) -> Result<TextContent, String> {
    let request = REQUESTS.try_acquire().map_err(|_| "视频批注正在处理")?;
    let job = admit_text_job(
        &app,
        &window,
        &request_id,
        &target,
        &annotation_id,
        &reference,
        false,
    )?;
    let guard = crate::video_host::InvokeCancel(job.cancel.clone());
    let worker = crate::visual_annotation_host::TABLE_WORKERS
        .acquire()
        .await
        .map_err(|_| "视频批注处理不可用")?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        check_text_job(&app, &job, &target, false)?;
        let layout =
            Store::read_video_text_layout(app.state::<Host>().root.join("spaces.db"), &reference)
                .map_err(|e| e.to_string())?;
        let content = layout.content().clone();
        check_text_job(&app, &job, &target, false)?;
        Ok(TextContent {
            request_id,
            target,
            annotation_id,
            reference,
            content,
        })
    })
    .await
    .map_err(|_| "视频文字读取中断")?;
    drop(guard);
    result
}

#[tauri::command]
pub async fn edit_video_annotation_text(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    target: VideoAnnotationTarget,
    annotation_id: String,
    reference: VideoTextLayoutRef,
    content: VideoTextContent,
) -> Result<HostSnapshot, String> {
    validate_content(&content)?;
    let request = REQUESTS.try_acquire().map_err(|_| "视频批注正在处理")?;
    let job = admit_text_job(
        &app,
        &window,
        &request_id,
        &target,
        &annotation_id,
        &reference,
        true,
    )?;
    let guard = crate::video_host::InvokeCancel(job.cancel.clone());
    let worker = crate::visual_annotation_host::TABLE_WORKERS
        .acquire()
        .await
        .map_err(|_| "视频批注处理不可用")?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        check_text_job(&app, &job, &target, true)?;
        let old =
            Store::read_video_text_layout(app.state::<Host>().root.join("spaces.db"), &reference)
                .map_err(|e| e.to_string())?;
        let (mutation, layouts) = prepare_text(
            job.origin
                .source
                .annotations
                .as_ref()
                .ok_or("视频尚无批注")?,
            &annotation_id,
            &reference,
            &old,
            content,
            &job.cancel,
        )?;
        let snapshot = {
            let host = app.state::<Host>();
            let mut engine = host.lock()?;
            crate::recording::ensure_idle(&app)?;
            crate::video_host::atomic_gate(&host, job.handle, true)?;
            // Acceptance boundary: later cancellation cannot undo a successful
            // transaction or turn its truthful success receipt into canceled.
            commit_text(
                &mut engine.store,
                &target,
                &job.origin,
                mutation,
                layouts,
                &job.cancel,
            )?
        };
        crate::video_host::reconcile(&app, &snapshot);
        let result = crate::publish(&app, &snapshot);
        Ok(result)
    })
    .await
    .map_err(|_| "视频文字保存中断")?;
    drop(guard);
    result
}

pub fn document_hash(doc: &VideoAnnotationDocument) -> Result<String, String> {
    let bytes = serde_json::to_vec(&Some(doc)).map_err(|_| "视频批注文档无效")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn ensure_owner(owner: &str) -> Result<(), String> {
    if owner != "space" {
        return Err("此窗口不能操作视频批注".into());
    }
    Ok(())
}
fn read_document(
    app: &AppHandle,
    owner: &str,
    target: &VideoAnnotationTarget,
) -> Result<(VideoAnnotationDocument, VideoRange), String> {
    ensure_owner(owner)?;
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    let engine = host.lock()?;
    let snapshot = engine.store.snapshot();
    let scene = snapshot
        .scenes
        .iter()
        .find(|s| s.id == target.scene_id && !s.closed && !s.frozen)
        .filter(|s| snapshot.active_scene_id == s.id)
        .ok_or("视频所在会话已更改")?;
    let item = scene
        .items
        .iter()
        .find(|i| i.id == target.item_id)
        .ok_or("视频已不存在")?;
    if item.asset.kind != mewu_core::AssetKind::Video
        || item.asset.id != target.source_id
        || item.video_edit.as_ref().map_or(0, |v| v.revision) != target.expected_range_revision
        || item.video_annotations.as_ref().map_or(0, |v| v.revision)
            != target.expected_annotation_revision
    {
        return Err("视频范围或批注已更改，请重试".into());
    }
    let doc = item.video_annotations.clone().ok_or("视频尚无批注")?;
    let range = item
        .video_edit
        .as_ref()
        .and_then(|v| v.range)
        .unwrap_or(VideoRange {
            start_ticks: 0,
            end_ticks: doc.source_duration_ticks,
        });
    Ok((doc, range))
}
fn render_object(
    root: &Path,
    object: &TimedVideoAnnotation,
    source_width: u32,
    source_height: u32,
) -> Result<raster::Raster, String> {
    raster::render_in_source(
        object,
        source_width,
        source_height,
        |reference| {
            Store::read_video_text_layout(root.join("spaces.db"), reference)
                .map_err(|e| e.to_string())
        },
        |reference| {
            Store::read_video_vector_layout(root.join("spaces.db"), reference)
                .map_err(|e| e.to_string())
        },
    )
}
pub fn render_plan(
    target: VideoAnnotationTarget,
    doc: &VideoAnnotationDocument,
    range: VideoRange,
    root: &Path,
) -> Result<Plan, String> {
    let mut entries = Vec::with_capacity(doc.objects.len());
    let (mut bytes, mut pixels) = (0usize, 0u64);
    for object in &doc.objects {
        let image = render_object(root, object, doc.source_width, doc.source_height)?;
        bytes = bytes
            .checked_add(image.png.len())
            .ok_or("视频批注图片过大")?;
        pixels = pixels
            .checked_add(u64::from(image.reference.width) * u64::from(image.reference.height))
            .ok_or("视频批注图层过大")?;
        if bytes > raster::MAX_TOTAL_PNG_BYTES || pixels > raster::MAX_TOTAL_PIXELS {
            return Err("视频批注图层预算超限".into());
        }
        entries.push(PlanEntry {
            annotation_id: object.id.clone(),
            interval: object.interval,
            bounds: image.bounds,
            reference: image.reference,
        });
        // PNG is not retained by this metadata plan or a process-global cache.
    }
    Ok(Plan {
        target,
        document_sha256: document_hash(doc)?,
        clock: doc.clock,
        source_duration_ticks: doc.source_duration_ticks,
        source_width: doc.source_width,
        source_height: doc.source_height,
        range,
        entries,
    })
}

#[tauri::command]
pub async fn get_video_annotation_plan(
    app: AppHandle,
    window: WebviewWindow,
    target: VideoAnnotationTarget,
) -> Result<Plan, String> {
    let request = REQUESTS.try_acquire().map_err(|_| "视频批注正在处理")?;
    let worker = crate::visual_annotation_host::TABLE_WORKERS
        .acquire()
        .await
        .map_err(|_| "视频批注处理不可用")?;
    let owner = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        let (doc, range) = read_document(&app, &owner, &target)?;
        let plan = render_plan(target.clone(), &doc, range, &app.state::<Host>().root)?;
        let (latest, latest_range) = read_document(&app, &owner, &target)?;
        if latest != doc || latest_range != range {
            return Err("视频批注已更改，请重试".into());
        }
        Ok(plan)
    })
    .await
    .map_err(|_| "视频批注读取中断")?
}
#[tauri::command]
pub async fn get_video_annotation_preview(
    app: AppHandle,
    window: WebviewWindow,
    target: VideoAnnotationTarget,
    document_sha256: String,
    annotation_id: String,
    reference: raster::RasterRef,
) -> Result<Preview, String> {
    let request = REQUESTS.try_acquire().map_err(|_| "视频批注正在处理")?;
    let worker = crate::visual_annotation_host::TABLE_WORKERS
        .acquire()
        .await
        .map_err(|_| "视频批注处理不可用")?;
    let owner = window.label().to_string();
    let preview_target = PreviewTarget {
        target,
        document_sha256,
        annotation_id,
        reference,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        let (doc, range) = read_document(&app, &owner, &preview_target.target)?;
        if document_hash(&doc)? != preview_target.document_sha256 {
            return Err("视频批注文档已更改".into());
        }
        let object = doc
            .objects
            .iter()
            .find(|o| o.id == preview_target.annotation_id)
            .ok_or("视频批注已不存在")?;
        let image = render_object(
            &app.state::<Host>().root,
            object,
            doc.source_width,
            doc.source_height,
        )?;
        if image.reference != preview_target.reference {
            return Err("视频批注图片已更改".into());
        }
        let (latest, latest_range) = read_document(&app, &owner, &preview_target.target)?;
        if latest != doc || latest_range != range {
            return Err("视频批注已更改，请重试".into());
        }
        Ok(Preview {
            target: preview_target,
            data_url: format!("data:image/png;base64,{}", STANDARD.encode(image.png)),
        })
    })
    .await
    .map_err(|_| "视频批注读取中断")?
}
#[tauri::command]
pub fn apply_video_annotation_document(
    app: AppHandle,
    window: WebviewWindow,
    target: VideoAnnotationTarget,
    action: DocumentAction,
) -> Result<HostSnapshot, String> {
    ensure_owner(window.label())?;
    let handle = window_handle(&window)?;
    let exit_flush = matches!(&action, DocumentAction::Move { .. });
    crate::recording::ensure_idle(&app)?;
    let host = app.state::<Host>();
    let mut engine = host.lock()?;
    crate::recording::ensure_idle(&app)?;
    crate::video_host::atomic_gate(&host, handle, exit_flush)?;
    let mutation = match action {
        DocumentAction::Move {
            annotation_id,
            from_top_left,
            to_top_left,
        } => {
            let origin = checked_origin(&engine.store, &target, true)?;
            move_mutation(
                origin.source.annotations.as_ref().ok_or("视频尚无批注")?,
                &annotation_id,
                from_top_left,
                to_top_left,
            )?
        }
        DocumentAction::Remove { annotation_id } => {
            VideoAnnotationMutation::Remove { id: annotation_id }
        }
        DocumentAction::Undo { operation_id } => VideoAnnotationMutation::Undo {
            expected_operation_id: operation_id,
        },
        DocumentAction::Redo { operation_id } => VideoAnnotationMutation::Redo {
            expected_operation_id: operation_id,
        },
    };
    let snapshot = engine
        .store
        .edit_video_annotation_with_layouts(&target, mutation, vec![])
        .map_err(|e| e.to_string())?;
    drop(engine);
    crate::video_host::reconcile(&app, &snapshot);
    Ok(crate::publish(&app, &snapshot))
}

#[cfg(test)]
mod manual_edit_tests {
    use super::*;
    use mewu_core::{
        Asset, AssetKind, SceneCommand, VerifiedVideoAnnotationSource, VerifiedVideoSource,
        VideoRectStyle,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::{atomic::Ordering, mpsc},
        time::Duration,
    };

    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn p(x: f64, y: f64) -> VideoPixelPoint {
        VideoPixelPoint { x, y }
    }
    fn content(text: &str) -> VideoTextContent {
        VideoTextContent {
            version: 1,
            text: text.into(),
            color: "#1357b9".into(),
            font_size: 24.,
        }
    }
    struct Db(PathBuf);
    impl Db {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("mewu-video-manual-{}.sqlite", uid()));
            assert!(!path
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("c:\\hermes"));
            Self(path)
        }
        fn sql(&self) -> Connection {
            Connection::open(&self.0).unwrap()
        }
        fn layout_count(&self) -> i64 {
            self.sql()
                .query_row("SELECT count(*) FROM video_text_layouts", [], |r| r.get(0))
                .unwrap()
        }
        fn state(&self) -> (String, i64) {
            self.sql()
                .query_row(
                    "SELECT payload,revision FROM app_state WHERE id=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        }
    }
    impl Drop for Db {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
            }
        }
    }
    // Store is dropped before Db's exact private test-file cleanup.
    struct Fixture {
        store: Store,
        scene: String,
        item: String,
        db: Db,
    }
    impl Fixture {
        fn new() -> Self {
            let db = Db::new();
            let mut store = Store::open(&db.0).unwrap();
            let scene = store.snapshot().active_scene_id;
            let asset = Asset {
                id: uid(),
                kind: AssetKind::Video,
                name: "synthetic.mp4".into(),
                path: "synthetic-owned.mp4".into(),
                width: Some(640),
                height: Some(360),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
            };
            let snapshot = store.add_asset(&scene, asset.clone()).unwrap();
            let item = snapshot
                .scenes
                .iter()
                .find(|s| s.id == scene)
                .unwrap()
                .items
                .last()
                .unwrap()
                .id
                .clone();
            let proof = VerifiedVideoAnnotationSource::new(
                VerifiedVideoSource {
                    asset: asset.clone(),
                    duration_ticks: 60_000_000,
                    width: 640,
                    height: 360,
                },
                "1".repeat(64),
            )
            .unwrap();
            let text = raster::render_text(content("原文字")).unwrap();
            let target = VideoAnnotationTarget {
                scene_id: scene.clone(),
                item_id: item.clone(),
                source_id: asset.id,
                expected_range_revision: 0,
                expected_annotation_revision: 0,
            };
            store
                .append_video_annotations(
                    &target,
                    &proof,
                    vec![
                        VideoAnnotationDraft {
                            interval: VideoRange {
                                start_ticks: 20_000_000,
                                end_ticks: 21_000_000,
                            },
                            primitive: VideoAnnotationPrimitive::Rect {
                                bounds: VideoPixelRect {
                                    x: 20.,
                                    y: 30.,
                                    width: 90.,
                                    height: 60.,
                                },
                                style: VideoRectStyle {
                                    color: "#aa3366".into(),
                                    stroke_width: 3.,
                                    opacity: 0.5,
                                },
                            },
                        },
                        VideoAnnotationDraft {
                            interval: VideoRange {
                                start_ticks: 30_000_000,
                                end_ticks: 45_000_000,
                            },
                            primitive: VideoAnnotationPrimitive::Text {
                                top_left: p(60., 120.),
                                layout: text.reference().clone(),
                            },
                        },
                    ],
                    vec![text],
                )
                .unwrap();
            Self {
                store,
                scene,
                item,
                db,
            }
        }
        fn target(&self) -> VideoAnnotationTarget {
            let source = self.store.video_source(&self.scene, &self.item).unwrap();
            VideoAnnotationTarget {
                scene_id: self.scene.clone(),
                item_id: self.item.clone(),
                source_id: source.asset.id,
                expected_range_revision: source.edit.map_or(0, |e| e.revision),
                expected_annotation_revision: source.annotations.map_or(0, |d| d.revision),
            }
        }
        fn doc(&self) -> VideoAnnotationDocument {
            self.store
                .video_source(&self.scene, &self.item)
                .unwrap()
                .annotations
                .unwrap()
        }
        fn text(&self) -> (TimedVideoAnnotation, VerifiedVideoTextLayout) {
            let object = self.doc().objects[1].clone();
            let VideoAnnotationPrimitive::Text { layout, .. } = &object.primitive else {
                panic!("text fixture")
            };
            let value = self.store.video_text_layout(layout).unwrap();
            (object, value)
        }
        fn prepared(
            &self,
            text: &str,
        ) -> (
            VideoAnnotationTarget,
            VideoExportOrigin,
            VideoAnnotationMutation,
            Vec<VerifiedVideoTextLayout>,
        ) {
            let target = self.target();
            let origin = checked_origin(&self.store, &target, true).unwrap();
            let (object, old) = self.text();
            let (mutation, layouts) = prepare_text(
                &self.doc(),
                &object.id,
                old.reference(),
                &old,
                content(text),
                &AtomicBool::new(false),
            )
            .unwrap();
            (target, origin, mutation, layouts)
        }
        fn reopen(self) -> Self {
            let Self {
                store,
                scene,
                item,
                db,
            } = self;
            drop(store);
            Self {
                store: Store::open(&db.0).unwrap(),
                scene,
                item,
                db,
            }
        }
    }

    #[test]
    fn move_wire_rejects_style_interval_kind_and_nonfinite_or_stale_positions() {
        let mut value = json!({"type":"move","annotationId":uid(),"fromTopLeft":{"x":1,"y":2},"toTopLeft":{"x":3,"y":4}});
        assert!(serde_json::from_value::<DocumentAction>(value.clone()).is_ok());
        for field in ["style", "interval", "primitive", "origin", "layout"] {
            value[field] = json!({});
            assert!(serde_json::from_value::<DocumentAction>(value.clone()).is_err());
            value.as_object_mut().unwrap().remove(field);
        }
        let f = Fixture::new();
        let doc = f.doc();
        let id = &doc.objects[0].id;
        assert!(move_mutation(&doc, id, p(21., 30.), p(30., 40.)).is_err());
        for point in [
            p(f64::NAN, 0.),
            p(f64::INFINITY, 0.),
            p(-1., 0.),
            p(551., 0.),
            p(0., 301.),
        ] {
            assert!(move_mutation(&doc, id, p(20., 30.), point).is_err());
        }
        assert!(move_mutation(&doc, id, p(20., 30.), p(550., 300.)).is_ok());
        assert!(move_mutation(&doc, &uid(), p(20., 30.), p(30., 40.)).is_err());
    }

    #[test]
    fn real_store_rect_and_text_move_keep_pixels_interval_id_and_single_undo() {
        let mut f = Fixture::new();
        let before = f.doc();
        let source = f.store.video_source(&f.scene, &f.item).unwrap().asset;
        for (index, from, to) in [
            (0, p(20., 30.), p(40.5, 50.25)),
            (1, p(60., 120.), p(85.25, 145.5)),
        ] {
            let old = f.doc();
            let mutation = move_mutation(&old, &old.objects[index].id, from, to).unwrap();
            f.store
                .edit_video_annotation_with_layouts(&f.target(), mutation, vec![])
                .unwrap();
            let after = f.doc();
            assert_eq!(after.undo.len(), old.undo.len() + 1);
            assert_eq!(after.objects[index].id, old.objects[index].id);
            assert_eq!(after.objects[index].origin, old.objects[index].origin);
            assert_eq!(after.objects[index].interval, old.objects[index].interval);
            let image_before = raster::render(&old.objects[index], |r| {
                f.store.video_text_layout(r).map_err(|e| e.to_string())
            })
            .unwrap();
            let image_after = raster::render(&after.objects[index], |r| {
                f.store.video_text_layout(r).map_err(|e| e.to_string())
            })
            .unwrap();
            assert_eq!(image_before.png, image_after.png);
            let undo = after.undo.last().unwrap().id.clone();
            f.store
                .edit_video_annotation_with_layouts(
                    &f.target(),
                    VideoAnnotationMutation::Undo {
                        expected_operation_id: undo.clone(),
                    },
                    vec![],
                )
                .unwrap();
            assert_eq!(f.doc().objects, old.objects);
            f.store
                .edit_video_annotation_with_layouts(
                    &f.target(),
                    VideoAnnotationMutation::Redo {
                        expected_operation_id: undo,
                    },
                    vec![],
                )
                .unwrap();
            assert_eq!(f.doc().objects, after.objects);
        }
        let expected = f.doc();
        let f = f.reopen();
        assert_eq!(f.doc(), expected);
        assert_eq!(
            f.store.video_source(&f.scene, &f.item).unwrap().asset,
            source
        );
        assert_eq!(f.db.layout_count(), 1);
        assert_ne!(f.doc().objects, before.objects);
    }

    #[test]
    fn exact_content_read_noop_and_actual_chinese_raster_edit_survive_restart() {
        let mut f = Fixture::new();
        let (object, old) = f.text();
        let before = f.doc();
        let state = f.db.state();
        let wire = TextContent {
            request_id: uid(),
            target: f.target(),
            annotation_id: object.id.clone(),
            reference: old.reference().clone(),
            content: old.content().clone(),
        };
        let encoded = serde_json::to_value(wire).unwrap();
        assert_eq!(encoded["content"]["text"], "原文字");
        assert_eq!(
            encoded["reference"]["rasterSha256"],
            old.reference().raster_sha256
        );
        assert_eq!(encoded["annotationId"], object.id);
        let (target, origin, mutation, layouts) = f.prepared("原文字");
        assert!(layouts.is_empty());
        commit_text(
            &mut f.store,
            &target,
            &origin,
            mutation,
            layouts,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(f.db.state(), state);
        assert_eq!(f.doc(), before);
        let moved =
            move_mutation(&before, &before.objects[0].id, p(20., 30.), p(20., 30.)).unwrap();
        f.store
            .edit_video_annotation_with_layouts(&f.target(), moved, vec![])
            .unwrap();
        assert_eq!(f.db.state(), state);

        let (target, origin, mutation, layouts) = f.prepared("修改中文\n<text>&字面值");
        assert_eq!(layouts.len(), 1);
        let next_ref = layouts[0].reference().clone();
        assert_ne!(layouts[0].png(), old.png());
        commit_text(
            &mut f.store,
            &target,
            &origin,
            mutation,
            layouts,
            &AtomicBool::new(false),
        )
        .unwrap();
        let after = f.doc();
        let next = &after.objects[1];
        assert_eq!(
            (&next.id, next.interval, &next.origin),
            (&object.id, object.interval, &object.origin)
        );
        assert_eq!(after.undo.len(), before.undo.len() + 1);
        assert_eq!(f.db.layout_count(), 2);
        let loaded = f.store.video_text_layout(&next_ref).unwrap();
        assert_eq!(loaded.content().text, "修改中文\n<text>&字面值");
        let pixels = raster::decode(loaded.png(), loaded.width(), loaded.height()).unwrap();
        assert!(pixels.pixels().any(|p| p[3] > 0));
        let undo = after.undo.last().unwrap().id.clone();
        f.store
            .edit_video_annotation_with_layouts(
                &f.target(),
                VideoAnnotationMutation::Undo {
                    expected_operation_id: undo.clone(),
                },
                vec![],
            )
            .unwrap();
        assert_eq!(f.doc().objects, before.objects);
        f.store
            .edit_video_annotation_with_layouts(
                &f.target(),
                VideoAnnotationMutation::Redo {
                    expected_operation_id: undo,
                },
                vec![],
            )
            .unwrap();
        let f = f.reopen();
        assert_eq!(f.doc().objects, after.objects);
        assert_eq!(
            f.store.video_text_layout(&next_ref).unwrap().content().text,
            "修改中文\n<text>&字面值"
        );
    }

    #[test]
    fn invalid_kind_reference_content_or_layout_bounds_register_nothing() {
        let f = Fixture::new();
        let (object, old) = f.text();
        let before = f.db.state();
        let doc = f.doc();
        assert!(exact_text(&doc, &doc.objects[0].id, old.reference()).is_err());
        let mut changed = old.reference().clone();
        changed.raster_sha256 = "0".repeat(64);
        assert!(exact_text(&doc, &object.id, &changed).is_err());
        for value in [
            content(""),
            content(&"x".repeat(501)),
            content("a\0b"),
            VideoTextContent {
                color: "url(a)".into(),
                ..content("ok")
            },
            VideoTextContent {
                font_size: 129.,
                ..content("ok")
            },
        ] {
            assert!(prepare_text(
                &doc,
                &object.id,
                old.reference(),
                &old,
                value,
                &AtomicBool::new(false)
            )
            .is_err());
        }
        assert!(prepare_text(
            &doc,
            &object.id,
            old.reference(),
            &old,
            content("中文\u{10ffff}"),
            &AtomicBool::new(false)
        )
        .is_err());
        assert!(prepare_text(
            &doc,
            &object.id,
            old.reference(),
            &old,
            content(&"宽".repeat(100)),
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(f.db.state(), before);
        assert_eq!(f.db.layout_count(), 1);
    }

    #[test]
    fn stale_cancel_and_real_sqlite_write_failure_leave_no_orphan_layout() {
        let mut f = Fixture::new();
        let (target, origin, mutation, layouts) = f.prepared("取消后不提交");
        let before = f.db.state();
        assert!(commit_text(
            &mut f.store,
            &target,
            &origin,
            mutation,
            layouts,
            &AtomicBool::new(true)
        )
        .is_err());
        assert_eq!(f.db.state(), before);
        assert_eq!(f.db.layout_count(), 1);
        let (target, origin, mutation, layouts) = f.prepared("来源冲突");
        let doc = f.doc();
        let moved = move_mutation(&doc, &doc.objects[0].id, p(20., 30.), p(21., 31.)).unwrap();
        f.store
            .edit_video_annotation_with_layouts(&f.target(), moved, vec![])
            .unwrap();
        let state = f.db.state();
        assert!(commit_text(
            &mut f.store,
            &target,
            &origin,
            mutation,
            layouts,
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(f.db.state(), state);
        assert_eq!(f.db.layout_count(), 1);
        let (target, origin, mutation, layouts) = f.prepared("事务失败");
        f.db.sql().execute_batch("CREATE TRIGGER test_manual_write_failure BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(commit_text(
            &mut f.store,
            &target,
            &origin,
            mutation,
            layouts,
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(f.db.state(), state);
        assert_eq!(f.db.layout_count(), 1);
        f.db.sql()
            .execute_batch("DROP TRIGGER test_manual_write_failure")
            .unwrap();
        let (target, origin, mutation, layouts) = f.prepared("完成后取消不回滚");
        let cancel = AtomicBool::new(false);
        let receipt =
            commit_text(&mut f.store, &target, &origin, mutation, layouts, &cancel).unwrap();
        cancel.store(true, Ordering::Release);
        assert_eq!(receipt, f.store.snapshot());
        assert_eq!(f.db.layout_count(), 2);
    }

    #[test]
    fn frozen_scene_and_same_revision_source_replacement_fail_full_origin_fence() {
        let mut f = Fixture::new();
        let target = f.target();
        let origin = checked_origin(&f.store, &target, true).unwrap();
        let mut changed = origin.clone();
        changed.source.asset.name = "different.mp4".into();
        assert!(same_origin(&f.store, &target, &changed, true).is_err());
        f.store
            .apply(SceneCommand::FreezeScene {
                scene_id: f.scene.clone(),
            })
            .unwrap();
        assert!(same_origin(&f.store, &target, &origin, true).is_err());
    }

    #[test]
    fn dropped_waiter_cancels_but_video_work_slot_lives_until_real_thread_exit() {
        let f = Fixture::new();
        let origin = checked_origin(&f.store, &f.target(), true).unwrap();
        let registry = crate::video_host::VideoRegistry::default();
        let (work, cancel) = registry.begin(&uid(), origin.clone(), || Ok(())).unwrap();
        let guard = crate::video_host::InvokeCancel(cancel.clone());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _work = work;
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(guard);
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry.begin(&uid(), origin.clone(), || Ok(())).is_err());
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        let (work, _) = registry.begin(&uid(), origin, || Ok(())).unwrap();
        drop(work);
    }
}

// Append this cfg(test) module to video_annotation_host.rs; private, not run.
#[cfg(all(test, windows))]
mod manual_edit_media_gate {
    use super::*;
    use crate::video_clip_contract::{
        AnnotatedGifSchedule, ClipJob, ClipJobResult, OverlayBounds, OverlayRaster,
        VideoRange as ClipRange,
    };
    use mewu_core::{
        Asset, AssetKind, VerifiedVideoAnnotationSource, VerifiedVideoSource, VideoRectStyle,
    };
    use std::{fs, path::PathBuf, sync::atomic::Ordering, time::Duration};
    fn hash(p: &Path) -> String {
        format!("{:x}", Sha256::digest(fs::read(p).unwrap()))
    }
    fn target(s: &Store, scene: &str, item: &str) -> VideoAnnotationTarget {
        let v = s.video_source(scene, item).unwrap();
        VideoAnnotationTarget {
            scene_id: scene.into(),
            item_id: item.into(),
            source_id: v.asset.id,
            expected_range_revision: v.edit.map_or(0, |v| v.revision),
            expected_annotation_revision: v.annotations.map_or(0, |v| v.revision),
        }
    }
    fn document(s: &Store, scene: &str, item: &str) -> VideoAnnotationDocument {
        s.video_source(scene, item).unwrap().annotations.unwrap()
    }
    // Two disjoint full-width cells keep the glyph oracle away from the other glyph.
    // This is fixture content only; the native renderer and codec remain unchanged.
    const OLD_TEXT: &str = "  旧\u{3000}";
    const NEW_TEXT: &str = "  \u{3000}新";
    const TEXT_X: u32 = 80;
    const TEXT_Y: u32 = 40;
    fn content(s: &str) -> VideoTextContent {
        VideoTextContent {
            version: 1,
            text: s.into(),
            color: "#ffffff".into(),
            font_size: 80.,
        }
    }
    fn xy(x: f64, y: f64) -> VideoPixelPoint {
        VideoPixelPoint { x, y }
    }
    // Select from native PNGs before any output decode: 5x5 fully opaque ink,
    // 17x17 fully transparent in the opposite layout. No codec-dependent search.
    fn witness(ink: &image::RgbaImage, clear: &image::RgbaImage, origin_delta: i64) -> (u32, u32) {
        let alpha = |im: &image::RgbaImage, x: i64, y: i64| -> u8 {
            if x < 0 || y < 0 || x >= i64::from(im.width()) || y >= i64::from(im.height()) {
                0
            } else {
                im.get_pixel(x as u32, y as u32)[3]
            }
        };
        for y in 2..ink.height() - 2 {
            for x in 2..ink.width() - 2 {
                if (-2..=2).all(|dy| {
                    (-2..=2).all(|dx| alpha(ink, i64::from(x) + dx, i64::from(y) + dy) == 255)
                }) && (-8..=8).all(|dy| {
                    (-8..=8).all(|dx| {
                        let px = i64::from(x) + dx;
                        let py = i64::from(y) + dy;
                        // Opposite glyph at this local point, and both cross-origin
                        // clear witnesses. All are native alpha, never output pixels.
                        alpha(clear, px, py) == 0
                            && alpha(ink, px + origin_delta, py) == 0
                            && alpha(clear, px - origin_delta, py) == 0
                    })
                }) {
                    return (x, y);
                }
            }
        }
        panic!("no distinct stable Chinese glyph witnesses; do not relax mask")
    }
    #[test]
    fn manual_media_witness_has_stable_native_neighborhoods() {
        let old = raster::render_text(content(OLD_TEXT)).unwrap();
        let new = raster::render_text(content(NEW_TEXT)).unwrap();
        let old = image::load_from_memory(old.png()).unwrap().to_rgba8();
        let new = image::load_from_memory(new.png()).unwrap().to_rgba8();
        eprintln!(
            "native PNG sizes old={:?}, new={:?}",
            old.dimensions(),
            new.dimensions()
        );
        for image in [&old, &new] {
            assert!(
                TEXT_X + image.width() <= 320,
                "native PNG width {}",
                image.width()
            );
            assert!(
                TEXT_Y + image.height() <= 180,
                "native PNG height {}",
                image.height()
            );
        }
        let a = witness(&old, &new, i64::from(TEXT_X) - 16);
        let b = witness(&new, &old, 16 - i64::from(TEXT_X));
        assert_ne!(a, b);
        eprintln!(
            "native-only witnesses: old={a:?}, new={b:?}; ink=5x5 alpha255, opposite=17x17 alpha0"
        );
    }
    #[tokio::test]
    #[ignore = "explicit newly-built EXE+SHA; real Store/Chinese raster/worker media/cancel"]
    async fn synthetic_manual_move_chinese_edit_undo_product_worker() {
        use crate::video_annotation_render::tests::media_gate as media;
        let exe = PathBuf::from(std::env::var_os("MEWU_VIDEO_CLIP_TEST_EXE").expect("new EXE"));
        let exe_sha = std::env::var("MEWU_VIDEO_CLIP_TEST_EXE_SHA256").expect("EXE SHA");
        assert!(exe.is_absolute());
        assert_eq!(hash(&exe), exe_sha.to_ascii_lowercase());
        // run() repeats the existing ordinary-file/size/hash fence per spawn.
        let (root, source, metadata, _) = media::worker_fixture();
        crate::video_host::tests::core_capabilities_native_admission(&root);
        let source_sha = hash(&source);
        assert_eq!((metadata.width, metadata.height), (320, 180));
        let database = root.join("spaces.db");
        assert!(!database.exists());
        let mut store = Store::open(&database).unwrap();
        let scene = store.snapshot().active_scene_id;
        let asset = Asset {
            id: uuid::Uuid::new_v4().to_string(),
            kind: AssetKind::Video,
            name: "synthetic.mp4".into(),
            path: source.to_string_lossy().into_owned(),
            width: Some(320),
            height: Some(180),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let snapshot = store.add_asset(&scene, asset.clone()).unwrap();
        let item = snapshot
            .scenes
            .iter()
            .find(|s| s.id == scene)
            .unwrap()
            .items
            .last()
            .unwrap()
            .id
            .clone();
        let proof = VerifiedVideoAnnotationSource::new(
            VerifiedVideoSource {
                asset,
                duration_ticks: metadata.duration_ticks,
                width: 320,
                height: 180,
            },
            source_sha.clone(),
        )
        .unwrap();
        let old = raster::render_text(content(OLD_TEXT)).unwrap();
        store
            .append_video_annotations(
                &target(&store, &scene, &item),
                &proof,
                vec![
                    VideoAnnotationDraft {
                        interval: VideoRange {
                            start_ticks: 3_150_000,
                            end_ticks: 4_150_000,
                        },
                        primitive: VideoAnnotationPrimitive::Rect {
                            bounds: VideoPixelRect {
                                x: 16.,
                                y: 16.,
                                width: 64.,
                                height: 40.,
                            },
                            style: VideoRectStyle {
                                color: "#ffffff".into(),
                                stroke_width: 6.,
                                opacity: 1.,
                            },
                        },
                    },
                    VideoAnnotationDraft {
                        interval: VideoRange {
                            start_ticks: 4_500_000,
                            end_ticks: 9_500_000,
                        },
                        primitive: VideoAnnotationPrimitive::Text {
                            top_left: xy(16., f64::from(TEXT_Y)),
                            layout: old.reference().clone(),
                        },
                    },
                ],
                vec![old.clone()],
            )
            .unwrap();
        let before = document(&store, &scene, &item);
        for (i, from, to) in [
            (0, xy(16., 16.), xy(128., 16.)),
            (
                1,
                xy(16., f64::from(TEXT_Y)),
                xy(f64::from(TEXT_X), f64::from(TEXT_Y)),
            ),
        ] {
            let d = document(&store, &scene, &item);
            let mutation = move_mutation(&d, &d.objects[i].id, from, to).unwrap();
            store
                .edit_video_annotation_with_layouts(
                    &target(&store, &scene, &item),
                    mutation,
                    vec![],
                )
                .unwrap();
        }
        let d = document(&store, &scene, &item);
        let t = target(&store, &scene, &item);
        let origin = checked_origin(&store, &t, true).unwrap();
        let (mutation, layouts) = prepare_text(
            &d,
            &d.objects[1].id,
            old.reference(),
            &old,
            content(NEW_TEXT),
            &AtomicBool::new(false),
        )
        .unwrap();
        let new = layouts[0].clone();
        assert_ne!(old.png(), new.png());
        commit_text(
            &mut store,
            &t,
            &origin,
            mutation,
            layouts,
            &AtomicBool::new(false),
        )
        .unwrap();
        let edited = document(&store, &scene, &item);
        for (a, b) in before.objects.iter().zip(&edited.objects) {
            assert_eq!(
                (&a.id, a.interval, &a.origin),
                (&b.id, b.interval, &b.origin)
            );
        }
        drop(store);
        let mut store = Store::open(&database).unwrap();
        assert_eq!(document(&store, &scene, &item), edited);
        let old_png = image::load_from_memory(old.png()).unwrap().to_rgba8();
        let new_png = image::load_from_memory(new.png()).unwrap().to_rgba8();
        let a = witness(&old_png, &new_png, i64::from(TEXT_X) - 16);
        let b = witness(&new_png, &old_png, 16 - i64::from(TEXT_X));
        let range = ClipRange {
            start_ticks: 2_150_000,
            end_ticks: 11_150_000,
        };
        let mut cases = Vec::new();
        let mut edited_overlays = Vec::new();
        for (name, is_edited) in [("edited", true), ("undo", false)] {
            if !is_edited {
                for _ in 0..3 {
                    let id = document(&store, &scene, &item)
                        .undo
                        .last()
                        .unwrap()
                        .id
                        .clone();
                    store
                        .edit_video_annotation_with_layouts(
                            &target(&store, &scene, &item),
                            VideoAnnotationMutation::Undo {
                                expected_operation_id: id,
                            },
                            vec![],
                        )
                        .unwrap();
                }
            }
            let d = document(&store, &scene, &item);
            if !is_edited {
                assert_eq!(d.objects, before.objects);
            }
            let state = store.snapshot();
            let mut overlays = Vec::new();
            for (i, o) in d.objects.iter().enumerate() {
                let r =
                    raster::render(o, |r| store.video_text_layout(r).map_err(|e| e.to_string()))
                        .unwrap();
                let png = root.join(format!("{name}-{i}.png"));
                assert!(!png.exists());
                fs::write(&png, &r.png).unwrap();
                overlays.push(OverlayRaster {
                    png,
                    png_sha256: r.reference.png_sha256,
                    width: r.reference.width,
                    height: r.reference.height,
                    bounds: OverlayBounds {
                        x: r.bounds.x,
                        y: r.bounds.y,
                        width: r.bounds.width,
                        height: r.bounds.height,
                    },
                    interval: ClipRange {
                        start_ticks: o.interval.start_ticks,
                        end_ticks: o.interval.end_ticks,
                    },
                });
            }
            if is_edited {
                edited_overlays = overlays.clone();
            }
            // Old/new rect; old-only/new-only glyph witnesses at both origins.
            let points = vec![
                (18, 30, 3_150_000, 4_150_000, !is_edited),
                (130, 30, 3_150_000, 4_150_000, is_edited),
                (16 + a.0, TEXT_Y + a.1, 4_500_000, 9_500_000, !is_edited),
                (16 + b.0, TEXT_Y + b.1, 4_500_000, 9_500_000, false),
                (TEXT_X + a.0, TEXT_Y + a.1, 4_500_000, 9_500_000, false),
                (TEXT_X + b.0, TEXT_Y + b.1, 4_500_000, 9_500_000, is_edited),
            ];
            let mp4 = root.join(format!("{name}.mp4"));
            let gif = root.join(format!("{name}.gif"));
            let noop: Arc<dyn Fn(u8) + Send + Sync> = Arc::new(|_| {});
            assert!(matches!(
                crate::video_clip_worker::run(
                    ClipJob::RenderAnnotated {
                        source: source.clone(),
                        output: mp4.clone(),
                        expected: metadata.clone(),
                        range: Some(range.clone()),
                        overlays: overlays.clone()
                    },
                    Arc::new(AtomicBool::new(false)),
                    noop.clone()
                )
                .await
                .unwrap(),
                ClipJobResult::Rendered(_)
            ));
            assert!(!crate::video_clip_worker::busy());
            let ClipJobResult::GifRendered(g) = crate::video_clip_worker::run(
                ClipJob::RenderGifAnnotated {
                    source: source.clone(),
                    output: gif.clone(),
                    expected: metadata.clone(),
                    range: Some(range.clone()),
                    overlays: overlays.clone(),
                },
                Arc::new(AtomicBool::new(false)),
                noop,
            )
            .await
            .unwrap() else {
                panic!("GIF")
            };
            assert!(!crate::video_clip_worker::busy());
            let schedule = AnnotatedGifSchedule::new(&metadata, Some(&range), &overlays).unwrap();
            crate::video_gif_backend::validate_annotated_file(
                &gif,
                &g,
                &schedule.delays,
                &AtomicBool::new(false),
            )
            .unwrap();
            cases.push(media::verify_manual_points(
                &source, &mp4, &gif, &range, &schedule, &points,
            ));
            assert_eq!(store.snapshot(), state);
        }
        for _ in 0..3 {
            let id = document(&store, &scene, &item)
                .redo
                .last()
                .unwrap()
                .id
                .clone();
            store
                .edit_video_annotation_with_layouts(
                    &target(&store, &scene, &item),
                    VideoAnnotationMutation::Redo {
                        expected_operation_id: id,
                    },
                    vec![],
                )
                .unwrap();
        }
        assert_eq!(document(&store, &scene, &item).objects, edited.objects);
        let state = store.snapshot();
        let output = root.join("cancelled.mp4");
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let result = crate::video_clip_worker::run(
            ClipJob::RenderAnnotated {
                source: source.clone(),
                output: output.clone(),
                expected: metadata,
                range: Some(range),
                overlays: edited_overlays,
            },
            cancel,
            Arc::new(move |p| {
                if p >= 5 {
                    signal.store(true, Ordering::Release)
                }
            }),
        )
        .await;
        assert_eq!(
            result.unwrap_err(),
            crate::video_clip_contract::ClipError::Cancelled
        );
        assert!(crate::video_clip_worker::wait_until_idle(Duration::from_secs(5)).await);
        assert!(!crate::video_clip_worker::busy());
        if output.exists() {
            fs::remove_file(&output).unwrap();
        }
        assert!(!output.exists());
        assert_eq!(store.snapshot(), state);
        assert_eq!(hash(&source), source_sha);
        fs::write(root.join("manual-edit-media.json"),serde_json::to_vec_pretty(&serde_json::json!({"syntheticOnly":true,"coreCapabilitiesNativeAdmission":true,"actualProductExecutable":exe,"executableSha256":exe_sha,"sourceSha256":source_sha,"cases":cases,"undoObjectsExact":true,"redoObjectsExact":true,"cancelTrueJobIoExit":true,"limits":["six stable interior witnesses, not every glyph-edge pixel","host helpers/core plus real worker, not Tauri invoke/window admission","existing worker gate retains extraction and full PCM coverage"]})).unwrap()).unwrap();
    }

    // Insert inside video_annotation_host::manual_edit_media_gate (test-only).
    mod vector_media_gate {
        use super::*;
        use crate::video_vector_raster::{self as vector, VideoSourceVector};
        use mewu_core::{VideoAnnotationLayouts, VideoTarget};

        // Verify native masks before encoding. Opaque witnesses have a 5x5 white
        // interior; clear witnesses have 17x17 transparency against EVERY active
        // layer, including timed text. No decoded output influences these points.
        fn native_masks(
            layers: &[OverlayRaster],
            points: &[(u32, u32, u64, u64, bool)],
            duration: u64,
        ) {
            let images: Vec<_> = layers
                .iter()
                .map(|r| {
                    assert_eq!(r.bounds.x.fract(), 0.);
                    assert_eq!(r.bounds.y.fract(), 0.);
                    image::open(&r.png).unwrap().to_rgba8()
                })
                .collect();
            let mut times = vec![0, duration];
            for r in layers {
                times.extend([r.interval.start_ticks, r.interval.end_ticks]);
            }
            for time in times {
                for &(x, y, start, end, ink) in points {
                    let active = ink && start <= time && time < end;
                    let radius: i64 = if active { 2 } else { 8 };
                    for dy in -radius..=radius {
                        for dx in -radius..=radius {
                            let mut opaque_white = false;
                            let mut transparent = true;
                            for (r, image) in layers.iter().zip(&images) {
                                if !(r.interval.start_ticks <= time && time < r.interval.end_ticks)
                                {
                                    continue;
                                }
                                let px = i64::from(x) + dx - r.bounds.x as i64;
                                let py = i64::from(y) + dy - r.bounds.y as i64;
                                if px >= 0
                                    && py >= 0
                                    && px < i64::from(image.width())
                                    && py < i64::from(image.height())
                                {
                                    let p = image.get_pixel(px as u32, py as u32).0;
                                    transparent &= p[3] == 0;
                                    opaque_white |= p == [255, 255, 255, 255];
                                }
                            }
                            assert!(
                                if active { opaque_white } else { transparent },
                                "native mask point {x},{y} time {time} offset {dx},{dy} ink {active}"
                            );
                        }
                    }
                }
            }
        }

        #[tokio::test]
        #[ignore = "explicit new EXE+SHA, synthetic native Vector/SQLite/MP4/GIF/copy/cancel"]
        async fn synthetic_manual_vector_edge_move_history_product_worker() {
            use crate::video_annotation_render::tests::media_gate as media;
            use crate::video_overlay_host::StagedOverlays;
            let exe = PathBuf::from(std::env::var_os("MEWU_VIDEO_CLIP_TEST_EXE").expect("new EXE"));
            let exe_sha = std::env::var("MEWU_VIDEO_CLIP_TEST_EXE_SHA256").expect("EXE SHA");
            assert!(exe.is_absolute());
            assert_eq!(hash(&exe), exe_sha.to_ascii_lowercase());
            // Existing fixture enforces an explicitly provided fresh .private output.
            let (root, source, metadata, _) = media::worker_fixture();
            crate::video_host::tests::core_capabilities_native_admission(&root);
            assert_eq!(
                (metadata.width, metadata.height, metadata.has_audio),
                (320, 180, true)
            );
            let source_sha = hash(&source);
            let db = root.join("spaces.db");
            assert!(!db.exists());
            let mut store = Store::open(&db).unwrap();
            let scene = store.snapshot().active_scene_id;
            let asset = Asset {
                id: uuid::Uuid::new_v4().to_string(),
                kind: AssetKind::Video,
                name: "synthetic-vector.mp4".into(),
                path: source.to_string_lossy().into_owned(),
                width: Some(320),
                height: Some(180),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
            };
            let snapshot = store.add_asset(&scene, asset.clone()).unwrap();
            let item = snapshot
                .scenes
                .iter()
                .find(|s| s.id == scene)
                .unwrap()
                .items
                .last()
                .unwrap()
                .id
                .clone();
            let verified = VerifiedVideoSource {
                asset,
                duration_ticks: metadata.duration_ticks,
                width: 320,
                height: 180,
            };
            let proof =
                VerifiedVideoAnnotationSource::new(verified.clone(), source_sha.clone()).unwrap();
            let range = VideoRange {
                start_ticks: 2_150_000,
                end_ticks: 11_150_000,
            };
            store
                .set_video_range(
                    &VideoTarget {
                        scene_id: scene.clone(),
                        item_id: item.clone(),
                        source_id: verified.asset.id.clone(),
                        expected_revision: 0,
                    },
                    &verified,
                    None,
                    Some(range),
                )
                .unwrap();
            // Keep the old strict schedule oracle's two independent timed boundaries.
            let text = raster::render_text(content(OLD_TEXT)).unwrap();
            assert!(95 + text.width() <= 320 && 60 + text.height() <= 180);
            store
                .append_video_annotations(
                    &target(&store, &scene, &item),
                    &proof,
                    vec![
                        VideoAnnotationDraft {
                            interval: VideoRange {
                                start_ticks: 3_150_000,
                                end_ticks: 4_150_000,
                            },
                            primitive: VideoAnnotationPrimitive::Rect {
                                bounds: VideoPixelRect {
                                    x: 285.,
                                    y: 5.,
                                    width: 20.,
                                    height: 40.,
                                },
                                style: VideoRectStyle {
                                    color: "#ffffff".into(),
                                    stroke_width: 4.,
                                    opacity: 1.,
                                },
                            },
                        },
                        VideoAnnotationDraft {
                            interval: VideoRange {
                                start_ticks: 4_500_000,
                                end_ticks: 9_500_000,
                            },
                            primitive: VideoAnnotationPrimitive::Text {
                                top_left: xy(95., 60.),
                                layout: text.reference().clone(),
                            },
                        },
                    ],
                    vec![text.clone()],
                )
                .unwrap();
            let mut primitives = vec![];
            let mut layouts = vec![];
            for source in [
                VideoSourceVector::Pen {
                    version: 1,
                    source_points: vec![xy(0., 25.), xy(50., 25.)],
                    color: "#ffffff".into(),
                    stroke_width: 24.,
                },
                VideoSourceVector::Arrow {
                    version: 1,
                    source_points: vec![xy(155., 25.), xy(240., 25.)],
                    color: "#ffffff".into(),
                    stroke_width: 16.,
                },
                VideoSourceVector::Number {
                    version: 1,
                    source_points: vec![xy(10., 82.)],
                    color: "#ffffff".into(),
                    number: 7,
                    diameter: 56.,
                },
            ] {
                let r = vector::render(&source, 320, 180, &AtomicBool::new(false)).unwrap();
                primitives.push(VideoAnnotationPrimitive::Vector {
                    top_left: r.top_left,
                    layout: r.layout.reference().clone(),
                });
                layouts.push(r.layout);
            }
            let pen = layouts[0].clone();
            store
                .append_manual_video_drawings(
                    &target(&store, &scene, &item),
                    &proof,
                    primitives,
                    VideoAnnotationLayouts {
                        text: vec![],
                        vector: layouts,
                    },
                )
                .unwrap();
            let before = document(&store, &scene, &item);
            assert_eq!(before.objects.len(), 5);
            for o in &before.objects[2..] {
                assert_eq!(
                    o.interval,
                    VideoRange {
                        start_ticks: 0,
                        end_ticks: metadata.duration_ticks
                    }
                );
                assert!(o.origin.is_none());
            }
            let VideoAnnotationPrimitive::Vector {
                top_left: old_position,
                layout: old_ref,
            } = before.objects[2].primitive.clone()
            else {
                panic!("pen")
            };
            assert!(old_position.x < 0.);
            let clipped = render_object(&root, &before.objects[2], 320, 180).unwrap();
            assert!(clipped.reference.width < old_ref.width);
            // The hidden left cap is still present in immutable PNG, never clipped in DB.
            let full = image::load_from_memory(pen.png()).unwrap().to_rgba8();
            assert!((0..(-old_position.x as u32))
                .any(|x| (0..full.height()).any(|y| full.get_pixel(x, y)[3] != 0)));
            let moved_position = xy(old_position.x + 80., old_position.y);
            let mutation =
                move_mutation(&before, &before.objects[2].id, old_position, moved_position)
                    .unwrap();
            store
                .edit_video_annotation_with_registered_layouts(
                    &target(&store, &scene, &item),
                    mutation,
                    VideoAnnotationLayouts {
                        text: vec![],
                        vector: vec![],
                    },
                )
                .unwrap();
            let moved = document(&store, &scene, &item);
            let restored = render_object(&root, &moved.objects[2], 320, 180).unwrap();
            assert_eq!(restored.png, pen.png());
            assert_eq!(
                store.video_vector_layout(&old_ref).unwrap().png(),
                pen.png()
            );
            assert!(
                matches!(&moved.objects[2].primitive, VideoAnnotationPrimitive::Vector { layout, .. } if layout == &old_ref)
            );
            drop(store);
            let mut store = Store::open(&db).unwrap();
            assert_eq!(document(&store, &scene, &item), moved);
            let glyph = image::load_from_memory(text.png()).unwrap().to_rgba8();
            let glyph_point = (2..glyph.height() - 2)
                .find_map(|y| {
                    (2..glyph.width() - 2).find_map(|x| {
                        (-2i32..=2)
                            .all(|dy| {
                                (-2i32..=2).all(|dx| {
                                    glyph
                                        .get_pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32)
                                        .0
                                        == [255; 4]
                                })
                            })
                            .then_some((95 + x, 60 + y))
                    })
                })
                .expect("5x5 native Chinese white interior; do not relax");
            let clip_range = ClipRange {
                start_ticks: range.start_ticks,
                end_ticks: range.end_ticks,
            };
            let mut cases = vec![];
            for (name, is_moved) in [("moved", true), ("undo", false)] {
                if !is_moved {
                    let id = document(&store, &scene, &item)
                        .undo
                        .last()
                        .unwrap()
                        .id
                        .clone();
                    store
                        .edit_video_annotation_with_registered_layouts(
                            &target(&store, &scene, &item),
                            VideoAnnotationMutation::Undo {
                                expected_operation_id: id,
                            },
                            VideoAnnotationLayouts {
                                text: vec![],
                                vector: vec![],
                            },
                        )
                        .unwrap();
                    assert_eq!(document(&store, &scene, &item).objects, before.objects);
                }
                let d = document(&store, &scene, &item);
                let state = store.snapshot();
                let origin = VideoExportOrigin {
                    scene_id: scene.clone(),
                    item_id: item.clone(),
                    source: store.video_source(&scene, &item).unwrap(),
                };
                let staged = StagedOverlays::create(&root, &origin, || Ok(())).unwrap();
                let plan = render_plan(target(&store, &scene, &item), &d, range, &root).unwrap();
                assert_eq!(plan.entries.len(), staged.overlays.len());
                for (entry, layer) in plan.entries.iter().zip(&staged.overlays) {
                    assert_eq!(entry.reference.png_sha256, hash(&layer.png));
                    assert_eq!(
                        (entry.reference.width, entry.reference.height),
                        (layer.width, layer.height)
                    );
                    assert_eq!(
                        (
                            entry.bounds.x,
                            entry.bounds.y,
                            entry.bounds.width,
                            entry.bounds.height
                        ),
                        (
                            layer.bounds.x,
                            layer.bounds.y,
                            layer.bounds.width,
                            layer.bounds.height
                        )
                    );
                    assert_eq!(
                        (entry.interval.start_ticks, entry.interval.end_ticks),
                        (layer.interval.start_ticks, layer.interval.end_ticks)
                    );
                }
                let points = vec![
                    (25, 25, 0, metadata.duration_ticks, !is_moved),
                    (73, 25, 0, metadata.duration_ticks, is_moved), // newly restored off-source cap
                    (105, 25, 0, metadata.duration_ticks, is_moved),
                    (190, 25, 0, metadata.duration_ticks, true),
                    (38, 110, 0, metadata.duration_ticks, true),
                    (glyph_point.0, glyph_point.1, 4_500_000, 9_500_000, true),
                ];
                native_masks(&staged.overlays, &points, metadata.duration_ticks);
                let mp4 = root.join(format!("vector-{name}.mp4"));
                let gif = root.join(format!("vector-{name}.gif"));
                let progress: Arc<dyn Fn(u8) + Send + Sync> = Arc::new(|_| {});
                assert!(matches!(
                    crate::video_clip_worker::run(
                        ClipJob::RenderAnnotated {
                            source: source.clone(),
                            output: mp4.clone(),
                            expected: metadata.clone(),
                            range: Some(clip_range.clone()),
                            overlays: staged.overlays.clone()
                        },
                        Arc::new(AtomicBool::new(false)),
                        progress.clone()
                    )
                    .await
                    .unwrap(),
                    ClipJobResult::Rendered(_)
                ));
                assert!(!crate::video_clip_worker::busy());
                let ClipJobResult::GifRendered(g) = crate::video_clip_worker::run(
                    ClipJob::RenderGifAnnotated {
                        source: source.clone(),
                        output: gif.clone(),
                        expected: metadata.clone(),
                        range: Some(clip_range.clone()),
                        overlays: staged.overlays.clone(),
                    },
                    Arc::new(AtomicBool::new(false)),
                    progress,
                )
                .await
                .unwrap() else {
                    panic!("GIF")
                };
                assert!(!crate::video_clip_worker::busy());
                let schedule =
                    AnnotatedGifSchedule::new(&metadata, Some(&clip_range), &staged.overlays)
                        .unwrap();
                crate::video_gif_backend::validate_annotated_file(
                    &gif,
                    &g,
                    &schedule.delays,
                    &AtomicBool::new(false),
                )
                .unwrap();
                cases.push(media::verify_manual_points(
                    &source,
                    &mp4,
                    &gif,
                    &clip_range,
                    &schedule,
                    &points,
                ));
                // Same materialized MP4 becomes a durable file-drop copy. No OS clipboard.
                {
                    let _serial = crate::clipboard_video_storage::test_serial_guard();
                    let storage =
                        crate::clipboard_video_storage::ClipboardVideoStorage::open(&root).unwrap();
                    let mut input = fs::File::open(&mp4).unwrap();
                    let copy = storage
                        .reserve(input.metadata().unwrap().len())
                        .unwrap()
                        .copy_from(&mut input, &AtomicBool::new(false))
                        .unwrap();
                    assert_eq!(hash(copy.path()), hash(&mp4));
                    let path = copy.path().to_owned();
                    drop(copy);
                    assert!(path.is_file());
                }
                let staged_paths: Vec<_> = staged.overlays.iter().map(|v| v.png.clone()).collect();
                drop(staged);
                assert!(staged_paths.iter().all(|p| !p.exists()));
                assert_eq!(store.snapshot(), state);
            }
            let id = document(&store, &scene, &item)
                .redo
                .last()
                .unwrap()
                .id
                .clone();
            store
                .edit_video_annotation_with_registered_layouts(
                    &target(&store, &scene, &item),
                    VideoAnnotationMutation::Redo {
                        expected_operation_id: id,
                    },
                    VideoAnnotationLayouts {
                        text: vec![],
                        vector: vec![],
                    },
                )
                .unwrap();
            assert_eq!(document(&store, &scene, &item).objects, moved.objects);
            let state = store.snapshot();
            drop(store);
            let store = Store::open(&db).unwrap();
            assert_eq!(store.snapshot(), state);
            let origin = VideoExportOrigin {
                scene_id: scene.clone(),
                item_id: item.clone(),
                source: store.video_source(&scene, &item).unwrap(),
            };
            let staged = StagedOverlays::create(&root, &origin, || Ok(())).unwrap();
            let output = root.join("vector-cancelled.mp4");
            let cancel = Arc::new(AtomicBool::new(false));
            let signal = cancel.clone();
            let result = crate::video_clip_worker::run(
                ClipJob::RenderAnnotated {
                    source: source.clone(),
                    output: output.clone(),
                    expected: metadata,
                    range: Some(clip_range),
                    overlays: staged.overlays.clone(),
                },
                cancel,
                Arc::new(move |p| {
                    if p >= 5 {
                        signal.store(true, Ordering::Release)
                    }
                }),
            )
            .await;
            assert_eq!(
                result.unwrap_err(),
                crate::video_clip_contract::ClipError::Cancelled
            );
            assert!(crate::video_clip_worker::wait_until_idle(Duration::from_secs(5)).await);
            assert!(!crate::video_clip_worker::busy());
            drop(staged);
            if output.exists() {
                fs::remove_file(&output).unwrap();
            }
            assert_eq!(store.snapshot(), state);
            assert_eq!(hash(&source), source_sha);
            fs::write(root.join("manual-vector-media.json"),serde_json::to_vec_pretty(&serde_json::json!({
                "syntheticOnly":true,"coreCapabilitiesNativeAdmission":true,"actualProductExecutable":exe,"executableSha256":exe_sha,"sourceSha256":source_sha,
                "cases":cases,"completeInkRecoveredWithoutReraster":true,"sameImmutableLayoutPng":true,"previewStagingExact":true,
                "undoRedoColdReopen":true,"durableCopyExactMp4":true,"cancelTrueJobIoExit":true,
                "limits":["native render/Store/move helpers and actual product child, not Tauri IPC/window admission",
                    "white interior/transparent witnesses; does not assert every antialiased edge or glyph readability",
                    "copy cache bytes only; no real clipboard","existing unmodified worker/audio gates retain JPEG extraction and full PCM correlation coverage"]
            })).unwrap()).unwrap();
        }
    }
}
