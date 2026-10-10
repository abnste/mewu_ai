// SPDX-License-Identifier: MPL-2.0
//! Recording-only preferences. This file never reads or writes shortcut settings.
use crate::recording_backend::AudioMode;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_BYTES: usize = 4096;
pub const MAX_REVISION: u64 = 9_007_199_254_740_991;
const FILE_NAME: &str = "recording-preferences.json";
const LOCK_NAME: &str = "recording-preferences.lock";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingAudioGrant {
    pub plugin_id: String,
    pub revision: u64,
    pub contribution_id: String,
}
impl RecordingAudioGrant {
    pub fn validate(&self) -> Result<(), Error> {
        fn id(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        }
        if !id(&self.plugin_id)
            || !id(&self.contribution_id)
            || self.revision == 0
            || self.revision > MAX_REVISION
        {
            Err(Error::Corrupt)
        } else {
            Ok(())
        }
    }
}

fn required_grant<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<RecordingAudioGrant>, D::Error> {
    Option::<RecordingAudioGrant>::deserialize(d)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingAudioSelection {
    pub revision: u64,
    pub mode: AudioMode,
    #[serde(deserialize_with = "required_grant")]
    pub grant: Option<RecordingAudioGrant>,
}
impl RecordingAudioSelection {
    pub fn validate(&self) -> Result<(), Error> {
        if self.revision > MAX_REVISION {
            return Err(Error::Corrupt);
        }
        validate_choice(self.mode, self.grant.as_ref())
    }
}
pub fn validate_choice(mode: AudioMode, grant: Option<&RecordingAudioGrant>) -> Result<(), Error> {
    match (mode, grant) {
        (AudioMode::Mute, None) => Ok(()),
        (AudioMode::Mute, Some(_)) | (_, None) => Err(Error::Corrupt),
        (_, Some(grant)) => grant.validate(),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub revision: u64,
    pub mode: AudioMode,
    #[serde(deserialize_with = "required_grant")]
    pub grant: Option<RecordingAudioGrant>,
}
impl Preferences {
    pub fn missing() -> Self {
        Self {
            version: 1,
            revision: 0,
            mode: AudioMode::Mute,
            grant: None,
        }
    }
    fn validate_saved(&self) -> Result<(), Error> {
        if self.version != 1 || self.revision == 0 || self.revision > MAX_REVISION {
            return Err(Error::Corrupt);
        }
        validate_choice(self.mode, self.grant.as_ref())
    }
}

/// Exact bounded bytes catch even same-revision external edits. Never serialized to a webview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stamp {
    Missing,
    Present(Vec<u8>),
}
#[derive(Clone, Debug)]
pub struct Loaded {
    pub preferences: Preferences,
    pub stamp: Stamp,
}
impl Loaded {
    pub fn missing() -> Self {
        Self {
            preferences: Preferences::missing(),
            stamp: Stamp::Missing,
        }
    }
    fn decode(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Corrupt);
        }
        let preferences: Preferences =
            serde_json::from_slice(&bytes).map_err(|_| Error::Corrupt)?;
        preferences.validate_saved()?;
        Ok(Self {
            preferences,
            stamp: Stamp::Present(bytes),
        })
    }
}
#[derive(Debug)]
pub struct Commit {
    pub stamp: Stamp,
    pub durability_warning: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Corrupt,
    UnsafePath,
    Busy,
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
            Self::Corrupt => "录屏声音配置无法读取，原文件已保留",
            Self::UnsafePath => "录屏声音配置路径不可用",
            Self::Busy => "录屏声音设置正在保存",
            Self::Conflict => "录屏声音配置已在其他位置改变，请重启后检查",
            Self::Access => "无法读取录屏声音设置",
            Self::WriteFailed => "无法保存录屏声音设置，原设置保持不变",
            Self::Indeterminate => "无法确认录屏声音的保存状态，请重启后检查",
            Self::NotAllowed => "当前无法修改录屏声音",
            Self::RevisionExhausted => "录屏声音设置版本已达到上限",
        })
    }
}
impl std::error::Error for Error {}

pub struct PreferencesFile {
    root: PathBuf,
}
impl PreferencesFile {
    pub fn open(root: &Path) -> Result<Self, Error> {
        if !root.is_absolute() {
            return Err(Error::UnsafePath);
        }
        let meta = fs::symlink_metadata(root).map_err(|_| Error::Access)?;
        if !meta.is_dir() || is_link(&meta) {
            return Err(Error::UnsafePath);
        }
        Ok(Self {
            root: fs::canonicalize(root).map_err(|_| Error::Access)?,
        })
    }
    fn check_root(&self) -> Result<(), Error> {
        let metadata = fs::symlink_metadata(&self.root).map_err(|_| Error::Access)?;
        if !metadata.is_dir() || is_link(&metadata) {
            Err(Error::UnsafePath)
        } else {
            Ok(())
        }
    }
    pub fn load(&self) -> Result<Loaded, Error> {
        self.check_root()?;
        read(&self.root.join(FILE_NAME))
    }
    pub fn check_current(&self, stamp: &Stamp) -> Result<(), Error> {
        if self.load()?.stamp == *stamp {
            Ok(())
        } else {
            Err(Error::Conflict)
        }
    }
    pub fn commit(
        &self,
        expected: &Stamp,
        next: &Preferences,
        allowed: impl Fn() -> bool,
    ) -> Result<Commit, Error> {
        self.commit_with_replace(expected, next, allowed, |from, to| fs::rename(from, to))
    }
    fn commit_with_replace(
        &self,
        expected: &Stamp,
        next: &Preferences,
        allowed: impl Fn() -> bool,
        replace: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<Commit, Error> {
        next.validate_saved()?;
        let revision = match expected {
            Stamp::Missing => 0,
            Stamp::Present(bytes) => Loaded::decode(bytes.clone())?.preferences.revision,
        };
        if revision >= MAX_REVISION {
            return Err(Error::RevisionExhausted);
        }
        if next.revision != revision + 1 {
            return Err(Error::Conflict);
        }
        if !allowed() {
            return Err(Error::NotAllowed);
        }
        self.check_root()?;
        let lock_path = self.root.join(LOCK_NAME);
        check_regular(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        no_follow(&mut options);
        let lock = options.open(lock_path).map_err(|_| Error::Access)?;
        regular_handle(&lock)?;
        lock.try_lock().map_err(|_| Error::Busy)?;
        // Never unlink the lock: cooperating writers must keep the same lock identity.
        let destination = self.root.join(FILE_NAME);
        if read(&destination)?.stamp != *expected {
            return Err(Error::Conflict);
        }
        let bytes = serde_json::to_vec(next).map_err(|_| Error::WriteFailed)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Corrupt);
        }
        let mut temporary = Temporary::create(&self.root)?;
        let file = temporary.file.as_mut().unwrap();
        file.write_all(&bytes).map_err(|_| Error::WriteFailed)?;
        file.sync_all().map_err(|_| Error::WriteFailed)?;
        temporary.file.take();
        if !allowed() {
            return Err(Error::NotAllowed);
        }
        self.check_root()?;
        if read(&destination)?.stamp != *expected {
            return Err(Error::Conflict);
        }
        let committed = Stamp::Present(bytes);
        match replace(temporary.path.as_ref().unwrap(), &destination) {
            Ok(()) => {
                temporary.path.take();
                #[cfg(unix)]
                let durability_warning = File::open(&self.root)
                    .and_then(|file| file.sync_all())
                    .is_err();
                #[cfg(not(unix))]
                let durability_warning = false;
                Ok(Commit {
                    stamp: committed,
                    durability_warning,
                })
            }
            Err(_) => match read(&destination) {
                Ok(actual) if actual.stamp == committed => Ok(Commit {
                    stamp: committed,
                    durability_warning: true,
                }),
                Ok(actual) if actual.stamp == *expected => Err(Error::WriteFailed),
                _ => Err(Error::Indeterminate),
            },
        }
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
        options.custom_flags(0x00200000);
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
        let path = root.join(format!(
            ".recording-preferences-{}.tmp",
            uuid::Uuid::new_v4()
        ));
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
                std::env::temp_dir().join(format!("mewu-recording-prefs-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn audio(revision: u64) -> Preferences {
        Preferences {
            version: 1,
            revision,
            mode: AudioMode::Both,
            grant: Some(RecordingAudioGrant {
                plugin_id: "mewu.recording-audio".into(),
                revision: 1,
                contribution_id: "audio".into(),
            }),
        }
    }
    #[test]
    fn missing_never_creates_files_or_changes_shortcuts() {
        let root = Directory::new();
        let shortcut = root.0.join("host-preferences.json");
        fs::write(&shortcut, b"exact shortcut bytes").unwrap();
        let file = PreferencesFile::open(&root.0).unwrap();
        assert_eq!(file.load().unwrap().preferences, Preferences::missing());
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        let receipt = file.commit(&Stamp::Missing, &audio(1), || true).unwrap();
        assert_eq!(file.load().unwrap().stamp, receipt.stamp);
        assert_eq!(fs::read(shortcut).unwrap(), b"exact shortcut bytes");
    }
    #[test]
    fn strict_shape_never_enables_a_missing_or_forged_grant() {
        let valid = serde_json::to_value(audio(1)).unwrap();
        for replacement in [
            serde_json::json!(null),
            serde_json::json!("System"),
            serde_json::json!({"both":null}),
        ] {
            let mut value = valid.clone();
            value["mode"] = replacement;
            assert!(Loaded::decode(serde_json::to_vec(&value).unwrap()).is_err());
        }
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("grant");
        assert!(Loaded::decode(serde_json::to_vec(&missing).unwrap()).is_err());
        let mut absent = valid.clone();
        absent["grant"] = serde_json::Value::Null;
        assert!(Loaded::decode(serde_json::to_vec(&absent).unwrap()).is_err());
        let raw = br#"{"version":1,"revision":1,"mode":"mute","mode":"both","grant":null}"#;
        assert!(Loaded::decode(raw.to_vec()).is_err());
        let mut unknown = valid;
        unknown["autoStart"] = serde_json::json!(true);
        assert!(Loaded::decode(serde_json::to_vec(&unknown).unwrap()).is_err());
        assert!(Loaded::decode(vec![b' '; MAX_BYTES + 1]).is_err());
        assert!(
            serde_json::from_str::<RecordingAudioSelection>(r#"{"revision":0,"mode":"mute"}"#)
                .is_err()
        );
    }
    #[test]
    fn writer_conflict_and_external_same_revision_edit_preserve_file() {
        let root = Directory::new();
        let file = PreferencesFile::open(&root.0).unwrap();
        let first = file.commit(&Stamp::Missing, &audio(1), || true).unwrap();
        let before = fs::read(root.0.join(FILE_NAME)).unwrap();
        assert!(matches!(
            file.commit(&Stamp::Missing, &audio(1), || true),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(root.0.join(FILE_NAME)).unwrap(), before);
        let mut changed = audio(1);
        changed.mode = AudioMode::System;
        fs::write(
            root.0.join(FILE_NAME),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert_eq!(
            file.check_current(&first.stamp).unwrap_err(),
            Error::Conflict
        );
    }
    #[test]
    fn cancel_replace_failure_and_ambiguous_success_are_not_fake_rollbacks() {
        let root = Directory::new();
        let file = PreferencesFile::open(&root.0).unwrap();
        assert!(matches!(
            file.commit(&Stamp::Missing, &audio(1), || false),
            Err(Error::NotAllowed)
        ));
        assert!(!root.0.join(FILE_NAME).exists());
        assert!(matches!(
            file.commit_with_replace(
                &Stamp::Missing,
                &audio(1),
                || true,
                |_, _| Err(std::io::ErrorKind::PermissionDenied.into())
            ),
            Err(Error::WriteFailed)
        ));
        assert!(!root.0.join(FILE_NAME).exists());
        let accepted = file
            .commit_with_replace(
                &Stamp::Missing,
                &audio(1),
                || true,
                |a, b| {
                    fs::rename(a, b)?;
                    Err(std::io::ErrorKind::Other.into())
                },
            )
            .unwrap();
        assert!(accepted.durability_warning);
        assert_eq!(file.load().unwrap().preferences, audio(1));
        assert!(fs::read_dir(&root.0).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }
    #[test]
    fn final_cancellation_and_cooperative_lock_leave_previous_choice_intact() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = Directory::new();
        let file = PreferencesFile::open(&root.0).unwrap();
        let first = file.commit(&Stamp::Missing, &audio(1), || true).unwrap();
        let before = fs::read(root.0.join(FILE_NAME)).unwrap();
        let count = AtomicUsize::new(0);
        assert!(matches!(
            file.commit(&first.stamp, &audio(2), || count
                .fetch_add(1, Ordering::SeqCst)
                == 0),
            Err(Error::NotAllowed)
        ));
        assert_eq!(fs::read(root.0.join(FILE_NAME)).unwrap(), before);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.0.join(LOCK_NAME))
            .unwrap();
        lock.try_lock().unwrap();
        assert!(matches!(
            file.commit(&first.stamp, &audio(2), || true),
            Err(Error::Busy)
        ));
        assert_eq!(fs::read(root.0.join(FILE_NAME)).unwrap(), before);
        drop(lock);
        assert!(file.commit(&first.stamp, &audio(2), || true).is_ok());
    }

    #[test]
    fn corrupt_and_nonregular_paths_are_not_overwritten() {
        let root = Directory::new();
        let file = PreferencesFile::open(&root.0).unwrap();
        fs::write(root.0.join(FILE_NAME), b"broken").unwrap();
        assert!(matches!(file.load(), Err(Error::Corrupt)));
        assert!(file.commit(&Stamp::Missing, &audio(1), || true).is_err());
        assert_eq!(fs::read(root.0.join(FILE_NAME)).unwrap(), b"broken");
        fs::remove_file(root.0.join(FILE_NAME)).unwrap();
        fs::create_dir(root.0.join(FILE_NAME)).unwrap();
        assert!(matches!(file.load(), Err(Error::UnsafePath)));
    }
}
