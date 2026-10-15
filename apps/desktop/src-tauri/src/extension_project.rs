// SPDX-License-Identifier: MPL-2.0
//! Optional, non-executable project metadata. The console validator includes this
//! same module; installation continues to use the unchanged API 2 catalog.
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, collections::BTreeSet, io::Cursor};

pub const DEFAULT_PROJECTS: &str =
    "https://raw.githubusercontent.com/abnste/mewu-plugins/main/mewu-projects.json";
pub const MAX_PROJECTS_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PROJECT_BYTES: usize = 64 * 1024;
pub const MAX_SCREENSHOT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SCREENSHOTS: usize = 6;
pub const MAX_IMAGE_SIDE: u32 = 4096;
pub const MAX_IMAGE_PIXELS: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Screenshot {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Project {
    pub id: String,
    pub version: String,
    pub package_sha256: String,
    pub source_repository: String,
    pub source_commit: String,
    pub screenshots: Vec<Screenshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectCatalog {
    pub schema_version: u32,
    pub projects: Vec<Project>,
}

fn segment(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn repository(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|s| segment(s, 160))
        && !parts[1].to_ascii_lowercase().ends_with(".git")
}

fn path_format(path: &str) -> Result<ImageFormat, String> {
    let parts: Vec<_> = path.split('/').collect();
    if path.len() > 240 || parts.len() > 8 || !parts.iter().all(|s| segment(s, 160)) {
        return Err("插件截图路径无效".into());
    }
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Ok(ImageFormat::Png),
        Some("jpg" | "jpeg") => Ok(ImageFormat::Jpeg),
        Some("webp") => Ok(ImageFormat::WebP),
        _ => Err("插件截图只支持 PNG、JPEG 或 WebP".into()),
    }
}

fn dimensions(width: u32, height: u32) -> bool {
    (16..=MAX_IMAGE_SIDE).contains(&width)
        && (16..=MAX_IMAGE_SIDE).contains(&height)
        && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

fn validate_screenshot(shot: &Screenshot) -> Result<(), String> {
    path_format(&shot.path)?;
    if !hex(&shot.sha256, 64)
        || shot.size == 0
        || shot.size > MAX_SCREENSHOT_BYTES as u64
        || !dimensions(shot.width, shot.height)
        || shot
            .caption
            .as_ref()
            .is_some_and(|s| s.len() > 240 || s.chars().any(char::is_control))
    {
        return Err("插件截图元数据无效或超出限制".into());
    }
    Ok(())
}

pub fn validate_project(project: &Project) -> Result<(), String> {
    if !segment(&project.id, 128)
        || project.id.to_ascii_lowercase().starts_with("mewu.")
        || project.version.len() > 128
        || semver::Version::parse(&project.version).is_err()
        || !hex(&project.package_sha256, 64)
        || !repository(&project.source_repository)
        || !hex(&project.source_commit, 40)
        || project.screenshots.len() > MAX_SCREENSHOTS
    {
        return Err("插件项目身份或源码来源无效".into());
    }
    let mut paths = BTreeSet::new();
    for shot in &project.screenshots {
        validate_screenshot(shot)?;
        if !paths.insert(shot.path.to_ascii_lowercase()) {
            return Err("插件截图路径重复".into());
        }
    }
    if serde_json::to_vec(project)
        .map_err(|_| "插件项目无法编码")?
        .len()
        > MAX_PROJECT_BYTES
    {
        return Err("插件项目元数据过大".into());
    }
    Ok(())
}

pub fn parse_project(bytes: &[u8]) -> Result<Project, String> {
    if bytes.len() > MAX_PROJECT_BYTES {
        return Err("插件项目元数据过大".into());
    }
    // Deserialize raw bytes, rather than Value, to retain duplicate-field errors.
    let project = serde_json::from_slice(bytes).map_err(|_| "插件项目 JSON 无效或含重复字段")?;
    validate_project(&project)?;
    Ok(project)
}

pub fn parse_projects(bytes: &[u8]) -> Result<ProjectCatalog, String> {
    if bytes.len() > MAX_PROJECTS_BYTES {
        return Err("插件项目目录过大".into());
    }
    let catalog: ProjectCatalog =
        serde_json::from_slice(bytes).map_err(|_| "插件项目目录 JSON 无效或含重复字段")?;
    if catalog.schema_version != 1 || catalog.projects.len() > 2048 {
        return Err("插件项目目录版本或数量无效".into());
    }
    let mut identities = BTreeSet::new();
    let mut sources = BTreeMap::new();
    for project in &catalog.projects {
        validate_project(project)?;
        if !identities.insert((project.id.to_ascii_lowercase(), project.version.clone())) {
            return Err("插件项目版本重复".into());
        }
        let source = project.source_repository.to_ascii_lowercase();
        if sources
            .insert(source, &project.id)
            .is_some_and(|id| id != &project.id)
        {
            return Err("不同插件应使用各自的源码项目".into());
        }
    }
    Ok(catalog)
}

/// Exact package identity; a later version must never borrow an older gallery.
pub fn find_project<'a>(
    catalog: &'a ProjectCatalog,
    id: &str,
    version: &str,
    package_sha256: &str,
) -> Option<&'a Project> {
    catalog.projects.iter().find(|project| {
        project.id == id && project.version == version && project.package_sha256 == package_sha256
    })
}

pub fn image_url(project: &Project, index: usize) -> Result<String, String> {
    validate_project(project)?;
    let shot = project.screenshots.get(index).ok_or("插件截图不存在")?;
    Ok(format!(
        "https://raw.githubusercontent.com/{}/{}/{}",
        project.source_repository, project.source_commit, shot.path
    ))
}

pub fn image_mime(shot: &Screenshot) -> Result<&'static str, String> {
    match path_format(&shot.path)? {
        ImageFormat::Png => Ok("image/png"),
        ImageFormat::Jpeg => Ok("image/jpeg"),
        ImageFormat::WebP => Ok("image/webp"),
        _ => Err("插件截图格式无效".into()),
    }
}

/// Bind a returned image to the metadata already shown by the caller. The
/// mutable feed may change while a details page is open, but its image may not.
pub fn image_for_snapshot<'a>(
    project: &'a Project,
    index: usize,
    expected_source_commit: &str,
    expected_image_sha256: &str,
) -> Result<&'a Screenshot, String> {
    let shot = project.screenshots.get(index).ok_or("插件截图不存在")?;
    if project.source_commit != expected_source_commit || shot.sha256 != expected_image_sha256 {
        return Err("插件图库已更新，请刷新详情".into());
    }
    Ok(shot)
}

/// Both publishing CI and the host fully decode the same bounded, static raster.
/// SHA/size alone are not an image validity check.
pub fn validate_image(shot: &Screenshot, bytes: &[u8]) -> Result<(), String> {
    validate_screenshot(shot)?;
    if bytes.len() as u64 != shot.size || format!("{:x}", Sha256::digest(bytes)) != shot.sha256 {
        return Err("插件截图与已发布的大小或 SHA-256 不符".into());
    }
    let format = path_format(&shot.path)?;
    if image::guess_format(bytes).map_err(|_| "插件截图格式无效")? != format {
        return Err("插件截图内容与扩展名不符".into());
    }
    let actual = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| "无法读取插件截图尺寸")?;
    if actual != (shot.width, shot.height) || !dimensions(actual.0, actual.1) {
        return Err("插件截图实际尺寸与声明不符".into());
    }
    let animated = match format {
        ImageFormat::Png => image::codecs::png::PngDecoder::new(Cursor::new(bytes))
            .map_err(|_| "插件截图 PNG 无效")?
            .is_apng()
            .map_err(|_| "插件截图 PNG 无效")?,
        ImageFormat::WebP => image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
            .map_err(|_| "插件截图 WebP 无效")?
            .has_animation(),
        _ => false,
    };
    if animated {
        return Err("插件截图必须是静态图片".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| "插件截图内容无效")?;
    if (decoded.width(), decoded.height()) != actual {
        return Err("插件截图解码尺寸不一致".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    fn project() -> Project {
        Project {
            id: "example.region-agent".into(),
            version: "0.1.2".into(),
            package_sha256: "a".repeat(64),
            source_repository: "abnste/mewu-plugin-region-agent".into(),
            source_commit: "b".repeat(40),
            screenshots: vec![],
        }
    }
    fn shot(format: ImageFormat, extension: &str) -> (Screenshot, Vec<u8>) {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 24, Rgb([17, 29, 43])));
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, format).unwrap();
        let bytes = output.into_inner();
        let shot = Screenshot {
            path: format!("screenshots/result.{extension}"),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            size: bytes.len() as u64,
            width: 32,
            height: 24,
            caption: Some("实际运行截图".into()),
        };
        (shot, bytes)
    }
    #[test]
    fn raw_project_json_rejects_unknown_and_duplicate_fields() {
        let raw = serde_json::to_string(&project()).unwrap();
        assert_eq!(parse_project(raw.as_bytes()).unwrap(), project());
        for extra in [r#""version":"0.1.2""#, r#""url":"https://evil.invalid""#] {
            let text = format!("{{{extra},{}", &raw[1..]);
            assert!(parse_project(text.as_bytes()).is_err());
        }
        assert!(parse_projects(br#"{"schemaVersion":1,"schemaVersion":1,"projects":[]}"#).is_err());
        assert!(parse_project(&vec![b' '; MAX_PROJECT_BYTES + 1]).is_err());
        assert!(parse_projects(&vec![b' '; MAX_PROJECTS_BYTES + 1]).is_err());
    }
    #[test]
    fn exact_version_and_package_hash_bind_the_project() {
        let catalog = ProjectCatalog {
            schema_version: 1,
            projects: vec![project()],
        };
        let p = &catalog.projects[0];
        assert_eq!(
            find_project(&catalog, &p.id, &p.version, &p.package_sha256),
            Some(p)
        );
        assert!(find_project(&catalog, &p.id, "0.1.1", &p.package_sha256).is_none());
        assert!(find_project(&catalog, &p.id, &p.version, &"c".repeat(64)).is_none());
        assert!(find_project(&catalog, "different", &p.version, &p.package_sha256).is_none());
    }
    #[test]
    fn immutable_source_and_safe_paths_are_required() {
        for source in [
            "https://github.com/a/b",
            "a/..",
            "a/b.git",
            "a/b?x",
            "a/b/c",
        ] {
            let mut p = project();
            p.source_repository = source.into();
            assert!(validate_project(&p).is_err(), "{source}");
        }
        for commit in ["main".into(), "B".repeat(40), "a".repeat(39)] {
            let mut p = project();
            p.source_commit = commit;
            assert!(validate_project(&p).is_err());
        }
        let (s, _) = shot(ImageFormat::Png, "png");
        for path in [
            "../x.png",
            "/x.png",
            "screenshots//x.png",
            "screenshots/x%2F.png",
            "x\\y.png",
            "x.svg",
            "https://a/x.png",
        ] {
            let mut p = project();
            let mut s = s.clone();
            s.path = path.into();
            p.screenshots.push(s);
            assert!(validate_project(&p).is_err(), "{path}");
        }
        let mut p = project();
        p.screenshots.push(s);
        assert_eq!(image_url(&p, 0).unwrap(), format!("https://raw.githubusercontent.com/abnste/mewu-plugin-region-agent/{}/screenshots/result.png", "b".repeat(40)));
        assert!(image_url(&p, 1).is_err());
    }
    #[test]
    fn catalog_prevents_duplicate_versions_and_shared_projects() {
        let p = project();
        let mut catalog = ProjectCatalog {
            schema_version: 1,
            projects: vec![p.clone(), p.clone()],
        };
        assert!(parse_projects(&serde_json::to_vec(&catalog).unwrap()).is_err());
        catalog.projects[1].id = "example.other".into();
        assert!(parse_projects(&serde_json::to_vec(&catalog).unwrap()).is_err());
        catalog.projects[1].id = p.id.clone();
        catalog.projects[1].version = "0.1.3".into();
        assert!(parse_projects(&serde_json::to_vec(&catalog).unwrap()).is_ok());
        catalog.schema_version = 2;
        assert!(parse_projects(&serde_json::to_vec(&catalog).unwrap()).is_err());
    }
    #[test]
    fn gallery_metadata_bounds_are_checked_before_fetching() {
        let (s, _) = shot(ImageFormat::Png, "png");
        let mut p = project();
        p.screenshots = vec![s.clone(); MAX_SCREENSHOTS + 1];
        assert!(validate_project(&p).is_err());
        p.screenshots = vec![s.clone(), s.clone()];
        p.screenshots[1].path = s.path.to_uppercase();
        assert!(validate_project(&p).is_err());
        for (width, height) in [(15, 32), (4097, 16), (4096, 4096)] {
            let mut s = s.clone();
            s.width = width;
            s.height = height;
            assert!(validate_screenshot(&s).is_err());
        }
        for size in [0, MAX_SCREENSHOT_BYTES as u64 + 1] {
            let mut s = s.clone();
            s.size = size;
            assert!(validate_screenshot(&s).is_err());
        }
        let mut s = s.clone();
        s.caption = Some("bad\ncaption".into());
        assert!(validate_screenshot(&s).is_err());
    }
    #[test]
    fn real_png_jpeg_and_webp_are_fully_decoded() {
        for (format, extension) in [
            (ImageFormat::Png, "png"),
            (ImageFormat::Jpeg, "jpg"),
            (ImageFormat::WebP, "webp"),
        ] {
            let (s, bytes) = shot(format, extension);
            validate_image(&s, &bytes).unwrap();
            let mut mismatch = s.clone();
            mismatch.width += 1;
            assert!(validate_image(&mismatch, &bytes).is_err());
            let mut changed = bytes.clone();
            changed[0] ^= 1;
            assert!(validate_image(&s, &changed).is_err());
            let mut truncated = bytes[..bytes.len() / 2].to_vec();
            let mut bound = s.clone();
            bound.size = truncated.len() as u64;
            bound.sha256 = format!("{:x}", Sha256::digest(&truncated));
            assert!(validate_image(&bound, &truncated).is_err());
            truncated.clear();
        }
    }
    #[test]
    fn hash_correct_html_and_extension_spoofing_are_not_images() {
        let (mut s, bytes) = shot(ImageFormat::Png, "png");
        s.path = "screenshots/result.jpg".into();
        assert!(validate_image(&s, &bytes).is_err());
        let html = b"<html><script>alert(1)</script></html>";
        s.path = "screenshots/result.png".into();
        s.size = html.len() as u64;
        s.sha256 = format!("{:x}", Sha256::digest(html));
        assert!(validate_image(&s, html).is_err());
    }
    #[test]
    fn a_changed_feed_cannot_replace_an_image_under_displayed_metadata() {
        let (shot, _) = shot(ImageFormat::Png, "png");
        let mut p = project();
        p.screenshots.push(shot.clone());
        let commit = p.source_commit.clone();
        assert_eq!(
            image_for_snapshot(&p, 0, &commit, &shot.sha256).unwrap(),
            &shot
        );
        p.source_commit = "c".repeat(40);
        assert!(image_for_snapshot(&p, 0, &commit, &shot.sha256).is_err());
        p.source_commit = commit.clone();
        p.screenshots[0].sha256 = "d".repeat(64);
        assert!(image_for_snapshot(&p, 0, &commit, &shot.sha256).is_err());
        assert!(image_for_snapshot(&p, 1, &commit, &shot.sha256).is_err());
    }
}
