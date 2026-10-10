// SPDX-License-Identifier: MPL-2.0
//! Persistent, independent video copies for CF_HDROP. Stable files never expire
//! or disappear on Drop. Only a definitive Unpublished receipt permits explicit
//! deletion; publication/close failures with an unknown outcome must retain them.
//! This is storage, not media validation: Host retains its verified SourceLease
//! or completed Precise render until copying and clipboard publication finish.

#[cfg(windows)]
use crate::table_staging::{mark_delete, pin_directory};
use crate::table_staging::{ordinary_directory, ordinary_file};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const DIRECTORY: &str = "clipboard-videos";
const CANCELED: &str = "视频操作已取消";
const FULL: &str = "视频复制缓存已满，请改用保存";
const BUSY: &str = "视频复制尚未结束";
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 128;
// Shared across independently opened storage handles. Permit moves into the
// actual blocking copy; cancellation of its caller cannot release it early.
static WRITER: AtomicBool = AtomicBool::new(false);

// Real storage/Host file tests use this same guard. Production still has one
// writer across all handles; test roots must not spuriously race that policy.
#[cfg(test)]
pub(crate) fn test_serial_guard() -> std::sync::MutexGuard<'static, ()> {
    static TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy)]
struct Limits {
    file: u64,
    total: u64,
    files: usize,
}
const LIMITS: Limits = Limits {
    file: MAX_FILE,
    total: MAX_TOTAL,
    files: MAX_FILES,
};

struct Permit;
impl Permit {
    fn acquire() -> Result<Self, String> {
        WRITER
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| BUSY.to_owned())?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        WRITER.store(false, Ordering::Release);
    }
}

struct Directory {
    path: PathBuf,
    #[cfg(windows)]
    _directory_pin: File,
    #[cfg(windows)]
    _root_pin: File,
}

pub(crate) struct ClipboardVideoStorage {
    directory: Arc<Directory>,
    limits: Limits,
}

/// Single-instance startup only, before accepting copy tasks. A missing cache
/// remains absent. Never deletes stable MP4s, unknown files or nested entries.
pub(crate) fn prepare(root: &Path) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = root;
        return Ok(());
    }
    #[cfg(windows)]
    {
        let _permit = Permit::acquire()?;
        if let Some(directory) = open_directory(root, false)? {
            cleanup_partials(&directory)?;
        }
        Ok(())
    }
}

impl ClipboardVideoStorage {
    /// First actual copy creates the directory; read-only startup does not.
    pub(crate) fn open(root: &Path) -> Result<Self, String> {
        Ok(Self {
            directory: Arc::new(open_directory(root, true)?.ok_or("无法创建视频复制目录")?),
            limits: LIMITS,
        })
    }

    /// Only call during single-instance startup, never during normal copying.
    pub(crate) fn cleanup_partials_at_startup(&self) -> Result<(), String> {
        let _permit = Permit::acquire()?;
        cleanup_partials(&self.directory)
    }

    /// Reserve a known, already validated final size. Trimmed media must finish
    /// and pass worker validation before this call; it is not rendered here.
    pub(crate) fn reserve(&self, length: u64) -> Result<Reservation, String> {
        let permit = Permit::acquire()?;
        validate_length(length, self.limits)?;
        check_budget(&self.directory.path, length, self.limits)?;
        Ok(Reservation {
            directory: Arc::clone(&self.directory),
            length,
            limits: self.limits,
            _permit: permit,
        })
    }
}

pub(crate) struct Reservation {
    directory: Arc<Directory>,
    length: u64,
    limits: Limits,
    _permit: Permit,
}

impl Reservation {
    /// Fixed-buffer physical copy, never a hard link to the user's scene asset.
    /// The source File must remain protected by Host's SourceLease. No source
    /// handle or path is exposed to the clipboard or renderer.
    pub(crate) fn copy_from(
        self,
        input: &mut File,
        cancel: &AtomicBool,
    ) -> Result<StableVideo, String> {
        check_cancel(cancel)?;
        let before = input.metadata().map_err(|_| "无法检查视频来源")?;
        if !ordinary_file(&before) || before.len() != self.length {
            return Err("视频来源已更改".into());
        }
        // Recheck after reservation, before creating anything, including any
        // external ordinary files added to the private directory in between.
        check_budget(&self.directory.path, self.length, self.limits)?;
        input
            .seek(SeekFrom::Start(0))
            .map_err(|_| "无法读取视频来源")?;
        let id = uuid::Uuid::new_v4().hyphenated().to_string();
        let path = self.directory.path.join(format!("{id}.partial.mp4"));
        let stable_path = self.directory.path.join(format!("{id}.mp4"));
        let mut partial = Partial {
            file: Some(create_partial(&path).map_err(|_| "无法准备视频副本")?),
            path,
            directory: Arc::clone(&self.directory),
            armed: true,
        };
        direct_file(&partial.path, &self.directory.path)?;
        let output = partial.file.as_mut().ok_or("无法准备视频副本")?;
        copy_exact(input, output, self.length, cancel)?;
        let after = input.metadata().map_err(|_| "无法再次检查视频来源")?;
        if !ordinary_file(&after)
            || after.len() != self.length
            || before.modified().ok() != after.modified().ok()
            || before.created().ok() != after.created().ok()
        {
            return Err("视频来源已更改".into());
        }
        output.flush().map_err(|_| "无法完成视频副本")?;
        output.sync_all().map_err(|_| "无法保存视频副本")?;
        if output.metadata().map_err(|_| "无法检查视频副本")?.len() != self.length {
            return Err("视频副本不完整".into());
        }
        check_cancel(cancel)?;

        // Keep an attributes-only identity handle across closing the writer and
        // opening the immutable read lease. Cloning the writer here would keep
        // GENERIC_WRITE alive and conflict with readers denying write sharing.
        let identity = identity_handle(&partial.path)?;
        if identity != handle(output)? {
            return Err("视频副本已更改".into());
        }
        direct_file(&partial.path, &self.directory.path)?;
        rename_new(&partial.path, &stable_path).map_err(|_| "无法保留视频副本")?;
        // From this point any unexpected failure is conservative retention.
        // In particular there is no Drop rollback around clipboard publication.
        partial.armed = false;
        drop(partial.file.take());
        let file = open_stable(&stable_path).map_err(|_| "无法读取视频副本")?;
        direct_file(&stable_path, &self.directory.path)?;
        if identity != handle(&file)?
            || file.metadata().map_err(|_| "无法检查视频副本")?.len() != self.length
        {
            return Err("视频副本已更改".into());
        }
        let stable = StableVideo {
            path: stable_path,
            file,
            identity,
            directory: Arc::clone(&self.directory),
        };
        // No publication has been attempted here, so a cancellation observed
        // after the rename is still a definitive unpublished copy.
        if cancel.load(Ordering::Acquire) {
            stable.discard_unpublished()?;
            return Err(CANCELED.into());
        }
        Ok(stable)
    }
}

/// Intentionally has NO Drop implementation deleting the path. Uncertain
/// publication, panic, normal close, shutdown and restart all keep this file.
pub(crate) struct StableVideo {
    path: PathBuf,
    file: File,
    identity: same_file::Handle,
    directory: Arc<Directory>,
}

impl StableVideo {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Caller must have a definitive Unpublished outcome. Never call this on a
    /// generic publication error, CloseClipboard failure, panic, or lost reply.
    pub(crate) fn discard_unpublished(self) -> Result<(), String> {
        let Self {
            path,
            file,
            identity,
            directory,
        } = self;
        // The attributes identity stays open, preventing file-ID reuse, while
        // the immutable read lease is released to obtain DELETE access.
        drop(file);
        delete_exact(&path, &directory.path, Some(&identity))
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELED.into())
    } else {
        Ok(())
    }
}

fn validate_length(length: u64, limits: Limits) -> Result<(), String> {
    if !(24..=limits.file).contains(&length) {
        Err("视频副本大小超出限制（最多 512 MiB）".into())
    } else {
        Ok(())
    }
}

fn copy_exact(
    input: &mut impl Read,
    output: &mut impl Write,
    length: u64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut buffer = [0u8; 64 * 1024];
    let mut left = length;
    while left > 0 {
        check_cancel(cancel)?;
        let size = usize::try_from(left.min(buffer.len() as u64)).map_err(|_| "视频大小无效")?;
        let count = input
            .read(&mut buffer[..size])
            .map_err(|_| "无法读取视频来源")?;
        if count == 0 {
            return Err("视频来源已更改".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|_| "无法写入视频副本")?;
        left -= count as u64;
    }
    check_cancel(cancel)?;
    if input
        .read(&mut buffer[..1])
        .map_err(|_| "无法检查视频来源结尾")?
        != 0
    {
        return Err("视频来源已更改".into());
    }
    Ok(())
}

fn direct_file(path: &Path, directory: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法检查视频副本")?;
    if !ordinary_file(&metadata) {
        return Err("视频复制目录含非普通文件".into());
    }
    let canonical = fs::canonicalize(path).map_err(|_| "无法确定视频副本位置")?;
    if canonical.parent() != Some(directory) || canonical.file_name() != path.file_name() {
        return Err("视频副本超出受控目录".into());
    }
    Ok(())
}

fn check_budget(directory: &Path, requested: u64, limits: Limits) -> Result<(), String> {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(directory).map_err(|_| "无法读取视频复制目录")? {
        let path = entry.map_err(|_| "无法读取视频副本记录")?.path();
        direct_file(&path, directory)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| "无法检查视频副本大小")?;
        count = count.checked_add(1).ok_or(FULL)?;
        bytes = bytes.checked_add(metadata.len()).ok_or(FULL)?;
        if count >= limits.files || bytes > limits.total.saturating_sub(requested) {
            return Err(FULL.into());
        }
    }
    if requested > limits.total || count >= limits.files {
        return Err(FULL.into());
    }
    Ok(())
}

fn open_directory(root: &Path, create: bool) -> Result<Option<Directory>, String> {
    #[cfg(not(windows))]
    {
        let _ = (root, create);
        return Err("视频文件复制暂仅支持 Windows".into());
    }
    #[cfg(windows)]
    {
        if !root.is_absolute() {
            return Err("应用数据目录必须是绝对路径".into());
        }
        ordinary_directory(root).map_err(|_| "应用数据目录不能是链接或重解析点")?;
        let root = fs::canonicalize(root).map_err(|_| "无法确定应用数据目录")?;
        let root_pin = pin_directory(&root).map_err(|_| "无法锁定应用数据目录")?;
        let path = root.join(DIRECTORY);
        match fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err("无法创建视频复制目录".into()),
            },
            Err(_) => return Err("无法检查视频复制目录".into()),
        }
        ordinary_directory(&path).map_err(|_| "视频复制目录不能是链接或重解析点")?;
        let directory_pin = pin_directory(&path).map_err(|_| "无法锁定视频复制目录")?;
        let path = fs::canonicalize(&path).map_err(|_| "无法确定视频复制目录")?;
        if path.parent() != Some(root.as_path()) || path.file_name().is_none_or(|s| s != DIRECTORY)
        {
            return Err("视频复制目录超出应用数据目录".into());
        }
        Ok(Some(Directory {
            path,
            _directory_pin: directory_pin,
            _root_pin: root_pin,
        }))
    }
}

fn partial_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let Some(id) = name.strip_suffix(".partial.mp4") else {
        return false;
    };
    uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.hyphenated().to_string() == id)
}

fn cleanup_partials(directory: &Directory) -> Result<(), String> {
    for (index, entry) in fs::read_dir(&directory.path)
        .map_err(|_| "无法读取视频复制目录")?
        .enumerate()
    {
        if index >= 4096 {
            return Err("视频复制目录条目过多，已停止清理".into());
        }
        let path = entry.map_err(|_| "无法读取视频副本记录")?.path();
        if !partial_name(&path) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| "无法检查临时视频副本")?;
        if !ordinary_file(&metadata) {
            continue;
        }
        delete_exact(&path, &directory.path, None)?;
    }
    Ok(())
}

fn handle(file: &File) -> Result<same_file::Handle, String> {
    same_file::Handle::from_file(file.try_clone().map_err(|_| "无法检查视频文件身份")?)
        .map_err(|_| "无法检查视频文件身份".into())
}

fn identity_handle(path: &Path) -> Result<same_file::Handle, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x80)
            .share_mode(1 | 2 | 4)
            .custom_flags(0x0020_0000);
    }
    let file = options.open(path).map_err(|_| "无法保留视频文件身份")?;
    if !ordinary_file(&file.metadata().map_err(|_| "无法检查视频副本")?) {
        return Err("视频副本不是普通文件".into());
    }
    same_file::Handle::from_file(file).map_err(|_| "无法保留视频文件身份".into())
}

fn create_partial(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
            .share_mode(1 | 4)
            .custom_flags(0x0020_0000);
    }
    options.open(path)
}

fn open_stable(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    options.open(path)
}

fn delete_exact(
    path: &Path,
    directory: &Path,
    expected: Option<&same_file::Handle>,
) -> Result<(), String> {
    direct_file(path, directory)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .access_mode(0x0001_0000 | 0x80)
            .share_mode(1)
            .custom_flags(0x0020_0000)
            .open(path)
            .map_err(|_| "无法清理未发布的视频副本")?;
        if !ordinary_file(&file.metadata().map_err(|_| "无法检查视频副本")?)
            || expected.is_some_and(|identity| handle(&file).as_ref() != Ok(identity))
        {
            return Err("视频副本已更改，未删除文件".into());
        }
        direct_file(path, directory)?;
        mark_delete(&file).map_err(|_| "无法清理未发布的视频副本")?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = expected;
        Err("视频文件复制暂仅支持 Windows".into())
    }
}

struct Partial {
    file: Option<File>,
    path: PathBuf,
    directory: Arc<Directory>,
    armed: bool,
}
impl Drop for Partial {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        #[cfg(windows)]
        if let Some(file) = &self.file {
            // Delete the held object, never an unverified replacement path.
            if direct_file(&self.path, &self.directory.path).is_ok()
                && identity_handle(&self.path)
                    .ok()
                    .zip(handle(file).ok())
                    .is_some_and(|(current, owned)| current == owned)
            {
                let _ = mark_delete(file);
            }
        }
    }
}

#[cfg(windows)]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // Unlike std::fs::rename on Windows, MoveFileW fails if the destination
    // exists. Both paths are direct children of our pinned canonical directory.
    // https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefilew
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileW(from: *const u16, to: *const u16) -> i32;
    }
    let wide = |p: &Path| -> io::Result<Vec<u16>> {
        let mut value = p.as_os_str().encode_wide().collect::<Vec<_>>();
        if value.contains(&0) || value.len() >= 32767 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid copy path",
            ));
        }
        value.push(0);
        Ok(value)
    };
    let from = wide(from)?;
    let to = wide(to)?;
    // SAFETY: two live, NUL-terminated UTF-16 buffers, no retained native pointer.
    if unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn rename_new(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Windows only"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn exact_copy_rejects_short_long_and_observes_cancel_between_chunks() {
        let cancel = AtomicBool::new(false);
        for bytes in [vec![3; 23], vec![3; 25]] {
            assert!(copy_exact(&mut Cursor::new(bytes), &mut Vec::new(), 24, &cancel).is_err());
        }
        struct CancelWriter<'a>(&'a AtomicBool, usize);
        impl Write for CancelWriter<'_> {
            fn write(&mut self, input: &[u8]) -> io::Result<usize> {
                self.1 += input.len();
                self.0.store(true, Ordering::Release);
                Ok(input.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut writer = CancelWriter(&cancel, 0);
        let result = copy_exact(
            &mut Cursor::new(vec![1; 128 * 1024]),
            &mut writer,
            128 * 1024,
            &cancel,
        );
        assert_eq!(result.unwrap_err(), CANCELED);
        assert_eq!(writer.1, 64 * 1024);
        assert!(validate_length(MAX_FILE + 1, LIMITS).is_err());
        assert!(validate_length(MAX_FILE, LIMITS).is_ok());
    }

    #[cfg(windows)]
    mod files {
        use super::*;
        struct Fixture {
            root: PathBuf,
            parent: PathBuf,
        }
        impl Fixture {
            fn new() -> Self {
                let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
                let root = parent.join(format!("mewu-video-copy-test-{}", uuid::Uuid::new_v4()));
                fs::create_dir(&root).unwrap();
                Self { root, parent }
            }
            fn input(&self, length: usize) -> File {
                let path = self.root.join("synthetic.mp4");
                fs::write(
                    &path,
                    (0..length).map(|n| (n % 251) as u8).collect::<Vec<_>>(),
                )
                .unwrap();
                File::open(path).unwrap()
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                if let Ok(path) = fs::canonicalize(&self.root) {
                    if path == self.root
                        && path.parent() == Some(self.parent.as_path())
                        && path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.starts_with("mewu-video-copy-test-"))
                    {
                        let _ = fs::remove_dir_all(path);
                    }
                }
            }
        }

        #[test]
        fn stable_survives_drop_restart_source_changes_and_partial_cleanup() {
            let _test = test_serial_guard();
            let f = Fixture::new();
            prepare(&f.root).unwrap();
            assert!(!f.root.join(DIRECTORY).exists());
            let storage = ClipboardVideoStorage::open(&f.root).unwrap();
            let mut input = f.input(100_000);
            let stable = storage
                .reserve(100_000)
                .unwrap()
                .copy_from(&mut input, &AtomicBool::new(false))
                .unwrap();
            let path = stable.path().to_owned();
            let original = fs::read(&path).unwrap();
            assert_eq!(original, fs::read(f.root.join("synthetic.mp4")).unwrap());
            assert!(!same_file::is_same_file(&path, f.root.join("synthetic.mp4")).unwrap());
            drop(stable);
            drop(input);
            fs::write(f.root.join("synthetic.mp4"), b"changed source").unwrap();
            let partial = storage
                .directory
                .path
                .join(format!("{}.partial.mp4", uuid::Uuid::new_v4()));
            fs::write(&partial, b"incomplete").unwrap();
            let unknown = storage.directory.path.join("keep.partial.mp4");
            fs::write(&unknown, b"unknown").unwrap();
            drop(storage);
            prepare(&f.root).unwrap();
            assert_eq!(fs::read(&path).unwrap(), original);
            assert!(!partial.exists());
            assert_eq!(fs::read(unknown).unwrap(), b"unknown");
        }

        #[test]
        fn unpublished_deletes_only_exact_identity_and_unknown_drop_keeps_file() {
            let _test = test_serial_guard();
            let f = Fixture::new();
            let storage = ClipboardVideoStorage::open(&f.root).unwrap();
            let mut input = f.input(80);
            let stable = storage
                .reserve(80)
                .unwrap()
                .copy_from(&mut input, &AtomicBool::new(false))
                .unwrap();
            let path = stable.path().to_owned();
            stable.discard_unpublished().unwrap();
            assert!(!path.exists());
            let stable = storage
                .reserve(80)
                .unwrap()
                .copy_from(&mut input, &AtomicBool::new(false))
                .unwrap();
            let StableVideo {
                path,
                file,
                identity,
                directory,
            } = stable;
            drop(file);
            let moved = directory.path.join("retained-original.mp4");
            fs::rename(&path, &moved).unwrap();
            fs::write(&path, b"replacement").unwrap();
            assert!(delete_exact(&path, &directory.path, Some(&identity)).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"replacement");
            assert_eq!(fs::metadata(&moved).unwrap().len(), 80);
        }

        #[test]
        fn quotas_include_unknown_files_and_reservation_permit_spans_real_worker() {
            let _test = test_serial_guard();
            let f = Fixture::new();
            let mut storage = ClipboardVideoStorage::open(&f.root).unwrap();
            storage.limits = Limits {
                file: 100,
                total: 160,
                files: 2,
            };
            fs::write(storage.directory.path.join("unknown.bin"), vec![9; 80]).unwrap();
            let reservation = storage.reserve(80).unwrap();
            assert!(storage.reserve(24).is_err());
            let (go, wait) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                wait.recv().unwrap();
                drop(reservation);
            });
            assert!(storage.reserve(24).is_err());
            go.send(()).unwrap();
            thread.join().unwrap();
            assert!(storage.reserve(81).is_err());
            let mut input = f.input(80);
            drop(
                storage
                    .reserve(80)
                    .unwrap()
                    .copy_from(&mut input, &AtomicBool::new(false))
                    .unwrap(),
            );
            assert!(storage.reserve(24).is_err());
            assert_eq!(
                fs::read(storage.directory.path.join("unknown.bin")).unwrap(),
                vec![9; 80]
            );
        }

        #[test]
        fn failed_and_cancelled_preparation_cleans_partial_without_touching_other_files() {
            let _test = test_serial_guard();
            let f = Fixture::new();
            let storage = ClipboardVideoStorage::open(&f.root).unwrap();
            let keep = storage.directory.path.join("keep.mp4");
            fs::write(&keep, b"keep").unwrap();
            let mut input = f.input(48);
            assert!(storage
                .reserve(49)
                .unwrap()
                .copy_from(&mut input, &AtomicBool::new(false))
                .is_err());
            assert_eq!(
                storage
                    .reserve(48)
                    .unwrap()
                    .copy_from(&mut input, &AtomicBool::new(true))
                    .err()
                    .unwrap(),
                CANCELED
            );
            let path = storage
                .directory
                .path
                .join(format!("{}.partial.mp4", uuid::Uuid::new_v4()));
            {
                let mut partial = Partial {
                    file: Some(create_partial(&path).unwrap()),
                    path: path.clone(),
                    directory: Arc::clone(&storage.directory),
                    armed: true,
                };
                partial
                    .file
                    .as_mut()
                    .unwrap()
                    .write_all(b"unfinished")
                    .unwrap();
            }
            assert!(!path.exists());
            assert_eq!(fs::read(keep).unwrap(), b"keep");
            assert_eq!(fs::read_dir(&storage.directory.path).unwrap().count(), 1);
        }

        #[test]
        fn startup_never_descends_and_existing_promotion_target_is_preserved() {
            let _test = test_serial_guard();
            let f = Fixture::new();
            let storage = ClipboardVideoStorage::open(&f.root).unwrap();
            let nested = storage
                .directory
                .path
                .join(format!("{}.partial.mp4", uuid::Uuid::new_v4()));
            fs::create_dir(&nested).unwrap();
            fs::write(nested.join("keep"), b"keep").unwrap();
            storage.cleanup_partials_at_startup().unwrap();
            assert_eq!(fs::read(nested.join("keep")).unwrap(), b"keep");
            assert!(storage.reserve(24).is_err());
            let source = f.root.join("partial.mp4");
            let target = f.root.join("stable.mp4");
            fs::write(&source, b"new").unwrap();
            fs::write(&target, b"old").unwrap();
            assert!(rename_new(&source, &target).is_err());
            assert_eq!(fs::read(target).unwrap(), b"old");
            assert_eq!(fs::read(source).unwrap(), b"new");
        }
    }
}
