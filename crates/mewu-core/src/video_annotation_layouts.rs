// SPDX-License-Identifier: MPL-2.0
//! Immutable native text rasters for video, never accepted as image Rich drawings.
use crate::{CoreError, RichRendererIdentity};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
type Result<T> = std::result::Result<T, CoreError>;
pub const MAX_VIDEO_TEXT_PNG_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const TABLE_SQL: &str = "CREATE TABLE video_text_layouts(layout_id TEXT PRIMARY KEY NOT NULL,layout_sha256 TEXT NOT NULL CHECK(length(layout_sha256)=64),descriptor TEXT NOT NULL CHECK(length(CAST(descriptor AS BLOB))<=65536),raster_png BLOB NOT NULL CHECK(typeof(raster_png)='blob' AND length(raster_png) BETWEEN 45 AND 8388608))";
pub(crate) const UPDATE_SQL: &str = "CREATE TRIGGER video_text_layouts_no_update BEFORE UPDATE ON video_text_layouts BEGIN SELECT RAISE(ABORT,'immutable video text layout'); END";
pub(crate) const DELETE_SQL: &str = "CREATE TRIGGER video_text_layouts_no_delete BEFORE DELETE ON video_text_layouts BEGIN SELECT RAISE(ABORT,'immutable video text layout'); END";
fn bad(s: &str) -> CoreError {
    CoreError::Invalid(s.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoTextContent {
    pub version: u32,
    pub text: String,
    pub color: String,
    pub font_size: f64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoTextLayoutRef {
    pub layout_id: String,
    pub layout_sha256: String,
    pub raster_sha256: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    version: u32,
    layout_id: String,
    content: VideoTextContent,
    renderer: RichRendererIdentity,
    width: u32,
    height: u32,
    raster_sha256: String,
}
#[derive(Debug, Clone)]
pub struct VerifiedVideoTextLayout {
    descriptor: Descriptor,
    reference: VideoTextLayoutRef,
    png: Vec<u8>,
}
impl VerifiedVideoTextLayout {
    /// Native must fully decode/validate transparent RGBA pixels before calling.
    /// Core validates typed metadata, header identity and hashes; no Deserialize.
    pub fn new(
        layout_id: String,
        content: VideoTextContent,
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
        let reference = reference(&descriptor)?;
        Ok(Self {
            descriptor,
            reference,
            png,
        })
    }
    pub fn reference(&self) -> &VideoTextLayoutRef {
        &self.reference
    }
    pub fn content(&self) -> &VideoTextContent {
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
    pub(crate) fn content_bytes(&self) -> Result<u64> {
        Ok(serde_json::to_vec(self.content())?.len() as u64)
    }
    pub(crate) fn pixels(&self) -> u64 {
        u64::from(self.width()) * u64::from(self.height())
    }
}
pub(crate) fn validate_ref_shape(v: &VideoTextLayoutRef) -> Result<()> {
    if uuid::Uuid::parse_str(&v.layout_id).is_err()
        || !valid_hash(&v.layout_sha256)
        || !valid_hash(&v.raster_sha256)
        || v.width == 0
        || v.height == 0
        || v.width > 6000
        || v.height > 6000
        || u64::from(v.width) * u64::from(v.height) > 4_000_000
    {
        return Err(bad("视频文字布局引用无效"));
    }
    Ok(())
}
fn validate_descriptor(v: &Descriptor) -> Result<()> {
    validate_ref_shape(&VideoTextLayoutRef {
        layout_id: v.layout_id.clone(),
        layout_sha256: "0".repeat(64),
        raster_sha256: v.raster_sha256.clone(),
        width: v.width,
        height: v.height,
    })?;
    let color = v.content.color.as_bytes();
    if v.version != 1
        || v.content.version != 1
        || v.content.text.trim().is_empty()
        || v.content.text.chars().count() > 500
        || v.content.text.len() > 4096
        || v.content
            .text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        || color.len() != 7
        || color[0] != b'#'
        || !color[1..].iter().all(u8::is_ascii_hexdigit)
        || !v.content.font_size.is_finite()
        || !(8. ..=128.).contains(&v.content.font_size)
        || v.renderer.style_version == 0
        || !valid_hash(&v.renderer.font_sha256)
        || [&v.renderer.id, &v.renderer.version]
            .iter()
            .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        || serde_json::to_vec(v)?.len() > 65536
    {
        return Err(bad("视频文字内容或渲染身份无效"));
    }
    Ok(())
}
fn reference(v: &Descriptor) -> Result<VideoTextLayoutRef> {
    let mut sha = Sha256::new();
    sha.update(b"mewu.video-text-layout.v1\0");
    sha.update(serde_json::to_vec(v)?);
    Ok(VideoTextLayoutRef {
        layout_id: v.layout_id.clone(),
        layout_sha256: format!("{:x}", sha.finalize()),
        raster_sha256: v.raster_sha256.clone(),
        width: v.width,
        height: v.height,
    })
}
fn validate_png(p: &[u8], w: u32, h: u32) -> Result<()> {
    if p.len() < 45
        || p.len() > MAX_VIDEO_TEXT_PNG_BYTES
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
        return Err(bad("视频文字PNG身份或预算无效"));
    }
    Ok(())
}
pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(&format!("{TABLE_SQL};{UPDATE_SQL};{DELETE_SQL};"))?;
    Ok(())
}
pub(crate) fn validate_schema(db: &Connection) -> Result<()> {
    for (name, expected) in [
        ("video_text_layouts", TABLE_SQL),
        ("video_text_layouts_no_update", UPDATE_SQL),
        ("video_text_layouts_no_delete", DELETE_SQL),
    ] {
        let stored: Option<String> = db
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .optional()?;
        if stored.as_deref() != Some(expected) {
            return Err(bad("视频文字存储结构或不可变约束缺失"));
        }
    }
    Ok(())
}
pub(crate) fn insert(db: &Connection, v: &VerifiedVideoTextLayout) -> Result<()> {
    validate_descriptor(&v.descriptor)?;
    validate_png(v.png(), v.width(), v.height())?;
    if reference(&v.descriptor)? != v.reference || hash(v.png()) != v.reference.raster_sha256 {
        return Err(bad("视频文字布局身份不匹配"));
    }
    db.execute("INSERT INTO video_text_layouts(layout_id,layout_sha256,descriptor,raster_png) VALUES(?1,?2,?3,?4)",params![v.reference.layout_id,v.reference.layout_sha256,serde_json::to_string(&v.descriptor)?,v.png])?;
    Ok(())
}
pub(crate) fn load(db: &Connection, r: &VideoTextLayoutRef) -> Result<VerifiedVideoTextLayout> {
    validate_ref_shape(r)?;
    let lengths:Option<(i64,i64,String)>=db.query_row("SELECT length(CAST(descriptor AS BLOB)),length(raster_png),typeof(raster_png) FROM video_text_layouts WHERE layout_id=?1",[&r.layout_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((dbytes, pbytes, ptype)) = lengths else {
        return Err(bad("视频文字布局不存在"));
    };
    if !(1..=65536).contains(&dbytes)
        || !(45..=MAX_VIDEO_TEXT_PNG_BYTES as i64).contains(&pbytes)
        || ptype != "blob"
    {
        return Err(bad("视频文字存储超出预算"));
    }
    let (stored, raw): (String, String) = db.query_row(
        "SELECT layout_sha256,descriptor FROM video_text_layouts WHERE layout_id=?1",
        [&r.layout_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let descriptor: Descriptor = serde_json::from_str(&raw)?;
    validate_descriptor(&descriptor)?;
    if reference(&descriptor)? != *r || stored != r.layout_sha256 {
        return Err(bad("视频文字注册身份不匹配"));
    }
    let png: Vec<u8> = db.query_row(
        "SELECT raster_png FROM video_text_layouts WHERE layout_id=?1",
        [&r.layout_id],
        |row| row.get(0),
    )?;
    validate_png(&png, r.width, r.height)?;
    if hash(&png) != r.raster_sha256 {
        return Err(bad("视频文字PNG摘要不匹配"));
    }
    Ok(VerifiedVideoTextLayout {
        descriptor,
        reference: r.clone(),
        png,
    })
}
pub(crate) fn read_layout(
    path: &std::path::Path,
    r: &VideoTextLayoutRef,
) -> Result<VerifiedVideoTextLayout> {
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.execute_batch("BEGIN")?;
    let result = (|| {
        let app: u32 = db.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if app != 0x4d455755 || version != crate::store::DB_SCHEMA_VERSION {
            return Err(bad("视频文字读取数据库版本不匹配"));
        }
        validate_schema(&db)?;
        load(&db, r)
    })();
    db.execute_batch("ROLLBACK")?;
    result
}
