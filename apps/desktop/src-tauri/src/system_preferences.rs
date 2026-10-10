// SPDX-License-Identifier: MPL-2.0
//! Host-owned network, startup and screen-sharing preferences.
//! Startup defaults are read-only; saving preserves strict CAS and shutdown drain.
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
const FILE_NAME: &str = "system-preferences.json";
const LOCK_NAME: &str = "system-preferences.lock";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    System,
    Direct,
    Custom,
}

pub fn validate_proxy(mode: ProxyMode, input: &str) -> Result<(), Error> {
    if input.len() > 2048 || input.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidProxy);
    }
    if mode != ProxyMode::Custom {
        return Ok(());
    }
    let url = url::Url::parse(input).map_err(|_| Error::InvalidProxy)?;
    if !matches!(url.scheme(), "http" | "https" | "socks5")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidProxy);
    }
    reqwest::Proxy::all(input).map_err(|_| Error::InvalidProxy)?;
    Ok(())
}

pub fn validate_startup_owner(owner: Option<&str>) -> Result<(), Error> {
    if let Some(value) = owner {
        let parsed = uuid::Uuid::parse_str(value).map_err(|_| Error::Corrupt)?;
        if parsed.is_nil() || parsed.to_string() != value {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemPreferences {
    pub version: u32,
    pub revision: u64,
    pub network_proxy_mode: ProxyMode,
    pub network_proxy_url: String,
    pub launch_at_startup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_owner: Option<String>,
    pub allow_screen_share: bool,
    #[serde(default = "default_auto_title")]
    pub auto_generate_title: bool,
}
fn default_auto_title() -> bool {
    true
}
impl SystemPreferences {
    pub fn defaults() -> Self {
        Self {
            version: 1,
            revision: 0,
            network_proxy_mode: ProxyMode::System,
            network_proxy_url: String::new(),
            launch_at_startup: false,
            startup_owner: None,
            allow_screen_share: true,
            auto_generate_title: true,
        }
    }
    fn validate_saved(&self) -> Result<(), Error> {
        if self.version != 1 || self.revision == 0 || self.revision > MAX_REVISION {
            Err(Error::Corrupt)
        } else {
            validate_startup_owner(self.startup_owner.as_deref())?;
            validate_proxy(self.network_proxy_mode, &self.network_proxy_url)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRequest {
    pub expected_revision: u64,
    pub network_proxy_mode: ProxyMode,
    pub network_proxy_url: String,
    pub launch_at_startup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_owner: Option<String>,
    pub allow_screen_share: bool,
    pub auto_generate_title: bool,
}
impl SaveRequest {
    fn same_choice(&self, current: &SystemPreferences) -> bool {
        self.network_proxy_mode == current.network_proxy_mode
            && self.network_proxy_url == current.network_proxy_url
            && self.launch_at_startup == current.launch_at_startup
            && self.startup_owner == current.startup_owner
            && self.allow_screen_share == current.allow_screen_share
            && self.auto_generate_title == current.auto_generate_title
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Corrupt,
    InvalidProxy,
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
            Self::InvalidProxy => {
                "代理地址必须是 http、https 或 socks5 地址，且不能包含账号密码、路径或查询参数"
            }
            Self::Corrupt => "系统设置无法读取，原文件已保留",
            Self::UnsafePath => "系统设置路径不可用",
            Self::Busy => "系统设置正在保存",
            Self::StaleRevision => "系统设置已改变，请载入最新设置",
            Self::Conflict => "系统设置文件已改变，请重启后检查",
            Self::Access => "无法读取系统设置",
            Self::WriteFailed => "无法保存系统设置，原设置保持不变",
            Self::Indeterminate => "无法确认系统设置的保存状态，请重启后检查",
            Self::NotAllowed => "当前无法修改系统设置",
            Self::RevisionExhausted => "系统设置版本已达到上限",
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
    value: SystemPreferences,
    stamp: Stamp,
}
impl Loaded {
    fn missing() -> Self {
        Self {
            value: SystemPreferences::defaults(),
            stamp: Stamp::Missing,
        }
    }
    fn decode(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Corrupt);
        }
        let value: SystemPreferences =
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
        next: &SystemPreferences,
        allowed: impl Fn() -> bool,
    ) -> Result<CommitReceipt, Error> {
        self.commit_with_replace(expected, next, allowed, |from, to| fs::rename(from, to))
    }
    fn commit_with_replace(
        &self,
        expected: &Loaded,
        next: &SystemPreferences,
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
    pub value: SystemPreferences,
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
pub struct SystemPreferencesActor(Arc<Inner>);
impl SystemPreferencesActor {
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
    pub fn view(&self) -> Result<SystemPreferences, Error> {
        let state = self.0.state.lock().map_err(|_| Error::Indeterminate)?;
        if let Some(error) = state.fault {
            return Err(error);
        }
        Ok(state.loaded.value.clone())
    }
    /// Capture admission takes one immutable snapshot; it must not silently use an in-flight old value.
    pub fn snapshot_for_operation(&self) -> Result<SystemPreferences, Error> {
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
        validate_startup_owner(request.startup_owner.as_deref())?;
        validate_proxy(request.network_proxy_mode, &request.network_proxy_url)?;
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
            let next = SystemPreferences {
                version: 1,
                revision: self.loaded.value.revision + 1,
                network_proxy_mode: self.request.network_proxy_mode,
                network_proxy_url: self.request.network_proxy_url.clone(),
                launch_at_startup: self.request.launch_at_startup,
                startup_owner: self.request.startup_owner.clone(),
                allow_screen_share: self.request.allow_screen_share,
                auto_generate_title: self.request.auto_generate_title,
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
        let path = root.join(format!(".system-preferences-{}.tmp", uuid::Uuid::new_v4()));
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
            #[cfg(windows)]
            let base = PathBuf::from("D:/tmp");
            #[cfg(not(windows))]
            let base = std::env::temp_dir();
            let path = base.join(format!("mewu-system-prefs-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            #[cfg(windows)]
            let base = PathBuf::from("D:/tmp");
            #[cfg(not(windows))]
            let base = std::env::temp_dir();
            if let (Ok(path), Ok(base)) = (self.0.canonicalize(), base.canonicalize()) {
                if path.parent() == Some(base.as_path())
                    && path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.starts_with("mewu-system-prefs-"))
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }
    }
    fn request(revision: u64) -> SaveRequest {
        SaveRequest {
            expected_revision: revision,
            network_proxy_mode: ProxyMode::Direct,
            network_proxy_url: String::new(),
            launch_at_startup: true,
            startup_owner: None,
            allow_screen_share: false,
            auto_generate_title: true,
        }
    }
    fn saved() -> SystemPreferences {
        SystemPreferences {
            revision: 1,
            network_proxy_mode: ProxyMode::Direct,
            launch_at_startup: true,
            allow_screen_share: false,
            ..SystemPreferences::defaults()
        }
    }
    #[test]
    fn strict_proxy_does_not_accept_credentials_extra_paths_or_other_protocols() {
        for address in [
            "http://127.0.0.1:7890",
            "https://proxy.example:443",
            "socks5://127.0.0.1:1080",
        ] {
            assert!(validate_proxy(ProxyMode::Custom, address).is_ok());
        }
        for address in [
            "",
            "ftp://proxy",
            "http://user:password@proxy",
            "http://proxy/path",
            "http://proxy?token=x",
            "http://proxy#fragment",
            "http://proxy\n",
            "http://proxy\0",
        ] {
            assert!(
                validate_proxy(ProxyMode::Custom, address).is_err(),
                "{address:?}"
            );
        }
        assert!(validate_proxy(ProxyMode::Custom, &"a".repeat(2049)).is_err());
    }
    #[test]
    fn legacy_title_default_does_not_rewrite_and_disabled_choice_reopens_exactly() {
        let dir = Directory::new();
        let mut legacy = serde_json::to_value(saved()).unwrap();
        legacy.as_object_mut().unwrap().remove("autoGenerateTitle");
        let raw = serde_json::to_vec(&legacy).unwrap();
        fs::write(dir.0.join(FILE_NAME), &raw).unwrap();
        let actor = SystemPreferencesActor::open(&dir.0);
        assert!(actor.view().unwrap().auto_generate_title);
        assert_eq!(fs::read(dir.0.join(FILE_NAME)).unwrap(), raw);
        let mut choice = request(1); choice.auto_generate_title = false;
        let receipt = actor.begin_save(choice).unwrap().commit(|| true).unwrap();
        assert!(receipt.changed); assert_eq!(receipt.value.revision, 2);
        assert!(!SystemPreferencesActor::open(&dir.0).view().unwrap().auto_generate_title);
        let mut choice = request(2); choice.auto_generate_title = false;
        assert!(!actor.begin_save(choice).unwrap().commit(|| true).unwrap().changed);
    }
    #[test]
    fn reads_defaults_and_noop_leave_no_file_and_never_enable_startup() {
        let dir = Directory::new();
        let actor = SystemPreferencesActor::open(&dir.0);
        let value = actor.view().unwrap();
        assert_eq!(value, SystemPreferences::defaults());
        assert!(value.allow_screen_share);
        assert!(!value.launch_at_startup);
        let receipt = actor
            .begin_save(SaveRequest {
                expected_revision: 0,
                network_proxy_mode: value.network_proxy_mode,
                network_proxy_url: value.network_proxy_url,
                launch_at_startup: value.launch_at_startup,
                startup_owner: value.startup_owner,
                allow_screen_share: value.allow_screen_share,
                auto_generate_title: value.auto_generate_title,
            })
            .unwrap()
            .commit(|| true)
            .unwrap();
        assert!(!receipt.changed);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    }
    #[test]
    fn saving_restarts_exact_user_choices_and_cas_never_overwrites_external_bytes() {
        let dir = Directory::new();
        let actor = SystemPreferencesActor::open(&dir.0);
        let receipt = actor
            .begin_save(request(0))
            .unwrap()
            .commit(|| true)
            .unwrap();
        assert_eq!(receipt.value, saved());
        assert_eq!(
            SystemPreferencesActor::open(&dir.0).view().unwrap(),
            saved()
        );
        assert!(matches!(
            actor.begin_save(request(0)),
            Err(Error::StaleRevision)
        ));
        let mut changed = saved();
        changed.allow_screen_share = true;
        let bytes = serde_json::to_vec_pretty(&changed).unwrap();
        fs::write(dir.0.join(FILE_NAME), &bytes).unwrap();
        assert!(matches!(
            actor.begin_save(request(1)).unwrap().commit(|| true),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
    }
    #[test]
    fn corrupted_unknown_missing_and_duplicate_fields_are_preserved() {
        let dir = Directory::new();
        let valid = serde_json::to_value(saved()).unwrap();
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("revision", serde_json::json!(0)),
            ("networkProxyMode", serde_json::json!("invalid")),
            ("allowScreenShare", serde_json::json!(0)),
            ("extra", serde_json::json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(Loaded::decode(serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("launchAtStartup");
        assert!(Loaded::decode(serde_json::to_vec(&missing).unwrap()).is_err());
        let bytes = br#"{"version":1,"revision":1,"revision":2,"networkProxyMode":"direct","networkProxyUrl":"","launchAtStartup":false,"allowScreenShare":true}"#;
        fs::write(dir.0.join(FILE_NAME), bytes).unwrap();
        let actor = SystemPreferencesActor::open(&dir.0);
        assert_eq!(actor.view().unwrap_err(), Error::Corrupt);
        assert!(actor.begin_save(request(0)).is_err());
        assert_eq!(fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
        assert!(!dir.0.join(LOCK_NAME).exists());
    }
    #[test]
    fn cancellation_and_worker_permit_prevent_late_settings_commits() {
        let dir = Directory::new();
        let actor = SystemPreferencesActor::open(&dir.0);
        let task = actor.begin_save(request(0)).unwrap();
        assert!(actor.busy());
        assert_eq!(actor.snapshot_for_operation().unwrap_err(), Error::Busy);
        assert!(matches!(actor.begin_save(request(0)), Err(Error::Busy)));
        actor.invalidate();
        assert!(matches!(task.commit(|| true), Err(Error::NotAllowed)));
        assert!(!actor.busy());
        assert!(!dir.0.join(FILE_NAME).exists());
    }
    #[test]
    fn replace_failure_or_exit_preserves_original_and_ambiguous_success_is_not_rolled_back() {
        let dir = Directory::new();
        let file = PreferencesFile::open(&dir.0).unwrap();
        let initial = file.load().unwrap();
        assert!(matches!(
            file.commit_with_replace(
                &initial,
                &saved(),
                || true,
                |_, _| Err(std::io::ErrorKind::PermissionDenied.into())
            ),
            Err(Error::WriteFailed)
        ));
        assert!(!dir.0.join(FILE_NAME).exists());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
        let n = std::cell::Cell::new(0);
        assert!(matches!(
            file.commit(&initial, &saved(), || {
                let before = n.get();
                n.set(before + 1);
                before == 0
            }),
            Err(Error::NotAllowed)
        ));
        let receipt = file
            .commit_with_replace(
                &initial,
                &saved(),
                || true,
                |from, to| {
                    fs::rename(from, to)?;
                    Err(std::io::Error::other("synthetic lost acknowledgement"))
                },
            )
            .unwrap();
        assert!(receipt.durability_warning);
        assert_eq!(file.load().unwrap().value, saved());
    }
}
