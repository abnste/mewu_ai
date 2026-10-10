// SPDX-License-Identifier: MPL-2.0
// Only video ranges are replayed; original assets and card geometry are immutable
// from the perspective of this module.
use crate::{
    AssetKind, CoreError, RunStatus, Snapshot, SpaceItem, VerifiedVideoSource, VideoEdit,
    VideoExportOrigin, VideoRange, VideoRangeOperation, VideoSourceView, VideoTarget,
};
use std::collections::HashSet;
use uuid::Uuid;

type Result<T> = std::result::Result<T, CoreError>;
const MAX_EDITS: usize = 50;
const MAX_BYTES: usize = 64 * 1024;
const MIN_RANGE_TICKS: u64 = 1_000_000; // 100 ms, approved by root.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
// Accepted source cap only; this does not extend the recorder's active limit.
const MAX_DURATION_TICKS: u64 = 30 * 60 * 10_000_000;

fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}
fn duration(value: u64) -> Result<()> {
    if value == 0 || value > MAX_DURATION_TICKS {
        return Err(invalid("视频时长无效"));
    }
    Ok(())
}

/// Exact full range is canonical None. Invalid inputs are never clamped.
fn canonical(value: Option<VideoRange>, total: u64) -> Result<Option<VideoRange>> {
    duration(total)?;
    let Some(value) = value else { return Ok(None) };
    if value.start_ticks >= value.end_ticks || value.end_ticks > total {
        return Err(invalid("视频范围超出原片段"));
    }
    if value.start_ticks == 0 && value.end_ticks == total {
        return Ok(None);
    }
    if total < MIN_RANGE_TICKS || value.end_ticks - value.start_ticks < MIN_RANGE_TICKS {
        return Err(invalid("视频范围不能短于 100 毫秒"));
    }
    Ok(Some(value))
}
fn stored_range(value: Option<VideoRange>, total: u64) -> Result<()> {
    if canonical(value, total)? != value {
        return Err(invalid("完整视频范围必须使用默认整段"));
    }
    Ok(())
}
fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}
fn next_revision(value: u64) -> Result<u64> {
    value
        .checked_add(1)
        .filter(|v| *v <= MAX_SAFE_INTEGER)
        .ok_or_else(|| invalid("视频修订号超出范围"))
}
fn history_bytes(edit: &VideoEdit) -> Result<usize> {
    #[derive(serde::Serialize)]
    struct Budget<'a> {
        revision: u64,
        undo: Vec<&'a VideoRangeOperation>,
        redo: [VideoRangeOperation; 0],
    }
    Ok(serde_json::to_vec(&Budget {
        revision: MAX_SAFE_INTEGER,
        undo: edit.undo.iter().chain(&edit.redo).collect(),
        redo: [],
    })?
    .len())
}

/// Used on stored reads and before every persistence. Never cleans corrupt data.
pub(crate) fn validate(item: &SpaceItem) -> Result<()> {
    let Some(edit) = &item.video_edit else {
        return Ok(());
    };
    if item.asset.kind != AssetKind::Video || edit.source_id != item.asset.id {
        return Err(invalid("视频裁剪来源不一致"));
    }
    duration(edit.source_duration_ticks)?;
    stored_range(edit.range, edit.source_duration_ticks)?;
    if edit.revision > MAX_SAFE_INTEGER
        || (edit.revision == 0
            && (edit.range.is_some() || !edit.undo.is_empty() || !edit.redo.is_empty()))
        || edit.undo.len() + edit.redo.len() > MAX_EDITS
        || history_bytes(edit)? > MAX_BYTES
    {
        return Err(invalid("视频历史预算或修订号无效"));
    }
    let mut ids = HashSet::new();
    for entry in edit.undo.iter().chain(&edit.redo) {
        if !valid_uuid(&entry.id) || !ids.insert(&entry.id) || entry.from == entry.to {
            return Err(invalid("视频历史操作无效"));
        }
        stored_range(entry.from, edit.source_duration_ticks)?;
        stored_range(entry.to, edit.source_duration_ticks)?;
    }
    for (stack, redo) in [(&edit.undo, false), (&edit.redo, true)] {
        let mut range = edit.range;
        for entry in stack.iter().rev() {
            let (expected, destination) = if redo {
                (entry.from, entry.to)
            } else {
                (entry.to, entry.from)
            };
            if range != expected {
                return Err(invalid("视频历史与当前范围不连续"));
            }
            range = destination;
        }
    }
    Ok(())
}

fn indexes(state: &Snapshot, scene_id: &str, item_id: &str) -> Result<(usize, usize)> {
    let s = state
        .scenes
        .iter()
        .position(|s| s.id == scene_id)
        .ok_or(CoreError::VideoEditConflict)?;
    let i = state.scenes[s]
        .items
        .iter()
        .position(|i| i.id == item_id)
        .ok_or(CoreError::VideoEditConflict)?;
    if state.scenes[s].items[i].asset.kind != AssetKind::Video {
        return Err(invalid("素材不是视频"));
    }
    validate(&state.scenes[s].items[i])?;
    Ok((s, i))
}
fn target_indexes(state: &Snapshot, target: &VideoTarget) -> Result<(usize, usize)> {
    let (s, i) = indexes(state, &target.scene_id, &target.item_id)?;
    let scene = &state.scenes[s];
    if state.active_scene_id != scene.id || scene.frozen || scene.closed {
        return Err(CoreError::VideoEditConflict);
    }
    if scene
        .run
        .as_ref()
        .is_some_and(|run| run.status == RunStatus::Running)
    {
        return Err(CoreError::RunInProgress);
    }
    let item = &scene.items[i];
    if item.asset.id != target.source_id
        || item.video_edit.as_ref().map_or(0, |edit| edit.revision) != target.expected_revision
    {
        return Err(CoreError::VideoEditConflict);
    }
    Ok((s, i))
}

pub(crate) fn source(state: &Snapshot, scene_id: &str, item_id: &str) -> Result<VideoSourceView> {
    let (s, i) = indexes(state, scene_id, item_id)?;
    let item = &state.scenes[s].items[i];
    Ok(VideoSourceView {
        asset: item.asset.clone(),
        edit: item.video_edit.clone(),
        annotations: item.video_annotations.clone(),
    })
}

/// Shape only: permissions, native source-file proof, exit and publication policy
/// belong to Host. Returning true grants none of those side effects by itself.
pub(crate) fn export_matches(state: &Snapshot, origin: &VideoExportOrigin) -> bool {
    let Some(scene) = state.scenes.iter().find(|s| s.id == origin.scene_id) else {
        return false;
    };
    scene.items.iter().any(|item| {
        item.id == origin.item_id
            && item.asset.kind == AssetKind::Video
            && item.asset == origin.source.asset
            && item.video_edit == origin.source.edit
            && item.video_annotations == origin.source.annotations
    })
}

pub(crate) fn set(
    state: &mut Snapshot,
    target: &VideoTarget,
    verified: &VerifiedVideoSource,
    from: Option<VideoRange>,
    to: Option<VideoRange>,
) -> Result<bool> {
    let (s, i) = target_indexes(state, target)?;
    let item = &mut state.scenes[s].items[i];
    if item.asset != verified.asset
        || verified.width == 0
        || verified.height == 0
        || item
            .asset
            .width
            .is_some_and(|width| width != verified.width)
        || item
            .asset
            .height
            .is_some_and(|height| height != verified.height)
    {
        return Err(CoreError::VideoEditConflict);
    }
    duration(verified.duration_ticks)?;
    if item
        .video_edit
        .as_ref()
        .is_some_and(|edit| edit.source_duration_ticks != verified.duration_ticks)
    {
        return Err(CoreError::VideoEditConflict);
    }
    let from = canonical(from, verified.duration_ticks)?;
    let to = canonical(to, verified.duration_ticks)?;
    if item.video_edit.as_ref().and_then(|edit| edit.range) != from {
        return Err(CoreError::VideoEditConflict);
    }
    // Even a successful metadata probe must not write a true no-op.
    if from == to {
        return Ok(false);
    }
    let mut edit = item.video_edit.clone().unwrap_or_else(|| VideoEdit {
        source_id: item.asset.id.clone(),
        source_duration_ticks: verified.duration_ticks,
        revision: 0,
        range: None,
        undo: vec![],
        redo: vec![],
    });
    edit.revision = next_revision(edit.revision)?;
    edit.range = to;
    edit.redo.clear();
    edit.undo.push(VideoRangeOperation {
        id: Uuid::new_v4().to_string(),
        from,
        to,
    });
    while edit.undo.len() + edit.redo.len() > MAX_EDITS || history_bytes(&edit)? > MAX_BYTES {
        if edit.undo.len() <= 1 {
            return Err(invalid("视频历史超出预算"));
        }
        edit.undo.remove(0);
    }
    item.video_edit = Some(edit);
    validate(item)?;
    Ok(true)
}

pub(crate) fn replay(
    state: &mut Snapshot,
    target: &VideoTarget,
    expected_operation_id: &str,
    from: Option<VideoRange>,
    redo: bool,
) -> Result<()> {
    let (s, i) = target_indexes(state, target)?;
    let item = &mut state.scenes[s].items[i];
    let mut edit = item
        .video_edit
        .clone()
        .ok_or(CoreError::VideoHistoryConflict)?;
    let from = canonical(from, edit.source_duration_ticks)?;
    if from != edit.range {
        return Err(CoreError::VideoEditConflict);
    }
    let stack = if redo { &edit.redo } else { &edit.undo };
    let entry = stack
        .last()
        .filter(|entry| entry.id == expected_operation_id)
        .cloned()
        .ok_or(CoreError::VideoHistoryConflict)?;
    let (expected, destination) = if redo {
        (entry.from, entry.to)
    } else {
        (entry.to, entry.from)
    };
    if expected != from {
        return Err(CoreError::VideoHistoryConflict);
    }
    edit.revision = next_revision(edit.revision)?;
    edit.range = destination;
    if redo {
        edit.redo.pop();
        edit.undo.push(entry);
    } else {
        edit.undo.pop();
        edit.redo.push(entry);
    }
    item.video_edit = Some(edit);
    validate(item)
}
