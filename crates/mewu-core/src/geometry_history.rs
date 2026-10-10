// SPDX-License-Identifier: MPL-2.0
//! Scene-ordered geometry history. Only display bounds are ever replayed.
use crate::{
    CoreError, Region, RegionGeometry, RegionGeometryEdit, RegionGeometryHistory, Scene, Snapshot,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

type Result<T> = std::result::Result<T, CoreError>;
const MAX_EDITS: usize = 50;
const MAX_BYTES: usize = 64 * 1024;

/// Reserve the largest revision and comma count so every accepted history can
/// move between stacks without exceeding its budget at a decimal boundary.
fn worst_case_bytes(history: &RegionGeometryHistory) -> Result<usize> {
    #[derive(serde::Serialize)]
    struct BudgetView<'a> {
        revision: u64,
        undo: Vec<&'a RegionGeometryEdit>,
        redo: [RegionGeometryEdit; 0],
    }
    Ok(serde_json::to_vec(&BudgetView {
        revision: u64::MAX,
        undo: history.undo.iter().chain(&history.redo).collect(),
        redo: [],
    })?
    .len())
}

pub(crate) struct Target<'a> {
    pub scene_id: &'a str,
    pub region_id: &'a str,
    pub background_id: &'a str,
    pub source_id: &'a str,
    pub revision: u64,
    pub from: RegionGeometry,
}

/// This is intentionally not serialized. Reopening cannot resume a gesture.
#[derive(Clone)]
pub(crate) struct ActiveEdit {
    scene_id: String,
    edit_id: String,
    entry_id: String,
    region_id: String,
    background_id: String,
    source_id: String,
    revision: u64,
}
impl ActiveEdit {
    pub fn matches(&self, scene_id: &str, edit_id: &str) -> bool {
        self.scene_id == scene_id && self.edit_id == edit_id
    }
    pub fn is_current(&self, state: &Snapshot) -> bool {
        if state.active_scene_id != self.scene_id {
            return false;
        }
        state
            .scenes
            .iter()
            .find(|s| s.id == self.scene_id)
            .is_some_and(|s| {
                !s.closed
                    && !s.frozen
                    && !running(s)
                    && s.background
                        .as_ref()
                        .is_some_and(|b| b.id == self.background_id)
                    && s.geometry_history.redo.is_empty()
                    && s.geometry_history
                        .undo
                        .last()
                        .is_some_and(|e| e.id == self.entry_id)
                    && s.regions.iter().any(|r| {
                        r.id == self.region_id
                            && r.drawing_revision == self.revision
                            && source_id(s, r) == Some(self.source_id.as_str())
                    })
            })
    }
    pub fn source_set_changed(&self, before: &Snapshot, after: &Snapshot) -> bool {
        let Some(old) = before.scenes.iter().find(|s| s.id == self.scene_id) else {
            return true;
        };
        let Some(new) = after.scenes.iter().find(|s| s.id == self.scene_id) else {
            return true;
        };
        old.background != new.background
            || old.regions.len() != new.regions.len()
            || old.regions.iter().any(|r| {
                new.regions
                    .iter()
                    .find(|n| n.id == r.id)
                    .is_none_or(|n| n.image_override != r.image_override)
            })
    }
}
fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}
pub(crate) fn validate_edit_id(id: &str) -> Result<()> {
    if !valid_uuid(id) {
        return Err(invalid("几何手势标识无效"));
    }
    Ok(())
}
fn valid_uuid(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|parsed| parsed.hyphenated().to_string() == id)
}
fn running(scene: &Scene) -> bool {
    scene
        .run
        .as_ref()
        .is_some_and(|r| r.status == crate::RunStatus::Running)
}
fn source_id<'a>(scene: &'a Scene, region: &'a Region) -> Option<&'a str> {
    region
        .image_override
        .as_ref()
        .or(scene.background.as_ref())
        .map(|a| a.id.as_str())
}
fn dimensions(scene: &Scene) -> Result<(f64, f64)> {
    let image = scene
        .background
        .as_ref()
        .ok_or(CoreError::GeometryConflict)?;
    Ok((
        image.width.ok_or(CoreError::GeometryConflict)? as f64,
        image.height.ok_or(CoreError::GeometryConflict)? as f64,
    ))
}
fn validate_geometry(g: RegionGeometry, width: f64, height: f64) -> Result<()> {
    if ![g.x, g.y, g.width, g.height].iter().all(|v| v.is_finite())
        || g.x < 0.
        || g.y < 0.
        || g.width <= 0.
        || g.height <= 0.
        || g.x + g.width > width
        || g.y + g.height > height
    {
        return Err(invalid("区域位置超出背景范围"));
    }
    Ok(())
}
fn scene_for_edit<'a>(state: &'a mut Snapshot, id: &str) -> Result<&'a mut Scene> {
    if state.active_scene_id != id {
        return Err(CoreError::GeometryConflict);
    }
    let scene = state
        .scenes
        .iter_mut()
        .find(|s| s.id == id && !s.closed && !s.frozen)
        .ok_or(CoreError::GeometryConflict)?;
    if running(scene) {
        return Err(CoreError::RunInProgress);
    }
    Ok(scene)
}
fn check_target(scene: &Scene, target: &Target<'_>) -> Result<usize> {
    if scene.background.as_ref().map(|a| a.id.as_str()) != Some(target.background_id) {
        return Err(CoreError::GeometryConflict);
    }
    scene
        .regions
        .iter()
        .position(|r| {
            r.id == target.region_id
                && r.drawing_revision == target.revision
                && RegionGeometry::from(r) == target.from
                && source_id(scene, r) == Some(target.source_id)
        })
        .ok_or(CoreError::GeometryConflict)
}
/// Shared by new edits and both replay directions; never reads stored content.
fn transition(region: &mut Region, geometry: RegionGeometry) -> Result<()> {
    region.drawing_revision = crate::drawing::next_revision(region)?;
    region.x = geometry.x;
    region.y = geometry.y;
    region.width = geometry.width;
    region.height = geometry.height;
    if region.image_override.is_some() {
        if let Some(ocr) = &mut region.ocr {
            ocr.drawing_revision = region.drawing_revision;
        }
        if let Some(translation) = &mut region.translation {
            translation.drawing_revision = region.drawing_revision;
        }
    } else {
        region.ocr = None;
        region.translation = None;
    }
    Ok(())
}
fn advance(scene: &mut Scene) -> Result<()> {
    scene.geometry_history.revision = scene
        .geometry_history
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("几何历史修订号超出范围"))?;
    Ok(())
}

pub(crate) fn edit(
    state: &mut Snapshot,
    target: Target<'_>,
    to: RegionGeometry,
    edit_id: Option<&str>,
    group: &mut Option<ActiveEdit>,
) -> Result<bool> {
    if let Some(id) = edit_id {
        validate_edit_id(id)?;
    }
    let scene = scene_for_edit(state, target.scene_id)?;
    let index = check_target(scene, &target)?;
    let (width, height) = dimensions(scene)?;
    validate_geometry(to, width, height)?;
    if target.from == to {
        if !group.as_ref().is_some_and(|g| {
            edit_id == Some(g.edit_id.as_str())
                && g.scene_id == target.scene_id
                && g.region_id == target.region_id
                && g.revision == target.revision
                && g.source_id == target.source_id
                && g.background_id == target.background_id
        }) {
            *group = None;
        }
        return Ok(false);
    }
    validate_geometry(target.from, width, height)?;
    let merge = group.as_ref().is_some_and(|g| {
        edit_id == Some(g.edit_id.as_str())
            && g.scene_id == target.scene_id
            && g.region_id == target.region_id
            && g.background_id == target.background_id
            && g.source_id == target.source_id
            && g.revision == target.revision
            && scene.geometry_history.redo.is_empty()
            && scene
                .geometry_history
                .undo
                .last()
                .is_some_and(|e| e.id == g.entry_id && e.to == target.from)
    });
    transition(&mut scene.regions[index], to)?;
    scene.geometry_history.redo.clear();
    if merge {
        let last = scene
            .geometry_history
            .undo
            .last_mut()
            .expect("checked merge head");
        last.to = to;
        if last.from == last.to {
            scene.geometry_history.undo.pop();
            *group = None;
        }
    } else {
        scene.geometry_history.undo.push(RegionGeometryEdit {
            id: Uuid::new_v4().to_string(),
            region_id: target.region_id.into(),
            background_id: target.background_id.into(),
            source_id: target.source_id.into(),
            from: target.from,
            to,
        });
        *group = None;
    }
    advance(scene)?;
    while scene.geometry_history.undo.len() > MAX_EDITS
        || worst_case_bytes(&scene.geometry_history)? > MAX_BYTES
    {
        if scene.geometry_history.undo.len() <= 1 {
            return Err(invalid("几何历史数据过大"));
        }
        scene.geometry_history.undo.remove(0);
    }
    if let (Some(edit_id), Some(entry)) = (edit_id, scene.geometry_history.undo.last()) {
        // A canceled-out merged entry must not adopt an older unrelated head.
        if entry.region_id == target.region_id && entry.to == to && (!merge || group.is_some()) {
            *group = Some(ActiveEdit {
                scene_id: target.scene_id.into(),
                edit_id: edit_id.into(),
                entry_id: entry.id.clone(),
                region_id: target.region_id.into(),
                background_id: target.background_id.into(),
                source_id: target.source_id.into(),
                revision: scene.regions[index].drawing_revision,
            });
        }
    }
    Ok(true)
}

pub(crate) fn replay(
    state: &mut Snapshot,
    target: Target<'_>,
    history_revision: u64,
    operation_id: &str,
    redo: bool,
) -> Result<()> {
    let scene = scene_for_edit(state, target.scene_id)?;
    if scene.geometry_history.revision != history_revision {
        return Err(CoreError::GeometryHistoryConflict);
    }
    let stack = if redo {
        &scene.geometry_history.redo
    } else {
        &scene.geometry_history.undo
    };
    let entry = stack
        .last()
        .filter(|e| {
            e.id == operation_id
                && e.region_id == target.region_id
                && e.background_id == target.background_id
                && e.source_id == target.source_id
        })
        .ok_or(CoreError::GeometryHistoryConflict)?
        .clone();
    let (expected, destination) = if redo {
        (entry.from, entry.to)
    } else {
        (entry.to, entry.from)
    };
    if target.from != expected {
        return Err(CoreError::GeometryConflict);
    }
    let index = check_target(scene, &target)?;
    let (width, height) = dimensions(scene)?;
    validate_geometry(destination, width, height)?;
    transition(&mut scene.regions[index], destination)?;
    if redo {
        scene.geometry_history.redo.pop();
        scene.geometry_history.undo.push(entry);
    } else {
        scene.geometry_history.undo.pop();
        scene.geometry_history.redo.push(entry);
    }
    advance(scene)
}

pub(crate) fn invalidate_region(scene: &mut Scene, region_id: &str) -> Result<()> {
    let old_undo = scene.geometry_history.undo.len();
    scene
        .geometry_history
        .undo
        .retain(|e| e.region_id != region_id);
    let changed =
        old_undo != scene.geometry_history.undo.len() || !scene.geometry_history.redo.is_empty();
    scene.geometry_history.redo.clear();
    if changed {
        advance(scene)?;
    }
    Ok(())
}

/// Run before persistence, never on database reads. This covers every host
/// source replacement/removal, including recording converting a region to video.
pub(crate) fn cleanup(previous: &Snapshot, next: &mut Snapshot) -> Result<()> {
    for scene in &mut next.scenes {
        let Some(old) = previous.scenes.iter().find(|s| s.id == scene.id) else {
            continue;
        };
        let background_changed = old.background != scene.background;
        let obsolete: HashSet<&str> = old
            .regions
            .iter()
            .filter(|r| {
                scene
                    .regions
                    .iter()
                    .find(|n| n.id == r.id)
                    .is_none_or(|n| n.image_override != r.image_override)
            })
            .map(|r| r.id.as_str())
            .collect();
        let added = scene
            .regions
            .iter()
            .any(|r| !old.regions.iter().any(|o| o.id == r.id));
        if !background_changed && obsolete.is_empty() && !added {
            continue;
        }
        let before = scene.geometry_history.undo.len();
        scene
            .geometry_history
            .undo
            .retain(|e| !background_changed && !obsolete.contains(e.region_id.as_str()));
        let changed =
            before != scene.geometry_history.undo.len() || !scene.geometry_history.redo.is_empty();
        scene.geometry_history.redo.clear();
        if changed {
            advance(scene)?;
        }
    }
    Ok(())
}

pub(crate) fn validate(scene: &Scene) -> Result<()> {
    let history = &scene.geometry_history;
    if history.undo.len() + history.redo.len() > MAX_EDITS
        || worst_case_bytes(history)? > MAX_BYTES
        || (history.revision == 0 && (!history.undo.is_empty() || !history.redo.is_empty()))
    {
        return Err(invalid("几何历史预算或修订号无效"));
    }
    let mut ids = HashSet::new();
    for entry in history.undo.iter().chain(&history.redo) {
        if !valid_uuid(&entry.id) || !ids.insert(&entry.id) || entry.from == entry.to {
            return Err(invalid("几何历史标识或操作无效"));
        }
        let region = scene
            .regions
            .iter()
            .find(|r| r.id == entry.region_id)
            .ok_or_else(|| invalid("几何历史区域不存在"))?;
        if scene.background.as_ref().map(|a| a.id.as_str()) != Some(entry.background_id.as_str())
            || source_id(scene, region) != Some(entry.source_id.as_str())
        {
            return Err(invalid("几何历史源已失效"));
        }
        let (width, height) = dimensions(scene)?;
        validate_geometry(entry.from, width, height)?;
        validate_geometry(entry.to, width, height)?;
    }
    // Each region forms an independent chain, interleaved in scene order. A
    // corrupt but in-bounds endpoint must not later teleport that region.
    for (stack, redo) in [(&history.undo, false), (&history.redo, true)] {
        let mut positions: HashMap<&str, RegionGeometry> = HashMap::new();
        for entry in stack.iter().rev() {
            let current = positions.entry(&entry.region_id).or_insert_with(|| {
                RegionGeometry::from(
                    scene
                        .regions
                        .iter()
                        .find(|r| r.id == entry.region_id)
                        .expect("validated region"),
                )
            });
            let (expected, destination) = if redo {
                (entry.from, entry.to)
            } else {
                (entry.to, entry.from)
            };
            if *current != expected {
                return Err(invalid("几何历史与当前区域不连续"));
            }
            *current = destination;
        }
    }
    Ok(())
}
