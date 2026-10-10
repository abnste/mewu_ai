// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0

//! Durable, bounded PNGs used by CF_HDROP. Do not remove a successful staging file
//! on application exit or clipboard publication failure. Only direct ordinary
//! lowercase UUID.png files at least seven days old are eligible for cleanup.

use image::{ImageEncoder, RgbaImage};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

const DIRECTORY: &str = "clipboard-staging";
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
static STAGING_WRITER: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
struct Limits {
    total_bytes: u64,
    files: usize,
    png_bytes: u64,
}

const LIMITS: Limits = Limits {
    total_bytes: 512 * 1024 * 1024,
    files: 256,
    png_bytes: 128 * 1024 * 1024,
};

fn is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn ordinary_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

pub(crate) fn ordinary_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法检查剪贴板图片目录")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse(&metadata) {
        return Err("剪贴板图片目录不能是链接或重解析点".into());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn pin_directory(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1 | 2) // READ/WRITE, no DELETE: prevent directory replacement.
        .custom_flags(0x0200_0000 | 0x0020_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
        .open(path)
        .map_err(|_| "无法锁定剪贴板图片目录")?;
    let metadata = file.metadata().map_err(|_| "无法检查剪贴板图片目录句柄")?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err("剪贴板图片目录不能是链接或重解析点".into());
    }
    Ok(file)
}

fn generated_name(path: &Path) -> bool {
    if path.extension().is_none_or(|extension| extension != "png") {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    uuid::Uuid::parse_str(stem).is_ok_and(|id| stem == id.hyphenated().to_string())
}

fn old_enough(metadata: &fs::Metadata, now: SystemTime) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= RETENTION)
}

fn direct_canonical_file(path: &Path, directory: &Path) -> Result<(), String> {
    let canonical = fs::canonicalize(path).map_err(|_| "无法确定剪贴板图片位置")?;
    if canonical.parent() != Some(directory) || canonical.file_name() != path.file_name() {
        return Err("剪贴板图片超出受控目录".into());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn mark_delete(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    // Official FILE_DISPOSITION_INFO has a one-byte BOOLEAN (not Win32 BOOL).
    // https://learn.microsoft.com/windows/win32/api/winbase/ns-winbase-file_disposition_info
    // https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            file: *mut std::ffi::c_void,
            class: i32,
            information: *const std::ffi::c_void,
            bytes: u32,
        ) -> i32;
    }
    let delete: u8 = 1;
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            4, // FileDispositionInfo
            (&delete as *const u8).cast(),
            1,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn clean_old(directory: &Path, now: SystemTime) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|_| "无法读取剪贴板图片目录")? {
        let entry = entry.map_err(|_| "无法读取剪贴板图片记录")?;
        let path = entry.path();
        if !generated_name(&path) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| "无法检查剪贴板图片")?;
        if !ordinary_file(&metadata) || !old_enough(&metadata, now) {
            continue;
        }
        direct_canonical_file(&path, directory)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // The final component is opened without following a reparse point.
            // No WRITE/DELETE sharing: its age and identity cannot change between
            // validating this handle and deleting THIS file via the same handle.
            let file = OpenOptions::new()
                .access_mode(0x0001_0000 | 0x80) // DELETE | FILE_READ_ATTRIBUTES
                .share_mode(1)
                .custom_flags(0x0020_0000) // OPEN_REPARSE_POINT
                .open(&path)
                .map_err(|_| "无法清理已过期的剪贴板图片，请稍后重试")?;
            let metadata = file.metadata().map_err(|_| "无法再次检查剪贴板图片")?;
            if !ordinary_file(&metadata) || !old_enough(&metadata, now) {
                continue;
            }
            direct_canonical_file(&path, directory)?;
            mark_delete(&file).map_err(|_| "无法清理已过期的剪贴板图片")?;
            // Closing the validated handle commits this non-recursive deletion.
        }
        #[cfg(not(windows))]
        {
            let metadata = fs::symlink_metadata(&path).map_err(|_| "无法再次检查剪贴板图片")?;
            if ordinary_file(&metadata) && old_enough(&metadata, now) {
                direct_canonical_file(&path, directory)?;
                fs::remove_file(&path).map_err(|_| "无法清理已过期的剪贴板图片")?;
            }
        }
    }
    Ok(())
}

fn inventory(directory: &Path, limits: Limits) -> Result<(usize, u64), String> {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(directory).map_err(|_| "无法读取剪贴板图片目录")? {
        let path = entry.map_err(|_| "无法读取剪贴板图片记录")?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| "无法检查剪贴板图片大小")?;
        // Unknown regular files remain untouched, but consume the same budget.
        // Never traverse a directory/link to calculate a misleading partial quota.
        if !ordinary_file(&metadata) {
            return Err("剪贴板图片目录含非普通文件，已停止写入且未删除该项".into());
        }
        direct_canonical_file(&path, directory)?;
        count = count.checked_add(1).ok_or("剪贴板图片数量溢出")?;
        bytes = bytes
            .checked_add(metadata.len())
            .ok_or("剪贴板图片容量溢出")?;
        if count >= limits.files || bytes >= limits.total_bytes {
            return Err(
                "剪贴板图片暂存已满（最多 256 个文件 / 512 MiB），最近 7 天的图片不会自动删除"
                    .into(),
            );
        }
    }
    Ok((count, bytes))
}

struct BoundedWriter<'a> {
    file: &'a mut File,
    bytes: u64,
    maximum: u64,
    exceeded: bool,
}

impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.maximum.saturating_sub(self.bytes) {
            self.exceeded = true;
            return Err(io::Error::other("table PNG exceeds staging budget"));
        }
        let count = self.file.write(bytes)?;
        self.bytes += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn new_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x4000_0000 | 0x0001_0000 | 0x80) // GENERIC_WRITE | DELETE | attributes
            .share_mode(1) // No replacement or concurrent writes while encoding.
            .custom_flags(0x0020_0000);
    }
    options.open(path)
}

/// Caller must supply the existing, absolute application data root. Successful
/// files are durable before being returned, and remain available after app exit.
pub fn stage(root: &Path, image: &RgbaImage) -> Result<PathBuf, String> {
    let _writer = STAGING_WRITER
        .try_lock()
        .map_err(|_| "已有表格图片正在准备，请稍后再试")?;
    stage_at(root, image, SystemTime::now(), LIMITS)
}

fn stage_at(
    root: &Path,
    image: &RgbaImage,
    now: SystemTime,
    limits: Limits,
) -> Result<PathBuf, String> {
    let (width, height) = image.dimensions();
    if width == 0
        || height == 0
        || width > 16_384
        || height > 16_384
        || u64::from(width) * u64::from(height) > 32 * 1024 * 1024
    {
        return Err("表格图片尺寸超过暂存限制".into());
    }
    if !root.is_absolute() {
        return Err("应用数据目录必须是绝对路径".into());
    }
    ordinary_directory(root)?;
    let root = fs::canonicalize(root).map_err(|_| "无法确定应用数据目录")?;
    #[cfg(windows)]
    let _root_pin = pin_directory(&root)?;
    let directory = root.join(DIRECTORY);
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("无法创建剪贴板图片目录".into()),
    }
    ordinary_directory(&directory)?;
    #[cfg(windows)]
    let _directory_pin = pin_directory(&directory)?;
    let directory = fs::canonicalize(&directory).map_err(|_| "无法确定剪贴板图片目录")?;
    if directory.parent() != Some(root.as_path())
        || directory.file_name() != Some(std::ffi::OsStr::new(DIRECTORY))
    {
        return Err("剪贴板图片目录超出应用数据目录".into());
    }
    clean_old(&directory, now)?;
    let (_, occupied) = inventory(&directory, limits)?;
    let available = limits.png_bytes.min(limits.total_bytes - occupied);
    let path = directory.join(format!("{}.png", uuid::Uuid::new_v4()));
    let mut file = new_file(&path).map_err(|_| "无法创建剪贴板图片")?;
    let result = (|| {
        direct_canonical_file(&path, &directory)?;
        let mut writer = BoundedWriter {
            file: &mut file,
            bytes: 0,
            maximum: available,
            exceeded: false,
        };
        let encoded = image::codecs::png::PngEncoder::new(&mut writer).write_image(
            image.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        );
        if writer.exceeded {
            return Err("表格 PNG 超过单文件 128 MiB 或暂存总容量限制".into());
        }
        encoded.map_err(|_| "编码表格 PNG 失败")?;
        writer.flush().map_err(|_| "写入表格 PNG 失败")?;
        file.sync_all().map_err(|_| "保存表格 PNG 失败")?;
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        #[cfg(windows)]
        let cleanup = mark_delete(&file);
        #[cfg(not(windows))]
        let cleanup = fs::remove_file(&path);
        drop(file);
        return if cleanup.is_ok() {
            Err(error)
        } else {
            Err(format!("{error}；未完成的图片将在安全期限后清理"))
        };
    }
    drop(file);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        parent: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
            let root = parent.join(format!("mewu-table-staging-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self { root, parent }
        }

        fn directory(&self) -> PathBuf {
            let directory = self.root.join(DIRECTORY);
            if !directory.exists() {
                fs::create_dir(&directory).unwrap();
            }
            directory
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only remove this uniquely created test root, after checking resolution.
            if let Ok(canonical) = fs::canonicalize(&self.root) {
                if canonical == self.root
                    && canonical.parent() == Some(self.parent.as_path())
                    && canonical.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with("mewu-table-staging-test-")
                    })
                {
                    let _ = fs::remove_dir_all(canonical);
                }
            }
        }
    }

    fn image() -> RgbaImage {
        RgbaImage::from_pixel(4, 3, image::Rgba([30, 60, 90, 255]))
    }

    fn dated_file(directory: &Path, name: &str, time: SystemTime) -> PathBuf {
        let path = directory.join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(b"fixture").unwrap();
        file.set_modified(time).unwrap();
        path
    }

    #[test]
    fn durable_png_and_fresh_files_survive_subsequent_staging() {
        let fixture = Fixture::new();
        let now = SystemTime::now();
        let first = stage_at(&fixture.root, &image(), now, LIMITS).unwrap();
        assert!(generated_name(&first));
        let second = stage_at(&fixture.root, &image(), now, LIMITS).unwrap();
        assert_ne!(first, second);
        assert_eq!(image::open(&first).unwrap().to_rgba8(), image());
        assert_eq!(image::open(&second).unwrap().to_rgba8(), image());
    }

    #[test]
    fn cleanup_only_deletes_old_direct_canonical_uuid_pngs() {
        let fixture = Fixture::new();
        let directory = fixture.directory();
        let now = SystemTime::now();
        let old = now - RETENTION - Duration::from_secs(1);
        let owned = dated_file(&directory, &format!("{}.png", uuid::Uuid::new_v4()), old);
        let recent = dated_file(
            &directory,
            &format!("{}.png", uuid::Uuid::new_v4()),
            now - RETENTION + Duration::from_secs(60),
        );
        let future = dated_file(
            &directory,
            &format!("{}.png", uuid::Uuid::new_v4()),
            now + Duration::from_secs(60),
        );
        let unknown = dated_file(&directory, "keep.png", old);
        let wrong_extension = dated_file(&directory, &format!("{}.txt", uuid::Uuid::new_v4()), old);
        let uppercase = dated_file(&directory, "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA.png", old);
        let nested = directory.join("nested");
        fs::create_dir(&nested).unwrap();
        let nested_file = dated_file(&nested, &format!("{}.png", uuid::Uuid::new_v4()), old);
        let assets = fixture.root.join("assets");
        fs::create_dir(&assets).unwrap();
        let asset = dated_file(&assets, &format!("{}.png", uuid::Uuid::new_v4()), old);
        clean_old(&directory, now).unwrap();
        assert!(!owned.exists());
        for path in [
            recent,
            future,
            unknown,
            wrong_extension,
            uppercase,
            nested_file,
            asset,
        ] {
            assert!(path.exists());
        }
        // Unexpected directories are not traversed or removed to make quota room.
        assert!(inventory(&directory, LIMITS).is_err());
    }

    #[test]
    fn unknown_regular_files_are_retained_and_count_against_budget() {
        let fixture = Fixture::new();
        let directory = fixture.directory();
        let unknown = dated_file(&directory, "keep.data", SystemTime::now() - RETENTION * 2);
        let limits = Limits {
            files: 2,
            total_bytes: 2048,
            png_bytes: 1024,
        };
        let path = stage_at(&fixture.root, &image(), SystemTime::now(), limits).unwrap();
        assert!(unknown.exists());
        assert!(path.exists());
        assert!(stage_at(&fixture.root, &image(), SystemTime::now(), limits).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
    }

    #[test]
    fn exceeded_total_or_png_budget_removes_only_its_incomplete_output() {
        let fixture = Fixture::new();
        let directory = fixture.directory();
        let keep = dated_file(&directory, "keep.data", SystemTime::now());
        let limits = Limits {
            files: 4,
            total_bytes: 16,
            png_bytes: 1024,
        };
        assert!(stage_at(&fixture.root, &image(), SystemTime::now(), limits).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert!(keep.exists());
        let limits = Limits {
            files: 4,
            total_bytes: 1024,
            png_bytes: 16,
        };
        assert!(stage_at(&fixture.root, &image(), SystemTime::now(), limits).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    }

    #[test]
    fn rejects_relative_root_file_in_place_of_directory_and_bad_dimensions() {
        assert!(stage_at(Path::new("relative"), &image(), SystemTime::now(), LIMITS).is_err());
        let fixture = Fixture::new();
        fs::write(fixture.root.join(DIRECTORY), b"not a directory").unwrap();
        assert!(stage_at(&fixture.root, &image(), SystemTime::now(), LIMITS).is_err());
        assert_eq!(
            fs::read(fixture.root.join(DIRECTORY)).unwrap(),
            b"not a directory"
        );
        assert!(stage_at(
            &fixture.root,
            &RgbaImage::new(0, 1),
            SystemTime::now(),
            LIMITS
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_pins_prevent_replacement_during_cleanup() {
        let fixture = Fixture::new();
        let directory = fixture.directory();
        let pin = pin_directory(&directory).unwrap();
        assert!(fs::rename(&directory, fixture.root.join("replacement")).is_err());
        drop(pin);
        assert!(directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_directory_or_file_is_never_followed_for_cleanup() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = fixture.root.join("outside");
        fs::create_dir(&outside).unwrap();
        let saved = dated_file(
            &outside,
            &format!("{}.png", uuid::Uuid::new_v4()),
            SystemTime::now() - RETENTION * 2,
        );
        symlink(&outside, fixture.root.join(DIRECTORY)).unwrap();
        assert!(stage_at(&fixture.root, &image(), SystemTime::now(), LIMITS).is_err());
        assert!(saved.exists());
        fs::remove_file(fixture.root.join(DIRECTORY)).unwrap();
        let directory = fixture.directory();
        symlink(
            &saved,
            directory.join(format!("{}.png", uuid::Uuid::new_v4())),
        )
        .unwrap();
        assert!(stage_at(&fixture.root, &image(), SystemTime::now(), LIMITS).is_err());
        assert!(saved.exists());
    }
}
