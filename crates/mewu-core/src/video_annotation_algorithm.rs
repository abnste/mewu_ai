// SPDX-License-Identifier: MPL-2.0
//! Pure state transitions, no Store/SQLite/renderer/media side effects.
// Integration must supply immutable layout validation and complete transaction.
use crate::video_annotations::*;
use crate::{Asset, AssetKind, CoreError, VideoRange};
use std::collections::HashSet;
type Result<T> = std::result::Result<T, CoreError>;
fn bad(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|v| v.hyphenated().to_string() == value)
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn revision(value: u64) -> Result<u64> {
    value
        .checked_add(1)
        .filter(|v| *v <= MAX_SAFE_REVISION)
        .ok_or_else(|| bad("视频批注修订号无效"))
}
fn source_dimensions(doc: &VideoAnnotationDocument) -> Result<()> {
    if doc.source_duration_ticks == 0
        || doc.source_duration_ticks > MAX_VIDEO_DURATION_TICKS
        || doc.source_width < 2
        || doc.source_height < 2
        || doc.source_width > 16384
        || doc.source_height > 16384
        || u64::from(doc.source_width) * u64::from(doc.source_height) > 4_000_000
    {
        return Err(bad("视频批注来源尺寸或时长无效"));
    }
    Ok(())
}
pub fn validate_interval(interval: VideoRange, duration: u64) -> Result<()> {
    if interval.start_ticks >= interval.end_ticks
        || interval.end_ticks > duration
        || interval.end_ticks - interval.start_ticks < MIN_VIDEO_INTERVAL_TICKS
    {
        return Err(bad("视频批注需为原视频内至少100毫秒的区间"));
    }
    Ok(())
}
fn color(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit)
}
fn within(point: VideoPixelPoint, width: u32, height: u32) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && point.x >= 0.
        && point.y >= 0.
        && point.x <= f64::from(width)
        && point.y <= f64::from(height)
}
pub fn validate_primitive(value: &VideoAnnotationPrimitive, width: u32, height: u32) -> Result<()> {
    match value {
        VideoAnnotationPrimitive::Rect { bounds, style } => {
            if !within(
                VideoPixelPoint {
                    x: bounds.x,
                    y: bounds.y,
                },
                width,
                height,
            ) || !bounds.width.is_finite()
                || !bounds.height.is_finite()
                || bounds.width <= 0.
                || bounds.height <= 0.
                || bounds.x + bounds.width > f64::from(width)
                || bounds.y + bounds.height > f64::from(height)
                || !color(&style.color)
                || !style.stroke_width.is_finite()
                || !(0.25..=64.).contains(&style.stroke_width)
                || style.stroke_width > bounds.width.min(bounds.height)
                || !style.opacity.is_finite()
                || !(0.05..=1.).contains(&style.opacity)
            {
                return Err(bad("视频矩形坐标或样式无效"));
            }
            // Native renders stroke inward to fixed bounds, not centered outside.
        }
        VideoAnnotationPrimitive::Text { top_left, layout } => {
            crate::video_annotation_layouts::validate_ref_shape(layout)?;
            if !within(*top_left, width, height)
                || top_left.x + f64::from(layout.width) > f64::from(width)
                || top_left.y + f64::from(layout.height) > f64::from(height)
            {
                return Err(bad("视频文字布局超出原视频"));
            }
        }
        VideoAnnotationPrimitive::Vector { top_left, layout } => {
            crate::video_vector_layouts::validate_placement(*top_left, layout, width, height)?;
        }
    }
    Ok(())
}
fn validate_objects(doc: &VideoAnnotationDocument, objects: &[TimedVideoAnnotation]) -> Result<()> {
    if objects.len() > MAX_VIDEO_ANNOTATIONS {
        return Err(bad("视频批注对象过多"));
    }
    let mut ids = HashSet::new();
    let mut pixels = 0u64;
    for object in objects {
        if !uuid(&object.id) || !ids.insert(&object.id) {
            return Err(bad("视频批注对象ID重复或无效"));
        }
        validate_interval(object.interval, doc.source_duration_ticks)?;
        if matches!(&object.primitive, VideoAnnotationPrimitive::Vector { .. })
            && (object.origin.is_some()
                || object.interval.start_ticks != 0
                || object.interval.end_ticks != doc.source_duration_ticks)
        {
            return Err(bad("视频手绘矢量只能为无模型来源的原视频全时段对象"));
        }
        validate_primitive(&object.primitive, doc.source_width, doc.source_height)?;
        let (w, h) = match &object.primitive {
            VideoAnnotationPrimitive::Rect { bounds, .. } => {
                (bounds.width.ceil(), bounds.height.ceil())
            }
            VideoAnnotationPrimitive::Text { layout, .. } => {
                (f64::from(layout.width), f64::from(layout.height))
            }
            VideoAnnotationPrimitive::Vector { layout, .. } => {
                (f64::from(layout.width), f64::from(layout.height))
            }
        };
        if w > 6000. || h > 6000. || w * h > 4_000_000. {
            return Err(bad("单个视频批注栅格面积超限"));
        }
        pixels = pixels
            .checked_add((w * h) as u64)
            .ok_or_else(|| bad("视频批注像素预算超限"))?;
        if pixels > 32 * 1024 * 1024 {
            return Err(bad("视频批注文档像素预算超限"));
        }
        if let Some(origin) = &object.origin {
            if [
                &origin.run_id,
                &origin.user_message_id,
                &origin.tool_event_id,
                &origin.group_id,
                &origin.target_handle,
            ]
            .iter()
            .any(|v| !uuid(v))
                || !hash(&origin.manifest_sha256)
                || !hash(&origin.created_sha256)
            {
                return Err(bad("视频批注来源无效"));
            }
        }
    }
    Ok(())
}
fn history_bytes(doc: &VideoAnnotationDocument) -> Result<usize> {
    // Worst revision digits and one stack avoid redo/undo repartition growing
    // bytes at a legal boundary. Full ordered snapshots remain cheap via refs.
    #[derive(serde::Serialize)]
    struct HistoryBudget<'a> {
        revision: u64,
        undo: Vec<&'a VideoAnnotationStep>,
        redo: [VideoAnnotationStep; 0],
    }
    Ok(serde_json::to_vec(&HistoryBudget {
        revision: MAX_SAFE_REVISION,
        undo: doc.undo.iter().chain(&doc.redo).collect(),
        redo: [],
    })?
    .len())
}
pub fn validate_document(asset: &Asset, doc: &VideoAnnotationDocument) -> Result<()> {
    if asset.kind != AssetKind::Video
        || doc.version != 1
        || doc.source_id != asset.id
        || !uuid(&doc.source_id)
        || asset.width.is_some_and(|v| v != doc.source_width)
        || asset.height.is_some_and(|v| v != doc.source_height)
        || doc.revision == 0
        || doc.revision > MAX_SAFE_REVISION
        || doc.undo.len() + doc.redo.len() > MAX_VIDEO_HISTORY
        || history_bytes(doc)? > MAX_VIDEO_HISTORY_BYTES
        || serde_json::to_vec(doc)?.len() > MAX_VIDEO_DOCUMENT_BYTES
    {
        return Err(bad("视频批注文档来源、预算或修订无效"));
    }
    source_dimensions(doc)?;
    validate_objects(doc, &doc.objects)?;
    let mut ids = HashSet::new();
    for entry in doc.undo.iter().chain(&doc.redo) {
        if !uuid(&entry.id) || !ids.insert(&entry.id) || entry.before == entry.after {
            return Err(bad("视频批注历史操作无效"));
        }
        validate_objects(doc, &entry.before)?;
        validate_objects(doc, &entry.after)?;
    }
    for (stack, redo) in [(&doc.undo, false), (&doc.redo, true)] {
        let mut objects = &doc.objects;
        for entry in stack.iter().rev() {
            let (expected, destination) = if redo {
                (&entry.before, &entry.after)
            } else {
                (&entry.after, &entry.before)
            };
            if objects != expected {
                return Err(bad("视频批注历史与当前文档不连续"));
            }
            objects = destination;
        }
    }
    Ok(())
}

// No source-PTS argument accepted here. First/held decoded frame identity never
// changes a logical interval's lifecycle; native ties output delay to this clock.
pub fn active_at(
    doc: &VideoAnnotationDocument,
    tick: SourcePlaybackTicks,
) -> impl Iterator<Item = &TimedVideoAnnotation> {
    doc.objects
        .iter()
        .filter(move |o| o.interval.start_ticks <= tick.0 && tick.0 < o.interval.end_ticks)
}
pub fn project_interval(interval: VideoRange, range: VideoRange) -> Option<VideoRange> {
    let start = interval.start_ticks.max(range.start_ticks);
    let end = interval.end_ticks.min(range.end_ticks);
    (start < end).then(|| VideoRange {
        start_ticks: start - range.start_ticks,
        end_ticks: end - range.start_ticks,
    })
}
pub fn create_document(
    verified: &VerifiedVideoAnnotationSource,
    objects: Vec<TimedVideoAnnotation>,
    operation_id: &str,
) -> Result<VideoAnnotationDocument> {
    if objects.is_empty() || !uuid(operation_id) || !hash(&verified.source_sha256) {
        return Err(bad("视频批注初始批次或来源无效"));
    }
    let doc = VideoAnnotationDocument {
        version: 1,
        clock: VideoSourceClock::SourcePlaybackTicks,
        source_id: verified.source.asset.id.clone(),
        source_duration_ticks: verified.source.duration_ticks,
        source_width: verified.source.width,
        source_height: verified.source.height,
        revision: 1,
        objects: objects.clone(),
        undo: vec![VideoAnnotationStep {
            id: operation_id.into(),
            before: vec![],
            after: objects,
        }],
        redo: vec![],
    };
    validate_document(&verified.source.asset, &doc)?;
    Ok(doc)
}

// Called on an owned cloned candidate only; Store swaps it after its SQL commit.
// Existing document never mutates or creates video_edit. Trim never calls this.
pub fn mutate_document(
    asset: &Asset,
    doc: &VideoAnnotationDocument,
    command: VideoAnnotationMutation,
    operation_id: &str,
) -> Result<Option<VideoAnnotationDocument>> {
    validate_document(asset, doc)?;
    let mut next = doc.clone();
    match command {
        VideoAnnotationMutation::Undo {
            expected_operation_id,
        } => return Ok(Some(replay(asset, doc, &expected_operation_id, false)?)),
        VideoAnnotationMutation::Redo {
            expected_operation_id,
        } => return Ok(Some(replay(asset, doc, &expected_operation_id, true)?)),
        VideoAnnotationMutation::Append(objects) => {
            if objects.is_empty() {
                return Err(bad("视频批注批次为空"));
            }
            next.objects.extend(objects);
        }
        VideoAnnotationMutation::Update { id, replacement } => {
            let object = next
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .ok_or_else(|| bad("视频批注不存在"))?;
            object.interval = replacement.interval;
            object.primitive = replacement.primitive;
            // Preserve immutable creation origin; receipt proves original
            // creation identity, not current user-edited interval/content.
        }
        VideoAnnotationMutation::Remove { id } => {
            let index = next
                .objects
                .iter()
                .position(|o| o.id == id)
                .ok_or_else(|| bad("视频批注不存在"))?;
            next.objects.remove(index);
        }
    }
    validate_objects(&next, &next.objects)?;
    if next.objects == doc.objects {
        return Ok(None);
    }
    if !uuid(operation_id) {
        return Err(bad("视频批注历史ID无效"));
    }
    next.revision = revision(doc.revision)?;
    next.redo.clear();
    next.undo.push(VideoAnnotationStep {
        id: operation_id.into(),
        before: doc.objects.clone(),
        after: next.objects.clone(),
    });
    while next.undo.len() + next.redo.len() > MAX_VIDEO_HISTORY
        || history_bytes(&next)? > MAX_VIDEO_HISTORY_BYTES
    {
        if next.undo.len() <= 1 {
            return Err(bad("视频批注单次历史超出预算"));
        }
        next.undo.remove(0);
    }
    validate_document(asset, &next)?;
    Ok(Some(next))
}
pub fn replay(
    asset: &Asset,
    doc: &VideoAnnotationDocument,
    operation_id: &str,
    redo: bool,
) -> Result<VideoAnnotationDocument> {
    validate_document(asset, doc)?;
    let stack = if redo { &doc.redo } else { &doc.undo };
    let entry = stack
        .last()
        .filter(|e| e.id == operation_id)
        .cloned()
        .ok_or_else(|| bad("视频批注历史已改变"))?;
    let (expected, destination) = if redo {
        (&entry.before, &entry.after)
    } else {
        (&entry.after, &entry.before)
    };
    if &doc.objects != expected {
        return Err(bad("视频批注历史不连续"));
    }
    let mut next = doc.clone();
    next.objects = destination.clone();
    next.revision = revision(doc.revision)?;
    if redo {
        next.redo.pop();
        next.undo.push(entry);
    } else {
        next.undo.pop();
        next.redo.push(entry);
    }
    validate_document(asset, &next)?;
    Ok(next)
}

pub fn resolve_model_interval(
    target: &VideoInputTarget,
    interval: &ModelVideoInterval,
) -> Result<VideoRange> {
    let start = target
        .frames
        .iter()
        .find(|f| f.handle == interval.start_frame_handle)
        .ok_or_else(|| bad("视频起始帧句柄无效"))?
        .source_playback_ticks;
    let end = if interval.end_handle == target.range_end_handle {
        target.sent_window.end_ticks
    } else {
        target
            .frames
            .iter()
            .find(|f| f.handle == interval.end_handle)
            .ok_or_else(|| bad("视频结束帧句柄无效"))?
            .source_playback_ticks
    };
    let result = VideoRange {
        start_ticks: start,
        end_ticks: end,
    };
    validate_interval(result, target.source.source_duration_ticks)?;
    if start < target.sent_window.start_ticks || end > target.sent_window.end_ticks {
        return Err(bad("视频区间超出已发送窗口"));
    }
    Ok(result)
}
pub fn source_point(point: VideoPixelPoint, width: u32, height: u32) -> Result<VideoPixelPoint> {
    if !within(point, 1000, 1000) {
        return Err(bad("视频模型坐标需在normalized1000范围内"));
    }
    Ok(VideoPixelPoint {
        x: point.x * f64::from(width) / 1000.,
        y: point.y * f64::from(height) / 1000.,
    })
}
pub fn source_rect(rect: VideoPixelRect, width: u32, height: u32) -> Result<VideoPixelRect> {
    if !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.
        || rect.height <= 0.
        || rect.x + rect.width > 1000.
        || rect.y + rect.height > 1000.
    {
        return Err(bad("视频模型矩形无效"));
    }
    let p = source_point(
        VideoPixelPoint {
            x: rect.x,
            y: rect.y,
        },
        width,
        height,
    )?;
    Ok(VideoPixelRect {
        x: p.x,
        y: p.y,
        width: rect.width * f64::from(width) / 1000.,
        height: rect.height * f64::from(height) / 1000.,
    })
}
