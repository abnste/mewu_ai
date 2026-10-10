// SPDX-License-Identifier: MPL-2.0
//! Shared PNG/JPEG encoding and same-directory publication.
//! Host owns authorization, source leases, protected-root checks, cancellation
//! routing and actual worker permits. No App/Store/IPC is accessed here.
use image::{GenericImageView, ImageEncoder, Rgb, RgbaImage};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_ENCODED_BYTES: u64 = 128 * 1024 * 1024;
pub const JPEG_QUALITY: u8 = 92;
pub const CANCELED: &str = "图片保存已取消";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageSaveFormat {
    Png,
    Jpeg,
}
impl ImageSaveFormat {
    pub fn accepts_extension(self, extension: &str) -> bool {
        match self {
            Self::Png => extension.eq_ignore_ascii_case("png"),
            Self::Jpeg => ["jpg", "jpeg"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate)),
        }
    }
}

/// Canonical destination and its directory lease. This does NOT grant source
/// access or approve exporting into any application-private directory.
pub struct ImageDestination {
    path: PathBuf,
    format: ImageSaveFormat,
    _directory: File,
}
impl ImageDestination {
    pub fn prepare(path: &Path, format: ImageSaveFormat) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("保存路径无效".into());
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("保存文件名无效")?;
        if name.is_empty()
            || name.ends_with(['.', ' '])
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, ':' | '/' | '\\'))
            || !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| format.accepts_extension(e))
        {
            return Err("文件扩展名与所选图片格式不一致".into());
        }
        let parent = path
            .parent()
            .ok_or("保存目录无效")?
            .canonicalize()
            .map_err(|_| "保存目录不可用")?;
        let directory = open_directory(&parent)?;
        let destination = parent.join(name);
        check_destination(&destination)?;
        Ok(Self {
            path: destination,
            format,
            _directory: directory,
        })
    }
    /// Host uses this canonical path for private-root/source protection both
    /// before encoding and inside its final short commit gate.
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn format(&self) -> ImageSaveFormat {
        self.format
    }
}

fn not_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}
fn open_directory(path: &Path) -> Result<File, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "保存目录不可用")?;
    if !metadata.is_dir() || !not_reparse(&metadata) {
        return Err("保存目录无效".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Read/write sharing, no delete sharing; backup semantics + open
        // reparse point. Keep the parent alive through actual publication.
        options
            .share_mode(3)
            .custom_flags(0x0200_0000 | 0x0020_0000);
    }
    let directory = options.open(path).map_err(|_| "无法锁定保存目录")?;
    let opened = directory.metadata().map_err(|_| "无法检查保存目录")?;
    if !opened.is_dir() || !not_reparse(&opened) {
        return Err("保存目录无效".into());
    }
    Ok(directory)
}
fn check_destination(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() || !not_reparse(&meta) => Err("保存目标不是普通文件".into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("无法检查保存目标".into()),
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELED.into())
    } else {
        Ok(())
    }
}
fn validate_image(image: &RgbaImage) -> Result<(), String> {
    let (width, height) = image.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || pixels > MAX_PIXELS
        || image.as_raw().len() as u64 != pixels * 4
    {
        return Err("图片超过 32 Mi 像素或 16384 像素边长".into());
    }
    Ok(())
}

/// A borrowed straight-RGB view: discard alpha without matting or premultiply.
/// JpegEncoder::encode_image processes 8x8 blocks, avoiding a second full RGB
/// allocation. The host must supply straight RGBA, not premultiplied BGRA.
struct StraightRgb<'a>(&'a RgbaImage);
impl GenericImageView for StraightRgb<'_> {
    type Pixel = Rgb<u8>;
    fn dimensions(&self) -> (u32, u32) {
        self.0.dimensions()
    }
    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let [r, g, b, _] = self.0.get_pixel(x, y).0;
        Rgb([r, g, b])
    }
}
struct Limited<'a, W> {
    writer: W,
    remaining: u64,
    cancel: &'a AtomicBool,
}
impl<W: Write> Write for Limited<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Acquire) {
            // write_all retries Interrupted, so cancellation must not use it.
            return Err(io::Error::other("cancelled"));
        }
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::other("image output budget"));
        }
        let written = self.writer.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(io::Error::other("cancelled"));
        }
        self.writer.flush()
    }
}
fn encode_to<W: Write>(
    format: ImageSaveFormat,
    image: &RgbaImage,
    writer: W,
    cancel: &AtomicBool,
    byte_limit: u64,
) -> Result<(), String> {
    check_cancel(cancel)?;
    validate_image(image)?;
    let mut writer = Limited {
        writer,
        remaining: byte_limit,
        cancel,
    };
    let result = match format {
        ImageSaveFormat::Png => image::codecs::png::PngEncoder::new(&mut writer).write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        ),
        ImageSaveFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, JPEG_QUALITY)
                .encode_image(&StraightRgb(image))
        }
    };
    check_cancel(cancel)?;
    result.map_err(|_| "图片编码失败或超过 128 MiB")?;
    writer.flush().map_err(|_| "无法写入完整图片")?;
    check_cancel(cancel)
}

/// Encode into an already-owned temporary file. This flushes but does not sync
/// or publish; caller retains file ownership even on failure.
pub fn encode_image(
    format: ImageSaveFormat,
    image: &RgbaImage,
    file: &mut File,
    cancel: &AtomicBool,
) -> Result<(), String> {
    encode_to(
        format,
        image,
        BufWriter::with_capacity(64 * 1024, file),
        cancel,
        MAX_ENCODED_BYTES,
    )
}

struct PendingFile {
    path: PathBuf,
    file: Option<File>,
}
impl Drop for PendingFile {
    fn drop(&mut self) {
        // Windows will not delete an open file without delete sharing.
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}
fn save_with_encoder(
    destination: &ImageDestination,
    cancel: &AtomicBool,
    encode: impl FnOnce(&mut File) -> Result<(), String>,
    commit: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    check_cancel(cancel)?;
    check_destination(destination.path())?;
    let parent = destination.path.parent().ok_or("保存目录无效")?;
    let temporary = parent.join(format!(".mewu-image-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options
        .open(&temporary)
        .map_err(|_| "无法创建图片临时文件")?;
    // Do not arm cleanup until create_new succeeds: a UUID collision must not
    // remove somebody else's file.
    let mut pending = PendingFile {
        path: temporary,
        file: Some(file),
    };
    let file = pending.file.as_mut().ok_or("图片临时文件不可用")?;
    encode(file)?;
    check_cancel(cancel)?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法保存完整图片")?;
    drop(pending.file.take());
    check_cancel(cancel)?;
    check_destination(destination.path())?;
    // Exactly one synchronous host callback. It must recheck owner/source/exit
    // and protected path, then rename THESE paths under its short commit gate.
    // Successful rename is final: do not recheck cancellation after publication.
    commit(&pending.path, destination.path())
}

/// Same-directory create_new -> encode -> flush/sync -> close -> host commit.
/// Any prepublication failure removes only our UUID temporary; no destination
/// file is opened for truncation. No permit is created/released by this helper.
pub fn save_image_atomic(
    format: ImageSaveFormat,
    image: &RgbaImage,
    destination: &ImageDestination,
    cancel: &AtomicBool,
    commit: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    if format != destination.format {
        return Err("保存格式已变化".into());
    }
    check_cancel(cancel)?;
    validate_image(image)?;
    save_with_encoder(
        destination,
        cancel,
        |file| encode_image(format, image, file, cancel),
        commit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestRoot {
        path: PathBuf,
        parent: PathBuf,
        leaf: String,
    }
    impl TestRoot {
        fn new() -> Self {
            let parent = std::env::temp_dir().canonicalize().unwrap();
            let leaf = format!("mewu-image-test-{}", uuid::Uuid::new_v4());
            let path = parent.join(&leaf);
            fs::create_dir(&path).unwrap();
            assert_eq!(path.canonicalize().unwrap(), path);
            Self { path, parent, leaf }
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            if let Ok(resolved) = self.path.canonicalize() {
                if resolved == self.parent.join(&self.leaf)
                    && resolved.parent() == Some(self.parent.as_path())
                    && self
                        .leaf
                        .strip_prefix("mewu-image-test-")
                        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
                {
                    let _ = fs::remove_dir_all(resolved);
                }
            }
        }
    }
    fn publish(temp: &Path, dest: &Path) -> Result<(), String> {
        fs::rename(temp, dest).map_err(|_| "rename failed".into())
    }
    #[test]
    fn png_keeps_exact_alpha_and_jpeg_drops_alpha_without_full_rgb_copy() {
        let root = TestRoot::new();
        let pixels = RgbaImage::from_fn(128, 32, |x, _| {
            image::Rgba(match x / 32 {
                0 => [255, 0, 0, 0],
                1 => [255, 0, 0, 128],
                2 => [255, 0, 0, 255],
                _ => [255, 255, 255, 0],
            })
        });
        let original = pixels.clone();
        for (format, name) in [
            (ImageSaveFormat::Png, "透明.png"),
            (ImageSaveFormat::Jpeg, "透明.JPEG"),
        ] {
            let dest = ImageDestination::prepare(&root.path.join(name), format).unwrap();
            save_image_atomic(format, &pixels, &dest, &AtomicBool::new(false), publish).unwrap();
            let bytes = fs::read(dest.path()).unwrap();
            let expected = match format {
                ImageSaveFormat::Png => image::ImageFormat::Png,
                ImageSaveFormat::Jpeg => image::ImageFormat::Jpeg,
            };
            assert_eq!(image::guess_format(&bytes).unwrap(), expected);
            let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
            assert_eq!(decoded.dimensions(), pixels.dimensions());
            if format == ImageSaveFormat::Png {
                assert_eq!(decoded, pixels);
            } else {
                for x in [16, 48, 80, 112] {
                    let actual = decoded.get_pixel(x, 16).0;
                    let source = pixels.get_pixel(x, 16).0;
                    for channel in 0..3 {
                        assert!(actual[channel].abs_diff(source[channel]) <= 5);
                    }
                    assert_eq!(actual[3], 255);
                }
            }
        }
        assert_eq!(pixels, original);
    }
    #[test]
    fn encoding_failure_cancel_and_denied_commit_preserve_original_and_cleanup() {
        let root = TestRoot::new();
        let path = root.path.join("existing.png");
        fs::write(&path, b"old bytes").unwrap();
        let dest = ImageDestination::prepare(&path, ImageSaveFormat::Png).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(save_with_encoder(
            &dest,
            &cancel,
            |file| {
                file.write_all(b"partial").unwrap();
                Err("encode failed".into())
            },
            |_, _| panic!("must not publish")
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old bytes");
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1);
        assert!(save_with_encoder(
            &dest,
            &cancel,
            |file| {
                file.write_all(b"partial").unwrap();
                cancel.store(true, Ordering::Release);
                Ok(())
            },
            |_, _| panic!("must not publish")
        )
        .is_err());
        cancel.store(false, Ordering::Release);
        let pixels = RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 4]));
        assert!(
            save_image_atomic(ImageSaveFormat::Png, &pixels, &dest, &cancel, |_, _| Err(
                "owner changed".into()
            ))
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"old bytes");
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1);
        save_image_atomic(
            ImageSaveFormat::Png,
            &pixels,
            &dest,
            &cancel,
            |temp, target| {
                publish(temp, target)?;
                cancel.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(image::open(&path).unwrap().into_rgba8(), pixels);
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1);
    }
    #[test]
    fn writer_enforces_budget_midstream_failure_and_flush_failure() {
        struct Fails {
            left: usize,
            fail_flush: bool,
        }
        impl Write for Fails {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.left == 0 {
                    return Err(io::Error::other("disk fault"));
                }
                let count = self.left.min(bytes.len());
                self.left -= count;
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                if self.fail_flush {
                    Err(io::Error::other("flush fault"))
                } else {
                    Ok(())
                }
            }
        }
        let pixels = RgbaImage::from_pixel(64, 64, image::Rgba([20, 40, 60, 80]));
        for format in [ImageSaveFormat::Png, ImageSaveFormat::Jpeg] {
            let cancel = AtomicBool::new(false);
            assert!(encode_to(format, &pixels, Vec::new(), &cancel, 16).is_err());
            assert!(encode_to(
                format,
                &pixels,
                Fails {
                    left: 32,
                    fail_flush: false
                },
                &cancel,
                MAX_ENCODED_BYTES
            )
            .is_err());
            assert!(encode_to(
                format,
                &pixels,
                Fails {
                    left: usize::MAX,
                    fail_flush: true
                },
                &cancel,
                MAX_ENCODED_BYTES
            )
            .is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn real_rename_failure_keeps_locked_destination_and_removes_own_temp() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = TestRoot::new();
        let path = root.path.join("existing.jpg");
        fs::write(&path, b"keep original").unwrap();
        let dest = ImageDestination::prepare(&path, ImageSaveFormat::Jpeg).unwrap();
        let _block = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let pixels = RgbaImage::from_pixel(8, 8, image::Rgba([12, 34, 56, 255]));
        assert!(save_image_atomic(
            ImageSaveFormat::Jpeg,
            &pixels,
            &dest,
            &AtomicBool::new(false),
            publish
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"keep original");
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1);
    }
}
