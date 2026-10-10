// SPDX-License-Identifier: MPL-2.0
//! Small native preferences, separate from scene/Agent data and webview storage.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_PREFERENCE_BYTES: usize = 16 * 1024;
const FILE_NAME: &str = "host-preferences.json";
const LOCK_NAME: &str = "host-preferences.lock";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShortcutChord {
    /// A native virtual letter, not a physical DOM KeyboardEvent.code.
    pub code: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}
impl ShortcutChord {
    pub fn validate(&self) -> Result<(), PreferencesError> {
        let bytes = self.code.as_bytes();
        if bytes.len() != 4
            || &bytes[..3] != b"Key"
            || !bytes[3].is_ascii_uppercase()
            || !(self.ctrl || self.shift || self.alt)
        {
            return Err(PreferencesError::InvalidChord);
        }
        Ok(())
    }
    pub fn default_capture() -> Self {
        Self {
            code: "KeyS".into(),
            ctrl: false,
            shift: true,
            alt: true,
        }
    }
    pub fn fallback_capture() -> Self {
        Self {
            code: "KeyS".into(),
            ctrl: true,
            shift: true,
            alt: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostPreferences {
    pub version: u32,
    pub revision: u64,
    pub capture_shortcut: Option<ShortcutChord>,
}
impl<'de> Deserialize<'de> for HostPreferences {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Option fields normally accept a missing key as None. Here null is an
        // explicit user choice, so absence must be invalid rather than disabled.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RequiredNullable {
            Disabled(()),
            Chord(ShortcutChord),
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            version: u32,
            revision: u64,
            capture_shortcut: RequiredNullable,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            version: wire.version,
            revision: wire.revision,
            capture_shortcut: match wire.capture_shortcut {
                RequiredNullable::Disabled(()) => None,
                RequiredNullable::Chord(chord) => Some(chord),
            },
        })
    }
}
impl HostPreferences {
    pub fn validate_saved(&self) -> Result<(), PreferencesError> {
        if self.version != 1 {
            return Err(PreferencesError::UnsupportedVersion);
        }
        if self.revision == 0 {
            return Err(PreferencesError::Corrupt);
        }
        if let Some(chord) = &self.capture_shortcut {
            chord.validate()?;
        }
        Ok(())
    }
    fn missing() -> Self {
        Self {
            version: 1,
            revision: 0,
            capture_shortcut: Some(ShortcutChord::default_capture()),
        }
    }
}

/// Never sent to a renderer. Full bounded bytes also notice same-revision edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStamp {
    Missing,
    Present(Vec<u8>),
}
#[derive(Clone, Debug)]
pub struct LoadedPreferences {
    pub preferences: HostPreferences,
    pub stamp: FileStamp,
}
impl LoadedPreferences {
    pub fn missing() -> Self {
        Self {
            preferences: HostPreferences::missing(),
            stamp: FileStamp::Missing,
        }
    }
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PreferencesError> {
        if bytes.len() > MAX_PREFERENCE_BYTES {
            return Err(PreferencesError::TooLarge);
        }
        let preferences: HostPreferences =
            serde_json::from_slice(&bytes).map_err(|_| PreferencesError::Corrupt)?;
        preferences.validate_saved()?;
        Ok(Self {
            preferences,
            stamp: FileStamp::Present(bytes),
        })
    }
}
#[derive(Clone, Debug)]
pub struct CommitReceipt {
    pub stamp: FileStamp,
    pub durability_warning: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferencesError {
    InvalidChord,
    Corrupt,
    UnsupportedVersion,
    TooLarge,
    UnsafePath,
    Busy,
    Conflict,
    Access,
    WriteFailed,
    CommitIndeterminate,
    NotAllowed,
    RevisionExhausted,
}
impl fmt::Display for PreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidChord => "请使用 Ctrl、Shift 或 Alt 加 A–Z",
            Self::Corrupt | Self::UnsupportedVersion | Self::TooLarge => {
                "快捷键配置无法读取，原文件已保留"
            }
            Self::UnsafePath => "快捷键配置路径不可用",
            Self::Busy => "快捷键设置正在保存，请稍后再试",
            Self::Conflict => "快捷键配置已在其他位置改变，请重启后检查",
            Self::Access => "无法读取快捷键设置",
            Self::WriteFailed => "无法保存快捷键设置，原设置保持不变",
            Self::CommitIndeterminate => "无法确认快捷键设置的保存状态，请重启后检查",
            Self::NotAllowed => "当前无法修改截图快捷键",
            Self::RevisionExhausted => "快捷键设置版本已达到上限",
        })
    }
}
impl std::error::Error for PreferencesError {}

pub struct PreferencesFile {
    root: PathBuf,
}
impl PreferencesFile {
    /// root is the host-resolved app data directory, never a renderer path.
    pub fn open(root: &Path) -> Result<Self, PreferencesError> {
        if !root.is_absolute() {
            return Err(PreferencesError::UnsafePath);
        }
        let meta = fs::symlink_metadata(root).map_err(|_| PreferencesError::Access)?;
        if !meta.is_dir() || is_link(&meta) {
            return Err(PreferencesError::UnsafePath);
        }
        let root = fs::canonicalize(root).map_err(|_| PreferencesError::Access)?;
        Ok(Self { root })
    }
    pub fn load(&self) -> Result<LoadedPreferences, PreferencesError> {
        self.check_root()?;
        read_preferences(&self.root.join(FILE_NAME))
    }
    pub fn check_current(&self, expected: &FileStamp) -> Result<(), PreferencesError> {
        if self.load()?.stamp == *expected {
            Ok(())
        } else {
            Err(PreferencesError::Conflict)
        }
    }
    fn check_root(&self) -> Result<(), PreferencesError> {
        let meta = fs::symlink_metadata(&self.root).map_err(|_| PreferencesError::Access)?;
        if !meta.is_dir() || is_link(&meta) {
            Err(PreferencesError::UnsafePath)
        } else {
            Ok(())
        }
    }
    pub fn commit(
        &self,
        expected: &FileStamp,
        next: &HostPreferences,
        allowed: impl Fn() -> bool,
    ) -> Result<CommitReceipt, PreferencesError> {
        self.commit_with_replace(expected, next, allowed, |from, to| fs::rename(from, to))
    }
    fn commit_with_replace(
        &self,
        expected: &FileStamp,
        next: &HostPreferences,
        allowed: impl Fn() -> bool,
        replace: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<CommitReceipt, PreferencesError> {
        next.validate_saved()?;
        let previous_revision = match expected {
            FileStamp::Missing => 0,
            FileStamp::Present(bytes) => {
                LoadedPreferences::from_bytes(bytes.clone())?
                    .preferences
                    .revision
            }
        };
        if next.revision
            != previous_revision
                .checked_add(1)
                .ok_or(PreferencesError::RevisionExhausted)?
        {
            return Err(PreferencesError::Conflict);
        }
        if !allowed() {
            return Err(PreferencesError::NotAllowed);
        }
        self.check_root()?;
        let lock_path = self.root.join(LOCK_NAME);
        check_existing_regular(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        no_follow_windows(&mut options);
        let lock = options
            .open(&lock_path)
            .map_err(|_| PreferencesError::Access)?;
        regular_handle(&lock)?;
        lock.try_lock().map_err(|_| PreferencesError::Busy)?;
        // The lock file is deliberately never unlinked: deleting it can split
        // cooperating writers across distinct inode/file locks after a crash.
        let destination = self.root.join(FILE_NAME);
        let actual = read_preferences(&destination)?;
        if actual.stamp != *expected {
            return Err(PreferencesError::Conflict);
        }
        let bytes = serde_json::to_vec(next).map_err(|_| PreferencesError::WriteFailed)?;
        if bytes.len() > MAX_PREFERENCE_BYTES {
            return Err(PreferencesError::TooLarge);
        }
        let mut temporary = Temporary::create(&self.root)?;
        temporary
            .file
            .as_mut()
            .unwrap()
            .write_all(&bytes)
            .map_err(|_| PreferencesError::WriteFailed)?;
        temporary
            .file
            .as_ref()
            .unwrap()
            .sync_all()
            .map_err(|_| PreferencesError::WriteFailed)?;
        temporary.file.take();
        if !allowed() {
            return Err(PreferencesError::NotAllowed);
        }
        self.check_root()?;
        if read_preferences(&destination)?.stamp != *expected {
            return Err(PreferencesError::Conflict);
        }
        let committed = FileStamp::Present(bytes);
        let result = replace(temporary.path.as_ref().unwrap(), &destination);
        match result {
            Ok(()) => {
                temporary.path.take();
                // Unix directory sync is best effort after a successful rename:
                // failure here must not falsely report a rolled-back config.
                #[cfg(unix)]
                let durability_warning = File::open(&self.root).and_then(|f| f.sync_all()).is_err();
                #[cfg(not(unix))]
                let durability_warning = false;
                Ok(CommitReceipt {
                    stamp: committed,
                    durability_warning,
                })
            }
            Err(_) => match read_preferences(&destination) {
                Ok(current) if current.stamp == committed => Ok(CommitReceipt {
                    stamp: committed,
                    durability_warning: true,
                }),
                Ok(current) if current.stamp == *expected => Err(PreferencesError::WriteFailed),
                _ => Err(PreferencesError::CommitIndeterminate),
            },
        }
    }
}
fn read_preferences(path: &Path) -> Result<LoadedPreferences, PreferencesError> {
    if !check_existing_regular(path)? {
        return Ok(LoadedPreferences::missing());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow_windows(&mut options);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options.open(path).map_err(|_| PreferencesError::Access)?;
    let before = regular_handle(&file)?;
    if before.len() > MAX_PREFERENCE_BYTES as u64 {
        return Err(PreferencesError::TooLarge);
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_PREFERENCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PreferencesError::Access)?;
    let after = regular_handle(&file)?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || after.len() != bytes.len() as u64
    {
        return Err(PreferencesError::Conflict);
    }
    LoadedPreferences::from_bytes(bytes)
}
fn check_existing_regular(path: &Path) -> Result<bool, PreferencesError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !is_link(&meta) => Ok(true),
        Ok(_) => Err(PreferencesError::UnsafePath),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(PreferencesError::Access),
    }
}
fn regular_handle(file: &File) -> Result<Metadata, PreferencesError> {
    let meta = file.metadata().map_err(|_| PreferencesError::Access)?;
    if !meta.is_file() || is_link(&meta) {
        return Err(PreferencesError::UnsafePath);
    }
    Ok(meta)
}
fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn no_follow_windows(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    #[cfg(not(windows))]
    {
        let _ = options;
    }
}
struct Temporary {
    path: Option<PathBuf>,
    file: Option<File>,
}
impl Temporary {
    fn create(root: &Path) -> Result<Self, PreferencesError> {
        let path = root.join(format!(".host-preferences.{}.tmp", uuid::Uuid::new_v4()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| PreferencesError::WriteFailed)?;
        Ok(Self {
            path: Some(path),
            file: Some(file),
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
    fn saved(revision: u64, chord: Option<ShortcutChord>) -> HostPreferences {
        HostPreferences {
            version: 1,
            revision,
            capture_shortcut: chord,
        }
    }
    struct TestDir(PathBuf);
    fn test_root() -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from("D:/tmp")
        }
        #[cfg(not(windows))]
        {
            std::env::temp_dir()
        }
    }
    impl TestDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let Ok(canonical) = self.0.canonicalize() else {
                return;
            };
            let Ok(parent) = test_root().canonicalize() else {
                return;
            };
            if canonical.parent() == Some(parent.as_path())
                && canonical.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with("mewu-shortcut-prefs-test-")
                })
            {
                let _ = fs::remove_dir_all(canonical);
            }
        }
    }
    fn fixture() -> (TestDir, PreferencesFile) {
        let dir =
            TestDir(test_root().join(format!("mewu-shortcut-prefs-test-{}", uuid::Uuid::new_v4())));
        fs::create_dir(dir.path()).unwrap();
        let file = PreferencesFile::open(dir.path()).unwrap();
        (dir, file)
    }
    #[test]
    fn missing_and_explicit_disabled_are_distinct_and_schema_is_strict() {
        let (_dir, file) = fixture();
        assert_eq!(file.load().unwrap().preferences.revision, 0);
        let receipt = file
            .commit(&FileStamp::Missing, &saved(1, None), || true)
            .unwrap();
        assert_eq!(file.load().unwrap().stamp, receipt.stamp);
        assert_eq!(file.load().unwrap().preferences.capture_shortcut, None);
        for bad in [
            r#"{"version":1,"revision":1}"#,
            r#"{"version":1,"revision":1,"captureShortcut":null,"extra":0}"#,
            r#"{"version":2,"revision":1,"captureShortcut":null}"#,
            r#"{"version":1,"revision":0,"captureShortcut":null}"#,
        ] {
            assert!(LoadedPreferences::from_bytes(bad.as_bytes().to_vec()).is_err());
        }
    }
    #[test]
    fn corrupt_and_stale_files_are_never_overwritten() {
        let (dir, file) = fixture();
        fs::write(dir.path().join(FILE_NAME), b"invalid").unwrap();
        assert_eq!(
            file.commit(&FileStamp::Missing, &saved(1, None), || true)
                .unwrap_err(),
            PreferencesError::Corrupt
        );
        assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap(), b"invalid");
        fs::remove_file(dir.path().join(FILE_NAME)).unwrap();
        file.commit(&FileStamp::Missing, &saved(1, None), || true)
            .unwrap();
        assert_eq!(
            file.commit(
                &FileStamp::Missing,
                &saved(1, Some(ShortcutChord::default_capture())),
                || true
            )
            .unwrap_err(),
            PreferencesError::Conflict
        );
    }
    #[test]
    fn replace_failure_leaves_bytes_and_removes_only_own_temporary() {
        let (dir, file) = fixture();
        let first = file
            .commit(&FileStamp::Missing, &saved(1, None), || true)
            .unwrap();
        let before = fs::read(dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(
            file.commit_with_replace(
                &first.stamp,
                &saved(2, Some(ShortcutChord::default_capture())),
                || true,
                |_, _| Err(std::io::Error::other("synthetic failure"))
            )
            .unwrap_err(),
            PreferencesError::WriteFailed
        );
        assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap(), before);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2); // config + stable lock file
    }
    #[test]
    fn final_exit_fence_prevents_replace_and_post_replace_error_is_not_rollback() {
        let (_dir, file) = fixture();
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            file.commit(&FileStamp::Missing, &saved(1, None), || {
                let n = calls.get();
                calls.set(n + 1);
                n == 0
            })
            .unwrap_err(),
            PreferencesError::NotAllowed
        );
        assert_eq!(file.load().unwrap().stamp, FileStamp::Missing);
        let receipt = file
            .commit_with_replace(
                &FileStamp::Missing,
                &saved(1, None),
                || true,
                |from, to| {
                    fs::rename(from, to)?;
                    Err(std::io::Error::other("ambiguous synthetic return"))
                },
            )
            .unwrap();
        assert!(receipt.durability_warning);
        assert_eq!(file.load().unwrap().preferences.revision, 1);
    }
    #[test]
    fn cooperating_writer_lock_and_stamp_conflicts_preserve_the_file() {
        let (dir, file) = fixture();
        let other = PreferencesFile::open(dir.path()).unwrap();
        let old = other.load().unwrap().stamp;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.path().join(LOCK_NAME))
            .unwrap();
        lock.try_lock().unwrap();
        assert_eq!(
            file.commit(&old, &saved(1, None), || true).unwrap_err(),
            PreferencesError::Busy
        );
        assert_eq!(file.load().unwrap().stamp, FileStamp::Missing);
        drop(lock);
        let first = file.commit(&old, &saved(1, None), || true).unwrap();
        assert_eq!(other.check_current(&old), Err(PreferencesError::Conflict));
        assert_eq!(
            other
                .commit(
                    &old,
                    &saved(1, Some(ShortcutChord::default_capture())),
                    || true
                )
                .unwrap_err(),
            PreferencesError::Conflict
        );
        assert_eq!(other.load().unwrap().stamp, first.stamp);
    }
    #[test]
    fn file_and_chord_budgets_are_strict_without_normalizing_disabled() {
        let (dir, file) = fixture();
        let mut bytes = serde_json::to_vec(&saved(1, None)).unwrap();
        bytes.resize(MAX_PREFERENCE_BYTES, b' ');
        fs::write(dir.path().join(FILE_NAME), &bytes).unwrap();
        assert_eq!(file.load().unwrap().preferences.capture_shortcut, None);
        bytes.push(b' ');
        fs::write(dir.path().join(FILE_NAME), &bytes).unwrap();
        assert_eq!(file.load().unwrap_err(), PreferencesError::TooLarge);
        for code in ["Keya", "KeyF8", "F8", "Digit1", "KeyA\0"] {
            assert!(ShortcutChord {
                code: code.into(),
                ctrl: true,
                shift: false,
                alt: false
            }
            .validate()
            .is_err());
        }
        assert!(ShortcutChord {
            code: "KeyA".into(),
            ctrl: false,
            shift: false,
            alt: false
        }
        .validate()
        .is_err());
    }
}
