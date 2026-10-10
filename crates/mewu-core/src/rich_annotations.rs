// SPDX-License-Identifier: MPL-2.0
//! Host-rendered immutable layouts. The core accepts no model supplied image
//! paths or SVG, and binary pixels never enter the drawing JSON or its history.
use crate::{CoreError, DrawingKind, Snapshot};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::Path, time::Duration};

type Result<T> = std::result::Result<T, CoreError>;
pub const MAX_RICH_CONTENT_BYTES: usize = 32 * 1024;
pub const MAX_RICH_PNG_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RICH_RUN_CONTENT_BYTES: u64 = 128 * 1024;
pub const MAX_RICH_RUN_PNG_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_RICH_PIXELS: u64 = 32 * 1024 * 1024;
pub const RICH_DRAWING_COLOR: &str = "#000000";
pub const RICH_DRAWING_STROKE_WIDTH: f64 = 1.;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RichKind {
    Table,
    Formula,
    Repair,
    Extracted,
}
impl RichKind {
    pub fn is_raster_edit(self) -> bool {
        matches!(self, Self::Repair | Self::Extracted)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RichAlignment {
    Left,
    Center,
    Right,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RichContent {
    Table {
        version: u32,
        header: Vec<String>,
        rows: Vec<Vec<String>>,
        align: Vec<Option<RichAlignment>>,
    },
    Formula {
        version: u32,
        tex: String,
        color: String,
        font_size: f64,
    },
    Raster {
        version: u32,
        role: RasterRole,
        source_sha256: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RasterRole {
    Repair,
    Extracted,
}
impl RichContent {
    pub fn kind(&self) -> RichKind {
        match self {
            Self::Table { .. } => RichKind::Table,
            Self::Formula { .. } => RichKind::Formula,
            Self::Raster {
                role: RasterRole::Repair,
                ..
            } => RichKind::Repair,
            Self::Raster {
                role: RasterRole::Extracted,
                ..
            } => RichKind::Extracted,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RichRendererIdentity {
    pub id: String,
    pub version: String,
    pub font_sha256: String,
    pub style_version: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RichDrawingRef {
    pub layout_id: String,
    pub layout_sha256: String,
    pub raster_sha256: String,
    pub kind: RichKind,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    version: u32,
    layout_id: String,
    content: RichContent,
    renderer: RichRendererIdentity,
    width: u32,
    height: u32,
    raster_sha256: String,
}

/// This Rust boundary is deliberately not Deserialize. A native renderer must
/// decode/check its own output before calling new; core checks binary identity,
/// PNG header dimensions and all typed metadata, but is not a PNG decoder.
#[derive(Debug, Clone)]
pub struct VerifiedRichLayout {
    descriptor: Descriptor,
    reference: RichDrawingRef,
    png: Vec<u8>,
}
impl VerifiedRichLayout {
    pub fn new(
        layout_id: String,
        content: RichContent,
        renderer: RichRendererIdentity,
        width: u32,
        height: u32,
        png: Vec<u8>,
    ) -> Result<Self> {
        let descriptor = Descriptor {
            version: 1,
            layout_id,
            content,
            renderer,
            width,
            height,
            raster_sha256: hash(&png),
        };
        validate_descriptor(&descriptor)?;
        validate_png(&png, width, height)?;
        let reference = descriptor_reference(&descriptor)?;
        Ok(Self {
            descriptor,
            reference,
            png,
        })
    }
    pub fn reference(&self) -> &RichDrawingRef {
        &self.reference
    }
    pub fn content(&self) -> &RichContent {
        &self.descriptor.content
    }
    pub fn renderer(&self) -> &RichRendererIdentity {
        &self.descriptor.renderer
    }
    pub fn png(&self) -> &[u8] {
        &self.png
    }
    pub fn width(&self) -> u32 {
        self.descriptor.width
    }
    pub fn height(&self) -> u32 {
        self.descriptor.height
    }
    pub(crate) fn content_bytes(&self) -> Result<u64> {
        Ok(serde_json::to_vec(self.content())?.len() as u64)
    }
    pub(crate) fn pixels(&self) -> u64 {
        u64::from(self.width()) * u64::from(self.height())
    }
}

/// A verified, bounded read result, never a renderer-provided DTO.
pub type RichLayout = VerifiedRichLayout;

fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn valid_text(value: &str) -> bool {
    !value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
}
fn validate_descriptor(value: &Descriptor) -> Result<()> {
    validate_ref_shape(&RichDrawingRef {
        layout_id: value.layout_id.clone(),
        layout_sha256: "0".repeat(64),
        raster_sha256: value.raster_sha256.clone(),
        kind: value.content.kind(),
        width: value.width,
        height: value.height,
    })?;
    if value.version != 1
        || value.renderer.style_version == 0
        || !valid_hash(&value.renderer.font_sha256)
        || [&value.renderer.id, &value.renderer.version]
            .iter()
            .any(|v| v.is_empty() || v.len() > 128 || v.chars().any(char::is_control))
        || serde_json::to_vec(&value.content)?.len() > MAX_RICH_CONTENT_BYTES
    {
        return Err(invalid("富标注布局身份或内容预算无效"));
    }
    match &value.content {
        RichContent::Raster {
            version,
            source_sha256,
            ..
        } => {
            if *version != 1
                || !valid_hash(source_sha256)
                || value.renderer.id != "mewu.local-raster-edit"
                || value.renderer.version != "1"
                || value.renderer.style_version != 1
            {
                return Err(invalid("本地修补图层来源或版本无效"));
            }
        }
        RichContent::Table {
            version,
            header,
            rows,
            align,
        } => {
            if *version != 1
                || header.is_empty()
                || header.len() > 32
                || rows.len() > 199
                || align.len() != header.len()
                || rows.iter().any(|r| r.len() != header.len())
                || header
                    .iter()
                    .chain(rows.iter().flatten())
                    .any(|c| !valid_text(c))
            {
                return Err(invalid("表格需为受限的等宽纯文本单元格"));
            }
        }
        RichContent::Formula {
            version,
            tex,
            color,
            font_size,
        } => {
            let bytes = color.as_bytes();
            if *version != 1
                || tex.trim().is_empty()
                || tex.chars().count() > 2048
                || !valid_text(tex)
                || bytes.len() != 7
                || bytes[0] != b'#'
                || !bytes[1..].iter().all(u8::is_ascii_hexdigit)
                || !font_size.is_finite()
                || !(6. ..=256.).contains(font_size)
            {
                return Err(invalid("公式内容、颜色或字号无效"));
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_ref_shape(value: &RichDrawingRef) -> Result<()> {
    if uuid::Uuid::parse_str(&value.layout_id).is_err()
        || !valid_hash(&value.layout_sha256)
        || !valid_hash(&value.raster_sha256)
        || value.width == 0
        || value.height == 0
        || value.width > 6000
        || value.height > 6000
        || u64::from(value.width) * u64::from(value.height) > MAX_RICH_PIXELS
    {
        return Err(invalid("富标注引用身份或尺寸无效"));
    }
    Ok(())
}
fn descriptor_reference(value: &Descriptor) -> Result<RichDrawingRef> {
    let mut digest = Sha256::new();
    digest.update(b"mewu.rich-layout.v1\0");
    digest.update(serde_json::to_vec(value)?);
    Ok(RichDrawingRef {
        layout_id: value.layout_id.clone(),
        layout_sha256: format!("{:x}", digest.finalize()),
        raster_sha256: value.raster_sha256.clone(),
        kind: value.content.kind(),
        width: value.width,
        height: value.height,
    })
}
fn validate_png(bytes: &[u8], width: u32, height: u32) -> Result<()> {
    // Typed PNG binary header, not string parsing. Native's bounded decoder is
    // responsible for CRC/deflate/full raster validation before this boundary.
    if bytes.len() < 45
        || bytes.len() > MAX_RICH_PNG_BYTES
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || bytes[8..12] != 13_u32.to_be_bytes()
        || &bytes[12..16] != b"IHDR"
        || bytes[16..20] != width.to_be_bytes()
        || bytes[20..24] != height.to_be_bytes()
        || bytes[24] != 8
        || ![0, 2, 4, 6].contains(&bytes[25])
        || bytes[26..29] != [0, 0, 0]
        || &bytes[bytes.len() - 12..] != b"\0\0\0\0IEND\xaeB`\x82"
    {
        return Err(invalid("富标注PNG格式、尺寸或字节预算无效"));
    }
    Ok(())
}
pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE drawing_layouts(
        layout_id TEXT PRIMARY KEY NOT NULL,
        layout_sha256 TEXT NOT NULL CHECK(length(layout_sha256)=64),
        descriptor TEXT NOT NULL CHECK(length(CAST(descriptor AS BLOB))<=65536),
        raster_png BLOB NOT NULL CHECK(typeof(raster_png)='blob' AND length(raster_png) BETWEEN 45 AND 8388608)
    );
    CREATE TRIGGER drawing_layouts_no_update BEFORE UPDATE ON drawing_layouts BEGIN SELECT RAISE(ABORT,'immutable drawing layout'); END;
    CREATE TRIGGER drawing_layouts_no_delete BEFORE DELETE ON drawing_layouts BEGIN SELECT RAISE(ABORT,'immutable drawing layout'); END;")?;
    Ok(())
}
pub(crate) fn insert(db: &Connection, layout: &VerifiedRichLayout) -> Result<()> {
    // Never upsert/reuse even an identical ID: a new batch must register its
    // own layouts, and retry of a committed lease is rejected by the journal.
    validate_descriptor(&layout.descriptor)?;
    validate_png(layout.png(), layout.width(), layout.height())?;
    if descriptor_reference(&layout.descriptor)? != layout.reference
        || hash(layout.png()) != layout.reference.raster_sha256
    {
        return Err(invalid("富标注布局摘要不匹配"));
    }
    db.execute("INSERT INTO drawing_layouts(layout_id,layout_sha256,descriptor,raster_png) VALUES(?1,?2,?3,?4)",
        params![layout.reference.layout_id,layout.reference.layout_sha256,serde_json::to_string(&layout.descriptor)?,layout.png()])?;
    Ok(())
}
pub(crate) fn load(db: &Connection, reference: &RichDrawingRef) -> Result<RichLayout> {
    validate_ref_shape(reference)?;
    // Check SQL lengths before copying a possibly corrupted unbounded blob.
    let row: Option<(i64,i64,String)> = db.query_row(
        "SELECT length(CAST(descriptor AS BLOB)),length(raster_png),typeof(raster_png) FROM drawing_layouts WHERE layout_id=?1",
        [&reference.layout_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((descriptor_bytes, png_bytes, png_type)) = row else {
        return Err(invalid("富标注布局不存在"));
    };
    if !(1..=65536).contains(&descriptor_bytes)
        || !(45..=MAX_RICH_PNG_BYTES as i64).contains(&png_bytes)
        || png_type != "blob"
    {
        return Err(invalid("富标注存储超出预算"));
    }
    let (stored_hash, raw): (String, String) = db.query_row(
        "SELECT layout_sha256,descriptor FROM drawing_layouts WHERE layout_id=?1",
        [&reference.layout_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let descriptor: Descriptor = serde_json::from_str(&raw)?;
    validate_descriptor(&descriptor)?;
    if descriptor_reference(&descriptor)? != *reference || stored_hash != reference.layout_sha256 {
        return Err(invalid("富标注注册布局与引用不一致"));
    }
    let png: Vec<u8> = db.query_row(
        "SELECT raster_png FROM drawing_layouts WHERE layout_id=?1",
        [&reference.layout_id],
        |r| r.get(0),
    )?;
    validate_png(&png, reference.width, reference.height)?;
    if hash(&png) != reference.raster_sha256 {
        return Err(invalid("富标注PNG摘要不匹配"));
    }
    Ok(VerifiedRichLayout {
        descriptor,
        reference: reference.clone(),
        png,
    })
}
pub(crate) fn validate(db: &Connection, state: &Snapshot) -> Result<()> {
    // Also require the new table when there are no rich drawings in old data.
    db.prepare(
        "SELECT layout_id,layout_sha256,descriptor,raster_png FROM drawing_layouts LIMIT 0",
    )?;
    let mut checked: HashMap<String, RichDrawingRef> = HashMap::new();
    for scene in &state.scenes {
        for region in &scene.regions {
            let objects = region.drawings.iter().chain(
                region
                    .drawing_history
                    .undo
                    .iter()
                    .chain(&region.drawing_history.redo)
                    .flat_map(|step| step.edits())
                    .flat_map(|edit| edit.before.iter().chain(&edit.after)),
            );
            for object in objects {
                if object.kind == DrawingKind::Rich {
                    let reference = object
                        .rich
                        .as_ref()
                        .ok_or_else(|| invalid("富标注缺少布局引用"))?;
                    if let Some(previous) = checked.get(&reference.layout_id) {
                        if previous != reference {
                            return Err(invalid("同一富标注布局有不同引用身份"));
                        }
                    } else {
                        load(db, reference)?;
                        checked.insert(reference.layout_id.clone(), reference.clone());
                    }
                }
            }
        }
    }
    Ok(())
}
/// Short read only query for a trusted database path; no Store::open,
/// migration, recovery or persistent reader/cached SQLite connection.
pub(crate) fn read_layout(path: &Path, reference: &RichDrawingRef) -> Result<RichLayout> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("BEGIN DEFERRED")?;
    let result = (|| {
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != crate::store::DB_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedSchema(u64::from(version)));
        }
        let application: i64 = db.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if application != 0x4d455755 {
            return Err(invalid("数据库应用标识不匹配"));
        }
        load(&db, reference)
    })();
    db.execute_batch("ROLLBACK")?;
    result
}
