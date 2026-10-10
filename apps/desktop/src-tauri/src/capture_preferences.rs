// SPDX-License-Identifier: MPL-2.0
//! Host-owned screenshot preferences.
//! A dedicated file/actor; never writes shortcut or recording preferences.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const MAX_BYTES: usize = 4096;
const MAX_REVISION: u64 = 9_007_199_254_740_991;
const FILE_NAME: &str = "capture-preferences.json";
const LOCK_NAME: &str = "capture-preferences.lock";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "u8", into = "u8")]
pub struct CaptureDelay(u8);
impl TryFrom<u8> for CaptureDelay {
    type Error = &'static str;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 | 3 | 5 => Ok(Self(value)),
            _ => Err("截图延迟只能为 0、3 或 5 秒"),
        }
    }
}
impl From<CaptureDelay> for u8 {
    fn from(value: CaptureDelay) -> Self {
        value.0
    }
}
impl CaptureDelay {
    pub fn seconds(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub enum DefaultImageFormat {
    Png,
    Jpeg,
}
impl TryFrom<String> for DefaultImageFormat {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "png" => Ok(Self::Png),
            "jpeg" => Ok(Self::Jpeg),
            _ => Err("不支持此图片格式"),
        }
    }
}
impl From<DefaultImageFormat> for String {
    fn from(value: DefaultImageFormat) -> Self {
        match value {
            DefaultImageFormat::Png => "png",
            DefaultImageFormat::Jpeg => "jpeg",
        }
        .into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapturePreferences {
    pub version: u32,
    pub revision: u64,
    pub capture_delay_seconds: CaptureDelay,
    pub include_cursor: bool,
    pub default_image_format: DefaultImageFormat,
}
impl CapturePreferences {
    pub fn defaults() -> Self {
        Self {
            version: 1,
            revision: 0,
            capture_delay_seconds: CaptureDelay(0),
            include_cursor: false,
            default_image_format: DefaultImageFormat::Png,
        }
    }
    fn validate_saved(&self) -> Result<(), Error> {
        if self.version != 1 || self.revision == 0 || self.revision > MAX_REVISION {
            Err(Error::Corrupt)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRequest {
    pub expected_revision: u64,
    pub capture_delay_seconds: CaptureDelay,
    pub include_cursor: bool,
    pub default_image_format: DefaultImageFormat,
}
impl SaveRequest {
    fn same_choice(&self, current: &CapturePreferences) -> bool {
        self.capture_delay_seconds == current.capture_delay_seconds
            && self.include_cursor == current.include_cursor
            && self.default_image_format == current.default_image_format
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Corrupt,
    UnsafePath,
    Busy,
    StaleRevision,
    Conflict,
    Access,
    WriteFailed,
    Indeterminate,
    NotAllowed,
    RevisionExhausted,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Corrupt => "截图设置无法读取，原文件已保留",
            Self::UnsafePath => "截图设置路径不可用",
            Self::Busy => "截图设置正在保存",
            Self::StaleRevision => "截图设置已改变，请载入最新设置",
            Self::Conflict => "截图设置文件已改变，请重启后检查",
            Self::Access => "无法读取截图设置",
            Self::WriteFailed => "无法保存截图设置，原设置保持不变",
            Self::Indeterminate => "无法确认截图设置的保存状态，请重启后检查",
            Self::NotAllowed => "当前无法修改截图设置",
            Self::RevisionExhausted => "截图设置版本已达到上限",
        })
    }
}
impl std::error::Error for Error {}

/// The host supplies its existing app root. No directory or preference is created here.
/// The Windows handle prevents replacement/deletion of the final managed directory.
/// This is not a sandbox against a process already allowed to rewrite the whole root.
pub(crate) struct ControlledDirectory {
    path: PathBuf,
    _pin: File,
}
impl ControlledDirectory {
    pub(crate) fn open(root: &Path) -> Result<Self, Error> {
        if !root.is_absolute() {
            return Err(Error::UnsafePath);
        }
        let meta = fs::symlink_metadata(root).map_err(|_| Error::Access)?;
        if !meta.is_dir() || is_link(&meta) {
            return Err(Error::UnsafePath);
        }
        let path = fs::canonicalize(root).map_err(|_| Error::Access)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // BACKUP_SEMANTICS | OPEN_REPARSE_POINT, no FILE_SHARE_DELETE.
            options
                .custom_flags(0x0200_0000 | 0x0020_0000)
                .share_mode(3);
        }
        let pin = options.open(&path).map_err(|_| Error::Access)?;
        let opened = pin.metadata().map_err(|_| Error::Access)?;
        if !opened.is_dir() || is_link(&opened) {
            return Err(Error::UnsafePath);
        }
        let result = Self { path, _pin: pin };
        result.check()?;
        Ok(result)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn check(&self) -> Result<(), Error> {
        let meta = fs::symlink_metadata(&self.path).map_err(|_| Error::Access)?;
        if !meta.is_dir()
            || is_link(&meta)
            || fs::canonicalize(&self.path).map_err(|_| Error::Access)? != self.path
        {
            Err(Error::UnsafePath)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Stamp {
    Missing,
    Present(Vec<u8>),
}
#[derive(Clone)]
struct Loaded {
    value: CapturePreferences,
    stamp: Stamp,
}
impl Loaded {
    fn missing() -> Self {
        Self {
            value: CapturePreferences::defaults(),
            stamp: Stamp::Missing,
        }
    }
    fn decode(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Corrupt);
        }
        let value: CapturePreferences =
            serde_json::from_slice(&bytes).map_err(|_| Error::Corrupt)?;
        value.validate_saved()?;
        Ok(Self {
            value,
            stamp: Stamp::Present(bytes),
        })
    }
}
struct PreferencesFile {
    directory: ControlledDirectory,
}
impl PreferencesFile {
    fn open(root: &Path) -> Result<Self, Error> {
        Ok(Self {
            directory: ControlledDirectory::open(root)?,
        })
    }
    fn load(&self) -> Result<Loaded, Error> {
        self.directory.check()?;
        read(&self.directory.path().join(FILE_NAME))
    }
    fn check_current(&self, expected: &Stamp) -> Result<(), Error> {
        if self.load()?.stamp == *expected {
            Ok(())
        } else {
            Err(Error::Conflict)
        }
    }
    fn commit(
        &self,
        expected: &Loaded,
        next: &CapturePreferences,
        allowed: impl Fn() -> bool,
    ) -> Result<CommitReceipt, Error> {
        self.commit_with_replace(expected, next, allowed, |from, to| fs::rename(from, to))
    }
    fn commit_with_replace(
        &self,
        expected: &Loaded,
        next: &CapturePreferences,
        allowed: impl Fn() -> bool,
        replace: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<CommitReceipt, Error> {
        next.validate_saved()?;
        if expected.value.revision >= MAX_REVISION {
            return Err(Error::RevisionExhausted);
        }
        if next.revision != expected.value.revision + 1 {
            return Err(Error::Conflict);
        }
        if !allowed() {
            return Err(Error::NotAllowed);
        }
        self.directory.check()?;
        let lock_path = self.directory.path().join(LOCK_NAME);
        check_regular(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        no_follow(&mut options);
        let lock = options.open(lock_path).map_err(|_| Error::Access)?;
        regular_handle(&lock)?;
        lock.try_lock().map_err(|_| Error::Busy)?;
        // Keep a stable lock file. Unlinking it could let two cooperating writers lock different inodes.
        self.check_current(&expected.stamp)?;
        let bytes = serde_json::to_vec(next).map_err(|_| Error::WriteFailed)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Corrupt);
        }
        let mut temporary = Temporary::create(self.directory.path())?;
        let file = temporary.file.as_mut().ok_or(Error::WriteFailed)?;
        file.write_all(&bytes).map_err(|_| Error::WriteFailed)?;
        file.sync_all().map_err(|_| Error::WriteFailed)?;
        temporary.file.take();
        self.directory.check()?;
        self.check_current(&expected.stamp)?;
        if !allowed() {
            return Err(Error::NotAllowed);
        }
        let destination = self.directory.path().join(FILE_NAME);
        let committed = Stamp::Present(bytes);
        // Accepted replacement is the commit boundary. Cancellation after it cannot undo this setting.
        let warning = match replace(
            temporary.path.as_ref().ok_or(Error::WriteFailed)?,
            &destination,
        ) {
            Ok(()) => {
                temporary.path.take();
                #[cfg(unix)]
                {
                    File::open(self.directory.path())
                        .and_then(|f| f.sync_all())
                        .is_err()
                }
                #[cfg(not(unix))]
                {
                    false
                }
            }
            Err(_) => match read(&destination) {
                Ok(actual) if actual.stamp == committed => true,
                Ok(actual) if actual.stamp == expected.stamp => return Err(Error::WriteFailed),
                _ => return Err(Error::Indeterminate),
            },
        };
        Ok(CommitReceipt {
            value: next.clone(),
            changed: true,
            durability_warning: warning,
            stamp: committed,
        })
    }
}

/// Return `value` to the renderer. `changed` controls the bounded event; warnings stay separate.
/// No source bytes, paths, or private stamp are serialized into IPC.
pub struct CommitReceipt {
    pub value: CapturePreferences,
    pub changed: bool,
    pub durability_warning: bool,
    stamp: Stamp,
}
struct State {
    loaded: Loaded,
    fault: Option<Error>,
}
struct Inner {
    file: Option<PreferencesFile>,
    state: Mutex<State>,
    busy: AtomicBool,
    generation: AtomicU64,
}
#[derive(Clone)]
pub struct CapturePreferencesActor(Arc<Inner>);
impl CapturePreferencesActor {
    /// A bad file does not abort app startup and is never replaced with defaults.
    pub fn open(root: &Path) -> Self {
        let (file, loaded, fault) = match PreferencesFile::open(root) {
            Ok(file) => match file.load() {
                Ok(loaded) => (Some(file), loaded, None),
                Err(error) => (Some(file), Loaded::missing(), Some(error)),
            },
            Err(error) => (None, Loaded::missing(), Some(error)),
        };
        Self(Arc::new(Inner {
            file,
            state: Mutex::new(State { loaded, fault }),
            busy: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }))
    }
    pub fn view(&self) -> Result<CapturePreferences, Error> {
        let state = self.0.state.lock().map_err(|_| Error::Indeterminate)?;
        if let Some(error) = state.fault {
            return Err(error);
        }
        Ok(state.loaded.value.clone())
    }
    /// Capture admission takes one immutable snapshot; it must not silently use an in-flight old value.
    pub fn snapshot_for_capture(&self) -> Result<CapturePreferences, Error> {
        if self.busy() {
            return Err(Error::Busy);
        }
        let value = self.view()?;
        if self.busy() {
            return Err(Error::Busy);
        }
        Ok(value)
    }
    pub fn busy(&self) -> bool {
        self.0.busy.load(Ordering::Acquire)
    }
    /// Called under the host admission gate BEFORE spawn_blocking. No OS or disk call here.
    pub fn begin_save(&self, request: SaveRequest) -> Result<SaveTask, Error> {
        if self
            .0
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::Busy);
        }
        let permit = WorkPermit(self.0.clone());
        let generation = self.0.generation.load(Ordering::Acquire);
        let state = self.0.state.lock().map_err(|_| Error::Indeterminate)?;
        if let Some(error) = state.fault {
            return Err(error);
        }
        if request.expected_revision != state.loaded.value.revision {
            return Err(Error::StaleRevision);
        }
        let task = SaveTask {
            permit,
            loaded: state.loaded.clone(),
            request,
            generation,
        };
        drop(state);
        Ok(task)
    }
    /// Invalidate accepted saves during shutdown. Does not release their real-work permit.
    pub fn invalidate(&self) {
        self.0.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub async fn drain(&self, timeout: Duration) -> Result<(), Error> {
        self.invalidate();
        let deadline = tokio::time::Instant::now() + timeout;
        while self.busy() {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Busy);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }
}
struct WorkPermit(Arc<Inner>);
impl Drop for WorkPermit {
    fn drop(&mut self) {
        if std::thread::panicking() {
            if let Ok(mut state) = self.0.state.lock() {
                state.fault = Some(Error::Indeterminate);
            }
        }
        self.0.busy.store(false, Ordering::Release);
    }
}
/// Move into the actual blocking closure. Dropping a JoinHandle must not drop this task early.
pub struct SaveTask {
    permit: WorkPermit,
    loaded: Loaded,
    request: SaveRequest,
    generation: u64,
}
impl SaveTask {
    pub fn commit(self, allowed: impl Fn() -> bool) -> Result<CommitReceipt, Error> {
        let inner = &self.permit.0;
        let admitted = || inner.generation.load(Ordering::Acquire) == self.generation && allowed();
        let result = (|| {
            if !admitted() {
                return Err(Error::NotAllowed);
            }
            let file = inner.file.as_ref().ok_or(Error::Access)?;
            if self.request.same_choice(&self.loaded.value) {
                file.check_current(&self.loaded.stamp)?;
                if !admitted() {
                    return Err(Error::NotAllowed);
                }
                return Ok(CommitReceipt {
                    value: self.loaded.value.clone(),
                    changed: false,
                    durability_warning: false,
                    stamp: self.loaded.stamp.clone(),
                });
            }
            if self.loaded.value.revision >= MAX_REVISION {
                return Err(Error::RevisionExhausted);
            }
            let next = CapturePreferences {
                version: 1,
                revision: self.loaded.value.revision + 1,
                capture_delay_seconds: self.request.capture_delay_seconds,
                include_cursor: self.request.include_cursor,
                default_image_format: self.request.default_image_format,
            };
            file.commit(&self.loaded, &next, admitted)
        })();
        let mut state = inner.state.lock().map_err(|_| Error::Indeterminate)?;
        match &result {
            Ok(commit) => {
                state.loaded = Loaded {
                    value: commit.value.clone(),
                    stamp: commit.stamp.clone(),
                };
            }
            Err(
                error @ (Error::Conflict
                | Error::Indeterminate
                | Error::Corrupt
                | Error::UnsafePath),
            ) => state.fault = Some(*error),
            _ => (),
        }
        drop(state);
        result
    }
}

fn read(path: &Path) -> Result<Loaded, Error> {
    if !check_regular(path)? {
        return Ok(Loaded::missing());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options.open(path).map_err(|_| Error::Access)?;
    let before = regular_handle(&file)?;
    if before.len() > MAX_BYTES as u64 {
        return Err(Error::Corrupt);
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Access)?;
    let after = regular_handle(&file)?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != before.len()
    {
        return Err(Error::Conflict);
    }
    Loaded::decode(bytes)
}
fn check_regular(path: &Path) -> Result<bool, Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !is_link(&meta) => Ok(true),
        Ok(_) => Err(Error::UnsafePath),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(Error::Access),
    }
}
fn regular_handle(file: &File) -> Result<Metadata, Error> {
    let meta = file.metadata().map_err(|_| Error::Access)?;
    if meta.is_file() && !is_link(&meta) {
        Ok(meta)
    } else {
        Err(Error::UnsafePath)
    }
}
fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}
fn no_follow(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    #[cfg(not(windows))]
    let _ = options;
}
struct Temporary {
    file: Option<File>,
    path: Option<PathBuf>,
}
impl Temporary {
    fn create(root: &Path) -> Result<Self, Error> {
        let path = root.join(format!(".capture-preferences-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        no_follow(&mut options);
        let file = options.open(&path).map_err(|_| Error::WriteFailed)?;
        Ok(Self {
            file: Some(file),
            path: Some(path),
        })
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("mewu-capture-prefs-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn request(revision: u64, delay: u8) -> SaveRequest {
        SaveRequest {
            expected_revision: revision,
            capture_delay_seconds: CaptureDelay::try_from(delay).unwrap(),
            include_cursor: true,
            default_image_format: DefaultImageFormat::Jpeg,
        }
    }
    fn saved() -> CapturePreferences {
        CapturePreferences {
            revision: 1,
            ..CapturePreferences::defaults()
        }
    }

    #[test]
    fn strict_wire_rejects_unknown_duplicate_wrong_type_and_missing_fields() {
        let valid = serde_json::to_value(saved()).unwrap();
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("revision", serde_json::json!(0)),
            ("revision", serde_json::json!(MAX_REVISION + 1)),
            ("captureDelaySeconds", serde_json::json!(1)),
            ("captureDelaySeconds", serde_json::json!("3")),
            ("defaultImageFormat", serde_json::json!({"png":null})),
            ("includeCursor", serde_json::json!(1)),
            ("extra", serde_json::json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(
                Loaded::decode(serde_json::to_vec(&invalid).unwrap()).is_err(),
                "{field}"
            );
        }
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("includeCursor");
        assert!(Loaded::decode(serde_json::to_vec(&missing).unwrap()).is_err());
        assert!(Loaded::decode(br#"{"version":1,"revision":1,"revision":2,"captureDelaySeconds":0,"includeCursor":false,"defaultImageFormat":"png"}"#.to_vec()).is_err());
        assert!(Loaded::decode(vec![b' '; MAX_BYTES + 1]).is_err());
    }
    #[test]
    fn defaults_noop_and_reads_create_no_files() {
        let dir = Directory::new();
        let actor = CapturePreferencesActor::open(&dir.0);
        assert_eq!(actor.view().unwrap(), CapturePreferences::defaults());
        let request = SaveRequest {
            expected_revision: 0,
            capture_delay_seconds: CaptureDelay(0),
            include_cursor: false,
            default_image_format: DefaultImageFormat::Png,
        };
        assert!(
            !actor
                .begin_save(request)
                .unwrap()
                .commit(|| true)
                .unwrap()
                .changed
        );
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    }
    #[test]
    fn restart_cas_and_external_same_revision_change_preserve_bytes() {
        let dir = Directory::new();
        let actor = CapturePreferencesActor::open(&dir.0);
        let receipt = actor
            .begin_save(request(0, 3))
            .unwrap()
            .commit(|| true)
            .unwrap();
        assert_eq!(receipt.value.revision, 1);
        assert!(matches!(
            actor.begin_save(request(0, 5)),
            Err(Error::StaleRevision)
        ));
        let restarted = CapturePreferencesActor::open(&dir.0);
        assert_eq!(restarted.view().unwrap(), receipt.value);
        let mut changed = receipt.value.clone();
        changed.include_cursor = false;
        let bytes = serde_json::to_vec_pretty(&changed).unwrap();
        fs::write(dir.0.join(FILE_NAME), &bytes).unwrap();
        assert!(matches!(
            actor.begin_save(request(1, 3)).unwrap().commit(|| true),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
        assert_eq!(actor.view().unwrap_err(), Error::Conflict);
    }
    #[test]
    fn corrupt_existing_file_is_preserved_and_not_editable() {
        let dir = Directory::new();
        fs::write(dir.0.join(FILE_NAME), b"broken").unwrap();
        let actor = CapturePreferencesActor::open(&dir.0);
        assert_eq!(actor.view().unwrap_err(), Error::Corrupt);
        assert!(matches!(
            actor.begin_save(request(0, 3)),
            Err(Error::Corrupt)
        ));
        assert!(!actor.busy());
        assert_eq!(fs::read(dir.0.join(FILE_NAME)).unwrap(), b"broken");
        assert!(!dir.0.join(LOCK_NAME).exists());
    }
    #[test]
    fn permit_follows_real_worker_and_invalidated_task_cannot_commit() {
        let dir = Directory::new();
        let actor = CapturePreferencesActor::open(&dir.0);
        let task = actor.begin_save(request(0, 3)).unwrap();
        let (started, wait) = std::sync::mpsc::channel();
        let (release, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            receive.recv().unwrap();
            task.commit(|| true).err()
        });
        wait.recv().unwrap();
        actor.invalidate();
        assert!(actor.busy());
        assert!(matches!(actor.begin_save(request(0, 5)), Err(Error::Busy)));
        assert_eq!(actor.snapshot_for_capture().unwrap_err(), Error::Busy);
        release.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), Some(Error::NotAllowed));
        assert!(!actor.busy());
        assert!(!dir.0.join(FILE_NAME).exists());
    }
    #[test]
    fn failed_replace_or_canceled_precommit_preserves_original_and_cleans_temporary() {
        let dir = Directory::new();
        let file = PreferencesFile::open(&dir.0).unwrap();
        let initial = file.load().unwrap();
        let next = saved();
        assert!(matches!(
            file.commit_with_replace(
                &initial,
                &next,
                || true,
                |_, _| Err(std::io::ErrorKind::PermissionDenied.into())
            ),
            Err(Error::WriteFailed)
        ));
        assert!(!dir.0.join(FILE_NAME).exists());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1); // only stable lock
        let checks = std::cell::Cell::new(0);
        assert!(matches!(
            file.commit(&initial, &next, || {
                let n = checks.get();
                checks.set(n + 1);
                n == 0
            }),
            Err(Error::NotAllowed)
        ));
        assert!(!dir.0.join(FILE_NAME).exists());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    #[test]
    fn replacement_error_after_actual_commit_is_reported_as_committed() {
        let dir = Directory::new();
        let file = PreferencesFile::open(&dir.0).unwrap();
        let initial = file.load().unwrap();
        let receipt = file
            .commit_with_replace(
                &initial,
                &saved(),
                || true,
                |from, to| {
                    fs::rename(from, to)?;
                    Err(std::io::ErrorKind::Other.into())
                },
            )
            .unwrap();
        assert!(receipt.durability_warning);
        assert_eq!(file.load().unwrap().value, receipt.value);
    }
    #[test]
    fn second_writer_lock_and_byte_cas_are_independent_of_ui_revision() {
        let dir = Directory::new();
        let first = CapturePreferencesActor::open(&dir.0);
        let second = CapturePreferencesActor::open(&dir.0);
        first
            .begin_save(request(0, 3))
            .unwrap()
            .commit(|| true)
            .unwrap();
        let bytes = fs::read(dir.0.join(FILE_NAME)).unwrap();
        assert!(matches!(
            second.begin_save(request(0, 5)).unwrap().commit(|| true),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        let lock = options.open(dir.0.join(LOCK_NAME)).unwrap();
        lock.try_lock().unwrap();
        assert!(matches!(
            first.begin_save(request(1, 5)).unwrap().commit(|| true),
            Err(Error::Busy)
        ));
        assert_eq!(first.view().unwrap().revision, 1);
        assert_eq!(fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
    }
    #[test]
    fn cancel_after_accepted_replace_does_not_lie_about_the_committed_value() {
        let dir = Directory::new();
        let file = PreferencesFile::open(&dir.0).unwrap();
        let original = file.load().unwrap();
        let allowed = AtomicBool::new(true);
        let receipt = file
            .commit_with_replace(
                &original,
                &saved(),
                || allowed.load(Ordering::Acquire),
                |from, to| {
                    fs::rename(from, to)?;
                    allowed.store(false, Ordering::Release);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(file.load().unwrap().value, receipt.value);
        assert!(!allowed.load(Ordering::Acquire));
    }
    #[test]
    fn foreign_file_in_preference_slot_and_missing_root_fail_closed() {
        let dir = Directory::new();
        fs::create_dir(dir.0.join(FILE_NAME)).unwrap();
        assert_eq!(
            CapturePreferencesActor::open(&dir.0).view().unwrap_err(),
            Error::UnsafePath
        );
        let missing = dir.0.join("missing");
        assert!(CapturePreferencesActor::open(&missing).view().is_err());
        assert!(!missing.exists());
    }
    #[cfg(windows)]
    #[test]
    fn controlled_directory_cannot_be_renamed_while_actor_is_alive() {
        let dir = Directory::new();
        let actor = CapturePreferencesActor::open(&dir.0);
        actor.view().unwrap();
        let other = dir.0.with_extension("moved");
        assert!(fs::rename(&dir.0, &other).is_err());
        drop(actor);
        fs::rename(&dir.0, &other).unwrap();
        fs::rename(&other, &dir.0).unwrap();
    }
}
