// SPDX-License-Identifier: MPL-2.0
//! Local pixel operations commit their immutable PNGs and one history step together.
use super::*;

impl Store {
    /// Not a renderer command: only the native image processor can supply verified layouts.
    pub fn append_raster_edit(
        &mut self,
        scene_id: &str,
        region_id: &str,
        expected: &VisualSourceFence,
        drawings: Vec<Drawing>,
        layouts: Vec<VerifiedRichLayout>,
    ) -> Result<Snapshot> {
        let invalid = || CoreError::Invalid("本地修补图层批次无效".into());
        if !(1..=2).contains(&drawings.len()) || layouts.len() != drawings.len() {
            return Err(invalid());
        }
        let mut ids = HashSet::new();
        for (index, (drawing, layout)) in drawings.iter().zip(&layouts).enumerate() {
            let role = if index == 0 {
                RasterRole::Repair
            } else {
                RasterRole::Extracted
            };
            if drawing.kind != DrawingKind::Rich
                || drawing.origin.is_some()
                || drawing.rich.as_ref() != Some(layout.reference())
                || !ids.insert(drawing.id.clone())
                || !matches!(layout.content(), RichContent::Raster { version: 1, role: actual, source_sha256 }
                    if *actual == role && *source_sha256 == expected.visual_sha256)
                || (index > 0 && drawing.points != drawings[0].points)
            {
                return Err(invalid());
            }
        }
        self.journal_transaction(|db, state| {
            if state.active_scene_id != scene_id {
                return Err(CoreError::DrawingConflict);
            }
            let target = scene_mut(state, scene_id)?;
            if target.closed
                || target.frozen
                || target
                    .run
                    .as_ref()
                    .is_some_and(|run| run.status == RunStatus::Running)
            {
                return Err(CoreError::DrawingConflict);
            }
            let background = target
                .background
                .as_ref()
                .ok_or(CoreError::DrawingConflict)?;
            let region = target
                .regions
                .iter_mut()
                .find(|r| r.id == region_id)
                .ok_or(CoreError::DrawingConflict)?;
            if visual_source_fence(background, region)? != *expected {
                return Err(CoreError::DrawingConflict);
            }
            let (x, y, width, height) = if region.image_override.is_some() {
                (
                    0.,
                    0.,
                    f64::from(expected.source_size.width),
                    f64::from(expected.source_size.height),
                )
            } else {
                (
                    region.x.floor(),
                    region.y.floor(),
                    (region.x + region.width).ceil() - region.x.floor(),
                    (region.y + region.height).ceil() - region.y.floor(),
                )
            };
            if drawings.iter().flat_map(|d| &d.points).any(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || p.x < x
                    || p.y < y
                    || p.x > x + width
                    || p.y > y + height
            }) {
                return Err(invalid());
            }
            for layout in &layouts {
                crate::rich_annotations::insert(db, layout)?;
            }
            crate::drawing::apply(
                region,
                background,
                crate::drawing::Mutation::AppendBatch(drawings),
            )?;
            region.ocr = None;
            if let Some(translation) = &mut region.translation {
                translation.drawing_revision = region.drawing_revision;
            }
            touch(target);
            Ok(())
        })?;
        Ok(self.snapshot())
    }
}
