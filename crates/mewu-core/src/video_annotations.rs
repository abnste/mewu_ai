// SPDX-License-Identifier: MPL-2.0
//! Durable source-time video annotations, separate from image drawings.
// No Video primitive contains Drawing, Region, renderer path, SVG or PNG bytes.
use crate::journal as j;
use crate::video_annotation_layouts::VideoTextLayoutRef;
use crate::video_vector_layouts::VideoVectorLayoutRef;
use crate::{Asset, VerifiedVideoSource, VideoRange, VisualAnnotationGrant};
use crate::{
    AssetKind, CoreError, JournalContent, ReceiptPhase, Snapshot, SpaceItem, ToolBinding,
    ToolResponseKind, VideoSourceView,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet};

type Result<T> = std::result::Result<T, CoreError>;
pub(crate) const INPUT_TABLE_SQL: &str = "CREATE TABLE video_run_inputs(run_id TEXT PRIMARY KEY NOT NULL,manifest_sha256 TEXT NOT NULL CHECK(length(manifest_sha256)=64),data TEXT NOT NULL CHECK(length(CAST(data AS BLOB))<=131072))";
fn required_range<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<VideoRange>, D::Error> {
    Option::<VideoRange>::deserialize(d)
}

pub const VIDEO_ANNOTATION_TOOL: &str = "video_annotate";
pub const VIDEO_PROFILE_ID: &str = "video-frame-handles-normalized1000-v1";
pub const MAX_VIDEO_ANNOTATIONS: usize = 48;
pub const MAX_VIDEO_HISTORY: usize = 50;
pub const MAX_VIDEO_HISTORY_BYTES: usize = 64 * 1024;
pub const MAX_VIDEO_DOCUMENT_BYTES: usize = 256 * 1024;
pub const MAX_VIDEO_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_VIDEO_SENT_FRAMES: usize = 6;
pub const MIN_VIDEO_INTERVAL_TICKS: u64 = 1_000_000;
pub const MAX_VIDEO_DURATION_TICKS: u64 = 18_000_000_000;
pub const MAX_VIDEO_WINDOW_TICKS: u64 = MAX_VIDEO_DURATION_TICKS;
pub const MAX_SAFE_REVISION: u64 = 9_007_199_254_740_991;

// Distinct evaluator argument types: decoded sample identity never accidentally
// becomes an overlay lifecycle clock. Neither wrapper alone proves native input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePlaybackTicks(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActualVideoPtsTicks(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoSourceClock {
    #[serde(rename = "sourcePlaybackTicks")]
    SourcePlaybackTicks,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoPixelPoint {
    pub x: f64,
    pub y: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoPixelRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoRectStyle {
    pub color: String,
    pub stroke_width: f64, // Source visible decoded pixels, never CSS/encoded px.
    pub opacity: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum VideoAnnotationPrimitive {
    Rect {
        bounds: VideoPixelRect,
        style: VideoRectStyle,
    },
    // Native fixed typography + PNG live in video_text_layouts. Fixed 1:1 source
    // pixel placement; user changes to text/style create a new verified layout.
    Text {
        top_left: VideoPixelPoint,
        layout: VideoTextLayoutRef,
    },
    // Complete immutable ink, including edge strokes; final consumers crop.
    // Only manual full-source-time objects can contain this typed reference.
    Vector {
        top_left: VideoPixelPoint,
        layout: VideoVectorLayoutRef,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoAnnotationOrigin {
    pub run_id: String,
    pub user_message_id: String,
    pub tool_event_id: String,
    pub group_id: String,
    pub target_handle: String,
    pub manifest_sha256: String,
    // Identity of the created interval + primitive. Does not claim subsequent
    // user edits were made by the model. Must equal immutable receipt identity.
    pub created_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimedVideoAnnotation {
    pub id: String,
    pub interval: VideoRange, // [start,end) on logical original playback clock.
    pub primitive: VideoAnnotationPrimitive,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<VideoAnnotationOrigin>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoAnnotationStep {
    pub id: String, // AI groupId or new manual operation UUID, never model ID.
    pub before: Vec<TimedVideoAnnotation>,
    pub after: Vec<TimedVideoAnnotation>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoAnnotationDocument {
    pub version: u32,
    pub clock: VideoSourceClock,
    pub source_id: String,
    pub source_duration_ticks: u64,
    pub source_width: u32,
    pub source_height: u32,
    pub revision: u64,
    pub objects: Vec<TimedVideoAnnotation>,
    pub undo: Vec<VideoAnnotationStep>,
    pub redo: Vec<VideoAnnotationStep>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoAnnotationTarget {
    pub scene_id: String,
    pub item_id: String,
    pub source_id: String,
    pub expected_range_revision: u64,
    pub expected_annotation_revision: u64, // Absent document reads 0, no creation.
}

// Trusted native-only source evidence; no Deserialize and no IPC constructor.
// The native caller must retain actual source lease through decode/commit/render.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedVideoAnnotationSource {
    pub(crate) source: VerifiedVideoSource,
    pub(crate) source_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoSourceFence {
    pub clock: VideoSourceClock,
    pub source_id: String,
    pub asset_sha256: String, // Core's deterministic serialization of exact Asset.
    pub source_sha256: String, // Native actual leased source bytes; not model data.
    pub source_duration_ticks: u64,
    pub source_width: u32,
    pub source_height: u32,
    pub range_revision: u64,
    #[serde(deserialize_with = "required_range")]
    pub range: Option<VideoRange>, // Required field, null = full original.
    pub annotation_revision: u64,
    pub document_sha256: String, // SHA over Option<Document>; None is distinct.
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoInputFrame {
    pub handle: String,
    pub attachment_id: String,
    pub attachment_ordinal: u32,
    pub jpeg_sha256: String,
    pub encoded_width: u32,
    pub encoded_height: u32,
    pub source_pts_ticks: u64, // Actual decode sample identity, never request time.
    pub sample_duration_ticks: u64, // Actual native sample duration, not frame rate.
    // Explicit native verified mapping into original Asset playback timeline;
    // no model inference, no provider-name scale guess, no nearest requested time.
    pub source_playback_ticks: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoInputTarget {
    pub handle: String,
    pub item_id: String,
    pub source: VideoSourceFence,
    pub sent_window: VideoRange, // Captured retained range; bounded sparse frames.
    pub range_end_handle: String, // Opaque boundary token at exact sent_window.end.
    pub frames: Vec<VideoInputFrame>, // Whole visible frame, no crop/letterbox.
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoInputManifest {
    pub version: u32,
    pub profile_id: String,
    pub targets: Vec<VideoInputTarget>, // First admission 1 video, total2..6frames.
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVideoInput {
    pub(crate) manifest: VideoInputManifest,
    pub(crate) sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVideoAnnotationAuthority {
    pub(crate) grant: VisualAnnotationGrant, // Existing plugin/rev/contribution data.
    pub(crate) manifest_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredVideoInput {
    pub manifest: VideoInputManifest,
    pub grant: VisualAnnotationGrant,
    pub cursors: BTreeMap<String, VideoSourceFence>, // itemId, own commits advance.
    pub created_count: u32,
    pub text_content_bytes: u64,
    pub text_png_bytes: u64,
    pub text_pixels: u64,
    pub revoked: bool,
}

// Model DTO has no sceneId/itemId/sourceId/raw seconds/ticks/layout/path/origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelVideoInterval {
    pub start_frame_handle: String,
    pub end_handle: String, // Later frame.handle or this target.range_end_handle.
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelVideoTextAnchor {
    #[serde(rename = "topLeft")]
    TopLeft,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ModelVideoPrimitive {
    Rect {
        bounds: VideoPixelRect,
        style: VideoRectStyle,
    }, // normalized1000 bounds.
    Text {
        top_left: VideoPixelPoint,
        anchor: ModelVideoTextAnchor,
        text: String,
        color: String,
        font_size: f64,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelVideoObject {
    pub target_handle: String,
    pub interval: ModelVideoInterval,
    pub primitive: ModelVideoPrimitive,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelVideoAppend {
    pub coordinate_space: String,       // Must equal normalized1000.
    pub style_unit: String,             // Must equal sourcePixels.
    pub time_space: String,             // Must equal sentFrameHandles.
    pub objects: Vec<ModelVideoObject>, // Entire call reject if any invalid.
}
#[derive(Debug, Clone, PartialEq)]
pub struct VideoAnnotationDraft {
    pub interval: VideoRange,
    pub primitive: VideoAnnotationPrimitive, // Native verified text ref supplied.
}
#[derive(Debug, Clone, PartialEq)]
pub struct VideoAppendGroup {
    pub target_handle: String,
    pub objects: Vec<VideoAnnotationDraft>, // No model IDs/origins accepted.
}
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedVideoAppendBatch {
    pub groups: Vec<VideoAppendGroup>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum VideoAnnotationMutation {
    Append(Vec<TimedVideoAnnotation>), // Core stamps IDs+origin before this call.
    Update {
        id: String,
        replacement: VideoAnnotationDraft,
    },
    Remove {
        id: String,
    },
    Undo {
        expected_operation_id: String,
    },
    Redo {
        expected_operation_id: String,
    },
}

// Include complete document in existing VideoSourceView/VideoExportOrigin.
// A card movement or playback position is deliberately outside this identity.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoAnnotatedSourceView {
    pub asset: Asset,
    pub edit: Option<crate::VideoEdit>,
    pub annotations: Option<VideoAnnotationDocument>,
}

impl VerifiedVideoAnnotationSource {
    /// Native owns actual source-file lease and decoded metadata. This boundary
    /// validates their shape; it cannot discover or authenticate a file itself.
    pub fn new(source: VerifiedVideoSource, source_sha256: String) -> Result<Self> {
        validate_source(&source)?;
        if !j::hash_valid(&source_sha256) {
            return Err(j::bad("视频文件摘要无效"));
        }
        Ok(Self {
            source,
            source_sha256,
        })
    }
    pub fn source(&self) -> &VerifiedVideoSource {
        &self.source
    }
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
}
fn validate_source(v: &VerifiedVideoSource) -> Result<()> {
    j::uuid(&v.asset.id)?;
    if v.asset.kind != AssetKind::Video
        || v.duration_ticks == 0
        || v.duration_ticks > MAX_VIDEO_DURATION_TICKS
        || v.width < 2
        || v.height < 2
        || v.width > 16384
        || v.height > 16384
        || u64::from(v.width) * u64::from(v.height) > 4_000_000
        || v.asset.width.is_some_and(|w| w != v.width)
        || v.asset.height.is_some_and(|h| h != v.height)
    {
        return Err(j::bad("视频来源元数据无效"));
    }
    Ok(())
}
pub fn video_source_fence(
    view: &VideoSourceView,
    proof: &VerifiedVideoAnnotationSource,
) -> Result<VideoSourceFence> {
    validate_source(&proof.source)?;
    if view.asset != proof.source.asset
        || !j::hash_valid(&proof.source_sha256)
        || view.edit.as_ref().is_some_and(|e| {
            e.source_id != view.asset.id || e.source_duration_ticks != proof.source.duration_ticks
        })
        || view.annotations.as_ref().is_some_and(|d| {
            d.source_id != view.asset.id
                || d.source_duration_ticks != proof.source.duration_ticks
                || d.source_width != proof.source.width
                || d.source_height != proof.source.height
        })
    {
        return Err(CoreError::VideoAnnotationConflict);
    }
    let fence = VideoSourceFence {
        clock: VideoSourceClock::SourcePlaybackTicks,
        source_id: view.asset.id.clone(),
        asset_sha256: j::digest(&serde_json::to_vec(&view.asset)?),
        source_sha256: proof.source_sha256.clone(),
        source_duration_ticks: proof.source.duration_ticks,
        source_width: proof.source.width,
        source_height: proof.source.height,
        range_revision: view.edit.as_ref().map_or(0, |e| e.revision),
        range: view.edit.as_ref().and_then(|e| e.range),
        annotation_revision: view.annotations.as_ref().map_or(0, |d| d.revision),
        document_sha256: j::digest(&serde_json::to_vec(&view.annotations)?),
    };
    validate_fence(&fence)?;
    Ok(fence)
}
pub(crate) fn validate_fence(v: &VideoSourceFence) -> Result<()> {
    j::uuid(&v.source_id)?;
    if [&v.asset_sha256, &v.source_sha256, &v.document_sha256]
        .iter()
        .any(|s| !j::hash_valid(s))
        || v.source_duration_ticks == 0
        || v.source_duration_ticks > MAX_VIDEO_DURATION_TICKS
        || v.source_width < 2
        || v.source_height < 2
        || v.source_width > 16384
        || v.source_height > 16384
        || u64::from(v.source_width) * u64::from(v.source_height) > 4_000_000
        || v.range_revision > MAX_SAFE_REVISION
        || v.annotation_revision > MAX_SAFE_REVISION
    {
        return Err(j::bad("视频来源围栏无效"));
    }
    if let Some(r) = v.range {
        crate::video_annotation_algorithm::validate_interval(r, v.source_duration_ticks)?;
        if r.start_ticks == 0 && r.end_ticks == v.source_duration_ticks || v.range_revision == 0 {
            return Err(j::bad("完整视频范围必须为空"));
        }
    }
    if v.annotation_revision == 0 && v.document_sha256 != j::digest(b"null") {
        return Err(j::bad("视频批注文档围栏无效"));
    }
    Ok(())
}
pub(crate) fn fence_for_item(
    item: &SpaceItem,
    original: &VideoSourceFence,
) -> Result<VideoSourceFence> {
    video_source_fence(
        &VideoSourceView {
            asset: item.asset.clone(),
            edit: item.video_edit.clone(),
            annotations: item.video_annotations.clone(),
        },
        &VerifiedVideoAnnotationSource::new(
            VerifiedVideoSource {
                asset: item.asset.clone(),
                duration_ticks: original.source_duration_ticks,
                width: original.source_width,
                height: original.source_height,
            },
            original.source_sha256.clone(),
        )?,
    )
}
pub(crate) fn validate_item(item: &SpaceItem) -> Result<()> {
    if let Some(doc) = &item.video_annotations {
        crate::video_annotation_algorithm::validate_document(&item.asset, doc)?;
        if item
            .video_edit
            .as_ref()
            .is_some_and(|e| e.source_duration_ticks != doc.source_duration_ticks)
        {
            return Err(j::bad("视频裁切与批注来源时长不一致"));
        }
    }
    Ok(())
}
impl VerifiedVideoInput {
    pub fn new(manifest: VideoInputManifest) -> Result<Self> {
        validate_manifest(&manifest)?;
        let sha256 = j::digest(&serde_json::to_vec(&manifest)?);
        Ok(Self { manifest, sha256 })
    }
    pub fn manifest(&self) -> &VideoInputManifest {
        &self.manifest
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}
impl VerifiedVideoAnnotationAuthority {
    pub fn new(grant: VisualAnnotationGrant, manifest_sha256: String) -> Result<Self> {
        crate::visual_annotations::validate_grant(&grant)?;
        if !j::hash_valid(&manifest_sha256) {
            return Err(j::bad("视频输入摘要无效"));
        }
        Ok(Self {
            grant,
            manifest_sha256,
        })
    }
}
impl VisualAnnotationGrant {
    pub fn video_binding(&self, manifest_sha256: &str) -> ToolBinding {
        ToolBinding::VideoAnnotation {
            plugin_id: self.plugin_id.clone(),
            plugin_revision: self.plugin_revision,
            contribution_id: self.contribution_id.clone(),
            name: VIDEO_ANNOTATION_TOOL.into(),
            target_manifest_sha256: manifest_sha256.into(),
        }
    }
}
#[derive(Debug, Clone)]
pub struct VerifiedVideoRunInput {
    pub(crate) grant: VisualAnnotationGrant,
    pub(crate) input: VerifiedVideoInput,
    pub(crate) assets: Vec<Asset>,
}
impl VerifiedVideoRunInput {
    pub fn new(
        grant: VisualAnnotationGrant,
        input: VerifiedVideoInput,
        assets: Vec<Asset>,
    ) -> Result<Self> {
        crate::visual_annotations::validate_grant(&grant)?;
        validate_manifest(input.manifest())?;
        validate_asset_binding(input.manifest(), &assets)?;
        Ok(Self {
            grant,
            input,
            assets,
        })
    }
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }
    pub fn input(&self) -> &VerifiedVideoInput {
        &self.input
    }
}
pub(crate) fn validate_manifest(v: &VideoInputManifest) -> Result<()> {
    if v.version != 1
        || v.profile_id != VIDEO_PROFILE_ID
        || v.targets.len() != 1
        || serde_json::to_vec(v)?.len() > MAX_VIDEO_INPUT_BYTES
    {
        return Err(j::bad("视频输入清单无效"));
    }
    let mut handles = HashSet::new();
    let mut asset_ids = HashSet::new();
    let mut ordinals = HashSet::new();
    let t = &v.targets[0];
    validate_fence(&t.source)?;
    j::uuid(&t.item_id)?;
    for h in [&t.handle, &t.range_end_handle] {
        j::uuid(h)?;
        if !handles.insert(h) {
            return Err(j::bad("视频句柄重复"));
        }
    }
    let range = t.source.range.unwrap_or(VideoRange {
        start_ticks: 0,
        end_ticks: t.source.source_duration_ticks,
    });
    crate::video_annotation_algorithm::validate_interval(
        t.sent_window,
        t.source.source_duration_ticks,
    )?;
    if t.sent_window != range || t.frames.len() < 2 || t.frames.len() > MAX_VIDEO_SENT_FRAMES {
        return Err(j::bad("视频帧必须覆盖当前保留范围"));
    }
    let mut previous = None;
    for (i, f) in t.frames.iter().enumerate() {
        for h in [&f.handle, &f.attachment_id] {
            j::uuid(h)?;
        }
        if !handles.insert(&f.handle)
            || !asset_ids.insert(&f.attachment_id)
            || !ordinals.insert(f.attachment_ordinal)
            || f.attachment_ordinal as usize >= MAX_VIDEO_SENT_FRAMES
            || i > 0 && t.frames[i - 1].attachment_ordinal >= f.attachment_ordinal
            || !j::hash_valid(&f.jpeg_sha256)
            || f.encoded_width == 0
            || f.encoded_height == 0
            || f.encoded_width > t.source.source_width
            || f.encoded_height > t.source.source_height
            || f.sample_duration_ticks == 0
            || f.source_pts_ticks > f.source_playback_ticks
            || f.source_pts_ticks >= t.source.source_duration_ticks
            || f.source_pts_ticks
                .checked_add(f.sample_duration_ticks)
                .is_none()
            || f.source_playback_ticks < t.sent_window.start_ticks
            || f.source_playback_ticks >= t.sent_window.end_ticks
            || (i == 0 && f.source_playback_ticks != t.sent_window.start_ticks)
        {
            return Err(j::bad("视频帧映射无效"));
        }
        // Whole decoded visible frame downsampling: at most one rounded output
        // pixel discrepancy, never letterbox/crop. Native proves actual bytes.
        let expected = f64::from(f.encoded_width) * f64::from(t.source.source_height)
            / f64::from(t.source.source_width);
        if (expected - f64::from(f.encoded_height)).abs() > 1. {
            return Err(j::bad("视频附件宽高比无效"));
        }
        if let Some((pts, playback)) = previous {
            if f.source_pts_ticks < pts || f.source_playback_ticks <= playback {
                return Err(j::bad("视频帧顺序无效"));
            }
        }
        previous = Some((f.source_pts_ticks, f.source_playback_ticks));
    }
    Ok(())
}
pub(crate) fn validate_asset_binding(v: &VideoInputManifest, assets: &[Asset]) -> Result<()> {
    validate_manifest(v)?;
    if assets.len() < v.targets[0].frames.len() || assets.len() > MAX_VIDEO_SENT_FRAMES {
        return Err(j::bad("视频附件数量不匹配"));
    }
    let mut ids = HashSet::new();
    for asset in assets {
        j::uuid(&asset.id)?;
        if !ids.insert(&asset.id)
            || asset.name.trim().is_empty()
            || asset.path.trim().is_empty()
            || asset.path.chars().any(char::is_control)
            || !matches!(
                asset.kind,
                AssetKind::Image | AssetKind::Html | AssetKind::Svg | AssetKind::Text
            )
            || asset.origin_x.is_some()
            || asset.origin_y.is_some()
            || asset.scale_factor.is_some()
        {
            return Err(j::bad("视频上下文附件身份或类型无效"));
        }
        match asset.kind {
            AssetKind::Image => {
                if !asset.width.zip(asset.height).is_some_and(|(w, h)| {
                    w > 0
                        && h > 0
                        && w <= 16384
                        && h <= 16384
                        && u64::from(w) * u64::from(h) <= 32 * 1024 * 1024
                }) {
                    return Err(j::bad("视频上下文图片尺寸无效"));
                }
            }
            AssetKind::Html | AssetKind::Svg | AssetKind::Text => {
                if asset.width.is_some() || asset.height.is_some() {
                    return Err(j::bad("视频上下文文档不能包含图像尺寸"));
                }
            }
            _ => return Err(j::bad("视频上下文附件类型无效")),
        }
    }
    for f in &v.targets[0].frames {
        let a = assets
            .get(f.attachment_ordinal as usize)
            .ok_or_else(|| j::bad("视频附件序号越界"))?;
        if a.id != f.attachment_id
            || a.kind != AssetKind::Image
            || a.width != Some(f.encoded_width)
            || a.height != Some(f.encoded_height)
        {
            return Err(j::bad("视频附件身份不匹配"));
        }
    }
    Ok(())
}
pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(&format!("{INPUT_TABLE_SQL};"))?;
    Ok(())
}
fn record_scene<'a>(state: &'a Snapshot, record: &j::Record) -> Result<&'a crate::Scene> {
    state
        .scenes
        .iter()
        .find(|s| s.id == record.scene && s.agent_id == record.agent)
        .ok_or(CoreError::JournalConflict)
}
fn live_record(state: &Snapshot, record: &j::Record) -> Result<()> {
    let scene = record_scene(state, record)?;
    if scene.closed
        || record.status != crate::RecordedRunStatus::Running
        || !scene
            .run
            .as_ref()
            .is_some_and(|r| r.id == record.id && r.status == crate::RunStatus::Running)
    {
        return Err(CoreError::StaleRun);
    }
    Ok(())
}
pub(crate) fn load(db: &Connection, run: &str) -> Result<Option<(String, StoredVideoInput)>> {
    let length: Option<i64> = db
        .query_row(
            "SELECT length(CAST(data AS BLOB)) FROM video_run_inputs WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )
        .optional()?;
    let Some(length) = length else {
        return Ok(None);
    };
    if !(1..=131072).contains(&length) {
        return Err(j::bad("视频输入记录超限"));
    }
    let (hash, raw): (String, String) = db.query_row(
        "SELECT manifest_sha256,data FROM video_run_inputs WHERE run_id=?1",
        [run],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Some((hash, serde_json::from_str(&raw)?)))
}
pub(crate) fn register_input(
    db: &Connection,
    state: &mut Snapshot,
    run: &str,
    v: VerifiedVideoRunInput,
) -> Result<()> {
    let record = j::required(db, run)?;
    live_record(state, &record)?;
    if record.kind == crate::RecordedRunKind::Continuation
        || load(db, run)?.is_some()
        || !j::events(db, run)?.is_empty()
    {
        return Err(CoreError::JournalConflict);
    }
    validate_asset_binding(v.input.manifest(), &v.assets)?;
    crate::visual_annotations::validate_grant(&v.grant)?;
    let s = state
        .scenes
        .iter()
        .find(|s| s.id == record.scene)
        .ok_or(CoreError::JournalConflict)?;
    let message = s.messages.last().ok_or(CoreError::JournalConflict)?;
    if message.id != record.user
        || message.role != crate::MessageRole::User
        || message.run_id.as_deref() != Some(run)
        || message.attachments.as_ref() != Some(&v.assets)
    {
        return Err(CoreError::JournalConflict);
    }
    let mut cursors = BTreeMap::new();
    for entry in &v.input.manifest.targets {
        if !message.refs.as_ref().is_some_and(|r| {
            r.iter()
                .any(|r| r.kind == crate::ReferenceKind::Item && r.id == entry.item_id)
        }) {
            return Err(CoreError::JournalConflict);
        }
        let item = s
            .items
            .iter()
            .find(|i| i.id == entry.item_id)
            .ok_or(CoreError::VideoAnnotationConflict)?;
        if fence_for_item(item, &entry.source)? != entry.source {
            return Err(CoreError::VideoAnnotationConflict);
        }
        cursors.insert(entry.item_id.clone(), entry.source.clone());
    }
    let data = StoredVideoInput {
        manifest: v.input.manifest,
        grant: v.grant,
        cursors,
        created_count: 0,
        text_content_bytes: 0,
        text_png_bytes: 0,
        text_pixels: 0,
        revoked: false,
    };
    db.execute(
        "INSERT INTO video_run_inputs(run_id,manifest_sha256,data) VALUES(?1,?2,?3)",
        params![run, v.input.sha256, serde_json::to_string(&data)?],
    )?;
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VideoCreatedObject {
    pub id: String,
    pub target_handle: String,
    pub created_sha256: String,
    pub interval: VideoRange,
    pub primitive: VideoAnnotationPrimitive,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VideoCommittedTarget {
    pub item_id: String,
    pub annotation_revision: u64,
    pub source: VideoSourceFence,
    pub objects: Vec<VideoCreatedObject>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VideoCommitReceipt {
    pub applied: bool,
    pub group_id: String,
    pub manifest_sha256: String,
    pub objects: u32,
    pub targets: Vec<VideoCommittedTarget>,
}
pub(crate) fn creation_hash(
    interval: VideoRange,
    primitive: &VideoAnnotationPrimitive,
) -> Result<String> {
    Ok(j::digest(&serde_json::to_vec(&(interval, primitive))?))
}
pub(crate) fn committed_receipts(
    db: &Connection,
    run: &str,
    input: &StoredVideoInput,
    hash: &str,
) -> Result<Vec<(String, VideoCommitReceipt)>> {
    let mut receipts = Vec::new();
    for e in j::events(db, run)? {
        if !matches!(&e.binding, Some(ToolBinding::VideoAnnotation { .. }))
            || e.summary.phase != ReceiptPhase::Responded
            || e.summary.response_kind != Some(ToolResponseKind::LocalCommit)
        {
            continue;
        }
        if e.binding != Some(input.grant.video_binding(hash)) {
            return Err(j::bad("视频批注回执授权不匹配"));
        }
        let (_, _, body) = j::event(db, &e.summary.id)?;
        let JournalContent::Json { value } = body else {
            return Err(j::bad("视频批注回执缺失"));
        };
        let receipt: VideoCommitReceipt = serde_json::from_value(value)?;
        if !receipt.applied
            || receipt.manifest_sha256 != hash
            || receipt.objects == 0
            || receipt.targets.is_empty()
        {
            return Err(j::bad("视频批注回执无效"));
        }
        j::uuid(&receipt.group_id)?;
        let mut count = 0usize;
        let mut ids = HashSet::new();
        let mut items = HashSet::new();
        for t in &receipt.targets {
            if t.annotation_revision == 0
                || t.annotation_revision > MAX_SAFE_REVISION
                || t.objects.is_empty()
                || !items.insert(&t.item_id)
            {
                return Err(j::bad("视频批注回执目标无效"));
            }
            for o in &t.objects {
                if matches!(&o.primitive, VideoAnnotationPrimitive::Vector { .. }) {
                    return Err(j::bad("模型回执不允许视频手绘矢量"));
                }
                j::uuid(&o.id)?;
                if !ids.insert(&o.id)
                    || o.created_sha256 != creation_hash(o.interval, &o.primitive)?
                {
                    return Err(j::bad("视频批注创建身份无效"));
                }
                let target = input
                    .manifest
                    .targets
                    .iter()
                    .find(|x| x.item_id == t.item_id && x.handle == o.target_handle)
                    .ok_or_else(|| j::bad("视频回执目标不匹配"))?;
                validate_sent_interval(target, o.interval)?;
                crate::video_annotation_algorithm::validate_primitive(
                    &o.primitive,
                    target.source.source_width,
                    target.source.source_height,
                )?;
                count += 1;
            }
        }
        if count != receipt.objects as usize || count > MAX_VIDEO_ANNOTATIONS {
            return Err(j::bad("视频批注回执数量无效"));
        }
        receipts.push((e.summary.id, receipt));
    }
    Ok(receipts)
}
pub(crate) fn validate_sent_interval(t: &VideoInputTarget, r: VideoRange) -> Result<()> {
    crate::video_annotation_algorithm::validate_interval(r, t.source.source_duration_ticks)?;
    if !t
        .frames
        .iter()
        .any(|f| f.source_playback_ticks == r.start_ticks)
        || !(r.end_ticks == t.sent_window.end_ticks
            || t.frames
                .iter()
                .any(|f| f.source_playback_ticks == r.end_ticks))
        || r.start_ticks < t.sent_window.start_ticks
        || r.end_ticks > t.sent_window.end_ticks
    {
        return Err(j::bad("视频批注区间必须来自已发送帧句柄"));
    }
    Ok(())
}
pub(crate) fn validate_usage(
    db: &Connection,
    run: &str,
    input: &StoredVideoInput,
    hash: &str,
) -> Result<()> {
    let (mut count, mut content, mut bytes, mut pixels) = (0u64, 0u64, 0u64, 0u64);
    let mut layouts = HashSet::new();
    let mut object_ids = HashSet::new();
    let mut cursors: BTreeMap<String, VideoSourceFence> = input
        .manifest
        .targets
        .iter()
        .map(|t| (t.item_id.clone(), t.source.clone()))
        .collect();
    let mut groups = HashSet::new();
    for (_, receipt) in committed_receipts(db, run, input, hash)? {
        if !groups.insert(receipt.group_id.clone()) {
            return Err(j::bad("视频批注提交组重复"));
        }
        for target in &receipt.targets {
            validate_fence(&target.source)?;
            let previous = cursors
                .get(&target.item_id)
                .ok_or_else(|| j::bad("视频批注提交游标目标无效"))?;
            let mut expected = previous.clone();
            expected.annotation_revision = previous
                .annotation_revision
                .checked_add(1)
                .filter(|v| *v <= MAX_SAFE_REVISION)
                .ok_or_else(|| j::bad("视频批注游标修订号无效"))?;
            expected.document_sha256 = target.source.document_sha256.clone();
            if target.annotation_revision != expected.annotation_revision
                || target.source != expected
            {
                return Err(j::bad("视频提交回执源或文档游标不连续"));
            }
            cursors.insert(target.item_id.clone(), target.source.clone());
        }
        for object in receipt.targets.iter().flat_map(|t| &t.objects) {
            if matches!(&object.primitive, VideoAnnotationPrimitive::Vector { .. }) {
                return Err(j::bad("模型回执不允许视频手绘矢量"));
            }
            if !object_ids.insert(object.id.clone()) {
                return Err(j::bad("视频对象在另一回执重复创建"));
            }
            count += 1;
            if let VideoAnnotationPrimitive::Text { layout, .. } = &object.primitive {
                if !layouts.insert(layout.layout_id.clone()) {
                    return Err(j::bad("视频文字布局在另一回执重复登记"));
                }
                let v = crate::video_annotation_layouts::load(db, layout)?;
                content += v.content_bytes()?;
                bytes += v.png().len() as u64;
                pixels += v.pixels();
            }
        }
    }
    if cursors != input.cursors
        || count != u64::from(input.created_count)
        || content != input.text_content_bytes
        || bytes != input.text_png_bytes
        || pixels != input.text_pixels
        || count > 48
        || content > 64 * 1024
        || bytes > 32 * 1024 * 1024
        || pixels > 32 * 1024 * 1024
    {
        return Err(j::bad("视频批注运行预算不匹配"));
    }
    Ok(())
}
pub(crate) fn validate_ledger(db: &Connection, state: &Snapshot) -> Result<()> {
    let sql: Option<String> = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='video_run_inputs'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if sql.as_deref() != Some(INPUT_TABLE_SQL) {
        return Err(j::bad("视频批注输入存储结构缺失"));
    }
    let mut stmt = db.prepare("SELECT run_id FROM video_run_inputs")?;
    let runs = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for run in runs {
        let (hash, input) = load(db, &run)?.ok_or(CoreError::JournalConflict)?;
        validate_manifest(&input.manifest)?;
        crate::visual_annotations::validate_grant(&input.grant)?;
        if hash != j::digest(&serde_json::to_vec(&input.manifest)?)
            || input.cursors.len() != input.manifest.targets.len()
        {
            return Err(j::bad("视频输入围栏损坏"));
        }
        let record = j::required(db, &run)?;
        let scene = record_scene(state, &record)?;
        let message = scene
            .messages
            .iter()
            .find(|m| m.id == record.user)
            .ok_or(CoreError::JournalConflict)?;
        if message.role != crate::MessageRole::User || message.run_id.as_deref() != Some(&run) {
            return Err(CoreError::JournalConflict);
        }
        validate_asset_binding(
            &input.manifest,
            message.attachments.as_deref().unwrap_or_default(),
        )?;
        for t in &input.manifest.targets {
            if !message.refs.as_ref().is_some_and(|r| {
                r.iter()
                    .any(|r| r.kind == crate::ReferenceKind::Item && r.id == t.item_id)
            }) {
                return Err(j::bad("视频输入原引用缺失"));
            }
            let c = input
                .cursors
                .get(&t.item_id)
                .ok_or_else(|| j::bad("视频输入游标缺失"))?;
            validate_fence(c)?;
            if c.source_id != t.source.source_id
                || c.source_sha256 != t.source.source_sha256
                || c.asset_sha256 != t.source.asset_sha256
                || c.source_width != t.source.source_width
                || c.source_height != t.source.source_height
                || c.source_duration_ticks != t.source.source_duration_ticks
                || c.range_revision != t.source.range_revision
                || c.range != t.source.range
                || c.annotation_revision < t.source.annotation_revision
            {
                return Err(j::bad("视频输入游标损坏"));
            }
        }
        validate_usage(db, &run, &input, &hash)?;
    }
    for scene in &state.scenes {
        for item in &scene.items {
            let Some(doc) = &item.video_annotations else {
                continue;
            };
            let mut origins = HashMap::new();
            for object in doc.objects.iter().chain(
                doc.undo
                    .iter()
                    .chain(&doc.redo)
                    .flat_map(|h| h.before.iter().chain(&h.after)),
            ) {
                if let Some(previous) = origins.insert(&object.id, &object.origin) {
                    if previous != &object.origin {
                        return Err(j::bad("视频对象来源不能被历史编辑替换"));
                    }
                    continue;
                }
                let Some(origin) = &object.origin else {
                    continue;
                };
                let run = j::required(db, &origin.run_id)?;
                let (hash, input) = load(db, &run.id)?.ok_or(CoreError::JournalConflict)?;
                if run.scene != scene.id
                    || run.user != origin.user_message_id
                    || hash != origin.manifest_sha256
                {
                    return Err(j::bad("视频对象来源不匹配"));
                }
                if !input.manifest.targets.iter().any(|t| {
                    t.item_id == item.id
                        && t.handle == origin.target_handle
                        && t.source.source_id == doc.source_id
                        && t.source.source_duration_ticks == doc.source_duration_ticks
                        && t.source.source_width == doc.source_width
                        && t.source.source_height == doc.source_height
                }) {
                    return Err(j::bad("视频对象不能更换原视频来源"));
                }
                let matched = committed_receipts(db, &run.id, &input, &hash)?
                    .into_iter()
                    .any(|(event, r)| {
                        event == origin.tool_event_id
                            && r.group_id == origin.group_id
                            && r.targets.iter().any(|t| {
                                t.item_id == item.id
                                    && t.objects.iter().any(|o| {
                                        o.id == object.id
                                            && o.target_handle == origin.target_handle
                                            && o.created_sha256 == origin.created_sha256
                                    })
                            })
                    });
                if !matched {
                    return Err(j::bad("视频对象缺少原子提交回执"));
                }
            }
        }
    }
    Ok(())
}
