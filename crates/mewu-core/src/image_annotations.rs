// SPDX-License-Identifier: MPL-2.0
//! Image-item documents share the bounded drawing algorithm, never a scene region.
use crate::{AssetKind, CoreError, Drawing, DrawingHistory, Region, SpaceItem, VisualSourceFence};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageAnnotationDocument {
    pub version: u32,
    pub source_asset_id: String,
    pub revision: u64,
    pub drawings: Vec<Drawing>,
    pub history: DrawingHistory,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum ImageAnnotationEdit {
    AddDrawing { drawing: Drawing },
    UpdateDrawing { drawing: Drawing },
    UpdateDrawings { drawings: Vec<Drawing> },
    EraseDrawingStrokes { erasures: Vec<crate::DrawingStrokeErase> },
    RemoveDrawing { drawing_id: String },
    RemoveDrawings { drawing_ids: Vec<String> },
    UndoDrawing,
    RedoDrawing,
}
impl ImageAnnotationEdit {
    pub(crate) fn mutation(self) -> crate::drawing::Mutation {
        use ImageAnnotationEdit::*;
        match self {
            AddDrawing { drawing } => crate::drawing::Mutation::Add(drawing),
            UpdateDrawing { drawing } => crate::drawing::Mutation::Update(drawing),
            UpdateDrawings { drawings } => crate::drawing::Mutation::UpdateBatch(drawings),
            EraseDrawingStrokes { erasures } => crate::drawing::Mutation::EraseStrokes(erasures),
            RemoveDrawing { drawing_id } => crate::drawing::Mutation::Remove(drawing_id),
            RemoveDrawings { drawing_ids } => crate::drawing::Mutation::RemoveBatch(drawing_ids),
            UndoDrawing => crate::drawing::Mutation::Undo,
            RedoDrawing => crate::drawing::Mutation::Redo,
        }
    }
}
type Result<T> = std::result::Result<T, CoreError>;
fn invalid(message: &str) -> CoreError { CoreError::Invalid(message.into()) }
pub fn image_annotation_writable(item: &SpaceItem) -> bool {
    item.asset.kind == AssetKind::Image && !item.state.as_ref().is_some_and(|state| state.contains_key("blackboardSceneId"))
}
pub fn image_annotation_region(item: &SpaceItem) -> Result<Region> {
    let width = item.asset.width.filter(|value| *value > 0).ok_or_else(|| invalid("图片尺寸无效"))?;
    let height = item.asset.height.filter(|value| *value > 0).ok_or_else(|| invalid("图片尺寸无效"))?;
    if item.asset.kind != AssetKind::Image { return Err(invalid("此素材不是图片")); }
    let mut region = Region { id: item.id.clone(), x: 0., y: 0., width: f64::from(width), height: f64::from(height), ..Region::default() };
    if let Some(document) = &item.image_annotations {
        if document.version != 1 || document.source_asset_id != item.asset.id { return Err(invalid("图片标注来源已变更")); }
        region.drawing_revision = document.revision;
        region.drawings = document.drawings.clone();
        region.drawing_history = document.history.clone();
    }
    Ok(region)
}
pub fn image_annotation_source_fence(item: &SpaceItem) -> Result<VisualSourceFence> {
    if !image_annotation_writable(item) { return Err(invalid("请在原黑板内编辑")); }
    let region = image_annotation_region(item)?;
    let mut fence = crate::visual_source_fence(&item.asset, &region)?;
    // Geometry, document/history and immutable asset identity belong to this
    // exact item; a move, source replacement or manual edit rejects late work.
    fence.visual_sha256 = crate::journal::digest(&serde_json::to_vec(item)?);
    Ok(fence)
}
pub(crate) fn validate(item: &SpaceItem) -> Result<()> {
    if item.image_annotations.is_none() { return Ok(()); }
    if !image_annotation_writable(item) { return Err(invalid("黑板预览不能携带独立图片标注")); }
    let region = image_annotation_region(item)?;
    for drawing in region.drawings.iter().chain(region.drawing_history.undo.iter().chain(&region.drawing_history.redo).flat_map(|step|step.edits()).flat_map(|edit|edit.before.iter().chain(&edit.after))) {
        if matches!(drawing.kind,crate::DrawingKind::Highlighter|crate::DrawingKind::Mark|crate::DrawingKind::Mosaic) { return Err(invalid("图片标注暂不支持高亮或马赛克")); }
    }
    crate::drawing::validate(&region, Some(&item.asset))
}
pub(crate) fn apply(item: &mut SpaceItem, mutation: crate::drawing::Mutation) -> Result<bool> {
    if !image_annotation_writable(item) { return Err(invalid("请在原黑板内编辑")); }
    let mut region = image_annotation_region(item)?;
    let changed = crate::drawing::apply(&mut region, &item.asset, mutation)?;
    if changed {
        item.image_annotations = Some(ImageAnnotationDocument { version: 1, source_asset_id: item.asset.id.clone(), revision: region.drawing_revision, drawings: region.drawings, history: region.drawing_history });
    }
    validate(item)?;
    Ok(changed)
}
pub(crate) fn documents(scene: &crate::Scene) -> impl Iterator<Item = (&str, bool, &[Drawing], &DrawingHistory)> {
    scene.regions.iter().map(|region| (region.id.as_str(), false, region.drawings.as_slice(), &region.drawing_history))
        .chain(scene.items.iter().filter_map(|item| item.image_annotations.as_ref().map(|document| (item.id.as_str(), true, document.drawings.as_slice(), &document.history))))
}
