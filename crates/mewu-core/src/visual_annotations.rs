// SPDX-License-Identifier: MPL-2.0
//! Immutable image-input authority for local, editable visual effects.
use crate::journal as j;
use crate::*;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const VISUAL_ANNOTATION_TOOL: &str = "visual_annotate";
pub const MAX_VISUAL_OBJECTS: usize = 48;
pub(crate) type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualAnnotationGrant {
    pub plugin_id: String,
    pub plugin_revision: u64,
    pub contribution_id: String,
}
impl VisualAnnotationGrant {
    pub fn binding(&self, target_manifest_sha256: &str) -> ToolBinding {
        ToolBinding::VisualAnnotation {
            plugin_id: self.plugin_id.clone(),
            plugin_revision: self.plugin_revision,
            contribution_id: self.contribution_id.clone(),
            name: VISUAL_ANNOTATION_TOOL.into(),
            target_manifest_sha256: target_manifest_sha256.into(),
        }
    }
}
pub(crate) fn validate_grant(grant: &VisualAnnotationGrant) -> Result<()> {
    let valid = |s: &str| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control);
    if grant.plugin_revision == 0 || !valid(&grant.plugin_id) || !valid(&grant.contribution_id) {
        return Err(j::bad("批注插件授权无效"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualDrawingOrigin {
    pub run_id: String,
    pub user_message_id: String,
    pub tool_event_id: String,
    pub group_id: String,
    pub target_handle: String,
    pub manifest_sha256: String,
}
pub(crate) fn validate_origin(origin: &VisualDrawingOrigin) -> Result<()> {
    for id in [
        &origin.run_id,
        &origin.user_message_id,
        &origin.tool_event_id,
        &origin.group_id,
        &origin.target_handle,
    ] {
        j::uuid(id)?;
    }
    if !j::hash_valid(&origin.manifest_sha256) {
        return Err(j::bad("批注来源摘要无效"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualPixelSize {
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualPixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl VisualPixelSize {
    fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= 16384
            && self.height <= 16384
            && u64::from(self.width) * u64::from(self.height) <= 32 * 1024 * 1024
    }
}
impl VisualPixelRect {
    fn within(&self, size: VisualPixelSize) -> bool {
        self.width > 0
            && self.height > 0
            && self
                .x
                .checked_add(self.width)
                .is_some_and(|x| x <= size.width)
            && self
                .y
                .checked_add(self.height)
                .is_some_and(|y| y <= size.height)
    }
}

/// The digest includes all visual metadata, not message text or mutable refs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualSourceFence {
    pub background_id: String,
    pub effective_source_id: String,
    pub source_size: VisualPixelSize,
    pub drawing_revision: u64,
    pub visual_sha256: String,
}
pub fn visual_source_fence(background: &Asset, region: &Region) -> Result<VisualSourceFence> {
    let source = region.image_override.as_ref().unwrap_or(background);
    let size = VisualPixelSize {
        width: source.width.unwrap_or(0),
        height: source.height.unwrap_or(0),
    };
    if background.kind != AssetKind::Image || source.kind != AssetKind::Image || !size.valid() {
        return Err(j::bad("批注图片尺寸无效"));
    }
    Ok(VisualSourceFence {
        background_id: background.id.clone(),
        effective_source_id: source.id.clone(),
        source_size: size,
        drawing_revision: region.drawing_revision,
        visual_sha256: j::digest(&serde_json::to_vec(&(
            background,
            &region.image_override,
            &region.id,
            region.x,
            region.y,
            region.width,
            region.height,
            region.drawing_revision,
            &region.drawings,
            &region.translation,
        ))?),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualInputTarget {
    pub handle: String,
    pub attachment_id: String,
    pub attachment_ordinal: u32,
    pub attachment_sha256: String,
    pub region_id: String,
    pub source: VisualSourceFence,
    pub crop: VisualPixelRect,
    pub resized: VisualPixelSize,
    pub tile: VisualPixelRect,
    pub encoded: VisualPixelSize,
    pub profile_id: String,
}
impl VisualInputTarget {
    /// Encoded content pixels only. Provider transforms, if any, must have been
    /// normalized by its explicit adapter; screen/DPI coordinates never enter.
    pub fn source_point(&self, point: DrawingPoint) -> Result<DrawingPoint> {
        if !point.x.is_finite()
            || !point.y.is_finite()
            || point.x < 0.
            || point.y < 0.
            || point.x > self.encoded.width as f64
            || point.y > self.encoded.height as f64
        {
            return Err(j::bad("批注坐标超出附件内容"));
        }
        Ok(DrawingPoint {
            x: self.crop.x as f64
                + (self.tile.x as f64
                    + point.x * self.tile.width as f64 / self.encoded.width as f64)
                    * self.crop.width as f64
                    / self.resized.width as f64,
            y: self.crop.y as f64
                + (self.tile.y as f64
                    + point.y * self.tile.height as f64 / self.encoded.height as f64)
                    * self.crop.height as f64
                    / self.resized.height as f64,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualInputManifest {
    pub version: u32,
    pub targets: Vec<VisualInputTarget>,
}

/// Rust-only construction boundary; never accepted by renderer IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVisualInput {
    manifest: VisualInputManifest,
    sha256: String,
}
impl VerifiedVisualInput {
    pub fn new(manifest: VisualInputManifest) -> Result<Self> {
        validate_manifest(&manifest)?;
        let sha256 = j::digest(&serde_json::to_vec(&manifest)?);
        Ok(Self { manifest, sha256 })
    }
    pub fn manifest(&self) -> &VisualInputManifest {
        &self.manifest
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVisualAuthority {
    pub(crate) grant: VisualAnnotationGrant,
    pub(crate) manifest_sha256: String,
}
impl VerifiedVisualAuthority {
    pub fn new(grant: VisualAnnotationGrant, manifest_sha256: String) -> Result<Self> {
        validate_grant(&grant)?;
        if !j::hash_valid(&manifest_sha256) {
            return Err(j::bad("批注输入摘要无效"));
        }
        Ok(Self {
            grant,
            manifest_sha256,
        })
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct VisualAppendGroup {
    pub target_handle: String,
    pub drawings: Vec<Drawing>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct VisualAppendBatch {
    pub groups: Vec<VisualAppendGroup>,
}

pub(crate) fn validate_manifest(manifest: &VisualInputManifest) -> Result<()> {
    if manifest.version != 1
        || manifest.targets.is_empty()
        || manifest.targets.len() > 6
        || serde_json::to_vec(manifest)?.len() > 64 * 1024
    {
        return Err(j::bad("批注输入映射无效"));
    }
    let mut handles = HashSet::new();
    let mut assets = HashSet::new();
    let mut ordinals = HashSet::new();
    let mut sources = BTreeMap::new();
    for target in &manifest.targets {
        for id in [
            &target.handle,
            &target.attachment_id,
            &target.region_id,
            &target.source.background_id,
            &target.source.effective_source_id,
        ] {
            j::uuid(id)?;
        }
        if !handles.insert(&target.handle)
            || !assets.insert(&target.attachment_id)
            || !ordinals.insert(target.attachment_ordinal)
            || target.attachment_ordinal >= 6
            || !j::hash_valid(&target.attachment_sha256)
            || !j::hash_valid(&target.source.visual_sha256)
            || !target.source.source_size.valid()
            || !target.resized.valid()
            || !target.encoded.valid()
            || !target.crop.within(target.source.source_size)
            || !target.tile.within(target.resized)
            || target.profile_id.is_empty()
            || target.profile_id.len() > 80
            || target.profile_id.chars().any(char::is_control)
        {
            return Err(j::bad("批注输入映射无效"));
        }
        if let Some(previous) = sources.insert(&target.region_id, &target.source) {
            if previous != &target.source {
                return Err(j::bad("同一选区批注来源不一致"));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredVisualInput {
    pub manifest: VisualInputManifest,
    pub grant: VisualAnnotationGrant,
    pub cursors: BTreeMap<String, VisualSourceFence>,
    pub created_count: u32,
    #[serde(default)]
    pub rich_content_bytes: u64,
    #[serde(default)]
    pub rich_png_bytes: u64,
    #[serde(default)]
    pub rich_pixels: u64,
    pub revoked: bool,
}
pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE visual_run_inputs(run_id TEXT PRIMARY KEY,manifest_sha256 TEXT NOT NULL CHECK(length(manifest_sha256)=64),data TEXT NOT NULL CHECK(length(CAST(data AS BLOB))<=131072));")?;
    Ok(())
}
pub(crate) fn load(db: &Connection, run: &str) -> Result<Option<(String, StoredVisualInput)>> {
    let row: Option<(String, String)> = db
        .query_row(
            "SELECT manifest_sha256,data FROM visual_run_inputs WHERE run_id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(hash, raw)| Ok((hash, serde_json::from_str(&raw)?)))
        .transpose()
}
/// Count from immutable committed layout identities, including results whose
/// drawing/history reference has since been removed. Mutable counters alone
/// must not let a damaged input record reset the run budget.
pub(crate) fn validate_rich_usage(
    db: &Connection,
    run_id: &str,
    input: &StoredVisualInput,
    hash: &str,
) -> Result<()> {
    let mut seen = HashSet::new();
    let mut content_bytes = 0_u64;
    let mut png_bytes = 0_u64;
    let mut pixels = 0_u64;
    for event in j::events(db, run_id)? {
        if event.summary.phase != ReceiptPhase::Responded
            || event.summary.response_kind != Some(ToolResponseKind::LocalCommit)
            || !matches!(&event.binding, Some(ToolBinding::VisualAnnotation { .. }))
        {
            continue;
        }
        if event.binding != Some(input.grant.binding(hash)) {
            return Err(j::bad("富标注预算回执授权不匹配"));
        }
        let (_, _, body) = j::event(db, &event.summary.id)?;
        let JournalContent::Json { value } = body else {
            return Err(j::bad("富标注预算回执缺失"));
        };
        let mut event_seen = std::collections::HashMap::new();
        for value in value
            .get("regions")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| j::bad("批注提交回执无效"))?
            .iter()
            .flat_map(|region| {
                region
                    .get("drawings")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
            })
        {
            if let Some(value) = value.get("rich") {
                let reference: crate::RichDrawingRef = serde_json::from_value(value.clone())?;
                if let Some(previous) =
                    event_seen.insert(reference.layout_id.clone(), reference.clone())
                {
                    if previous != reference {
                        return Err(j::bad("同一回执布局引用不一致"));
                    }
                    continue;
                }
                if !seen.insert(reference.layout_id.clone()) {
                    return Err(j::bad("布局不能在另一提交回执重复登记"));
                }
                let layout = crate::rich_annotations::load(db, &reference)?;
                content_bytes = content_bytes
                    .checked_add(layout.content_bytes()?)
                    .ok_or_else(|| j::bad("富标注预算超限"))?;
                png_bytes = png_bytes
                    .checked_add(layout.png().len() as u64)
                    .ok_or_else(|| j::bad("富标注预算超限"))?;
                pixels = pixels
                    .checked_add(layout.pixels())
                    .ok_or_else(|| j::bad("富标注预算超限"))?;
            }
        }
    }
    if (content_bytes, png_bytes, pixels)
        != (
            input.rich_content_bytes,
            input.rich_png_bytes,
            input.rich_pixels,
        )
        || content_bytes > crate::MAX_RICH_RUN_CONTENT_BYTES
        || png_bytes > crate::MAX_RICH_RUN_PNG_BYTES
        || pixels > crate::MAX_RICH_PIXELS
    {
        return Err(j::bad("富标注累计预算与已提交布局不一致"));
    }
    Ok(())
}
pub(crate) fn validate(db: &Connection, state: &Snapshot) -> Result<()> {
    let mut statement = db.prepare("SELECT run_id,manifest_sha256,data FROM visual_run_inputs")?;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (run_id, hash, raw) = row?;
        let input: StoredVisualInput = serde_json::from_str(&raw)?;
        validate_manifest(&input.manifest)?;
        validate_grant(&input.grant)?;
        if j::digest(&serde_json::to_vec(&input.manifest)?) != hash
            || input.created_count > MAX_VISUAL_OBJECTS as u32
            || input.rich_content_bytes > crate::MAX_RICH_RUN_CONTENT_BYTES
            || input.rich_png_bytes > crate::MAX_RICH_RUN_PNG_BYTES
            || input.rich_pixels > crate::MAX_RICH_PIXELS
        {
            return Err(j::bad("批注输入记录损坏"));
        }
        validate_rich_usage(db, &run_id, &input, &hash)?;
        let run = j::required(db, &run_id)?;
        let scene = state
            .scenes
            .iter()
            .find(|s| s.id == run.scene && s.agent_id == run.agent)
            .ok_or_else(|| j::bad("批注会话来源不存在"))?;
        let message = scene
            .messages
            .iter()
            .find(|m| m.id == run.user && m.run_id.as_deref() == Some(&run_id))
            .ok_or_else(|| j::bad("批注消息来源不存在"))?;
        validate_asset_binding(
            &input.manifest,
            message
                .attachments
                .as_deref()
                .ok_or_else(|| j::bad("批注附件缺失"))?,
        )?;
        let expected: HashSet<_> = input
            .manifest
            .targets
            .iter()
            .map(|t| t.region_id.as_str())
            .collect();
        if expected.len() != input.cursors.len()
            || input.cursors.iter().any(|(region, cursor)| {
                !expected.contains(region.as_str()) || !j::hash_valid(&cursor.visual_sha256)
            })
        {
            return Err(j::bad("批注绘制游标损坏"));
        }
        for (region_id, cursor) in &input.cursors {
            let source = &input
                .manifest
                .targets
                .iter()
                .find(|t| t.region_id == *region_id)
                .expect("checked key")
                .source;
            if cursor.background_id != source.background_id
                || cursor.effective_source_id != source.effective_source_id
                || cursor.source_size != source.source_size
                || cursor.drawing_revision < source.drawing_revision
            {
                return Err(j::bad("批注来源游标损坏"));
            }
        }
    }
    // Origin is document provenance, including removed objects retained by undo.
    for scene in &state.scenes {
        for region in &scene.regions {
            let objects = region.drawings.iter().chain(
                region
                    .drawing_history
                    .undo
                    .iter()
                    .chain(&region.drawing_history.redo)
                    .flat_map(|s| s.edits())
                    .flat_map(|e| e.before.iter().chain(&e.after)),
            );
            for object in objects {
                if let Some(origin) = &object.origin {
                    let rich_value = object.rich.as_ref().map(serde_json::to_value).transpose()?;
                    validate_origin(origin)?;
                    let run = j::required(db, &origin.run_id)?;
                    let (hash, input) =
                        load(db, &origin.run_id)?.ok_or_else(|| j::bad("批注图元缺少来源"))?;
                    let (event_run, event, body) = j::event(db, &origin.tool_event_id)?;
                    if run.scene != scene.id
                        || run.user != origin.user_message_id
                        || hash != origin.manifest_sha256
                        || event_run != run.id
                        || event.summary.phase != ReceiptPhase::Responded
                        || event.summary.response_kind != Some(ToolResponseKind::LocalCommit)
                        || event.binding != Some(input.grant.binding(&hash))
                        || !input
                            .manifest
                            .targets
                            .iter()
                            .any(|t| t.handle == origin.target_handle && t.region_id == region.id)
                    {
                        return Err(j::bad("批注图元来源不匹配"));
                    }
                    let JournalContent::Json { value } = body else {
                        return Err(j::bad("批注图元缺少提交回执"));
                    };
                    if value.get("groupId").and_then(serde_json::Value::as_str)
                        != Some(origin.group_id.as_str())
                        || value
                            .get("manifestSha256")
                            .and_then(serde_json::Value::as_str)
                            != Some(hash.as_str())
                        || !value
                            .get("regions")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|regions| {
                                regions.iter().any(|r| {
                                    r.get("regionId").and_then(serde_json::Value::as_str)
                                        == Some(region.id.as_str())
                                        && r.get("drawings")
                                            .and_then(serde_json::Value::as_array)
                                            .is_some_and(|drawings| {
                                                drawings.iter().any(|d| {
                                                    d.get("id").and_then(serde_json::Value::as_str)
                                                        == Some(object.id.as_str())
                                                        && d.get("targetHandle")
                                                            .and_then(serde_json::Value::as_str)
                                                            == Some(origin.target_handle.as_str())
                                                        && rich_value.as_ref().is_none_or(
                                                            |reference| {
                                                                d.get("rich") == Some(reference)
                                                            },
                                                        )
                                                })
                                            })
                                })
                            })
                    {
                        return Err(j::bad("批注图元不属于已提交批次"));
                    }
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_asset_binding(
    manifest: &VisualInputManifest,
    assets: &[Asset],
) -> Result<()> {
    for target in &manifest.targets {
        let asset = assets
            .get(target.attachment_ordinal as usize)
            .ok_or_else(|| j::bad("批注附件顺序不匹配"))?;
        if asset.id != target.attachment_id
            || asset.kind != AssetKind::Image
            || asset.width != Some(target.encoded.width)
            || asset.height != Some(target.encoded.height)
        {
            return Err(j::bad("批注附件来源不匹配"));
        }
    }
    Ok(())
}
