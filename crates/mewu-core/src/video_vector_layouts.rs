// SPDX-License-Identifier: MPL-2.0
//! Complete editable vector ink, immutable descriptor and native RGBA raster.
//! Native proves actual pixels; no browser/model path or PNG is accepted here.
use crate::{CoreError, RichRendererIdentity, VideoPixelPoint, VideoPixelRect};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, CoreError>;
pub const MAX_VIDEO_VECTOR_POINTS: usize = 4096;
pub const MAX_VIDEO_VECTOR_DESCRIPTOR_BYTES: usize = 262144;
pub const MAX_VIDEO_VECTOR_PNG_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_VIDEO_VECTOR_OUTSET: f64 = 256.;
pub(crate) const TABLE_SQL: &str = "CREATE TABLE video_vector_layouts(layout_id TEXT PRIMARY KEY NOT NULL,layout_sha256 TEXT NOT NULL CHECK(length(layout_sha256)=64),descriptor TEXT NOT NULL CHECK(length(CAST(descriptor AS BLOB))<=262144),raster_png BLOB NOT NULL CHECK(typeof(raster_png)='blob' AND length(raster_png) BETWEEN 45 AND 8388608))";
pub(crate) const UPDATE_SQL: &str = "CREATE TRIGGER video_vector_layouts_no_update BEFORE UPDATE ON video_vector_layouts BEGIN SELECT RAISE(ABORT,'immutable video vector layout'); END";
pub(crate) const DELETE_SQL: &str = "CREATE TRIGGER video_vector_layouts_no_delete BEFORE DELETE ON video_vector_layouts BEGIN SELECT RAISE(ABORT,'immutable video vector layout'); END";
fn bad(s: &str) -> CoreError {
    CoreError::Invalid(s.into())
}
fn hash(p: &[u8]) -> String {
    format!("{:x}", Sha256::digest(p))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoVectorKind {
    Pen,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Number,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum VideoVectorContent {
    Pen {
        version: u32,
        local_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Line {
        version: u32,
        local_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Arrow {
        version: u32,
        local_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Rect {
        version: u32,
        local_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    Ellipse {
        version: u32,
        local_points: Vec<VideoPixelPoint>,
        color: String,
        stroke_width: f64,
    },
    // The single point is circle TOP LEFT, consistent with screenshot Drawing.
    Number {
        version: u32,
        local_points: Vec<VideoPixelPoint>,
        color: String,
        number: u16,
        diameter: f64,
    },
}
impl VideoVectorContent {
    pub fn kind(&self) -> VideoVectorKind {
        match self {
            Self::Pen { .. } => VideoVectorKind::Pen,
            Self::Line { .. } => VideoVectorKind::Line,
            Self::Arrow { .. } => VideoVectorKind::Arrow,
            Self::Rect { .. } => VideoVectorKind::Rect,
            Self::Ellipse { .. } => VideoVectorKind::Ellipse,
            Self::Number { .. } => VideoVectorKind::Number,
        }
    }
    pub fn version(&self) -> u32 {
        match self {
            Self::Pen { version, .. }
            | Self::Line { version, .. }
            | Self::Arrow { version, .. }
            | Self::Rect { version, .. }
            | Self::Ellipse { version, .. }
            | Self::Number { version, .. } => *version,
        }
    }
    pub fn points(&self) -> &[VideoPixelPoint] {
        match self {
            Self::Pen { local_points, .. }
            | Self::Line { local_points, .. }
            | Self::Arrow { local_points, .. }
            | Self::Rect { local_points, .. }
            | Self::Ellipse { local_points, .. }
            | Self::Number { local_points, .. } => local_points,
        }
    }
    pub fn color(&self) -> &str {
        match self {
            Self::Pen { color, .. }
            | Self::Line { color, .. }
            | Self::Arrow { color, .. }
            | Self::Rect { color, .. }
            | Self::Ellipse { color, .. }
            | Self::Number { color, .. } => color,
        }
    }
    pub fn stroke_width(&self) -> Option<f64> {
        match self {
            Self::Pen { stroke_width, .. }
            | Self::Line { stroke_width, .. }
            | Self::Arrow { stroke_width, .. }
            | Self::Rect { stroke_width, .. }
            | Self::Ellipse { stroke_width, .. } => Some(*stroke_width),
            Self::Number { .. } => None,
        }
    }
    pub fn number(&self) -> Option<(u16, f64)> {
        match self {
            Self::Number {
                number, diameter, ..
            } => Some((*number, *diameter)),
            _ => None,
        }
    }
    pub fn map_points(&self, mut map: impl FnMut(VideoPixelPoint) -> VideoPixelPoint) -> Self {
        let mut next = self.clone();
        let points = match &mut next {
            Self::Pen { local_points, .. }
            | Self::Line { local_points, .. }
            | Self::Arrow { local_points, .. }
            | Self::Rect { local_points, .. }
            | Self::Ellipse { local_points, .. }
            | Self::Number { local_points, .. } => local_points,
        };
        for p in points {
            *p = map(*p);
        }
        next
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoVectorLayoutRef {
    pub layout_id: String,
    pub layout_sha256: String,
    pub raster_sha256: String,
    pub width: u32,
    pub height: u32,
    pub geometry_bounds: VideoPixelRect,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    version: u32,
    layout_id: String,
    content: VideoVectorContent,
    renderer: RichRendererIdentity,
    width: u32,
    height: u32,
    geometry_bounds: VideoPixelRect,
    raster_sha256: String,
}
#[derive(Debug, Clone)]
pub struct VerifiedVideoVectorLayout {
    descriptor: Descriptor,
    reference: VideoVectorLayoutRef,
    png: Vec<u8>,
}
impl VerifiedVideoVectorLayout {
    /// Trusted native caller must fully decode/check the complete RGBA image.
    /// The constructor never clips or scales ink and derives its own controls.
    pub fn new(
        layout_id: String,
        content: VideoVectorContent,
        renderer: RichRendererIdentity,
        width: u32,
        height: u32,
        png: Vec<u8>,
    ) -> Result<Self> {
        let geometry_bounds = geometry_bounds(&content, width, height)?;
        let descriptor = Descriptor {
            version: 1,
            layout_id,
            content,
            renderer,
            width,
            height,
            geometry_bounds,
            raster_sha256: hash(&png),
        };
        validate_descriptor(&descriptor)?;
        validate_png(&png, width, height)?;
        let reference = reference(&descriptor)?;
        Ok(Self {
            descriptor,
            reference,
            png,
        })
    }
    pub fn reference(&self) -> &VideoVectorLayoutRef {
        &self.reference
    }
    pub fn content(&self) -> &VideoVectorContent {
        &self.descriptor.content
    }
    pub fn renderer(&self) -> &RichRendererIdentity {
        &self.descriptor.renderer
    }
    pub fn png(&self) -> &[u8] {
        &self.png
    }
    pub fn width(&self) -> u32 {
        self.reference.width
    }
    pub fn height(&self) -> u32 {
        self.reference.height
    }
}
fn geometry_bounds(c: &VideoVectorContent, w: u32, h: u32) -> Result<VideoPixelRect> {
    let points = c.points();
    let color = c.color().as_bytes();
    if c.version() != 1
        || color.len() != 7
        || color[0] != b'#'
        || !color[1..].iter().all(u8::is_ascii_hexdigit)
        || points.is_empty()
        || points.len() > MAX_VIDEO_VECTOR_POINTS
        || points.iter().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || p.x < 0.
                || p.y < 0.
                || p.x > f64::from(w)
                || p.y > f64::from(h)
        })
        || c.stroke_width()
            .is_some_and(|v| !v.is_finite() || !(0.5..=64.).contains(&v))
    {
        return Err(bad("视频矢量点或样式无效"));
    }
    match c.kind() {
        VideoVectorKind::Pen => {}
        VideoVectorKind::Number => {
            let (number, diameter) = c.number().ok_or_else(|| bad("视频序号无效"))?;
            if points.len() != 1
                || !(1..=9999).contains(&number)
                || !diameter.is_finite()
                || !(22. ..=64.).contains(&diameter)
                || points[0].x + diameter > f64::from(w)
                || points[0].y + diameter > f64::from(h)
            {
                return Err(bad("视频序号尺寸或布局无效"));
            }
            return Ok(VideoPixelRect {
                x: points[0].x,
                y: points[0].y,
                width: diameter,
                height: diameter,
            });
        }
        VideoVectorKind::Line | VideoVectorKind::Arrow => {
            if points.len() != 2 || points[0] == points[1] {
                return Err(bad("视频线段需两个不同控制点"));
            }
        }
        VideoVectorKind::Rect | VideoVectorKind::Ellipse => {
            if points.len() != 2 || points[0].x == points[1].x || points[0].y == points[1].y {
                return Err(bad("视频形状需正尺寸"));
            }
        }
    }
    let x = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let right = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let bottom = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    Ok(VideoPixelRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}
pub(crate) fn validate_ref_shape(r: &VideoVectorLayoutRef) -> Result<()> {
    let b = r.geometry_bounds;
    if !uuid::Uuid::parse_str(&r.layout_id).is_ok_and(|v| v.hyphenated().to_string() == r.layout_id)
        || !valid_hash(&r.layout_sha256)
        || !valid_hash(&r.raster_sha256)
        || r.width == 0
        || r.height == 0
        || r.width > 6000
        || r.height > 6000
        || u64::from(r.width) * u64::from(r.height) > 4_000_000
        || [b.x, b.y, b.width, b.height]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.)
        || b.x + b.width > f64::from(r.width)
        || b.y + b.height > f64::from(r.height)
        || b.x > MAX_VIDEO_VECTOR_OUTSET
        || b.y > MAX_VIDEO_VECTOR_OUTSET
        || f64::from(r.width) - b.x - b.width > MAX_VIDEO_VECTOR_OUTSET
        || f64::from(r.height) - b.y - b.height > MAX_VIDEO_VECTOR_OUTSET
    {
        return Err(bad("视频矢量完整布局引用无效"));
    }
    Ok(())
}
/// Integer raster placement with full control geometry inside the source.
/// The image may extend past an edge; consumers crop a temporary copy only.
pub(crate) fn validate_placement(
    top_left: VideoPixelPoint,
    r: &VideoVectorLayoutRef,
    w: u32,
    h: u32,
) -> Result<()> {
    validate_ref_shape(r)?;
    let b = r.geometry_bounds;
    let x = top_left.x;
    let y = top_left.y;
    let sw = f64::from(w);
    let sh = f64::from(h);
    let rw = f64::from(r.width);
    let rh = f64::from(r.height);
    if !x.is_finite()
        || !y.is_finite()
        || x.fract() != 0.
        || y.fract() != 0.
        || x + b.x < 0.
        || y + b.y < 0.
        || x + b.x + b.width > sw
        || y + b.y + b.height > sh
        || x >= sw
        || y >= sh
        || x + rw <= 0.
        || y + rh <= 0.
        || (-x).max(0.) > MAX_VIDEO_VECTOR_OUTSET
        || (-y).max(0.) > MAX_VIDEO_VECTOR_OUTSET
        || (x + rw - sw).max(0.) > MAX_VIDEO_VECTOR_OUTSET
        || (y + rh - sh).max(0.) > MAX_VIDEO_VECTOR_OUTSET
    {
        return Err(bad("视频矢量需整数位置且控制点在原视频内"));
    }
    Ok(())
}
fn validate_descriptor(v: &Descriptor) -> Result<()> {
    validate_ref_shape(&VideoVectorLayoutRef {
        layout_id: v.layout_id.clone(),
        layout_sha256: "0".repeat(64),
        raster_sha256: v.raster_sha256.clone(),
        width: v.width,
        height: v.height,
        geometry_bounds: v.geometry_bounds,
    })?;
    if v.version != 1
        || geometry_bounds(&v.content, v.width, v.height)? != v.geometry_bounds
        || v.renderer.style_version == 0
        || !valid_hash(&v.renderer.font_sha256)
        || [&v.renderer.id, &v.renderer.version]
            .iter()
            .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        || serde_json::to_vec(v)?.len() > MAX_VIDEO_VECTOR_DESCRIPTOR_BYTES
    {
        return Err(bad("视频矢量描述或渲染身份无效"));
    }
    Ok(())
}
fn reference(v: &Descriptor) -> Result<VideoVectorLayoutRef> {
    let mut sha = Sha256::new();
    sha.update(b"mewu.video-vector-layout.v1\0");
    sha.update(serde_json::to_vec(v)?);
    Ok(VideoVectorLayoutRef {
        layout_id: v.layout_id.clone(),
        layout_sha256: format!("{:x}", sha.finalize()),
        raster_sha256: v.raster_sha256.clone(),
        width: v.width,
        height: v.height,
        geometry_bounds: v.geometry_bounds,
    })
}
fn validate_png(p: &[u8], w: u32, h: u32) -> Result<()> {
    if p.len() < 45
        || p.len() > MAX_VIDEO_VECTOR_PNG_BYTES
        || &p[..8] != b"\x89PNG\r\n\x1a\n"
        || p[8..12] != 13u32.to_be_bytes()
        || &p[12..16] != b"IHDR"
        || p[16..20] != w.to_be_bytes()
        || p[20..24] != h.to_be_bytes()
        || p[24] != 8
        || p[25] != 6
        || p[26..29] != [0, 0, 0]
        || &p[p.len() - 12..] != b"\0\0\0\0IEND\xaeB`\x82"
    {
        return Err(bad("视频矢量PNG身份或预算无效"));
    }
    Ok(())
}
pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(&format!("{TABLE_SQL};{UPDATE_SQL};{DELETE_SQL};"))?;
    Ok(())
}
pub(crate) fn validate_schema(db: &Connection) -> Result<()> {
    for (name, expected) in [
        ("video_vector_layouts", TABLE_SQL),
        ("video_vector_layouts_no_update", UPDATE_SQL),
        ("video_vector_layouts_no_delete", DELETE_SQL),
    ] {
        let stored: Option<String> = db
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .optional()?;
        if stored.as_deref() != Some(expected) {
            return Err(bad("视频矢量存储或不可变约束缺失"));
        }
    }
    Ok(())
}
pub(crate) fn insert(db: &Connection, v: &VerifiedVideoVectorLayout) -> Result<()> {
    validate_descriptor(&v.descriptor)?;
    validate_png(v.png(), v.width(), v.height())?;
    if reference(&v.descriptor)? != v.reference || hash(v.png()) != v.reference.raster_sha256 {
        return Err(bad("视频矢量注册身份不匹配"));
    }
    db.execute("INSERT INTO video_vector_layouts(layout_id,layout_sha256,descriptor,raster_png) VALUES(?1,?2,?3,?4)",params![v.reference.layout_id,v.reference.layout_sha256,serde_json::to_string(&v.descriptor)?,v.png])?;
    Ok(())
}
pub(crate) fn load(db: &Connection, r: &VideoVectorLayoutRef) -> Result<VerifiedVideoVectorLayout> {
    validate_ref_shape(r)?;
    let sizes:Option<(i64,i64,String)>=db.query_row("SELECT length(CAST(descriptor AS BLOB)),length(raster_png),typeof(raster_png) FROM video_vector_layouts WHERE layout_id=?1",[&r.layout_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((d, p, ptype)) = sizes else {
        return Err(bad("视频矢量布局不存在"));
    };
    if !(1..=MAX_VIDEO_VECTOR_DESCRIPTOR_BYTES as i64).contains(&d)
        || !(45..=MAX_VIDEO_VECTOR_PNG_BYTES as i64).contains(&p)
        || ptype != "blob"
    {
        return Err(bad("视频矢量存储字节预算无效"));
    }
    let (stored, raw): (String, String) = db.query_row(
        "SELECT layout_sha256,descriptor FROM video_vector_layouts WHERE layout_id=?1",
        [&r.layout_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let descriptor: Descriptor = serde_json::from_str(&raw)?;
    validate_descriptor(&descriptor)?;
    if reference(&descriptor)? != *r || stored != r.layout_sha256 {
        return Err(bad("视频矢量注册身份不匹配"));
    }
    let png: Vec<u8> = db.query_row(
        "SELECT raster_png FROM video_vector_layouts WHERE layout_id=?1",
        [&r.layout_id],
        |row| row.get(0),
    )?;
    validate_png(&png, r.width, r.height)?;
    if hash(&png) != r.raster_sha256 {
        return Err(bad("视频矢量PNG摘要不匹配"));
    }
    Ok(VerifiedVideoVectorLayout {
        descriptor,
        reference: r.clone(),
        png,
    })
}
pub(crate) fn read_layout(
    path: &std::path::Path,
    r: &VideoVectorLayoutRef,
) -> Result<VerifiedVideoVectorLayout> {
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.execute_batch("BEGIN")?;
    let result = (|| {
        let app: u32 = db.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if app != 0x4d455755 || version != crate::store::DB_SCHEMA_VERSION {
            return Err(bad("视频矢量读取数据库版本不匹配"));
        }
        validate_schema(&db)?;
        load(&db, r)
    })();
    db.execute_batch("ROLLBACK")?;
    result
}
