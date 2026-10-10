// SPDX-License-Identifier: MPL-2.0
//! One authorized video input permits optional annotations to its own real frames.
use crate::{
    ai, journal_runtime,
    visual_annotation_host::{RunOrigin, SourceKind},
    Host,
};
use mewu_core::{
    DispatchLease, ModelVideoAppend, ModelVideoPrimitive, NotSentReason, ToolBinding,
    ValidatedVideoAppendBatch, VerifiedVideoAnnotationAuthority, VerifiedVideoInput,
    VerifiedVideoTextLayout, VideoAnnotationDraft, VideoAnnotationPrimitive, VideoAppendGroup,
    VideoTextContent, VisualAnnotationGrant, VIDEO_ANNOTATION_TOOL,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub struct RunScope {
    pub origin: RunOrigin,
    pub input: VerifiedVideoInput,
    authority: VerifiedVideoAnnotationAuthority,
}
impl RunScope {
    pub fn new(
        scene: String,
        grant: VisualAnnotationGrant,
        input: VerifiedVideoInput,
    ) -> Result<Self, String> {
        let authority = VerifiedVideoAnnotationAuthority::new(grant.clone(), input.sha256().into())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            origin: RunOrigin {
                scene_id: scene,
                grant,
                manifest_sha256: input.sha256().into(),
                source_kind: SourceKind::Video,
            },
            input,
            authority,
        })
    }
    pub fn binding(&self) -> ToolBinding {
        self.origin.grant.video_binding(self.input.sha256())
    }
    pub fn live(&self, engine: &crate::Engine, scene: &str, run: &str) -> bool {
        self.origin.scene_id == scene
            && engine.visual_runs.get(run) == Some(&self.origin)
            && crate::run_is_authorized(engine, scene, run, None)
    }
    pub fn check_engine(
        &self,
        engine: &crate::Engine,
        scene: &str,
        run: &str,
    ) -> Result<(), String> {
        if !self.live(engine, scene, run) {
            return Err("视频原位作答已取消或插件已停用".into());
        }
        if engine
            .store
            .video_input_for_run(scene, run, &self.authority)
            .map_err(journal_runtime::store_error)?
            != self.input
        {
            return Err("视频原位作答来源已变更".into());
        }
        Ok(())
    }
    pub fn definition(&self) -> Value {
        let target = &self.input.manifest().targets[0];
        let starts: Vec<_> = target.frames.iter().map(|f| f.handle.clone()).collect();
        let mut ends = starts.clone();
        ends.push(target.range_end_handle.clone());
        let point = json!({"type":"object","additionalProperties":false,"required":["x","y"],"properties":{"x":{"type":"number","minimum":0,"maximum":1000},"y":{"type":"number","minimum":0,"maximum":1000}}});
        let bounds = json!({"type":"object","additionalProperties":false,"required":["x","y","width","height"],"properties":{"x":{"type":"number","minimum":0,"maximum":1000},"y":{"type":"number","minimum":0,"maximum":1000},"width":{"type":"number","exclusiveMinimum":0,"maximum":1000},"height":{"type":"number","exclusiveMinimum":0,"maximum":1000}}});
        let color = json!({"type":"string","pattern":"^#[0-9a-fA-F]{6}$"});
        let rect = json!({"type":"object","additionalProperties":false,"required":["kind","bounds","style"],"properties":{"kind":{"const":"rect"},"bounds":bounds,"style":{"type":"object","additionalProperties":false,"required":["color","strokeWidth","opacity"],"properties":{"color":color,"strokeWidth":{"type":"number","minimum":0.25,"maximum":64},"opacity":{"type":"number","minimum":0.05,"maximum":1}}}}});
        let text = json!({"type":"object","additionalProperties":false,"required":["kind","topLeft","anchor","text","color","fontSize"],"properties":{"kind":{"const":"text"},"topLeft":point,"anchor":{"const":"topLeft"},"text":{"type":"string","minLength":1,"maxLength":500},"color":color,"fontSize":{"type":"number","minimum":8,"maximum":128}}});
        json!({"type":"function","function":{"name":VIDEO_ANNOTATION_TOOL,
            "description":"Optional video presentation: add editable interval rectangle or literal text annotations to this current video. Decide from the user's intent and task whether to answer in the conversation or annotate the video; do not ask the user to select a send mode. Do not call merely because this tool is available, and never add annotations when the user requests a text-only reply. Position uses normalized1000 independently per axis. Stroke width/font size are ORIGINAL source pixels. Text topLeft anchors a native measured layout with 4px padding and first baseline at 4+fontSize; never center it by assumption or provide HTML/SVG/PNG. Interval must use a sent startFrameHandle and later frame/end boundary handle and last at least 100ms. These are sparse sampled frames, not evidence of motion between frames. No raw seconds/PTS, scene/item/source IDs, tracking or unsent frames. If using this tool, submit the entire intended batch once; invalid content rejects the whole batch.",
            "parameters":{"type":"object","additionalProperties":false,"required":["coordinateSpace","styleUnit","timeSpace","objects"],"properties":{
                "coordinateSpace":{"const":"normalized1000"},"styleUnit":{"const":"sourcePixels"},"timeSpace":{"const":"sentFrameHandles"},
                "objects":{"type":"array","minItems":1,"maxItems":48,"items":{"type":"object","additionalProperties":false,"required":["targetHandle","interval","primitive"],"properties":{
                    "targetHandle":{"const":target.handle},"interval":{"type":"object","additionalProperties":false,"required":["startFrameHandle","endHandle"],"properties":{"startFrameHandle":{"type":"string","enum":starts},"endHandle":{"type":"string","enum":ends}}},"primitive":{"oneOf":[rect,text]}}}}}}}})
    }
    fn check_live(&self, app: &AppHandle, scene: &str, run: &str) -> Result<(), String> {
        let host = app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins.store.visual_annotations(&self.origin.grant)?;
        let engine = host.lock()?;
        host.exit.ensure_background()?;
        if !self.live(&engine, scene, run) {
            return Err("原位作答已取消或插件已停用".into());
        }
        if engine
            .store
            .video_input_for_run(scene, run, &self.authority)
            .map_err(journal_runtime::store_error)?
            != self.input
        {
            return Err("视频原位作答来源已变更".into());
        }
        Ok(())
    }
    fn not_sent(
        &self,
        app: &AppHandle,
        scene: &str,
        run: &str,
        lease: &DispatchLease,
        reason: NotSentReason,
        error: String,
    ) -> Result<journal_runtime::CommittedToolOutput, String> {
        let host = app.state::<Host>();
        let mut engine = host.lock()?;
        match engine.store.record_tool_not_sent(lease, reason) {
            Ok(receipt) => {
                journal_runtime::emit_changed(app, scene, run, receipt.journal_revision);
                crate::publish(app, &engine.store.snapshot());
                Err(error)
            }
            Err(e) => Err(format!(
                "{error}；执行记录未能更新：{}",
                journal_runtime::store_error(e)
            )),
        }
    }
    pub async fn execute(
        self: &Arc<Self>,
        app: &AppHandle,
        scene: &str,
        run: &str,
        call: ai::ModelToolCall,
        lease: DispatchLease,
    ) -> Result<journal_runtime::CommittedToolOutput, String> {
        if call.name != VIDEO_ANNOTATION_TOOL {
            return self.not_sent(
                app,
                scene,
                run,
                &lease,
                NotSentReason::NotAdvertised,
                "本次运行未启用视频批注".into(),
            );
        }
        let parsed = match parse_model(&call.arguments, &self.input) {
            Ok(v) => v,
            Err(e) => {
                return self.not_sent(app, scene, run, &lease, NotSentReason::InvalidArguments, e)
            }
        };
        if let Err(e) = self.check_live(app, scene, run) {
            return self.not_sent(app, scene, run, &lease, NotSentReason::AuthorityChanged, e);
        }
        let worker = crate::visual_annotation_host::TABLE_WORKERS
            .acquire()
            .await
            .map_err(|_| "视频文字处理不可用")?;
        let scope = self.clone();
        let app2 = app.clone();
        let scene2 = scene.to_string();
        let run2 = run.to_string();
        let rendered = tauri::async_runtime::spawn_blocking(move || {
            let _worker = worker;
            render_model(parsed, &scope.input, || {
                scope.check_live(&app2, &scene2, &run2)
            })
        })
        .await
        .map_err(|_| "视频批注排版中断".to_string())
        .and_then(|v| v);
        let (batch, layouts) = match rendered {
            Ok(v) => v,
            Err(e) => {
                return self.not_sent(app, scene, run, &lease, NotSentReason::PreparationFailed, e)
            }
        };
        let result = (|| {
            let host = app.state::<Host>();
            let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
            plugins.store.visual_annotations(&self.origin.grant)?;
            let mut engine = host.lock()?;
            host.exit.ensure_background()?;
            if !self.live(&engine, scene, run)
                || engine
                    .store
                    .video_input_for_run(scene, run, &self.authority)
                    .map_err(journal_runtime::store_error)?
                    != self.input
            {
                return Err("视频原位作答来源或授权已更改".into());
            }
            let committed = engine
                .store
                .apply_video_annotations_with_receipt(&lease, &self.authority, batch, layouts)
                .map_err(journal_runtime::store_error)?;
            journal_runtime::emit_changed(app, scene, run, committed.receipt.journal_revision);
            crate::publish(app, &committed.snapshot);
            committed
                .result
                .model_visible_json
                .map(|model_json| journal_runtime::CommittedToolOutput { model_json })
                .ok_or_else(|| "视频原位作答缺少提交回执".into())
        })();
        match result {
            Ok(v) => Ok(v),
            Err(e) => self.not_sent(app, scene, run, &lease, NotSentReason::AuthorityChanged, e),
        }
    }
}
/// Only this module can construct a plan accepted by the renderer. Validate
/// every object's cheap fields before the first font lookup or raster allocation.
struct PreparedModel(ModelVideoAppend);

fn valid_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
}

fn parse_model(arguments: &str, input: &VerifiedVideoInput) -> Result<PreparedModel, String> {
    if arguments.len() > 64 * 1024 {
        return Err("视频批注参数过大".into());
    }
    let parsed: ModelVideoAppend =
        serde_json::from_str(arguments).map_err(|_| "视频批注参数无效")?;
    if parsed.coordinate_space != "normalized1000"
        || parsed.style_unit != "sourcePixels"
        || parsed.time_space != "sentFrameHandles"
        || parsed.objects.is_empty()
        || parsed.objects.len() > 48
    {
        return Err("视频批注单位或数量无效".into());
    }
    for object in &parsed.objects {
        let target = input
            .manifest()
            .targets
            .iter()
            .find(|t| t.handle == object.target_handle)
            .ok_or("视频目标句柄无效")?;
        mewu_core::resolve_video_model_interval(target, &object.interval)
            .map_err(|e| e.to_string())?;
        match &object.primitive {
            ModelVideoPrimitive::Rect { bounds, style } => {
                let source = mewu_core::video_annotation_source_rect(
                    *bounds,
                    target.source.source_width,
                    target.source.source_height,
                )
                .map_err(|e| e.to_string())?;
                // Width is in original source pixels; normalized bounds must
                // be mapped before checking the inward stroke fits the shape.
                if !valid_color(&style.color)
                    || !style.stroke_width.is_finite()
                    || !(0.25..=64.).contains(&style.stroke_width)
                    || style.stroke_width > source.width.min(source.height)
                    || !style.opacity.is_finite()
                    || !(0.05..=1.).contains(&style.opacity)
                {
                    return Err("视频矩形坐标或样式无效".into());
                }
            }
            ModelVideoPrimitive::Text {
                top_left,
                text,
                color,
                font_size,
                ..
            } => {
                let point = mewu_core::video_annotation_source_point(
                    *top_left,
                    target.source.source_width,
                    target.source.source_height,
                )
                .map_err(|e| e.to_string())?;
                // Same literal-text contract as native render_text and the
                // durable layout validator; actual glyph fit remains a render check.
                if !valid_color(color)
                    || text.trim().is_empty()
                    || text.len() > 4096
                    || text.chars().count() > 500
                    || text
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t')
                    || !font_size.is_finite()
                    || !(8. ..=128.).contains(font_size)
                    || point.x >= f64::from(target.source.source_width)
                    || point.y >= f64::from(target.source.source_height)
                {
                    return Err("视频批注文字无效".into());
                }
            }
        }
    }
    Ok(PreparedModel(parsed))
}
fn render_model(
    parsed: PreparedModel,
    input: &VerifiedVideoInput,
    live: impl Fn() -> Result<(), String>,
) -> Result<(ValidatedVideoAppendBatch, Vec<VerifiedVideoTextLayout>), String> {
    let target = &input.manifest().targets[0];
    let mut objects = Vec::new();
    let mut layouts = Vec::new();
    let (mut bytes, mut pixels) = (0usize, 0u64);
    for object in parsed.0.objects {
        live()?;
        let interval = mewu_core::resolve_video_model_interval(target, &object.interval)
            .map_err(|e| e.to_string())?;
        let primitive = match object.primitive {
            ModelVideoPrimitive::Rect { bounds, style } => VideoAnnotationPrimitive::Rect {
                bounds: mewu_core::video_annotation_source_rect(
                    bounds,
                    target.source.source_width,
                    target.source.source_height,
                )
                .map_err(|e| e.to_string())?,
                style,
            },
            ModelVideoPrimitive::Text {
                top_left,
                text,
                color,
                font_size,
                ..
            } => {
                let top_left = mewu_core::video_annotation_source_point(
                    top_left,
                    target.source.source_width,
                    target.source.source_height,
                )
                .map_err(|e| e.to_string())?;
                let layout = crate::video_annotation_raster::render_text(VideoTextContent {
                    version: 1,
                    text,
                    color,
                    font_size,
                })?;
                bytes = bytes
                    .checked_add(layout.png().len())
                    .ok_or("视频文字图片过大")?;
                pixels = pixels
                    .checked_add(u64::from(layout.width()) * u64::from(layout.height()))
                    .ok_or("视频文字图层过大")?;
                if bytes > crate::video_annotation_raster::MAX_TOTAL_PNG_BYTES
                    || pixels > crate::video_annotation_raster::MAX_TOTAL_PIXELS
                {
                    return Err("视频文字图层预算超限".into());
                }
                let primitive = VideoAnnotationPrimitive::Text {
                    top_left,
                    layout: layout.reference().clone(),
                };
                layouts.push(layout);
                primitive
            }
        };
        objects.push(VideoAnnotationDraft {
            interval,
            primitive,
        });
    }
    live()?;
    Ok((
        ValidatedVideoAppendBatch {
            groups: vec![VideoAppendGroup {
                target_handle: target.handle.clone(),
                objects,
            }],
        },
        layouts,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::*;
    use rusqlite::{types::Value as SqlValue, Connection};
    use sha2::{Digest, Sha256};
    use std::{cell::Cell, path::PathBuf};

    const SECOND: u64 = 10_000_000;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    struct Db(PathBuf);
    impl Db {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("mewu-video-tool-test-{}", uid()));
            assert!(!root
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("c:\\hermes"));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn path(&self) -> PathBuf {
            self.0.join("spaces.db")
        }
        fn conn(&self) -> Connection {
            Connection::open(self.path()).unwrap()
        }
    }
    impl Drop for Db {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // Geometry/identity is synthetic; these tests do not claim MP4 frame decoding.
    // JPEG bytes, native text glyph PNGs, SQLite transactions and receipts are real.
    struct Fixture {
        store: Store,
        scene: String,
        item: String,
        run: String,
        input: VerifiedVideoInput,
        authority: VerifiedVideoAnnotationAuthority,
        grant: VisualAnnotationGrant,
    }
    impl Fixture {
        fn new(db: &Db) -> Self {
            let mut store = Store::open(db.path()).unwrap();
            let scene = store.snapshot().active_scene_id;
            let asset = Asset {
                id: uid(),
                name: "synthetic-video.mp4".into(),
                kind: AssetKind::Video,
                path: db
                    .0
                    .join("synthetic-video.mp4")
                    .to_string_lossy()
                    .into_owned(),
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
                    asset,
                    duration_ticks: 6 * SECOND,
                    width: 640,
                    height: 360,
                },
                hash(b"synthetic source identity"),
            )
            .unwrap();
            let source =
                video_source_fence(&store.video_source(&scene, &item).unwrap(), &proof).unwrap();
            let mut frames = Vec::new();
            let mut assets = Vec::new();
            for (ordinal, tick) in [0, 3 * SECOND].into_iter().enumerate() {
                let pixels =
                    image::RgbImage::from_pixel(160, 90, image::Rgb([ordinal as u8 * 120, 80, 30]));
                let mut jpeg = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
                    .encode_image(&pixels)
                    .unwrap();
                let id = uid();
                let path = db.0.join(format!("{id}.jpg"));
                std::fs::write(&path, &jpeg).unwrap();
                frames.push(VideoInputFrame {
                    handle: uid(),
                    attachment_id: id.clone(),
                    attachment_ordinal: ordinal as u32,
                    jpeg_sha256: hash(&jpeg),
                    encoded_width: 160,
                    encoded_height: 90,
                    source_pts_ticks: tick,
                    sample_duration_ticks: SECOND / 30,
                    source_playback_ticks: tick,
                });
                assets.push(Asset {
                    id,
                    name: format!("frame-{ordinal}.jpg"),
                    kind: AssetKind::Image,
                    path: path.to_string_lossy().into_owned(),
                    width: Some(160),
                    height: Some(90),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                });
            }
            let input = VerifiedVideoInput::new(VideoInputManifest {
                version: 1,
                profile_id: VIDEO_PROFILE_ID.into(),
                targets: vec![VideoInputTarget {
                    handle: uid(),
                    item_id: item.clone(),
                    source,
                    sent_window: VideoRange {
                        start_ticks: 0,
                        end_ticks: 6 * SECOND,
                    },
                    range_end_handle: uid(),
                    frames,
                }],
            })
            .unwrap();
            let grant = VisualAnnotationGrant {
                plugin_id: "mewu.annotations".into(),
                plugin_revision: 1,
                contribution_id: "answer".into(),
            };
            store
                .apply(SceneCommand::SetRefs {
                    scene_id: scene.clone(),
                    refs: vec![Reference {
                        kind: ReferenceKind::Item,
                        id: item.clone(),
                    }],
                })
                .unwrap();
            store
                .apply(SceneCommand::SetDraft {
                    scene_id: scene.clone(),
                    draft: "在视频上标出答案".into(),
                })
                .unwrap();
            let run = store
                .begin_run_with_video_input(
                    &scene,
                    None,
                    VerifiedVideoRunInput::new(grant.clone(), input.clone(), assets).unwrap(),
                )
                .unwrap()
                .run_id;
            let authority =
                VerifiedVideoAnnotationAuthority::new(grant.clone(), input.sha256().into())
                    .unwrap();
            Self {
                store,
                scene,
                item,
                run,
                input,
                authority,
                grant,
            }
        }
        fn target(&self) -> VideoAnnotationTarget {
            let source = self.store.video_source(&self.scene, &self.item).unwrap();
            VideoAnnotationTarget {
                scene_id: self.scene.clone(),
                item_id: self.item.clone(),
                source_id: source.asset.id,
                expected_range_revision: source.edit.map_or(0, |v| v.revision),
                expected_annotation_revision: source.annotations.map_or(0, |v| v.revision),
            }
        }
        fn lease(&mut self, arguments: &str) -> DispatchLease {
            let model = self
                .store
                .begin_model_request(
                    &self.scene,
                    &self.run,
                    ModelDispatchInput {
                        round: 0,
                        projected_request_sha256: hash(b"synthetic request, no network"),
                    },
                )
                .unwrap();
            self.store
                .record_model_turn(
                    &model,
                    CompletePublicTurn {
                        text: "已识别目标".into(),
                        calls: vec![ProposedTool {
                            call_id: uid(),
                            binding: self.grant.video_binding(self.input.sha256()),
                            arguments_wire_sha256: hash(arguments.as_bytes()),
                        }],
                    },
                )
                .unwrap()
                .tools
                .remove(0)
                .lease
        }
        fn document(&self) -> VideoAnnotationDocument {
            self.store
                .video_source(&self.scene, &self.item)
                .unwrap()
                .annotations
                .unwrap()
        }
        fn phase(&self) -> ReceiptPhase {
            self.store
                .run_journal_page(&self.scene, &self.run, None, 25)
                .unwrap()
                .unwrap()
                .entries
                .into_iter()
                .find(|v| v.kind == ReceiptKind::Tool)
                .unwrap()
                .phase
        }
    }
    fn rect() -> Value {
        json!({"kind":"rect","bounds":{"x":0,"y":0,"width":1000,"height":1000},
            "style":{"color":"#1357b9","strokeWidth":4,"opacity":0.5}})
    }
    fn text() -> Value {
        json!({"kind":"text","topLeft":{"x":100,"y":250},"anchor":"topLeft",
            "text":"答案 42\n<svg>&","color":"#123456","fontSize":24})
    }
    fn args(input: &VerifiedVideoInput, primitives: Vec<Value>) -> Value {
        let t = &input.manifest().targets[0];
        json!({"coordinateSpace":"normalized1000","styleUnit":"sourcePixels","timeSpace":"sentFrameHandles",
            "objects":primitives.into_iter().map(|primitive|json!({"targetHandle":t.handle,
                "interval":{"startFrameHandle":t.frames[0].handle,"endHandle":t.range_end_handle},"primitive":primitive})).collect::<Vec<_>>()})
    }
    fn proof(db: &Connection) -> Vec<Vec<Vec<SqlValue>>> {
        [
            "app_state",
            "agent_run_records",
            "agent_run_events",
            "video_run_inputs",
            "video_text_layouts",
        ]
        .iter()
        .map(|table| {
            let mut stmt = db
                .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
                .unwrap();
            let count = stmt.column_count();
            let rows = stmt
                .query_map([], |row| {
                    (0..count).map(|i| row.get::<_, SqlValue>(i)).collect()
                })
                .unwrap()
                .collect::<Result<Vec<Vec<SqlValue>>, _>>()
                .unwrap();
            rows
        })
        .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn video_tool_can_reply_in_text_without_effects_or_a_second_request() {
        let db = Db::new();
        let f = Fixture::new(&db);
        let before = f.store.snapshot();
        let sql = db.conn();
        let before_sql = proof(&sql);
        let scope = RunScope::new(f.scene.clone(), f.grant.clone(), f.input.clone()).unwrap();
        ai::tests::assert_optional_annotation_text_turn(scope.definition()).await;
        assert_eq!(f.store.snapshot(), before);
        assert_eq!(proof(&sql), before_sql);
        assert!(f
            .store
            .video_source(&f.scene, &f.item)
            .unwrap()
            .annotations
            .is_none());
    }

    #[test]
    fn last_of_48_invalid_style_or_text_is_rejected_before_any_render() {
        let db = Db::new();
        let mut f = Fixture::new(&db);
        let mut invalid = Vec::new();
        for (path, value) in [
            ("/style/color", json!("#12345g")),
            ("/style/strokeWidth", json!(0.24)),
            ("/style/strokeWidth", json!(65)),
            ("/style/opacity", json!(0.049)),
            ("/style/opacity", json!(1.01)),
            ("/bounds/width", json!(1)),
        ] {
            let mut v = rect();
            *v.pointer_mut(path).unwrap() = value;
            invalid.push(v);
        }
        for (path, value) in [
            ("/color", json!("red")),
            ("/fontSize", json!(7.9)),
            ("/fontSize", json!(128.1)),
            ("/text", json!(" \n\t")),
            ("/text", json!("x".repeat(501))),
            ("/text", json!("a\u{0000}b")),
            ("/text", json!("a\rb")),
            ("/topLeft/x", json!(1000)),
            ("/anchor", json!("center")),
        ] {
            let mut v = text();
            *v.pointer_mut(path).unwrap() = value;
            invalid.push(v);
        }
        let raw = args(&f.input, vec![text(), rect()]).to_string();
        let lease = f.lease(&raw);
        let sql = db.conn();
        let before = proof(&sql);
        for bad in invalid {
            let mut primitives = vec![text(); 47];
            primitives.push(bad);
            let raw = args(&f.input, primitives).to_string();
            let entered = Cell::new(0);
            let result = parse_model(&raw, &f.input).and_then(|plan| {
                render_model(plan, &f.input, || {
                    entered.set(entered.get() + 1);
                    Ok(())
                })
            });
            assert!(result.is_err());
            assert_eq!(entered.get(), 0, "invalid batch reached renderer");
            assert_eq!(proof(&sql), before);
        }
        f.store
            .record_tool_not_sent(&lease, NotSentReason::InvalidArguments)
            .unwrap();
        assert_eq!(f.phase(), ReceiptPhase::NotSent);
        assert!(f
            .store
            .video_source(&f.scene, &f.item)
            .unwrap()
            .annotations
            .is_none());
        assert_eq!(
            sql.query_row("SELECT count(*) FROM video_text_layouts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn normalized_edges_handle_identity_and_wire_units_are_strict() {
        let db = Db::new();
        let f = Fixture::new(&db);
        let valid = args(&f.input, vec![rect()]);
        let (batch, layouts) = render_model(
            parse_model(&valid.to_string(), &f.input).unwrap(),
            &f.input,
            || Ok(()),
        )
        .unwrap();
        assert!(layouts.is_empty());
        let VideoAnnotationPrimitive::Rect { bounds, style } =
            &batch.groups[0].objects[0].primitive
        else {
            panic!("expected rect")
        };
        assert_eq!(
            *bounds,
            VideoPixelRect {
                x: 0.,
                y: 0.,
                width: 640.,
                height: 360.
            }
        );
        assert_eq!(style.stroke_width, 4.);
        for (path, value) in [
            ("/coordinateSpace", json!("encodedPixels")),
            ("/styleUnit", json!("encodedPixels")),
            ("/timeSpace", json!("seconds")),
            ("/objects/0/targetHandle", json!(uid())),
            ("/objects/0/interval/startFrameHandle", json!(uid())),
            ("/objects/0/interval/endHandle", json!(uid())),
            (
                "/objects/0/interval/endHandle",
                json!(f.input.manifest().targets[0].frames[0].handle),
            ),
            ("/objects/0/primitive/bounds/x", json!(-0.01)),
            ("/objects/0/primitive/bounds/x", json!(0.01)),
            ("/objects/0/primitive/bounds/width", json!(1000.01)),
            ("/objects/0/primitive/bounds/height", json!(0)),
        ] {
            let mut bad = valid.clone();
            *bad.pointer_mut(path).unwrap() = value;
            assert!(
                parse_model(&bad.to_string(), &f.input).is_err(),
                "accepted {path}"
            );
        }
        let mut bad = valid.clone();
        bad["objects"][0]["interval"]["startTicks"] = json!(0);
        assert!(parse_model(&bad.to_string(), &f.input).is_err());
        let mut bad = valid.clone();
        bad["objects"][0]["primitive"]["origin"] = json!({"runId":uid()});
        assert!(parse_model(&bad.to_string(), &f.input).is_err());
        assert!(parse_model(&args(&f.input, vec![rect(); 49]).to_string(), &f.input).is_err());
        let duplicate=format!("{{\"coordinateSpace\":\"normalized1000\",\"coordinateSpace\":\"normalized1000\",\"styleUnit\":\"sourcePixels\",\"timeSpace\":\"sentFrameHandles\",\"objects\":{}}}",valid["objects"]);
        assert!(parse_model(&duplicate, &f.input).is_err());
        // Original item/frame UUIDs cannot substitute for opaque target handles.
        let mut bad = valid;
        bad["objects"][0]["targetHandle"] = json!(f.item);
        assert!(parse_model(&bad.to_string(), &f.input).is_err());
    }

    #[test]
    fn mixed_native_rasters_commit_once_with_receipt_then_undo_redo_and_reopen() {
        let db = Db::new();
        let mut f = Fixture::new(&db);
        let raw = args(&f.input, vec![rect(), text()]).to_string();
        let lease = f.lease(&raw);
        let (batch, layouts) =
            render_model(parse_model(&raw, &f.input).unwrap(), &f.input, || Ok(())).unwrap();
        assert_eq!(layouts.len(), 1);
        let layout = layouts[0].clone();
        let pixels =
            crate::video_annotation_raster::decode(layout.png(), layout.width(), layout.height())
                .unwrap();
        assert!(pixels.pixels().any(|p| p[3] > 200));
        assert!(pixels.pixels().any(|p| p[3] == 0));
        assert_eq!(layout.content().text, "答案 42\n<svg>&");
        let VideoAnnotationPrimitive::Text { top_left, .. } = &batch.groups[0].objects[1].primitive
        else {
            panic!("expected text")
        };
        assert_eq!(*top_left, VideoPixelPoint { x: 64., y: 90. });
        let committed = f
            .store
            .apply_video_annotations_with_receipt(
                &lease,
                &f.authority,
                batch.clone(),
                layouts.clone(),
            )
            .unwrap();
        let receipt: Value =
            serde_json::from_str(committed.result.model_visible_json.as_ref().unwrap()).unwrap();
        assert_eq!(receipt["applied"], true);
        assert_eq!(receipt["objects"], 2);
        assert_eq!(f.phase(), ReceiptPhase::Responded);
        let doc = f.document();
        assert_eq!(doc.objects.len(), 2);
        assert_eq!(doc.undo.len(), 1);
        assert_eq!(doc.revision, 1);
        let group_id = receipt["groupId"].as_str().unwrap();
        assert!(doc.objects.iter().all(|o| o
            .origin
            .as_ref()
            .is_some_and(|v| v.run_id == f.run && v.group_id == group_id)));
        assert_eq!(
            f.store.video_text_layout(layout.reference()).unwrap().png(),
            layout.png()
        );
        // Both primitive types use the real native preview/export raster path.
        for annotation in &doc.objects {
            let raster = crate::video_annotation_raster::render(annotation, |r| {
                f.store.video_text_layout(r).map_err(|e| e.to_string())
            })
            .unwrap();
            let pixels = crate::video_annotation_raster::decode(
                &raster.png,
                raster.reference.width,
                raster.reference.height,
            )
            .unwrap();
            assert!(pixels.pixels().any(|p| p[3] > 0));
            if matches!(annotation.primitive, VideoAnnotationPrimitive::Text { .. }) {
                assert_eq!(raster.png.as_slice(), layout.png());
            }
        }
        let state = f.store.snapshot();
        let before = proof(&db.conn());
        assert!(f
            .store
            .apply_video_annotations_with_receipt(&lease, &f.authority, batch, layouts)
            .is_err());
        assert_eq!(f.store.snapshot(), state);
        assert_eq!(proof(&db.conn()), before);
        f.store
            .finish_run(&f.scene, &f.run, "视频标注完成")
            .unwrap();
        let operation = doc.undo[0].id.clone();
        let target = f.target();
        f.store
            .edit_video_annotation_with_layouts(
                &target,
                VideoAnnotationMutation::Undo {
                    expected_operation_id: operation.clone(),
                },
                vec![],
            )
            .unwrap();
        assert!(f.document().objects.is_empty());
        let target = f.target();
        f.store
            .edit_video_annotation_with_layouts(
                &target,
                VideoAnnotationMutation::Redo {
                    expected_operation_id: operation,
                },
                vec![],
            )
            .unwrap();
        let restored = f.document();
        assert_eq!(restored.objects, doc.objects);
        assert_eq!(
            f.store.video_text_layout(layout.reference()).unwrap().png(),
            layout.png()
        );
        let scene = f.scene.clone();
        let item = f.item.clone();
        let run = f.run.clone();
        drop(f);
        let reopened = Store::open(db.path()).unwrap();
        assert_eq!(
            reopened.video_source(&scene, &item).unwrap().annotations,
            Some(restored)
        );
        assert_eq!(
            reopened
                .video_text_layout(layout.reference())
                .unwrap()
                .png(),
            layout.png()
        );
        assert_eq!(
            reopened
                .run_journal_page(&scene, &run, None, 25)
                .unwrap()
                .unwrap()
                .entries
                .into_iter()
                .find(|v| v.kind == ReceiptKind::Tool)
                .unwrap()
                .phase,
            ReceiptPhase::Responded
        );
    }

    #[test]
    fn native_rendered_layout_is_not_left_by_late_fit_error_or_receipt_sql_failure() {
        for sql_failure in [false, true] {
            let db = Db::new();
            let mut f = Fixture::new(&db);
            let mut last = text();
            if !sql_failure {
                last["topLeft"] = json!({"x":999,"y":999});
            }
            let raw = args(&f.input, vec![rect(), last]).to_string();
            let lease = f.lease(&raw);
            // 999 is a valid anchor, but actual measured glyph width cannot fit.
            let (batch, layouts) =
                render_model(parse_model(&raw, &f.input).unwrap(), &f.input, || Ok(())).unwrap();
            assert_eq!(layouts.len(), 1);
            let sql = db.conn();
            let before = proof(&sql);
            let state = f.store.snapshot();
            if sql_failure {
                sql.execute_batch("CREATE TRIGGER fail_video_tool_receipt BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'synthetic receipt failure'); END").unwrap();
            }
            assert!(f
                .store
                .apply_video_annotations_with_receipt(&lease, &f.authority, batch, layouts)
                .is_err());
            assert_eq!(f.store.snapshot(), state);
            assert_eq!(proof(&sql), before);
            assert_eq!(f.phase(), ReceiptPhase::Prepared);
            assert_eq!(
                sql.query_row("SELECT count(*) FROM video_text_layouts", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            if sql_failure {
                sql.execute_batch("DROP TRIGGER fail_video_tool_receipt")
                    .unwrap();
            }
            f.store
                .record_tool_not_sent(&lease, NotSentReason::PreparationFailed)
                .unwrap();
            f.store
                .fail_run(&f.scene, &f.run, "合成失败，未提交批注")
                .unwrap();
            let scene = f.scene.clone();
            let item = f.item.clone();
            drop(sql);
            drop(f);
            let reopened = Store::open(db.path()).unwrap();
            assert!(reopened
                .video_source(&scene, &item)
                .unwrap()
                .annotations
                .is_none());
            assert_eq!(
                db.conn()
                    .query_row("SELECT count(*) FROM video_text_layouts", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}
