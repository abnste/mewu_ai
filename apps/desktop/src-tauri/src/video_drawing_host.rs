// SPDX-License-Identifier: MPL-2.0
//! Private candidate: bounded manual video authoring, distinct from AI receipts.
use crate::{
    plugins::{VideoDrawingGrant, VideoDrawingTool},
    video_host,
    video_vector_raster::{self, VideoSourceVector},
    Host, HostSnapshot,
};
use mewu_core::{
    Store, VerifiedVideoAnnotationSource, VerifiedVideoSource, VerifiedVideoVectorLayout,
    VideoAnnotationDraft, VideoAnnotationLayouts, VideoAnnotationMutation,
    VideoAnnotationPrimitive, VideoAnnotationTarget, VideoExportOrigin, VideoPixelPoint,
    VideoTextContent, VideoVectorContent, VideoVectorLayoutRef,
};
use serde::{Deserialize, Serialize};
use std::sync::{atomic::AtomicBool, Arc};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;
static REQUESTS: Semaphore = Semaphore::const_new(16);

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase", try_from = "String")]
pub enum TextKind {
    Text,
}
impl TryFrom<String> for TextKind {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "text" {
            Ok(Self::Text)
        } else {
            Err("视频文字类型无效")
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextAdd {
    kind: TextKind,
    top_left: VideoPixelPoint,
    content: VideoTextContent,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SourceDrawing {
    Vector(VideoSourceVector),
    Text(TextAdd),
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DrawingAction {
    Add {
        content: SourceDrawing,
    },
    Update {
        annotation_id: String,
        reference: VideoVectorLayoutRef,
        content: VideoSourceVector,
    },
}
impl DrawingAction {
    fn tool(&self) -> VideoDrawingTool {
        match self {
            Self::Add {
                content: SourceDrawing::Text(_),
            } => VideoDrawingTool::Text,
            Self::Add {
                content: SourceDrawing::Vector(v),
            }
            | Self::Update { content: v, .. } => v.tool(),
        }
    }
    fn is_add(&self) -> bool {
        matches!(self, Self::Add { .. })
    }
    fn validate(&self, width: u32, height: u32) -> Result<(), String> {
        match self {
            Self::Add {
                content: SourceDrawing::Text(t),
            } => {
                crate::video_annotation_host::validate_content(&t.content)?;
                if !t.top_left.x.is_finite()
                    || !t.top_left.y.is_finite()
                    || t.top_left.x < 0.
                    || t.top_left.y < 0.
                    || t.top_left.x >= f64::from(width)
                    || t.top_left.y >= f64::from(height)
                {
                    return Err("视频文字位置无效".into());
                }
                Ok(())
            }
            Self::Add {
                content: SourceDrawing::Vector(v),
            }
            | Self::Update { content: v, .. } => v.validate(width, height),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorContent {
    request_id: String,
    target: VideoAnnotationTarget,
    annotation_id: String,
    reference: VideoVectorLayoutRef,
    content: VideoVectorContent,
}
struct Job {
    _work: video_host::Work,
    cancel: Arc<AtomicBool>,
    handle: usize,
    origin: VideoExportOrigin,
    grant: Option<(VideoDrawingGrant, VideoDrawingTool)>,
    add: bool,
    editing: bool,
}
fn window_handle(window: &WebviewWindow) -> Result<usize, String> {
    if window.label() != "space" {
        return Err("此窗口不能操作视频绘制".into());
    }
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|v| v.0 as usize)
            .map_err(|_| "空间窗口不可用".into())
    }
    #[cfg(not(windows))]
    {
        Err("当前系统不支持视频绘制".into())
    }
}
fn origin(
    store: &Store,
    target: &VideoAnnotationTarget,
    add: bool,
    editing: bool,
) -> Result<VideoExportOrigin, String> {
    let source = if add {
        store.video_annotation_creation_source(target)
    } else {
        store.video_annotation_source(target, editing)
    }
    .map_err(|e| e.to_string())?;
    Ok(VideoExportOrigin {
        scene_id: target.scene_id.clone(),
        item_id: target.item_id.clone(),
        source,
    })
}
fn exact_vector<'a>(
    origin: &'a VideoExportOrigin,
    id: &str,
    reference: &VideoVectorLayoutRef,
) -> Result<&'a mewu_core::TimedVideoAnnotation, String> {
    origin.source.annotations.as_ref().and_then(|d|d.objects.iter().find(|v|v.id==id))
        .filter(|v|matches!(&v.primitive,VideoAnnotationPrimitive::Vector{layout,..} if layout==reference))
        .ok_or_else(||"视频绘制图层已更改".into())
}
fn check_locked(store: &Store, target: &VideoAnnotationTarget, job: &Job) -> Result<(), String> {
    video_host::check_cancel(&job.cancel)?;
    if origin(store, target, job.add, job.editing)? != job.origin {
        return Err("视频范围或批注已更改，请重试".into());
    }
    Ok(())
}
fn validate_grant(
    store: &crate::plugins::PluginStore,
    grant: &VideoDrawingGrant,
    tool: VideoDrawingTool,
) -> Result<(), String> {
    if !store
        .video_drawing_tools(&grant.plugin_id, grant.revision, &grant.contribution_id)?
        .contains(&tool)
    {
        return Err("视频绘制工具已不可用".into());
    }
    Ok(())
}
fn admit(
    app: &AppHandle,
    window: &WebviewWindow,
    request_id: &str,
    target: &VideoAnnotationTarget,
    grant: Option<(VideoDrawingGrant, VideoDrawingTool)>,
    add: bool,
    editing: bool,
) -> Result<Arc<Job>, String> {
    let handle = window_handle(window)?;
    crate::recording::ensure_idle(app)?;
    let host = app.state::<Host>();
    // Same lock order as plugin changes and final acceptance, never reversed.
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    if let Some((grant, tool)) = &grant {
        validate_grant(&plugins.store, grant, *tool)?;
    }
    let engine = host.lock()?;
    crate::recording::ensure_idle(app)?;
    video_host::atomic_gate(&host, handle, false)?; // fresh authoring is RUNNING only
    let origin = origin(&engine.store, target, add, editing)?;
    let (work, cancel) = host.video.begin(request_id, origin.clone(), || {
        video_host::atomic_gate(&host, handle, false)
    })?;
    if let Some((grant, tool)) = &grant {
        host.video.bind_drawing(request_id, grant.clone(), *tool)?;
    }
    Ok(Arc::new(Job {
        _work: work,
        cancel,
        handle,
        origin,
        grant,
        add,
        editing,
    }))
}
fn check(app: &AppHandle, target: &VideoAnnotationTarget, job: &Job) -> Result<(), String> {
    video_host::check_cancel(&job.cancel)?;
    crate::recording::ensure_idle(app)?;
    let host = app.state::<Host>();
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    if let Some((grant, tool)) = &job.grant {
        validate_grant(&plugins.store, grant, *tool)?;
    }
    let engine = host.lock()?;
    crate::recording::ensure_idle(app)?;
    video_host::atomic_gate(&host, job.handle, false)?;
    check_locked(&engine.store, target, job)
}
fn prepare_add(
    content: &SourceDrawing,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<(VideoAnnotationPrimitive, VideoAnnotationLayouts), String> {
    video_host::check_cancel(cancel)?;
    match content {
        SourceDrawing::Vector(source) => {
            let rendered = video_vector_raster::render(source, width, height, cancel)?;
            let primitive = VideoAnnotationPrimitive::Vector {
                top_left: rendered.top_left,
                layout: rendered.layout.reference().clone(),
            };
            Ok((
                primitive,
                VideoAnnotationLayouts {
                    text: vec![],
                    vector: vec![rendered.layout],
                },
            ))
        }
        SourceDrawing::Text(source) => {
            let layout = crate::video_annotation_raster::render_text(source.content.clone())?;
            video_host::check_cancel(cancel)?;
            if source.top_left.x + f64::from(layout.width()) > f64::from(width)
                || source.top_left.y + f64::from(layout.height()) > f64::from(height)
            {
                return Err("视频文字布局超出原视频".into());
            }
            Ok((
                VideoAnnotationPrimitive::Text {
                    top_left: source.top_left,
                    layout: layout.reference().clone(),
                },
                VideoAnnotationLayouts {
                    text: vec![layout],
                    vector: vec![],
                },
            ))
        }
    }
}
fn prepare_update(
    job_origin: &VideoExportOrigin,
    id: &str,
    reference: &VideoVectorLayoutRef,
    old: &VerifiedVideoVectorLayout,
    source: &VideoSourceVector,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<(VideoAnnotationMutation, VideoAnnotationLayouts), String> {
    video_host::check_cancel(cancel)?;
    let object = exact_vector(job_origin, id, reference)?;
    if old.reference() != reference {
        return Err("视频绘制图层已更改".into());
    }
    let VideoAnnotationPrimitive::Vector { top_left, .. } = &object.primitive else {
        return Err("视频绘制类型无效".into());
    };
    if VideoSourceVector::from_local(old.content(), *top_left).tool() != source.tool() {
        return Err("视频绘制类型已更改".into());
    }
    // Compare the exact source DTO the editor was given, before any new UUID or raster.
    let (primitive, layouts) = if VideoSourceVector::from_local(old.content(), *top_left) == *source
    {
        (
            object.primitive.clone(),
            VideoAnnotationLayouts {
                text: vec![],
                vector: vec![],
            },
        )
    } else {
        let rendered = video_vector_raster::render(source, width, height, cancel)?;
        // Styling one object cannot convert its semantic tool kind.
        if rendered.layout.content().kind() != old.content().kind() {
            return Err("视频绘制类型已更改".into());
        }
        (
            VideoAnnotationPrimitive::Vector {
                top_left: rendered.top_left,
                layout: rendered.layout.reference().clone(),
            },
            VideoAnnotationLayouts {
                text: vec![],
                vector: vec![rendered.layout],
            },
        )
    };
    Ok((
        VideoAnnotationMutation::Update {
            id: id.into(),
            replacement: VideoAnnotationDraft {
                interval: object.interval,
                primitive,
            },
        },
        layouts,
    ))
}

#[tauri::command]
pub async fn get_video_annotation_vector(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    target: VideoAnnotationTarget,
    annotation_id: String,
    reference: VideoVectorLayoutRef,
) -> Result<VectorContent, String> {
    let request = REQUESTS.try_acquire().map_err(|_| "视频绘制正在处理")?;
    let job = admit(&app, &window, &request_id, &target, None, false, false)?;
    exact_vector(&job.origin, &annotation_id, &reference)?;
    let guard = video_host::InvokeCancel(job.cancel.clone());
    let worker = crate::visual_annotation_host::TABLE_WORKERS
        .acquire()
        .await
        .map_err(|_| "视频绘制处理不可用")?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        check(&app, &target, &job)?;
        let layout =
            Store::read_video_vector_layout(app.state::<Host>().root.join("spaces.db"), &reference)
                .map_err(|e| e.to_string())?;
        let content = layout.content().clone();
        check(&app, &target, &job)?;
        Ok(VectorContent {
            request_id,
            target,
            annotation_id,
            reference,
            content,
        })
    })
    .await
    .map_err(|_| "视频绘制读取中断")?;
    drop(guard);
    result
}

#[tauri::command]
pub async fn apply_video_drawing(
    app: AppHandle,
    window: WebviewWindow,
    request_id: String,
    target: VideoAnnotationTarget,
    plugin_id: String,
    plugin_revision: u64,
    contribution_id: String,
    action: DrawingAction,
) -> Result<HostSnapshot, String> {
    let request = REQUESTS.try_acquire().map_err(|_| "视频绘制正在处理")?;
    let grant = VideoDrawingGrant {
        plugin_id,
        revision: plugin_revision,
        contribution_id,
    };
    let job = admit(
        &app,
        &window,
        &request_id,
        &target,
        Some((grant, action.tool())),
        action.is_add(),
        true,
    )?;
    let width = job.origin.source.asset.width.ok_or("视频尺寸不可用")?;
    let height = job.origin.source.asset.height.ok_or("视频尺寸不可用")?;
    action.validate(width, height)?;
    if let DrawingAction::Update {
        annotation_id,
        reference,
        ..
    } = &action
    {
        exact_vector(&job.origin, annotation_id, reference)?;
    }
    let guard = video_host::InvokeCancel(job.cancel.clone());
    // The detached owned task survives invoke cancellation. Drop only revokes
    // its flag; Work and source lease remain until native child + I/O/render end.
    let task = tauri::async_runtime::spawn(async move {
        let _request = request;
        check(&app, &target, &job)?;
        let lease = {
            let app = app.clone();
            let job = job.clone();
            let target = target.clone();
            tauri::async_runtime::spawn_blocking(move || {
                check(&app, &target, &job)?;
                video_host::SourceLease::open(&job.origin.source.asset, &app.state::<Host>().assets)
            })
            .await
            .map_err(|_| "视频来源读取中断")??
        };
        let proof = if job.add {
            let host = app.state::<Host>();
            let metadata = video_host::metadata(
                &host.video,
                &job.origin.source.asset,
                &lease,
                job.cancel.clone(),
            )
            .await?;
            check(&app, &target, &job)?;
            let (lease, sha) = {
                let job = job.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let sha = lease.source_sha256(&job.cancel)?;
                    Ok::<_, String>((lease, sha))
                })
                .await
                .map_err(|_| "视频来源读取中断")??
            };
            let proof = VerifiedVideoAnnotationSource::new(
                VerifiedVideoSource {
                    asset: job.origin.source.asset.clone(),
                    duration_ticks: metadata.duration_ticks,
                    width: metadata.width,
                    height: metadata.height,
                },
                sha,
            )
            .map_err(|e| e.to_string())?;
            (lease, Some(proof))
        } else {
            (lease, None)
        };
        let worker = crate::visual_annotation_host::TABLE_WORKERS
            .acquire()
            .await
            .map_err(|_| "视频绘制处理不可用")?;
        tauri::async_runtime::spawn_blocking(move || {
            let (_source, proof) = proof;
            let _worker = worker;
            check(&app, &target, &job)?;
            let prepared = match &action {
                DrawingAction::Add { content } => {
                    let (primitive, layouts) = prepare_add(content, width, height, &job.cancel)?;
                    Prepared::Add(primitive, layouts)
                }
                DrawingAction::Update {
                    annotation_id,
                    reference,
                    content,
                } => {
                    let old = Store::read_video_vector_layout(
                        app.state::<Host>().root.join("spaces.db"),
                        reference,
                    )
                    .map_err(|e| e.to_string())?;
                    let (mutation, layouts) = prepare_update(
                        &job.origin,
                        annotation_id,
                        reference,
                        &old,
                        content,
                        width,
                        height,
                        &job.cancel,
                    )?;
                    Prepared::Update(mutation, layouts)
                }
            };
            let snapshot = {
                let host = app.state::<Host>();
                let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
                let (grant, tool) = job.grant.as_ref().ok_or("视频绘制授权不可用")?;
                validate_grant(&plugins.store, grant, *tool)?;
                let mut engine = host.lock()?;
                crate::recording::ensure_idle(&app)?;
                video_host::atomic_gate(&host, job.handle, false)?;
                check_locked(&engine.store, &target, &job)?;
                // DB acceptance is linearized here. Late cancel cannot fabricate failure.
                match prepared {
                    Prepared::Add(primitive, layouts) => engine.store.append_manual_video_drawings(
                        &target,
                        proof.as_ref().ok_or("视频来源证明不可用")?,
                        vec![primitive],
                        layouts,
                    ),
                    Prepared::Update(mutation, layouts) => engine
                        .store
                        .edit_video_annotation_with_registered_layouts(&target, mutation, layouts),
                }
                .map_err(|e| e.to_string())?
            };
            video_host::reconcile(&app, &snapshot);
            Ok(crate::publish(&app, &snapshot))
        })
        .await
        .map_err(|_| "视频绘制保存中断")?
    });
    let result = task.await.map_err(|_| "视频绘制任务中断")?;
    drop(guard);
    result
}
enum Prepared {
    Add(VideoAnnotationPrimitive, VideoAnnotationLayouts),
    Update(VideoAnnotationMutation, VideoAnnotationLayouts),
}
#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{Asset, AssetKind, VideoRange};
    use rusqlite::Connection;
    use std::path::PathBuf;
    fn p(x: f64, y: f64) -> VideoPixelPoint {
        VideoPixelPoint { x, y }
    }
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    struct Db(PathBuf);
    impl Db {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("mewu-video-vector-{}.sqlite", uid())))
        }
        fn state(&self) -> (String, i64, i64, i64) {
            let c = Connection::open(&self.0).unwrap();
            c.query_row("SELECT payload,revision,(SELECT count(*) FROM video_vector_layouts),(SELECT count(*) FROM video_text_layouts) FROM app_state WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
        }
    }
    impl Drop for Db {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
            }
        }
    }
    struct Fixture {
        store: Store,
        target: VideoAnnotationTarget,
        proof: VerifiedVideoAnnotationSource,
        db: Db,
    }
    impl Fixture {
        fn new() -> Self {
            let db = Db::new();
            let mut store = Store::open(&db.0).unwrap();
            let scene = store.snapshot().active_scene_id;
            let asset = Asset {
                id: uid(),
                name: "synthetic.mp4".into(),
                kind: AssetKind::Video,
                path: "synthetic-no-media-request.mp4".into(),
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
            Self {
                store,
                target: VideoAnnotationTarget {
                    scene_id: scene,
                    item_id: item,
                    source_id: asset.id,
                    expected_range_revision: 0,
                    expected_annotation_revision: 0,
                },
                proof,
                db,
            }
        }
        fn refresh(&mut self) {
            self.target.expected_annotation_revision = self
                .store
                .video_source(&self.target.scene_id, &self.target.item_id)
                .unwrap()
                .annotations
                .as_ref()
                .map_or(0, |d| d.revision);
        }
        fn doc(&self) -> mewu_core::VideoAnnotationDocument {
            self.store
                .video_source(&self.target.scene_id, &self.target.item_id)
                .unwrap()
                .annotations
                .unwrap()
        }
        fn origin(&self) -> VideoExportOrigin {
            origin(&self.store, &self.target, false, true).unwrap()
        }
    }
    fn line(width: f64) -> VideoSourceVector {
        VideoSourceVector::Arrow {
            version: 1,
            source_points: vec![p(0., 40.25), p(90.5, 1.)],
            color: "#234567".into(),
            stroke_width: width,
        }
    }
    fn vector_part() -> (VideoAnnotationPrimitive, VideoAnnotationLayouts) {
        prepare_add(
            &SourceDrawing::Vector(line(12.)),
            640,
            360,
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    #[test]
    fn real_vector_and_literal_text_commit_reopen_and_history_keep_complete_png() {
        let mut f = Fixture::new();
        let (primitive, mut layouts) = vector_part();
        let text = TextAdd {
            kind: TextKind::Text,
            top_left: p(150., 80.),
            content: VideoTextContent {
                version: 1,
                text: "中文<&>\n literal".into(),
                color: "#123456".into(),
                font_size: 24.,
            },
        };
        let (t, ts) = prepare_add(
            &SourceDrawing::Text(text),
            640,
            360,
            &AtomicBool::new(false),
        )
        .unwrap();
        layouts.text = ts.text;
        let full = layouts.vector[0].png().to_vec();
        f.store
            .append_manual_video_drawings(&f.target, &f.proof, vec![primitive, t], layouts)
            .unwrap();
        f.refresh();
        let original = f.doc();
        assert_eq!(original.objects.len(), 2);
        assert!(original.objects.iter().all(|v| v.origin.is_none()
            && v.interval
                == VideoRange {
                    start_ticks: 0,
                    end_ticks: 60_000_000
                }));
        let VideoAnnotationPrimitive::Vector { top_left, layout } = &original.objects[0].primitive
        else {
            panic!()
        };
        assert!(top_left.x < 0.);
        let old_ref = layout.clone();
        let saved = f.store.video_vector_layout(&old_ref).unwrap();
        assert_eq!(saved.png(), full);
        let moved = VideoAnnotationPrimitive::Vector {
            top_left: p(20., 40.),
            layout: old_ref.clone(),
        };
        f.store
            .edit_video_annotation_with_registered_layouts(
                &f.target,
                VideoAnnotationMutation::Update {
                    id: original.objects[0].id.clone(),
                    replacement: VideoAnnotationDraft {
                        interval: original.objects[0].interval,
                        primitive: moved,
                    },
                },
                VideoAnnotationLayouts {
                    text: vec![],
                    vector: vec![],
                },
            )
            .unwrap();
        f.refresh();
        let image = crate::video_annotation_raster::render_in_source(
            &f.doc().objects[0],
            640,
            360,
            |r| f.store.video_text_layout(r).map_err(|e| e.to_string()),
            |r| f.store.video_vector_layout(r).map_err(|e| e.to_string()),
        )
        .unwrap();
        assert_eq!(image.png, full);
        let undo = f.doc().undo.last().unwrap().id.clone();
        f.store
            .edit_video_annotation_with_registered_layouts(
                &f.target,
                VideoAnnotationMutation::Undo {
                    expected_operation_id: undo,
                },
                VideoAnnotationLayouts {
                    text: vec![],
                    vector: vec![],
                },
            )
            .unwrap();
        f.refresh();
        assert_eq!(f.doc().objects, original.objects);
        let expected = f.store.snapshot();
        drop(f.store);
        let reopened = Store::open(&f.db.0).unwrap();
        assert_eq!(reopened.snapshot(), expected);
        assert_eq!(reopened.video_vector_layout(&old_ref).unwrap().png(), full);
    }
    #[test]
    fn style_update_uses_source_control_points_and_equal_content_is_zero_write() {
        let mut f = Fixture::new();
        let (primitive, layouts) = vector_part();
        f.store
            .append_manual_video_drawings(&f.target, &f.proof, vec![primitive], layouts)
            .unwrap();
        f.refresh();
        let original = f.doc();
        let object = &original.objects[0];
        let VideoAnnotationPrimitive::Vector { top_left, layout } = &object.primitive else {
            panic!()
        };
        let old = f.store.video_vector_layout(layout).unwrap();
        let same = VideoSourceVector::from_local(old.content(), *top_left);
        let (mutation, layouts) = prepare_update(
            &f.origin(),
            &object.id,
            layout,
            &old,
            &same,
            640,
            360,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(layouts.vector.is_empty());
        let before = f.db.state();
        Connection::open(&f.db.0).unwrap().execute_batch("CREATE TRIGGER forbid_noop BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'no writes'); END;").unwrap();
        f.store
            .edit_video_annotation_with_registered_layouts(&f.target, mutation, layouts)
            .unwrap();
        assert_eq!(f.db.state(), before);
        Connection::open(&f.db.0)
            .unwrap()
            .execute_batch("DROP TRIGGER forbid_noop;")
            .unwrap();
        let (mutation, layouts) = prepare_update(
            &f.origin(),
            &object.id,
            layout,
            &old,
            &line(20.),
            640,
            360,
            &AtomicBool::new(false),
        )
        .unwrap();
        let VideoAnnotationMutation::Update { replacement, .. } = &mutation else {
            panic!()
        };
        let VideoAnnotationPrimitive::Vector {
            top_left: new_top, ..
        } = &replacement.primitive
        else {
            panic!()
        };
        assert_eq!(
            VideoSourceVector::from_local(layouts.vector[0].content(), *new_top).points(),
            same.points()
        );
        assert_ne!(layouts.vector[0].reference(), layout);
        f.store
            .edit_video_annotation_with_registered_layouts(&f.target, mutation, layouts)
            .unwrap();
        assert_eq!(f.doc().objects[0].id, object.id);
        assert_eq!(f.doc().objects[0].origin, object.origin);
    }
    #[test]
    fn real_raster_insert_then_sql_abort_leaves_no_orphans_and_cancel_leaves_no_write() {
        let mut f = Fixture::new();
        let before = f.db.state();
        let (primitive, layouts) = vector_part();
        Connection::open(&f.db.0).unwrap().execute_batch("CREATE TRIGGER abort_accept BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(f
            .store
            .append_manual_video_drawings(&f.target, &f.proof, vec![primitive], layouts)
            .is_err());
        assert_eq!(f.db.state(), before);
        assert!(prepare_add(
            &SourceDrawing::Vector(line(12.)),
            640,
            360,
            &AtomicBool::new(true)
        )
        .is_err());
        assert_eq!(f.db.state(), before);
    }
    #[test]
    fn strict_action_cannot_supply_local_points_intervals_png_or_change_tool_kind() {
        let good = serde_json::json!({"type":"add","content":{"kind":"pen","version":1,"sourcePoints":[{"x":0,"y":0}],"color":"#123456","strokeWidth":2}});
        assert!(serde_json::from_value::<DrawingAction>(good.clone()).is_ok());
        assert!(serde_json::from_value::<DrawingAction>(serde_json::json!({"type":"add","content":{"kind":{"text":null},"topLeft":{"x":0,"y":0},"content":{"version":1,"text":"x","color":"#ffffff","fontSize":24}}})).is_err());
        for key in ["interval", "png", "layout", "localPoints"] {
            let mut v = good.clone();
            v["content"][key] = serde_json::json!([]);
            assert!(serde_json::from_value::<DrawingAction>(v).is_err());
        }
        let mut f = Fixture::new();
        let (primitive, layouts) = vector_part();
        f.store
            .append_manual_video_drawings(&f.target, &f.proof, vec![primitive], layouts)
            .unwrap();
        f.refresh();
        let o = f.doc().objects[0].clone();
        let VideoAnnotationPrimitive::Vector { layout, .. } = &o.primitive else {
            panic!()
        };
        let old = f.store.video_vector_layout(layout).unwrap();
        let other = VideoSourceVector::Line {
            version: 1,
            source_points: line(12.).points().to_vec(),
            color: "#234567".into(),
            stroke_width: 12.,
        };
        assert!(prepare_update(
            &f.origin(),
            &o.id,
            layout,
            &old,
            &other,
            640,
            360,
            &AtomicBool::new(false)
        )
        .is_err());
    }
    #[tokio::test]
    async fn dropped_invoker_cancels_but_blocking_work_still_owns_video_slot() {
        let f = Fixture::new();
        let registry = video_host::VideoRegistry::default();
        let source = origin(&f.store, &f.target, true, true).unwrap();
        let (work, cancel) = registry.begin(&uid(), source.clone(), || Ok(())).unwrap();
        let (started_rx_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _work = work;
                let _ = started_rx_tx.send(());
                wait.recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
            })
            .await
            .unwrap();
            let _ = finished_tx.send(());
        });
        started_rx.await.unwrap();
        drop(video_host::InvokeCancel(cancel.clone()));
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
        drop(task); // owned task is detached, not aborted
        assert!(registry.begin(&uid(), source.clone(), || Ok(())).is_err());
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), finished_rx)
            .await
            .unwrap()
            .unwrap();
        let (work, _) = registry.begin(&uid(), source, || Ok(())).unwrap();
        drop(work);
    }
}
