// SPDX-License-Identifier: MPL-2.0
//! One transaction and one resource budget for typed Text and Vector layouts.
use crate::video_annotation_layouts as text;
use crate::video_vector_layouts as vector;
use crate::{
    CoreError, Snapshot, VerifiedVideoTextLayout, VerifiedVideoVectorLayout,
    VideoAnnotationDocument, VideoAnnotationPrimitive, VideoTextLayoutRef, VideoVectorLayoutRef,
};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, HashSet};
type Result<T> = std::result::Result<T, CoreError>;
pub const MAX_VIDEO_DOCUMENT_LAYOUT_PNG_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_VIDEO_DOCUMENT_LAYOUT_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_VIDEO_DOCUMENT_LAYOUT_DESCRIPTOR_BYTES: u64 = 4 * 1024 * 1024;
fn bad(s: &str) -> CoreError {
    CoreError::Invalid(s.into())
}
#[derive(Debug, Clone, Default)]
pub struct VideoAnnotationLayouts {
    pub text: Vec<VerifiedVideoTextLayout>,
    pub vector: Vec<VerifiedVideoVectorLayout>,
}
impl VideoAnnotationLayouts {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.vector.is_empty()
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum LayoutRef<'a> {
    Text(&'a VideoTextLayoutRef),
    Vector(&'a VideoVectorLayoutRef),
}
impl LayoutRef<'_> {
    fn id(&self) -> &str {
        match self {
            Self::Text(r) => &r.layout_id,
            Self::Vector(r) => &r.layout_id,
        }
    }
    fn table(&self) -> &'static str {
        match self {
            Self::Text(_) => "video_text_layouts",
            Self::Vector(_) => "video_vector_layouts",
        }
    }
    fn other_table(&self) -> &'static str {
        match self {
            Self::Text(_) => "video_vector_layouts",
            Self::Vector(_) => "video_text_layouts",
        }
    }
    fn pixels(&self) -> u64 {
        match self {
            Self::Text(r) => u64::from(r.width) * u64::from(r.height),
            Self::Vector(r) => u64::from(r.width) * u64::from(r.height),
        }
    }
}
fn references(doc: &VideoAnnotationDocument) -> impl Iterator<Item = LayoutRef<'_>> {
    doc.objects
        .iter()
        .chain(
            doc.undo
                .iter()
                .chain(&doc.redo)
                .flat_map(|s| s.before.iter().chain(&s.after)),
        )
        .filter_map(|o| match &o.primitive {
            VideoAnnotationPrimitive::Text { layout, .. } => Some(LayoutRef::Text(layout)),
            VideoAnnotationPrimitive::Vector { layout, .. } => Some(LayoutRef::Vector(layout)),
            VideoAnnotationPrimitive::Rect { .. } => None,
        })
}
fn unique<'a>(
    iter: impl Iterator<Item = LayoutRef<'a>>,
) -> Result<BTreeMap<String, LayoutRef<'a>>> {
    let mut refs = BTreeMap::new();
    for r in iter {
        if let Some(old) = refs.insert(r.id().to_string(), r) {
            if old != r {
                return Err(bad("同一视频布局ID引用或类型不一致"));
            }
        }
    }
    Ok(refs)
}
pub(crate) fn registered_layouts(
    db: &Connection,
    before: Option<&VideoAnnotationDocument>,
    after: &VideoAnnotationDocument,
    new_text: &[VerifiedVideoTextLayout],
    new_vector: &[VerifiedVideoVectorLayout],
    fresh_only: bool,
) -> Result<()> {
    // No duplicate even when its metadata is identical: the owned input must
    // have exactly one unique layout and every supplied layout must be used.
    let mut registered = BTreeMap::new();
    for r in new_text
        .iter()
        .map(|v| LayoutRef::Text(v.reference()))
        .chain(new_vector.iter().map(|v| LayoutRef::Vector(v.reference())))
    {
        if registered.insert(r.id().to_string(), r).is_some() {
            return Err(bad("本次视频布局ID重复或跨类型复用"));
        }
        let exists: Option<i64> = db
            .query_row(
                &format!("SELECT 1 FROM {} WHERE layout_id=?1", r.other_table()),
                [r.id()],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(bad("视频布局ID不能跨类型复用"));
        }
    }
    let existing = unique(before.into_iter().flat_map(references))?;
    let after_refs = unique(references(after))?;
    let mut used = HashSet::new();
    for (id, r) in after_refs {
        if registered.get(&id) == Some(&r) {
            used.insert(id);
        } else if !fresh_only && existing.get(&id) == Some(&r) {
        } else {
            return Err(bad("视频布局未由本次宿主登记或精确旧文档引用"));
        }
    }
    if used.len() != registered.len() {
        return Err(bad("不能登记未使用视频布局"));
    }
    for v in new_text {
        text::insert(db, v)?;
    }
    for v in new_vector {
        vector::insert(db, v)?;
    }
    validate_document(db, after)
}
pub(crate) fn validate_document(db: &Connection, doc: &VideoAnnotationDocument) -> Result<()> {
    let mut bytes = 0u64;
    let mut pixels = 0u64;
    let mut descriptors = 0u64;
    for r in unique(references(doc))?.into_values() {
        // Bound BEFORE loading either descriptor or PNG; one sequential owned
        // layout is read at a time, never all reachable pixels in one vector.
        let (d,p):(i64,i64)=db.query_row(&format!("SELECT length(CAST(descriptor AS BLOB)),length(raster_png) FROM {} WHERE layout_id=?1",r.table()),[r.id()],|row|Ok((row.get(0)?,row.get(1)?)))?;
        descriptors = descriptors
            .checked_add(u64::try_from(d).map_err(|_| bad("视频布局描述预算无效"))?)
            .ok_or_else(|| bad("视频布局描述预算超限"))?;
        bytes = bytes
            .checked_add(u64::try_from(p).map_err(|_| bad("视频布局PNG预算无效"))?)
            .ok_or_else(|| bad("视频布局PNG预算超限"))?;
        pixels = pixels
            .checked_add(r.pixels())
            .ok_or_else(|| bad("视频布局像素预算超限"))?;
        if descriptors > MAX_VIDEO_DOCUMENT_LAYOUT_DESCRIPTOR_BYTES
            || bytes > MAX_VIDEO_DOCUMENT_LAYOUT_PNG_BYTES
            || pixels > MAX_VIDEO_DOCUMENT_LAYOUT_PIXELS
        {
            return Err(bad("视频批注含历史的文字与矢量合计预算超限"));
        }
        match r {
            LayoutRef::Text(r) => {
                text::load(db, r)?;
            }
            LayoutRef::Vector(r) => {
                vector::load(db, r)?;
            }
        }
    }
    Ok(())
}
pub(crate) fn validate(db: &Connection, state: &Snapshot) -> Result<()> {
    text::validate_schema(db)?;
    vector::validate_schema(db)?;
    let duplicate:Option<String>=db.query_row("SELECT t.layout_id FROM video_text_layouts t JOIN video_vector_layouts v USING(layout_id) LIMIT 1",[],|row|row.get(0)).optional()?;
    if duplicate.is_some() {
        return Err(bad("视频布局ID跨类型冲突"));
    }
    for item in state.scenes.iter().flat_map(|s| &s.items) {
        if let Some(doc) = &item.video_annotations {
            validate_document(db, doc)?;
        }
    }
    Ok(())
}
