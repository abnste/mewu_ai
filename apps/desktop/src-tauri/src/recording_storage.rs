// SPDX-License-Identifier: MPL-2.0
//! Crash leftovers live separately from published assets. Call prepare only
//! after acquiring the application's single-instance ownership, with no live
//! recorder. The caller moves a finalized file into assets before publishing it.

use std::{
    fs,
    path::{Path, PathBuf},
};

const STAGING: &str = "recording-staging";

fn is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn ordinary_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法检查录制临时目录")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse(&metadata) {
        return Err("录制临时目录不能是链接或重解析点".into());
    }
    Ok(())
}

fn partial_name(path: &Path) -> bool {
    partial_name_for(path, false)
}
fn partial_name_for(path: &Path, video_export: bool) -> bool {
    if path
        .extension()
        .is_none_or(|ext| ext != "mp4" && !(video_export && ext == "gif"))
    {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let Ok(id) = uuid::Uuid::parse_str(stem) else {
        return false;
    };
    stem.eq_ignore_ascii_case(&id.hyphenated().to_string())
}

// Deny renaming/replacing the staging directory while cleanup is in progress.
// OPEN_REPARSE_POINT prevents following a link swapped in before opening it.
#[cfg(windows)]
fn pin_directory(path: &Path) -> Result<fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE, not DELETE
        .custom_flags(0x0200_0000 | 0x0020_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
        .open(path)
        .map_err(|_| "无法锁定录制临时目录")?;
    let metadata = file.metadata().map_err(|_| "无法检查录制临时目录句柄")?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err("录制临时目录不能是链接或重解析点".into());
    }
    Ok(file)
}

/// Creates root/recording-staging and removes only its direct, ordinary
/// canonical UUID.mp4 files. Never traverses subdirectories or touches assets.
pub fn prepare(root: &Path) -> Result<PathBuf, String> {
    prepare_named(root, STAGING)
}

/// Same direct UUID-file cleanup, isolated from any recording operation.
/// Called only at single-instance startup, before media workers can run.
pub fn prepare_video(root: &Path) -> Result<PathBuf, String> {
    prepare_named(root, "video-staging")
}

fn prepare_named(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("应用数据目录必须是绝对路径".into());
    }
    ordinary_directory(root)?;
    let canonical_root = fs::canonicalize(root).map_err(|_| "无法确定应用数据目录")?;
    #[cfg(windows)]
    let _root_pin = pin_directory(&canonical_root)?;
    let staging = canonical_root.join(name);
    match fs::create_dir(&staging) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("无法创建录制临时目录".into()),
    }
    ordinary_directory(&staging)?;
    #[cfg(windows)]
    let _staging_pin = pin_directory(&staging)?;
    let canonical_staging = fs::canonicalize(&staging).map_err(|_| "无法确定录制临时目录")?;
    if canonical_staging.parent() != Some(canonical_root.as_path())
        || canonical_staging.file_name() != Some(std::ffi::OsStr::new(name))
    {
        return Err("录制临时目录超出应用数据目录".into());
    }
    for entry in fs::read_dir(&canonical_staging).map_err(|_| "无法读取录制临时目录")? {
        let entry = entry.map_err(|_| "无法读取录制临时文件")?;
        let path = entry.path();
        let partial = if name == "video-staging" {
            partial_name_for(&path, true)
        } else {
            partial_name(&path)
        };
        if !partial {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| "无法检查录制临时文件")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse(&metadata) {
            continue;
        }
        let canonical_file = fs::canonicalize(&path).map_err(|_| "无法确定录制临时文件位置")?;
        if canonical_file.parent() != Some(canonical_staging.as_path())
            || canonical_file.file_name() != path.file_name()
        {
            return Err("录制临时文件超出清理范围".into());
        }
        // Recheck immediately before a non-recursive deletion. A directory or
        // reparse-point replacement is never treated as a normal recording.
        let metadata = fs::symlink_metadata(&path).map_err(|_| "无法再次检查录制临时文件")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse(&metadata) {
            continue;
        }
        fs::remove_file(&path).map_err(|_| "无法清理未完成的录制文件")?;
    }
    Ok(canonical_staging)
}

fn version_supported(major: u32, build: u32) -> bool {
    major > 10 || (major == 10 && build >= 19_041)
}

/// WDA_EXCLUDEFROMCAPTURE only excludes windows starting with Windows 10 2004.
/// Earlier versions can return success while substituting WDA_MONITOR (black).
/// https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
pub fn supported() -> Result<(), String> {
    #[cfg(windows)]
    {
        // Same ABI as OSVERSIONINFOW in Microsoft's windows-rs generated
        // ntdll!RtlGetVersion binding. Unlike GetVersionEx, query the native
        // version directly instead of relying on an application compatibility
        // manifest. Capture/encoder capability checks still happen at startup.
        // https://microsoft.github.io/windows-docs-rs/doc/windows/Wdk/System/SystemServices/fn.RtlGetVersion.html
        #[repr(C)]
        struct OsVersion {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            service_pack: [u16; 128],
        }
        #[link(name = "ntdll")]
        extern "system" {
            fn RtlGetVersion(version: *mut OsVersion) -> i32;
        }
        let mut version = OsVersion {
            size: std::mem::size_of::<OsVersion>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            service_pack: [0; 128],
        };
        if unsafe { RtlGetVersion(&mut version) } < 0 {
            return Err("无法确认 Windows 录制能力".into());
        }
        if !version_supported(version.major, version.build) {
            return Err("录屏需要 Windows 10 2004（19041）或更新版本".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("当前平台尚未实现原生录屏".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory {
        root: PathBuf,
        parent: PathBuf,
    }
    impl TestDirectory {
        fn new() -> Self {
            let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
            let root = parent.join(format!("mewu-recording-storage-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self { root, parent }
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            // Verify the absolute, unique test target before recursive cleanup.
            if let Ok(resolved) = fs::canonicalize(&self.root) {
                if resolved == self.root
                    && resolved.parent() == Some(self.parent.as_path())
                    && resolved.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with("mewu-recording-storage-")
                    })
                {
                    let _ = fs::remove_dir_all(&resolved);
                }
            }
        }
    }

    #[test]
    fn startup_only_removes_direct_uuid_recordings() {
        let fixture = TestDirectory::new();
        let staging = prepare(&fixture.root).unwrap();
        let uuid_name = format!("{}.mp4", uuid::Uuid::new_v4());
        fs::write(staging.join(&uuid_name), b"partial").unwrap();
        fs::write(staging.join("keep.mp4"), b"not a generated recording").unwrap();
        fs::write(staging.join("keep.txt"), b"unrelated").unwrap();
        fs::create_dir(staging.join("nested")).unwrap();
        fs::write(staging.join("nested").join(&uuid_name), b"nested").unwrap();
        fs::create_dir(fixture.root.join("assets")).unwrap();
        fs::write(fixture.root.join("assets").join(&uuid_name), b"published").unwrap();
        assert_eq!(prepare(&fixture.root).unwrap(), staging);
        assert!(!staging.join(&uuid_name).exists());
        assert!(staging.join("keep.mp4").exists());
        assert!(staging.join("keep.txt").exists());
        assert!(staging.join("nested").join(&uuid_name).exists());
        assert!(fixture.root.join("assets").join(&uuid_name).exists());
    }

    #[test]
    fn video_startup_cleanup_never_touches_recording_staging_or_other_files() {
        let fixture = TestDirectory::new();
        let recordings = prepare(&fixture.root).unwrap();
        let clips = prepare_video(&fixture.root).unwrap();
        let name = format!("{}.mp4", uuid::Uuid::new_v4());
        let gif_name = format!("{}.gif", uuid::Uuid::new_v4());
        fs::write(recordings.join(&name), b"recording").unwrap();
        fs::write(recordings.join(&gif_name), b"not a recording partial").unwrap();
        fs::write(clips.join(&name), b"unfinished clip").unwrap();
        fs::write(clips.join(&gif_name), b"unfinished gif").unwrap();
        fs::write(clips.join("keep.gif"), b"unrelated gif").unwrap();
        fs::write(clips.join("keep.mp4"), b"unrelated").unwrap();
        prepare_video(&fixture.root).unwrap();
        assert!(recordings.join(&name).exists());
        assert!(!clips.join(&name).exists());
        assert!(!clips.join(&gif_name).exists());
        assert!(clips.join("keep.gif").exists());
        prepare(&fixture.root).unwrap();
        assert!(recordings.join(&gif_name).exists());
        assert!(clips.join("keep.mp4").exists());
    }

    #[test]
    fn rejects_relative_root_and_staging_file() {
        assert!(prepare(Path::new("relative")).is_err());
        let fixture = TestDirectory::new();
        fs::write(fixture.root.join(STAGING), b"not a directory").unwrap();
        assert!(prepare(&fixture.root).is_err());
        assert_eq!(
            fs::read(fixture.root.join(STAGING)).unwrap(),
            b"not a directory"
        );
    }

    #[test]
    fn minimum_version_requires_real_exclusion_support() {
        assert!(!version_supported(6, 19_041));
        assert!(!version_supported(10, 18_362));
        assert!(version_supported(10, 19_041));
        assert!(version_supported(10, 26_100));
        assert!(version_supported(11, 1));
    }

    #[test]
    fn only_canonical_uuid_filenames_are_cleanup_candidates() {
        let id = uuid::Uuid::new_v4();
        assert!(partial_name(Path::new(&format!("{id}.mp4"))));
        assert!(!partial_name(Path::new(&format!("{}.mp4", id.simple()))));
        assert!(!partial_name(Path::new(&format!("{id}.png"))));
        assert!(!partial_name(Path::new("recording.mp4")));
        assert!(!partial_name(Path::new(&format!("{id}.gif"))));
        assert!(partial_name_for(Path::new(&format!("{id}.gif")), true));
        assert!(!partial_name_for(Path::new("keep.gif"), true));
    }
}
