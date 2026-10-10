//! Cold business-root relocation. The only authority is bootstrap/storage.db.
//! Call before any Store, plugin, cleanup actor or file-backed worker is opened.
use crate::asset_locations::{self, AssetLocations};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

const APPLICATION_ID: i64 = 0x4d535452;
const MAX_GENERATION: u64 = 9_007_199_254_740_991;
const OWNER_FILE: &str = "owner.json";
const SOURCE_MARKER_PREFIX: &str = ".mewu-migrated-";

#[derive(Debug)]
pub struct RuntimeLock {
    _file: File,
    _root: RootPin,
}
/// Hold until OS process death, including Tauri restart's spawn-before-exit gap.
/// Worker CLI and readonly --storage-info never acquire this writer lock.
pub fn acquire_runtime_lock(
    paths: &StoragePaths,
    timeout: Duration,
) -> Result<RuntimeLock, StorageError> {
    validate_paths(paths)?;
    if timeout > Duration::from_secs(60) {
        return Err(err("runtime_lock_timeout_limit"));
    }
    create_root(&paths.bootstrap)?;
    let root = pin_root(&paths.bootstrap)?;
    let path = paths.bootstrap.join("runtime.lock");
    let began = Instant::now();
    loop {
        if path.exists() {
            asset_locations::ordinary(&fs::symlink_metadata(&path).map_err(ioe)?, false)
                .map_err(ioe)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x00200000);
        }
        let attempt = options.open(&path);
        let busy = match attempt {
            Ok(file) => {
                asset_locations::ordinary(&file.metadata().map_err(ioe)?, false).map_err(ioe)?;
                #[cfg(not(windows))]
                {
                    match file.try_lock() {
                        Ok(()) => {
                            return Ok(RuntimeLock {
                                _file: file,
                                _root: root,
                            })
                        }
                        Err(std::fs::TryLockError::WouldBlock) => true,
                        Err(std::fs::TryLockError::Error(e)) => return Err(ioe(e)),
                    }
                }
                #[cfg(windows)]
                {
                    return Ok(RuntimeLock {
                        _file: file,
                        _root: root,
                    });
                }
            }
            Err(e) => {
                #[cfg(windows)]
                {
                    if matches!(e.raw_os_error(), Some(32 | 33)) {
                        true
                    } else {
                        return Err(ioe(e));
                    }
                }
                #[cfg(not(windows))]
                {
                    return Err(ioe(e));
                }
            }
        };
        if busy && began.elapsed() >= timeout {
            return Err(err("runtime_lock_busy"));
        }
        std::thread::sleep(Duration::from_millis(40).min(timeout.saturating_sub(began.elapsed())));
    }
}

#[derive(Clone, Debug)]
pub struct StoragePaths {
    pub bootstrap: PathBuf,
    pub legacy_root: PathBuf,
    pub default_root: PathBuf,
}
#[derive(Clone, Debug)]
pub struct ResolvedStorage {
    pub root: PathBuf,
    pub generation: u64,
    pub dataset_id: String,
    pub locations: AssetLocations,
    pub trial: Option<StartupTrial>,
    _root_pin: Arc<RootPin>,
    _required_pins: Arc<Vec<File>>,
}
#[derive(Clone, Debug)]
pub struct StartupTrial {
    id: String,
    dataset_id: String,
    generation: u64,
    source: PathBuf,
    target: PathBuf,
}
impl StartupTrial {
    pub fn id(&self) -> &str {
        &self.id
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoragePhase {
    Idle,
    Requested,
    Copying,
    Verified,
    Activated,
    Committed,
    NeedsRecovery,
    Failed,
    RolledBack,
}
impl StoragePhase {
    fn text(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Requested => "Requested",
            Self::Copying => "Copying",
            Self::Verified => "Verified",
            Self::Activated => "Activated",
            Self::Committed => "Committed",
            Self::NeedsRecovery => "NeedsRecovery",
            Self::Failed => "Failed",
            Self::RolledBack => "RolledBack",
        }
    }
    fn parse(s: &str) -> Result<Self, StorageError> {
        match s {
            "Idle" => Ok(Self::Idle),
            "Requested" => Ok(Self::Requested),
            "Copying" => Ok(Self::Copying),
            "Verified" => Ok(Self::Verified),
            "Activated" => Ok(Self::Activated),
            "Committed" => Ok(Self::Committed),
            "NeedsRecovery" => Ok(Self::NeedsRecovery),
            "Failed" => Ok(Self::Failed),
            "RolledBack" => Ok(Self::RolledBack),
            _ => Err(err("ledger_phase")),
        }
    }
}
#[derive(Clone, Debug)]
pub struct StorageState {
    pub root: PathBuf,
    pub generation: u64,
    pub dataset_id: String,
    pub phase: StoragePhase,
    pub pending_id: Option<String>,
}
#[derive(Debug)]
pub struct StorageError {
    pub code: &'static str,
    detail: String,
}
impl StorageError {
    pub fn interrupted() -> Self {
        err("interrupted")
    }
}
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.code,
            if self.detail.is_empty() {
                String::new()
            } else {
                format!(": {}", self.detail)
            }
        )
    }
}
impl std::error::Error for StorageError {}
fn err(code: &'static str) -> StorageError {
    StorageError {
        code,
        detail: String::new(),
    }
}
fn ioe(e: std::io::Error) -> StorageError {
    StorageError {
        code: "storage_io",
        detail: format!("{:?}", e.kind()),
    }
}
fn sql(e: rusqlite::Error) -> StorageError {
    StorageError {
        code: "storage_ledger",
        detail: e.to_string(),
    }
}
fn json(e: serde_json::Error) -> StorageError {
    StorageError {
        code: "storage_metadata",
        detail: format!("{:?}", e.classify()),
    }
}
fn path_text(path: &Path) -> Result<&str, StorageError> {
    asset_locations::lexical(path).map_err(|_| err("unsafe_path"))?;
    path.to_str().ok_or_else(|| err("path_encoding"))
}
fn same(a: &Path, b: &Path) -> Result<bool, StorageError> {
    Ok(asset_locations::components_eq(
        &asset_locations::lexical(a).map_err(|_| err("unsafe_path"))?,
        &asset_locations::lexical(b).map_err(|_| err("unsafe_path"))?,
    ))
}
fn overlaps(a: &Path, b: &Path) -> Result<bool, StorageError> {
    let a = asset_locations::lexical(a).map_err(|_| err("unsafe_path"))?;
    let b = asset_locations::lexical(b).map_err(|_| err("unsafe_path"))?;
    let n = a.len().min(b.len());
    Ok(asset_locations::components_eq(&a[..n], &b[..n]))
}
fn next_generation(g: u64) -> Result<u64, StorageError> {
    g.checked_add(1)
        .filter(|n| *n <= MAX_GENERATION)
        .ok_or_else(|| err("generation_exhausted"))
}
fn sqlite_int(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| err("sqlite_integer_limit"))
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    volume: u64,
    file: u64,
}
#[cfg(windows)]
fn identity(file: &File) -> Result<FileIdentity, StorageError> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Time {
        lo: u32,
        hi: u32,
    }
    #[repr(C)]
    struct Info {
        attrs: u32,
        creation: Time,
        access: Time,
        write: Time,
        volume: u32,
        size_hi: u32,
        size_lo: u32,
        links: u32,
        index_hi: u32,
        index_lo: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Info) -> i32;
    }
    let mut info: Info = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(ioe(std::io::Error::last_os_error()));
    }
    Ok(FileIdentity {
        volume: info.volume as u64,
        file: ((info.index_hi as u64) << 32) | info.index_lo as u64,
    })
}
#[cfg(unix)]
fn identity(file: &File) -> Result<FileIdentity, StorageError> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata().map_err(ioe)?;
    Ok(FileIdentity {
        volume: m.dev(),
        file: m.ino(),
    })
}
#[cfg(not(any(unix, windows)))]
fn identity(_file: &File) -> Result<FileIdentity, StorageError> {
    Err(err("platform_not_supported"))
}
fn local(path: &Path) -> Result<(), StorageError> {
    path_text(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let first = path.components().next().ok_or_else(|| err("unsafe_path"))?;
        let disk = match first {
            std::path::Component::Prefix(p) => match p.kind() {
                std::path::Prefix::Disk(d) | std::path::Prefix::VerbatimDisk(d) => d,
                _ => return Err(err("network_or_device_path")),
            },
            _ => return Err(err("unsafe_path")),
        };
        let mut root: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", disk as char))
            .encode_wide()
            .collect();
        root.push(0);
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3) {
            return Err(err("nonlocal_storage"));
        }
    }
    Ok(())
}
#[derive(Debug)]
struct RootPin {
    _parents: Vec<File>,
    stamp: FileIdentity,
}
fn pin_root(path: &Path) -> Result<RootPin, StorageError> {
    local(path)?;
    let parents = asset_locations::pin_ancestors(path).map_err(ioe)?;
    let last = parents.last().ok_or_else(|| err("root_pin"))?;
    let stamp = identity(last)?;
    Ok(RootPin {
        _parents: parents,
        stamp,
    })
}
fn supports_copy_publication(path: &Path) -> Result<(), StorageError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let disk = match path.components().next() {
            Some(std::path::Component::Prefix(p)) => match p.kind() {
                std::path::Prefix::Disk(d) | std::path::Prefix::VerbatimDisk(d) => d,
                _ => return Err(err("network_or_device_path")),
            },
            _ => return Err(err("unsafe_path")),
        };
        let mut root: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", disk as char))
            .encode_wide()
            .collect();
        root.push(0);
        let mut flags = 0;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetVolumeInformationW(
                root: *const u16,
                name: *mut u16,
                name_size: u32,
                serial: *mut u32,
                max_component: *mut u32,
                flags: *mut u32,
                fs_name: *mut u16,
                fs_name_size: u32,
            ) -> i32;
        }
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(ioe(std::io::Error::last_os_error()));
        }
        if flags & 0x00080000 != 0 {
            return Err(err("target_filesystem_unsupported"));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    Ok(())
}
fn create_root(path: &Path) -> Result<(), StorageError> {
    local(path)?;
    if path.exists() {
        let _pin = pin_root(path)?;
        return Ok(());
    }
    let parent = path.parent().ok_or_else(|| err("root_parent"))?;
    let _pin = pin_root(parent)?;
    fs::create_dir(path).map_err(ioe)
}
fn validate_paths(paths: &StoragePaths) -> Result<(), StorageError> {
    for p in [&paths.bootstrap, &paths.legacy_root, &paths.default_root] {
        local(p)?;
    }
    if overlaps(&paths.bootstrap, &paths.legacy_root)?
        || overlaps(&paths.bootstrap, &paths.default_root)?
    {
        return Err(err("bootstrap_overlaps_data"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Profile {
    dataset_id: String,
    generation: u64,
    root: PathBuf,
    aliases: Vec<PathBuf>,
    pending: Option<String>,
    stamp: FileIdentity,
    required_dbs: Vec<String>,
}
#[derive(Clone, Debug)]
struct Transfer {
    id: String,
    source: PathBuf,
    target: PathBuf,
    generation: u64,
    aliases: Vec<PathBuf>,
    source_stamp: FileIdentity,
    target_stamp: FileIdentity,
    phase: StoragePhase,
    plan: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    relative: String,
    directory: bool,
    length: u64,
    sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    id: String,
    dataset_id: String,
    source: String,
    target: String,
    source_generation: u64,
    plan_sha256: String,
}

const SCHEMA:&str="
CREATE TABLE profile(id INTEGER PRIMARY KEY CHECK(id=1),dataset_id TEXT NOT NULL,generation INTEGER NOT NULL CHECK(generation>=0),active_root TEXT NOT NULL,aliases TEXT NOT NULL,pending_id TEXT,active_identity TEXT NOT NULL,required_dbs TEXT NOT NULL);
CREATE TABLE transfers(id TEXT PRIMARY KEY,source_root TEXT NOT NULL,target_root TEXT NOT NULL,source_generation INTEGER NOT NULL,source_aliases TEXT NOT NULL,source_identity TEXT NOT NULL,target_identity TEXT NOT NULL,phase TEXT NOT NULL,plan_sha256 TEXT);
CREATE TABLE entries(transfer_id TEXT NOT NULL,ordinal INTEGER NOT NULL,relative_path TEXT NOT NULL,directory INTEGER NOT NULL,length INTEGER NOT NULL,sha256 TEXT NOT NULL,temp_identity TEXT,PRIMARY KEY(transfer_id,ordinal),UNIQUE(transfer_id,relative_path));
";
fn ledger_path(paths: &StoragePaths) -> PathBuf {
    paths.bootstrap.join("storage.db")
}
struct Ledger {
    db: Connection,
    _root: RootPin,
    _file: File,
}
impl Deref for Ledger {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.db
    }
}
impl DerefMut for Ledger {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.db
    }
}
fn open_ledger(paths: &StoragePaths, write: bool) -> Result<Ledger, StorageError> {
    validate_paths(paths)?;
    let pin = pin_root(&paths.bootstrap)?;
    let path = ledger_path(paths);
    asset_locations::ordinary(&fs::symlink_metadata(&path).map_err(ioe)?, false).map_err(ioe)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x00200000);
    }
    let file = options.open(&path).map_err(ioe)?;
    asset_locations::ordinary(&file.metadata().map_err(ioe)?, false).map_err(ioe)?;
    let db = Connection::open_with_flags(
        path,
        if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sql)?;
    db.busy_timeout(Duration::from_secs(2)).map_err(sql)?;
    let app: i64 = db
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(sql)?;
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(sql)?;
    if app != APPLICATION_ID || version != 1 {
        return Err(err("ledger_format"));
    }
    if write {
        db.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
            .map_err(sql)?;
    }
    Ok(Ledger {
        db,
        _root: pin,
        _file: file,
    })
}
fn profile(db: &Connection) -> Result<Profile, StorageError> {
    let row: (String, i64, String, String, Option<String>,String,String) = db
        .query_row(
            "SELECT dataset_id,generation,active_root,aliases,pending_id,active_identity,required_dbs FROM profile WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?,r.get(5)?,r.get(6)?)),
        )
        .map_err(sql)?;
    if Uuid::parse_str(&row.0).is_err() || row.1 < 0 || row.1 as u64 > MAX_GENERATION {
        return Err(err("ledger_profile"));
    }
    let aliases: Vec<String> = serde_json::from_str(&row.3).map_err(json)?;
    let root = PathBuf::from(row.2);
    local(&root)?;
    let aliases = aliases.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    for p in &aliases {
        asset_locations::lexical(p).map_err(|_| err("ledger_alias"))?;
    }
    let required_dbs: Vec<String> = serde_json::from_str(&row.6).map_err(json)?;
    if required_dbs
        .iter()
        .any(|s| !matches!(s.as_str(), "spaces.db" | "plugins/plugins.db"))
        || required_dbs.len() > 2
        || required_dbs.len() == 2 && required_dbs[0] == required_dbs[1]
        || !required_dbs.is_empty() && !required_dbs.iter().any(|s| s == "spaces.db")
    {
        return Err(err("ledger_required_dbs"));
    }
    Ok(Profile {
        dataset_id: row.0,
        generation: row.1 as u64,
        root,
        aliases,
        pending: row.4,
        stamp: serde_json::from_str(&row.5).map_err(json)?,
        required_dbs,
    })
}
fn transfer(db: &Connection, id: &str) -> Result<Transfer, StorageError> {
    let row:(String,String,i64,String,String,String,String,Option<String>)=db.query_row("SELECT source_root,target_root,source_generation,source_aliases,source_identity,target_identity,phase,plan_sha256 FROM transfers WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(sql)?;
    if Uuid::parse_str(id).is_err() || row.2 < 0 {
        return Err(err("ledger_transfer"));
    }
    let aliases: Vec<String> = serde_json::from_str(&row.3).map_err(json)?;
    let source = PathBuf::from(row.0);
    let target = PathBuf::from(row.1);
    local(&source)?;
    local(&target)?;
    Ok(Transfer {
        id: id.into(),
        source,
        target,
        generation: row.2 as u64,
        aliases: aliases.into_iter().map(PathBuf::from).collect(),
        source_stamp: serde_json::from_str(&row.4).map_err(json)?,
        target_stamp: serde_json::from_str(&row.5).map_err(json)?,
        phase: StoragePhase::parse(&row.6)?,
        plan: row.7,
    })
}
fn aliases_json(aliases: &[PathBuf]) -> Result<String, StorageError> {
    serde_json::to_string(
        &aliases
            .iter()
            .map(|p| path_text(p).map(str::to_owned))
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(json)
}
fn phase(db: &Connection, id: &str, p: StoragePhase) -> Result<(), StorageError> {
    if db
        .execute(
            "UPDATE transfers SET phase=?1 WHERE id=?2",
            params![p.text(), id],
        )
        .map_err(sql)?
        != 1
    {
        return Err(err("ledger_transfer"));
    }
    Ok(())
}
fn entries(db: &Connection, id: &str) -> Result<Vec<Entry>, StorageError> {
    let mut q=db.prepare("SELECT relative_path,directory,length,sha256 FROM entries WHERE transfer_id=?1 ORDER BY ordinal").map_err(sql)?;
    let rows = q
        .query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(sql)?;
    let mut out = Vec::new();
    for row in rows {
        let (relative, directory, length, sha256) = row.map_err(sql)?;
        if length < 0 || !matches!(directory, 0 | 1) {
            return Err(err("ledger_entry"));
        }
        validate_relative(&relative)?;
        out.push(Entry {
            relative,
            directory: directory == 1,
            length: length as u64,
            sha256,
        });
    }
    Ok(out)
}
fn validate_relative(value: &str) -> Result<(), StorageError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(err("manifest_path"));
    }
    for p in value.split('/') {
        if p.is_empty() || p == "." || p == ".." || p.contains(':') || p.ends_with(['.', ' ']) {
            return Err(err("manifest_path"));
        }
    }
    if !path
        .components()
        .all(|p| matches!(p, std::path::Component::Normal(_)))
    {
        return Err(err("manifest_path"));
    }
    Ok(())
}
fn hash_file(
    path: &Path,
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
) -> Result<(u64, String), StorageError> {
    let mut file = asset_locations::open_regular(path).map_err(ioe)?;
    let initial = file.metadata().map_err(ioe)?.len();
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        hook(Checkpoint::HashChunk)?;
        let n = file.read(&mut buffer).map_err(ioe)?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .ok_or_else(|| err("file_too_large"))?;
        digest.update(&buffer[..n]);
    }
    if initial != size || file.metadata().map_err(ioe)?.len() != initial {
        return Err(err("source_changed"));
    }
    Ok((size, format!("{:x}", digest.finalize())))
}
fn inventory(
    root: &Path,
    exclude: Option<&str>,
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
) -> Result<Vec<Entry>, StorageError> {
    fn visit(
        root: &Path,
        path: &Path,
        exclude: Option<&str>,
        output: &mut Vec<Entry>,
        hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
    ) -> Result<(), StorageError> {
        let _pin = asset_locations::pin_directory(path).map_err(ioe)?;
        for item in fs::read_dir(path).map_err(ioe)? {
            hook(Checkpoint::Inventory)?;
            let item = item.map_err(ioe)?;
            let path = item.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| err("inventory_path"))?
                .components()
                .map(|p| {
                    p.as_os_str()
                        .to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| err("path_encoding"))
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("/");
            validate_relative(&relative)?;
            if exclude == Some(relative.as_str()) {
                continue;
            }
            let m = fs::symlink_metadata(&path).map_err(ioe)?;
            if m.is_dir() {
                asset_locations::ordinary(&m, true).map_err(ioe)?;
                output.push(Entry {
                    relative,
                    directory: true,
                    length: 0,
                    sha256: String::new(),
                });
                visit(root, &path, None, output, hook)?;
            } else {
                asset_locations::ordinary(&m, false).map_err(ioe)?;
                let (length, sha256) = hash_file(&path, hook)?;
                if length > i64::MAX as u64 {
                    return Err(err("file_too_large"));
                }
                output.push(Entry {
                    relative,
                    directory: false,
                    length,
                    sha256,
                });
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    visit(root, root, exclude, &mut output, hook)?;
    output.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(output)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checkpoint {
    Inventory,
    HashChunk,
    PlanDurable,
    OwnerDurable,
    BeforeCopy,
    CopyChunk,
    FileDurable,
    VerifiedDurable,
    SourceMarkerDurable,
    ActivatedDurable,
}
pub fn resolve_startup(paths: &StoragePaths) -> Result<ResolvedStorage, StorageError> {
    resolve_startup_with_hook(paths, &mut |_| Ok(()))
}
pub fn resolve_startup_with_hook(
    paths: &StoragePaths,
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
) -> Result<ResolvedStorage, StorageError> {
    validate_paths(paths)?;
    if !ledger_path(paths).exists() {
        initialize(paths)?;
    }
    let mut db = open_ledger(paths, true)?;
    let mut p = profile(&db)?;
    if let Some(id) = p.pending.clone() {
        let mut t = transfer(&db, &id)?;
        match t.phase {
            StoragePhase::Requested | StoragePhase::Copying | StoragePhase::Verified => {
                if p.generation != t.generation || !same(&p.root, &t.source)? {
                    return Err(err("migration_source_cas"));
                }
                match perform_copy(&mut db, &p, &mut t, hook) {
                    Ok(()) => {}
                    Err(e) => {
                        if e.code != "interrupted" {
                            phase(
                                &db,
                                &id,
                                if matches!(
                                    e.code,
                                    "needs_recovery" | "source_changed" | "root_identity_changed"
                                ) {
                                    StoragePhase::NeedsRecovery
                                } else {
                                    StoragePhase::Failed
                                },
                            )?;
                        }
                        return Err(e);
                    }
                }
                p = profile(&db)?;
            }
            StoragePhase::Activated => {}
            StoragePhase::Committed => return Err(err("ledger_committed_pending")),
            StoragePhase::NeedsRecovery | StoragePhase::Failed => {
                return Err(err("migration_needs_recovery"))
            }
            StoragePhase::RolledBack | StoragePhase::Idle => {
                return Err(err("ledger_pending_phase"))
            }
        }
    }
    let root_pin = pin_root(&p.root)?;
    if root_pin.stamp != p.stamp {
        return Err(err("active_root_identity_changed"));
    }
    let db_pins = required_db_pins(&p.root, &p.required_dbs)?;
    let trial = if let Some(id) = &p.pending {
        let t = transfer(&db, id)?;
        if t.phase != StoragePhase::Activated || !same(&p.root, &t.target)? {
            return Err(err("ledger_trial"));
        }
        if root_pin.stamp != t.target_stamp {
            return Err(err("root_identity_changed"));
        }
        exact_control(
            &t.target.join(control_dir(&t)).join(OWNER_FILE),
            &marker(&p, &t)?,
        )?;
        Some(StartupTrial {
            id: id.clone(),
            dataset_id: p.dataset_id.clone(),
            generation: p.generation,
            source: t.source,
            target: t.target,
        })
    } else {
        None
    };
    Ok(ResolvedStorage {
        root: p.root.clone(),
        generation: p.generation,
        dataset_id: p.dataset_id,
        locations: AssetLocations::new(p.root.join("assets"), p.aliases)
            .map_err(|_| err("ledger_alias"))?,
        trial,
        _root_pin: Arc::new(root_pin),
        _required_pins: Arc::new(db_pins),
    })
}
fn has_marker(root: &Path) -> Result<bool, StorageError> {
    if !root.exists() {
        return Ok(false);
    }
    let _pin = pin_root(root)?;
    for e in fs::read_dir(root).map_err(ioe)? {
        let e = e.map_err(ioe)?;
        if e.file_name()
            .to_str()
            .is_some_and(|s| s.starts_with(SOURCE_MARKER_PREFIX))
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn initialize(paths: &StoragePaths) -> Result<(), StorageError> {
    // Missing ledger may never resurrect a retained, migrated-out source.
    if has_marker(&paths.legacy_root)? || has_marker(&paths.default_root)? {
        return Err(err("locator_missing_migrated_source"));
    }
    if paths.legacy_root.exists() && !paths.legacy_root.join("spaces.db").exists() {
        let _legacy = pin_root(&paths.legacy_root)?;
        if fs::read_dir(&paths.legacy_root)
            .map_err(ioe)?
            .next()
            .is_some()
        {
            return Err(err("legacy_root_needs_recovery"));
        }
    }
    let root = if paths.legacy_root.join("spaces.db").exists() {
        paths.legacy_root.clone()
    } else if paths.default_root.join("spaces.db").exists() {
        paths.default_root.clone()
    } else {
        create_root(&paths.default_root)?;
        if fs::read_dir(&paths.default_root)
            .map_err(ioe)?
            .next()
            .is_some()
        {
            return Err(err("default_root_not_empty"));
        }
        paths.default_root.clone()
    };
    let root_pin = pin_root(&root)?;
    let mut required = Vec::new();
    if root.join("spaces.db").exists() {
        required.push("spaces.db".to_owned());
        if root.join("plugins/plugins.db").exists() {
            required.push("plugins/plugins.db".to_owned());
        }
    }
    let _db_pins = required_db_pins(&root, &required)?;
    create_root(&paths.bootstrap)?;
    let _bootstrap = pin_root(&paths.bootstrap)?;
    let path = ledger_path(paths);
    let reservation = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(ioe)?;
    reservation.sync_all().map_err(ioe)?;
    drop(reservation);
    // If interrupted before this transaction commits, the existing invalid ledger
    // causes recovery, never implicit profile reinitialization.
    let mut db = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sql)?;
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
        .map_err(sql)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    tx.execute_batch(SCHEMA).map_err(sql)?;
    tx.pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(sql)?;
    tx.pragma_update(None, "user_version", 1).map_err(sql)?;
    tx.execute(
        "INSERT INTO profile VALUES(1,?1,0,?2,'[]',NULL,?3,?4)",
        params![
            Uuid::new_v4().to_string(),
            path_text(&root)?,
            serde_json::to_string(&root_pin.stamp).map_err(json)?,
            serde_json::to_string(&required).map_err(json)?
        ],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(())
}

pub fn request_migration(
    paths: &StoragePaths,
    active_root: &Path,
    expected_generation: u64,
    target: &Path,
) -> Result<String, StorageError> {
    let mut db = open_ledger(paths, true)?;
    let source = pin_root(active_root)?;
    let target_pin = pin_root(target)?;
    supports_copy_publication(target)?;
    if overlaps(active_root, target)? || overlaps(&paths.bootstrap, target)? {
        return Err(err("target_overlaps_data"));
    }
    if fs::read_dir(target).map_err(ioe)?.next().is_some() {
        return Err(err("target_not_empty"));
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let p = profile(&tx)?;
    if p.generation != expected_generation || !same(&p.root, active_root)? {
        return Err(err("storage_generation_changed"));
    }
    if p.pending.is_some() {
        return Err(err("migration_pending"));
    }
    if p.required_dbs.is_empty() {
        return Err(err("dataset_not_initialized"));
    }
    if source.stamp != p.stamp {
        return Err(err("active_root_identity_changed"));
    }
    let _required = required_db_pins(&p.root, &p.required_dbs)?;
    next_generation(p.generation)?;
    let id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO transfers VALUES(?1,?2,?3,?4,?5,?6,?7,'Requested',NULL)",
        params![
            id,
            path_text(active_root)?,
            path_text(target)?,
            sqlite_int(expected_generation)?,
            aliases_json(&p.aliases)?,
            serde_json::to_string(&source.stamp).map_err(json)?,
            serde_json::to_string(&target_pin.stamp).map_err(json)?
        ],
    )
    .map_err(sql)?;
    tx.execute("UPDATE profile SET pending_id=?1 WHERE id=1", [&id])
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(id)
}
pub fn cancel_requested(paths: &StoragePaths, id: &str) -> Result<bool, StorageError> {
    let mut db = open_ledger(paths, true)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let p = profile(&tx)?;
    if p.pending.as_deref() != Some(id) {
        return Ok(false);
    }
    let t = transfer(&tx, id)?;
    if t.phase != StoragePhase::Requested {
        return Ok(false);
    }
    phase(&tx, id, StoragePhase::RolledBack)?;
    tx.execute("UPDATE profile SET pending_id=NULL WHERE id=1", [])
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(true)
}
/// Explicit recovery for a failed *unactivated* copy. Leaves all source/target
/// bytes intact and retains its audit rows. A new attempt needs an empty target.
pub fn abandon_failed_migration(paths: &StoragePaths, id: &str) -> Result<bool, StorageError> {
    let mut db = open_ledger(paths, true)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let p = profile(&tx)?;
    if p.pending.as_deref() != Some(id) {
        return Ok(false);
    }
    let t = transfer(&tx, id)?;
    if !matches!(t.phase, StoragePhase::Failed | StoragePhase::NeedsRecovery)
        || p.generation != t.generation
        || !same(&p.root, &t.source)?
    {
        return Err(err("migration_not_abandonable"));
    }
    let _source = pin_root(&t.source)?;
    if _source.stamp != p.stamp || _source.stamp != t.source_stamp {
        return Err(err("active_root_identity_changed"));
    }
    let _required = required_db_pins(&p.root, &p.required_dbs)?;
    phase(&tx, id, StoragePhase::RolledBack)?;
    tx.execute("UPDATE profile SET pending_id=NULL WHERE id=1", [])
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(true)
}
pub fn commit_startup(paths: &StoragePaths, trial: &StartupTrial) -> Result<(), StorageError> {
    finish_trial(paths, trial, true)
}
pub fn rollback_startup(paths: &StoragePaths, trial: &StartupTrial) -> Result<(), StorageError> {
    finish_trial(paths, trial, false)
}
/// Call after Store + PluginStore initialization, before admitting user actions.
/// The required files must keep existing; their normal-growing bytes are not pinned.
pub fn mark_initialized(
    paths: &StoragePaths,
    resolved: &ResolvedStorage,
) -> Result<(), StorageError> {
    let mut db = open_ledger(paths, true)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let p = profile(&tx)?;
    if p.pending.is_some()
        || p.generation != resolved.generation
        || p.dataset_id != resolved.dataset_id
        || !same(&p.root, &resolved.root)?
    {
        return Err(err("startup_state_changed"));
    }
    let root = pin_root(&p.root)?;
    if root.stamp != p.stamp {
        return Err(err("active_root_identity_changed"));
    }
    let required = vec!["spaces.db".to_owned(), "plugins/plugins.db".to_owned()];
    let _db_pins = required_db_pins(&p.root, &required)?;
    tx.execute(
        "UPDATE profile SET required_dbs=?1 WHERE id=1",
        [serde_json::to_string(&required).map_err(json)?],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(())
}
fn required_db_pins(root: &Path, required: &[String]) -> Result<Vec<File>, StorageError> {
    let mut pins = Vec::new();
    for relative in required {
        let path = root.join(relative);
        let parent = path
            .parent()
            .ok_or_else(|| err("required_database_missing"))?;
        let parents =
            asset_locations::pin_ancestors(parent).map_err(|_| err("required_database_missing"))?;
        pins.extend(parents);
        asset_locations::ordinary(
            &fs::symlink_metadata(&path).map_err(|_| err("required_database_missing"))?,
            false,
        )
        .map_err(|_| err("required_database_missing"))?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x00200000);
        }
        let mut file = options
            .open(path)
            .map_err(|_| err("required_database_missing"))?;
        asset_locations::ordinary(&file.metadata().map_err(ioe)?, false)
            .map_err(|_| err("required_database_missing"))?;
        let mut magic = [0u8; 16];
        if file.metadata().map_err(ioe)?.len() < 100
            || file.read_exact(&mut magic).is_err()
            || &magic != b"SQLite format 3\0"
        {
            return Err(err("required_database_invalid"));
        }
        pins.push(file);
    }
    Ok(pins)
}
fn finish_trial(
    paths: &StoragePaths,
    trial: &StartupTrial,
    commit: bool,
) -> Result<(), StorageError> {
    let mut db = open_ledger(paths, true)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let p = profile(&tx)?;
    let t = transfer(&tx, &trial.id)?;
    if t.phase == StoragePhase::Committed
        && commit
        && p.pending.is_none()
        && p.generation == trial.generation
        && p.dataset_id == trial.dataset_id
        && same(&p.root, &trial.target)?
    {
        return Ok(());
    }
    if p.pending.as_deref() != Some(trial.id.as_str())
        || p.generation != trial.generation
        || p.dataset_id != trial.dataset_id
        || t.phase != StoragePhase::Activated
        || !same(&p.root, &trial.target)?
        || !same(&t.source, &trial.source)?
        || !same(&t.target, &trial.target)?
    {
        return Err(err("startup_trial_stale"));
    }
    let target_guard = pin_root(&t.target)?;
    if target_guard.stamp != t.target_stamp {
        return Err(err("root_identity_changed"));
    }
    let source_guard = if !commit {
        Some(pin_root(&t.source)?)
    } else {
        None
    };
    if source_guard
        .as_ref()
        .is_some_and(|p| p.stamp != t.source_stamp)
    {
        return Err(err("root_identity_changed"));
    }
    if commit {
        let required = vec!["spaces.db".to_owned(), "plugins/plugins.db".to_owned()];
        let _pins = required_db_pins(&t.target, &required)?;
        phase(&tx, &trial.id, StoragePhase::Committed)?;
        tx.execute(
            "UPDATE profile SET pending_id=NULL,required_dbs=?1 WHERE id=1",
            [serde_json::to_string(&required).map_err(json)?],
        )
        .map_err(sql)?;
    } else {
        phase(&tx, &trial.id, StoragePhase::RolledBack)?;
        tx.execute(
            "UPDATE profile SET active_root=?1,aliases=?2,generation=?3,pending_id=NULL,active_identity=?4 WHERE id=1",
            params![
                path_text(&t.source)?,
                aliases_json(&t.aliases)?,
                sqlite_int(next_generation(p.generation)?)?,serde_json::to_string(&t.source_stamp).map_err(json)?
            ],
        )
        .map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    Ok(())
}
pub fn readonly_state(paths: &StoragePaths) -> Result<Option<StorageState>, StorageError> {
    validate_paths(paths)?;
    if !ledger_path(paths).exists() {
        return Ok(None);
    }
    let db = open_ledger(paths, false)?;
    let tx = db.unchecked_transaction().map_err(sql)?;
    let p = profile(&tx)?;
    let root = pin_root(&p.root)?;
    if root.stamp != p.stamp {
        return Err(err("active_root_identity_changed"));
    }
    let _required = required_db_pins(&p.root, &p.required_dbs)?;
    let phase = if let Some(id) = &p.pending {
        transfer(&tx, id)?.phase
    } else {
        tx.query_row(
            "SELECT phase FROM transfers ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql)?
        .map(|s| StoragePhase::parse(&s))
        .transpose()?
        .unwrap_or(StoragePhase::Idle)
    };
    Ok(Some(StorageState {
        root: p.root,
        generation: p.generation,
        dataset_id: p.dataset_id,
        phase,
        pending_id: p.pending,
    }))
}

fn control_dir(t: &Transfer) -> String {
    format!(".mewu-transfer-{}", t.id)
}
fn marker(p: &Profile, t: &Transfer) -> Result<Marker, StorageError> {
    Ok(Marker {
        version: 1,
        id: t.id.clone(),
        dataset_id: p.dataset_id.clone(),
        source: path_text(&t.source)?.into(),
        target: path_text(&t.target)?.into(),
        source_generation: t.generation,
        plan_sha256: t.plan.clone().ok_or_else(|| err("plan_missing"))?,
    })
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0).custom_flags(0x00200000);
    }
    let mut file = options.open(path).map_err(ioe)?;
    file.write_all(bytes).map_err(ioe)?;
    file.sync_all().map_err(ioe)
}
fn exact_control(path: &Path, expected: &Marker) -> Result<(), StorageError> {
    let mut file = asset_locations::open_regular(path).map_err(ioe)?;
    if file.metadata().map_err(ioe)?.len() > 8192 {
        return Err(err("owner_marker_invalid"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(ioe)?;
    let actual: Marker = serde_json::from_slice(&bytes).map_err(json)?;
    if &actual != expected {
        return Err(err("owner_marker_mismatch"));
    }
    Ok(())
}
fn perform_copy(
    db: &mut Connection,
    p: &Profile,
    t: &mut Transfer,
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let source = pin_root(&t.source)?;
    let target = pin_root(&t.target)?;
    supports_copy_publication(&t.target)?;
    if source.stamp != t.source_stamp || target.stamp != t.target_stamp {
        return Err(err("root_identity_changed"));
    }
    let control = control_dir(t);
    let work = t.target.join(&control);
    if t.phase == StoragePhase::Requested {
        if fs::read_dir(&t.target).map_err(ioe)?.next().is_some() {
            return Err(err("target_not_empty"));
        }
        let plan = inventory(&t.source, None, hook)?;
        reject_journals(&plan)?;
        let digest = sha(&serde_json::to_vec(&plan).map_err(json)?);
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        for (n, e) in plan.iter().enumerate() {
            tx.execute(
                "INSERT INTO entries VALUES(?1,?2,?3,?4,?5,?6,NULL)",
                params![
                    t.id,
                    n as i64,
                    e.relative,
                    e.directory as i64,
                    sqlite_int(e.length)?,
                    e.sha256
                ],
            )
            .map_err(sql)?;
        }
        tx.execute(
            "UPDATE transfers SET phase='Copying',plan_sha256=?1 WHERE id=?2",
            params![digest, t.id],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        t.plan = Some(digest);
        t.phase = StoragePhase::Copying;
        hook(Checkpoint::PlanDurable)?;
    }
    let plan = entries(db, &t.id)?;
    if Some(sha(&serde_json::to_vec(&plan).map_err(json)?)) != t.plan {
        return Err(err("plan_digest_changed"));
    }
    let expected = marker(p, t)?;
    if !work.exists() {
        if fs::read_dir(&t.target).map_err(ioe)?.next().is_some() {
            return Err(err("target_unowned_files"));
        }
        fs::create_dir(&work).map_err(ioe)?;
        write_new(
            &work.join(OWNER_FILE),
            &serde_json::to_vec(&expected).map_err(json)?,
        )?;
        hook(Checkpoint::OwnerDurable)?;
    }
    let _work = pin_root(&work)?;
    exact_control(&work.join(OWNER_FILE), &expected)?;
    validate_target_partial(db, t, &plan, &control, hook)?;
    if t.phase == StoragePhase::Copying {
        if inventory(&t.source, None, hook)? != plan {
            return Err(err("source_changed"));
        }
        for (n, e) in plan.iter().enumerate() {
            hook(Checkpoint::BeforeCopy)?;
            let output = t.target.join(&e.relative);
            if e.directory {
                if !output.exists() {
                    let parent = output.parent().ok_or_else(|| err("manifest_path"))?;
                    let _parent = pin_root(parent)?;
                    fs::create_dir(&output).map_err(ioe)?;
                }
                let _pin = pin_root(&output)?;
                continue;
            }
            copy_entry(db, t, n, e, &work, hook)?;
        }
        validate_databases(&t.target, &work, &plan, hook)?;
        if inventory(&t.source, None, hook)? != plan
            || inventory(&t.target, Some(&control), hook)? != plan
        {
            return Err(err("source_changed"));
        }
        phase(db, &t.id, StoragePhase::Verified)?;
        t.phase = StoragePhase::Verified;
        hook(Checkpoint::VerifiedDurable)?;
    }
    if t.phase != StoragePhase::Verified {
        return Err(err("copy_phase"));
    }
    if inventory(&t.target, Some(&control), hook)? != plan {
        return Err(err("target_changed"));
    }
    let source_marker = format!("{SOURCE_MARKER_PREFIX}{}.json", t.id);
    let marker_path = t.source.join(&source_marker);
    // Existing markers are accepted only with the exact same typed identity.
    if !marker_path.exists() {
        write_new(&marker_path, &serde_json::to_vec(&expected).map_err(json)?)?;
    }
    exact_control(&marker_path, &expected)?;
    if inventory(&t.source, Some(&source_marker), hook)? != plan {
        return Err(err("source_changed"));
    }
    hook(Checkpoint::SourceMarkerDurable)?;
    if inventory(&t.source, Some(&source_marker), hook)? != plan
        || inventory(&t.target, Some(&control), hook)? != plan
    {
        return Err(err("source_changed"));
    }
    validate_target_partial(db, t, &plan, &control, hook)?;
    exact_control(&work.join(OWNER_FILE), &expected)?;
    let mut aliases = t.aliases.clone();
    let old_assets = t.source.join("assets");
    if !aliases
        .iter()
        .any(|a| same(a, &old_assets).unwrap_or(false))
    {
        aliases.push(old_assets);
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let current = profile(&tx)?;
    if current.pending.as_deref() != Some(t.id.as_str())
        || current.generation != t.generation
        || !same(&current.root, &t.source)?
    {
        return Err(err("migration_source_cas"));
    }
    tx.execute(
        "UPDATE profile SET active_root=?1,generation=?2,aliases=?3,active_identity=?4 WHERE id=1",
        params![
            path_text(&t.target)?,
            sqlite_int(next_generation(t.generation)?)?,
            aliases_json(&aliases)?,
            serde_json::to_string(&t.target_stamp).map_err(json)?
        ],
    )
    .map_err(sql)?;
    phase(&tx, &t.id, StoragePhase::Activated)?;
    tx.commit().map_err(sql)?;
    t.phase = StoragePhase::Activated;
    hook(Checkpoint::ActivatedDurable)?;
    Ok(())
}
fn reject_journals(plan: &[Entry]) -> Result<(), StorageError> {
    // A nonempty rollback journal needs the owning DB's normal recovery first.
    // Never recover the retained source in the copier or ignore a WAL.
    if plan
        .iter()
        .any(|e| !e.directory && e.relative.ends_with("-journal") && e.length > 0)
    {
        return Err(err("needs_recovery"));
    }
    Ok(())
}
fn temp_path(work: &Path, ordinal: usize) -> PathBuf {
    work.join(format!("file-{ordinal}.partial"))
}
fn temp_identity(
    db: &Connection,
    id: &str,
    n: usize,
) -> Result<Option<FileIdentity>, StorageError> {
    let v: Option<String> = db
        .query_row(
            "SELECT temp_identity FROM entries WHERE transfer_id=?1 AND ordinal=?2",
            params![id, n as i64],
            |r| r.get(0),
        )
        .map_err(sql)?;
    v.map(|s| serde_json::from_str(&s).map_err(json))
        .transpose()
}
fn owned_temp(path: &Path, stamp: &FileIdentity) -> Result<(), StorageError> {
    let file = asset_locations::open_regular(path).map_err(ioe)?;
    if identity(&file)? != *stamp {
        return Err(err("temporary_identity_changed"));
    }
    Ok(())
}
fn copy_entry(
    db: &Connection,
    t: &Transfer,
    n: usize,
    e: &Entry,
    work: &Path,
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let output = t.target.join(&e.relative);
    let temp = temp_path(work, n);
    let tracked = temp_identity(db, &t.id, n)?;
    let _parent = pin_root(output.parent().ok_or_else(|| err("manifest_path"))?)?;
    if output.exists() {
        let actual = hash_file(&output, hook)?;
        if actual != (e.length, e.sha256.clone()) {
            return Err(err("target_file_conflict"));
        }
        if temp.exists() {
            delete_owned(&temp, &tracked.ok_or_else(|| err("unowned_temporary"))?)?;
        }
        return Ok(());
    }
    if temp.exists() {
        delete_owned(&temp, &tracked.ok_or_else(|| err("unowned_temporary"))?)?;
    }
    let source_path = t.source.join(&e.relative);
    let _source_parent = pin_root(source_path.parent().ok_or_else(|| err("manifest_path"))?)?;
    let mut source = asset_locations::open_regular(&source_path).map_err(ioe)?;
    if source.metadata().map_err(ioe)?.len() != e.length {
        return Err(err("source_changed"));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(0)
            .custom_flags(0x00200000)
            .access_mode(0x40010000);
    }
    let mut file = options.open(&temp).map_err(ioe)?;
    let stamp = identity(&file)?;
    db.execute(
        "UPDATE entries SET temp_identity=?1 WHERE transfer_id=?2 AND ordinal=?3",
        params![serde_json::to_string(&stamp).map_err(json)?, t.id, n as i64],
    )
    .map_err(sql)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut count = 0u64;
    let mut digest = Sha256::new();
    loop {
        hook(Checkpoint::CopyChunk)?;
        let bytes = source.read(&mut buffer).map_err(ioe)?;
        if bytes == 0 {
            break;
        }
        count = count
            .checked_add(bytes as u64)
            .ok_or_else(|| err("file_too_large"))?;
        if count > e.length {
            return Err(err("source_changed"));
        }
        file.write_all(&buffer[..bytes]).map_err(ioe)?;
        digest.update(&buffer[..bytes]);
    }
    if count != e.length || format!("{:x}", digest.finalize()) != e.sha256 {
        return Err(err("source_changed"));
    }
    file.sync_all().map_err(ioe)?;
    publish_file(&file, &temp, &output, &stamp)?;
    file.sync_all().map_err(ioe)?;
    drop(file);
    hook(Checkpoint::FileDurable)?;
    Ok(())
}
fn open_mutation(path: &Path, directory: bool) -> Result<File, StorageError> {
    asset_locations::ordinary(&fs::symlink_metadata(path).map_err(ioe)?, directory).map_err(ioe)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x80010000)
            .share_mode(if directory { 3 } else { 1 })
            .custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 });
    }
    let file = options.open(path).map_err(ioe)?;
    asset_locations::ordinary(&file.metadata().map_err(ioe)?, directory).map_err(ioe)?;
    Ok(file)
}
#[cfg(windows)]
fn disposition(file: &File) -> Result<(), StorageError> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            file: *mut std::ffi::c_void,
            class: i32,
            info: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    let delete = 1u8;
    if unsafe {
        SetFileInformationByHandle(file.as_raw_handle(), 4, (&delete as *const u8).cast(), 1)
    } == 0
    {
        return Err(ioe(std::io::Error::last_os_error()));
    }
    Ok(())
}
fn delete_owned(path: &Path, expected: &FileIdentity) -> Result<(), StorageError> {
    let file = open_mutation(path, false)?;
    if identity(&file)? != *expected {
        return Err(err("temporary_identity_changed"));
    }
    #[cfg(windows)]
    {
        disposition(&file)?;
        drop(file);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        drop(file);
        fs::remove_file(path).map_err(ioe)
    }
}
fn publish_file(
    file: &File,
    temp: &Path,
    output: &Path,
    expected: &FileIdentity,
) -> Result<(), StorageError> {
    if identity(file)? != *expected {
        return Err(err("temporary_identity_changed"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        #[repr(C)]
        struct Rename {
            flags: u32,
            root: *mut std::ffi::c_void,
            length: u32,
            name: [u16; 1],
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn SetFileInformationByHandle(
                file: *mut std::ffi::c_void,
                class: i32,
                info: *const std::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        let name = output.as_os_str().encode_wide().collect::<Vec<_>>();
        let offset = std::mem::offset_of!(Rename, name);
        let bytes = offset
            .checked_add(
                (name.len() + 1)
                    .checked_mul(2)
                    .ok_or_else(|| err("path_too_long"))?,
            )
            .ok_or_else(|| err("path_too_long"))?;
        if bytes > u32::MAX as usize {
            return Err(err("path_too_long"));
        }
        let words = bytes.div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0usize; words];
        let info = storage.as_mut_ptr().cast::<Rename>();
        unsafe {
            (*info).flags = 0;
            (*info).root = std::ptr::null_mut();
            (*info).length = (name.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                storage.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
                name.len(),
            );
        }
        if unsafe { SetFileInformationByHandle(file.as_raw_handle(), 3, info.cast(), bytes as u32) }
            == 0
        {
            return Err(ioe(std::io::Error::last_os_error()));
        }
        let _ = temp;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(temp, output).map_err(ioe)?;
        fs::remove_file(temp).map_err(ioe)
    }
}
fn pin_owned_directory(path: &Path) -> Result<RootPin, StorageError> {
    let mut parents =
        asset_locations::pin_ancestors(path.parent().ok_or_else(|| err("root_parent"))?)
            .map_err(ioe)?;
    let file = open_mutation(path, true)?;
    let stamp = identity(&file)?;
    parents.push(file);
    Ok(RootPin {
        _parents: parents,
        stamp,
    })
}
fn validate_target_partial(
    db: &Connection,
    t: &Transfer,
    plan: &[Entry],
    control: &str,
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let present = inventory(&t.target, Some(control), hook)?;
    for e in present {
        let expected = plan
            .iter()
            .find(|p| p.relative == e.relative)
            .ok_or_else(|| err("target_unowned_files"))?;
        if &e != expected {
            return Err(err("target_file_conflict"));
        }
    }
    let work = t.target.join(control);
    for child in fs::read_dir(&work).map_err(ioe)? {
        let child = child.map_err(ioe)?;
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| err("path_encoding"))?;
        if name == OWNER_FILE {
            continue;
        }
        let n = plan
            .iter()
            .enumerate()
            .find(|(n, _)| name == format!("file-{n}.partial"))
            .map(|(n, _)| n)
            .ok_or_else(|| err("target_unowned_files"))?;
        if plan[n].directory {
            return Err(err("target_unowned_files"));
        }
        owned_temp(
            &child.path(),
            &temp_identity(db, &t.id, n)?.ok_or_else(|| err("unowned_temporary"))?,
        )?;
    }
    Ok(())
}
fn validate_databases(
    root: &Path,
    work: &Path,
    plan: &[Entry],
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    for (n, e) in plan.iter().enumerate() {
        if e.directory {
            continue;
        }
        let source_path = root.join(&e.relative);
        let _source_parents = pin_root(source_path.parent().ok_or_else(|| err("manifest_path"))?)?;
        let mut source = asset_locations::open_regular(&source_path).map_err(ioe)?;
        let mut header = [0u8; 16];
        let bytes = source.read(&mut header).map_err(ioe)?;
        if bytes != 16 || &header != b"SQLite format 3\0" {
            continue;
        }
        hook(Checkpoint::BeforeCopy)?;
        let scratch = work.join(format!("db-{n}.scratch"));
        if scratch.exists() {
            return Err(err("unowned_database_scratch"));
        }
        fs::create_dir(&scratch).map_err(ioe)?;
        let scratch_guard = pin_owned_directory(&scratch)?;
        let mut known = Vec::new();
        let result = (|| {
            let main = scratch.join("check.db");
            scratch_copy(&root.join(&e.relative), &main, hook, &mut known)?;
            for suffix in ["-wal", "-shm", "-journal"] {
                let relative = format!("{}{suffix}", e.relative);
                let target = scratch.join(format!("check.db{suffix}"));
                if plan.iter().any(|p| p.relative == relative) {
                    scratch_copy(&root.join(&relative), &target, hook, &mut known)?;
                } else {
                    let file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)
                        .map_err(ioe)?;
                    known.push((target, identity(&file)?));
                    file.sync_all().map_err(ioe)?;
                }
            }
            let mut pins = Vec::new();
            for (path, expected) in &known {
                let mut options = OpenOptions::new();
                options.read(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.share_mode(3).custom_flags(0x00200000);
                }
                let file = options.open(path).map_err(ioe)?;
                if identity(&file)? != *expected {
                    return Err(err("temporary_identity_changed"));
                }
                pins.push(file);
            }
            let db = Connection::open_with_flags(
                &main,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(sql)?;
            let result: String = db
                .query_row("PRAGMA quick_check", [], |r| r.get(0))
                .map_err(sql)?;
            if result != "ok" {
                return Err(err("database_invalid"));
            }
            let app_state:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='app_state')",[],|r|r.get(0)).map_err(sql)?;
            if app_state {
                let payload: String = db
                    .query_row("SELECT payload FROM app_state WHERE id=1", [], |r| r.get(0))
                    .map_err(sql)?;
                let value: serde_json::Value = serde_json::from_str(&payload).map_err(json)?;
                if value
                    .get("scenes")
                    .and_then(|v| v.as_array())
                    .is_some_and(|scenes| {
                        scenes.iter().any(|s| {
                            s.get("run")
                                .and_then(|r| r.get("status"))
                                .and_then(|s| s.as_str())
                                == Some("running")
                        })
                    })
                {
                    return Err(err("needs_recovery"));
                }
            }
            let intents:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='file_intent')",[],|r|r.get(0)).map_err(sql)?;
            if intents {
                let count: i64 = db
                    .query_row("SELECT COUNT(*) FROM file_intent", [], |r| r.get(0))
                    .map_err(sql)?;
                if count < 0 {
                    return Err(err("database_invalid"));
                }
                if count > 0 {
                    return Err(err("needs_recovery"));
                }
            }
            drop(db);
            drop(pins);
            Ok(())
        })();
        // Never recursively delete a path merely because we created it earlier.
        // SQLite may create only these exact sibling files in this private scratch.
        // Unknown additions or a replaced directory are preserved for recovery.
        cleanup_scratch(&scratch, scratch_guard, &known)?;
        result?;
    }
    Ok(())
}
fn cleanup_scratch(
    path: &Path,
    guard: RootPin,
    known: &[(PathBuf, FileIdentity)],
) -> Result<(), StorageError> {
    let children = fs::read_dir(path)
        .map_err(ioe)?
        .map(|e| e.map_err(ioe))
        .collect::<Result<Vec<_>, _>>()?;
    let mut pinned = Vec::new();
    for child in children {
        let expected = known
            .iter()
            .find(|(p, _)| *p == child.path())
            .ok_or_else(|| err("unowned_database_scratch"))?;
        let file = open_mutation(&child.path(), false)?;
        if identity(&file)? != expected.1 {
            return Err(err("temporary_identity_changed"));
        }
        pinned.push((child.path(), file));
    }
    for (_path, file) in pinned {
        #[cfg(windows)]
        {
            disposition(&file)?;
            drop(file);
        }
        #[cfg(not(windows))]
        {
            drop(file);
            fs::remove_file(_path).map_err(ioe)?;
        }
    }
    #[cfg(windows)]
    {
        disposition(guard._parents.last().ok_or_else(|| err("root_pin"))?)?;
        drop(guard);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        drop(guard);
        fs::remove_dir(path).map_err(ioe)
    }
}
fn scratch_copy(
    source: &Path,
    target: &Path,
    hook: &mut dyn FnMut(Checkpoint) -> Result<(), StorageError>,
    known: &mut Vec<(PathBuf, FileIdentity)>,
) -> Result<(), StorageError> {
    let _parent = pin_root(source.parent().ok_or_else(|| err("manifest_path"))?)?;
    let mut source = asset_locations::open_regular(source).map_err(ioe)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0).custom_flags(0x00200000);
    }
    let mut file = options.open(target).map_err(ioe)?;
    known.push((target.to_path_buf(), identity(&file)?));
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        hook(Checkpoint::CopyChunk)?;
        let n = source.read(&mut buffer).map_err(ioe)?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).map_err(ioe)?;
    }
    file.sync_all().map_err(ioe)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        base: PathBuf,
        paths: StoragePaths,
        target: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let base =
                std::env::temp_dir().join(format!("mewu-storage-synthetic-{}", Uuid::new_v4()));
            assert!(!base
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("c:\\hermes"));
            fs::create_dir(&base).unwrap();
            let source = base.join("legacy");
            let target = base.join("target");
            fs::create_dir(&source).unwrap();
            fs::create_dir(&target).unwrap();
            fs::create_dir(source.join("assets")).unwrap();
            fs::create_dir(source.join("empty")).unwrap();
            fs::create_dir_all(source.join("plugins/packages/example/1.0")).unwrap();
            fs::create_dir_all(source.join("connections/id")).unwrap();
            fs::write(source.join("assets/picture.bin"), b"original image bytes").unwrap();
            fs::write(source.join("connections/id/key.bin"), [0, 1, 2, 255, 9]).unwrap();
            fs::write(source.join("unknown.bin"), b"unrecognized plugin data").unwrap();
            fs::write(
                source.join("plugins/packages/example/1.0/plugin.json"),
                b"{\"id\":\"example\"}",
            )
            .unwrap();
            let db = Connection::open(source.join("spaces.db")).unwrap();
            db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,payload TEXT);CREATE TABLE history(id INTEGER PRIMARY KEY,payload BLOB);INSERT INTO app_state VALUES(1,' { \"scenes\": [] } ');INSERT INTO history VALUES(17,X'0001FEFF');PRAGMA user_version=8;").unwrap();
            drop(db);
            let plugins = Connection::open(source.join("plugins/plugins.db")).unwrap();
            plugins.execute_batch("CREATE TABLE file_intent(slot INTEGER PRIMARY KEY,payload TEXT);CREATE TABLE registry(id TEXT PRIMARY KEY,payload TEXT);INSERT INTO registry VALUES('example',' { \"enabled\":true } ');").unwrap();
            drop(plugins);
            let paths = StoragePaths {
                bootstrap: base.join("bootstrap"),
                legacy_root: source,
                default_root: base.join(".mewu"),
            };
            Self {
                base,
                paths,
                target,
            }
        }
        fn initial(&self) -> ResolvedStorage {
            resolve_startup(&self.paths).unwrap()
        }
        fn request(&self) -> String {
            let r = self.initial();
            request_migration(&self.paths, &r.root, r.generation, &self.target).unwrap()
        }
        fn source_plan(&self) -> Vec<Entry> {
            inventory(&self.paths.legacy_root, None, &mut |_| Ok(())).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
    fn fail_once(at: Checkpoint) -> impl FnMut(Checkpoint) -> Result<(), StorageError> {
        let mut fired = false;
        move |p| {
            if p == at && !fired {
                fired = true;
                Err(StorageError::interrupted())
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn copies_whole_tree_raw_sqlite_ciphertext_empty_dirs_and_aliases() {
        let f = Fixture::new();
        let before = f.source_plan();
        let id = f.request();
        let resolved = resolve_startup(&f.paths).unwrap();
        assert_eq!(resolved.root, f.target);
        assert_eq!(resolved.generation, 1);
        let target = inventory(
            &f.target,
            Some(&format!(".mewu-transfer-{id}")),
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(before, target);
        let source = inventory(
            &f.paths.legacy_root,
            Some(&format!("{SOURCE_MARKER_PREFIX}{id}.json")),
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(before, source);
        let mapped = resolved
            .locations
            .resolve_path(
                &f.paths.legacy_root.join("assets/picture.bin"),
                &f.target.join("assets"),
            )
            .unwrap();
        assert_eq!(fs::read(mapped).unwrap(), b"original image bytes");
        commit_startup(&f.paths, resolved.trial.as_ref().unwrap()).unwrap();
        let state = readonly_state(&f.paths).unwrap().unwrap();
        assert_eq!(state.phase, StoragePhase::Committed);
        assert_eq!(state.root, f.target);
    }
    #[test]
    fn requested_cancel_is_exact_and_stale_generation_rejected() {
        let f = Fixture::new();
        let r = f.initial();
        assert_eq!(
            request_migration(&f.paths, &r.root, 9, &f.target)
                .unwrap_err()
                .code,
            "storage_generation_changed"
        );
        let id = f.request();
        assert!(!cancel_requested(&f.paths, "other").unwrap());
        assert!(cancel_requested(&f.paths, &id).unwrap());
        assert!(!cancel_requested(&f.paths, &id).unwrap());
        assert_eq!(f.initial().root, f.paths.legacy_root);
        assert!(fs::read_dir(&f.target).unwrap().next().is_none());
    }
    #[test]
    fn fixed_plan_and_partial_file_resume_same_uuid() {
        for checkpoint in [
            Checkpoint::PlanDurable,
            Checkpoint::OwnerDurable,
            Checkpoint::CopyChunk,
            Checkpoint::FileDurable,
            Checkpoint::VerifiedDurable,
            Checkpoint::SourceMarkerDurable,
            Checkpoint::ActivatedDurable,
        ] {
            let f = Fixture::new();
            let before = f.source_plan();
            let id = f.request();
            let e = resolve_startup_with_hook(&f.paths, &mut fail_once(checkpoint)).unwrap_err();
            assert_eq!(e.code, "interrupted");
            let r = f.initial();
            assert_eq!(r.trial.as_ref().unwrap().id(), id);
            assert_eq!(
                inventory(
                    &f.target,
                    Some(&format!(".mewu-transfer-{id}")),
                    &mut |_| Ok(())
                )
                .unwrap(),
                before
            );
            commit_startup(&f.paths, r.trial.as_ref().unwrap()).unwrap();
        }
    }
    #[test]
    fn source_changed_after_plan_never_activates_and_explicit_abandon_preserves_target() {
        let f = Fixture::new();
        let id = f.request();
        assert_eq!(
            resolve_startup_with_hook(&f.paths, &mut fail_once(Checkpoint::PlanDurable))
                .unwrap_err()
                .code,
            "interrupted"
        );
        fs::write(f.paths.legacy_root.join("unknown.bin"), b"changed").unwrap();
        assert_eq!(
            resolve_startup(&f.paths).unwrap_err().code,
            "source_changed"
        );
        assert_eq!(
            readonly_state(&f.paths).unwrap().unwrap().root,
            f.paths.legacy_root
        );
        assert!(abandon_failed_migration(&f.paths, &id).unwrap());
        assert_eq!(f.initial().root, f.paths.legacy_root);
        assert_eq!(
            fs::read(f.paths.legacy_root.join("unknown.bin")).unwrap(),
            b"changed"
        );
    }
    #[test]
    fn target_unknown_file_never_overwritten_or_deleted() {
        let f = Fixture::new();
        let id = f.request();
        assert_eq!(
            resolve_startup_with_hook(&f.paths, &mut fail_once(Checkpoint::OwnerDurable))
                .unwrap_err()
                .code,
            "interrupted"
        );
        fs::write(f.target.join("mine.txt"), b"do not erase").unwrap();
        assert_eq!(
            resolve_startup(&f.paths).unwrap_err().code,
            "target_unowned_files"
        );
        assert_eq!(
            fs::read(f.target.join("mine.txt")).unwrap(),
            b"do not erase"
        );
        assert!(abandon_failed_migration(&f.paths, &id).unwrap());
    }
    #[test]
    fn unknown_temp_and_replaced_temp_identity_fail_closed() {
        let f = Fixture::new();
        let id = f.request();
        assert_eq!(
            resolve_startup_with_hook(&f.paths, &mut fail_once(Checkpoint::CopyChunk))
                .unwrap_err()
                .code,
            "interrupted"
        );
        let work = f.target.join(format!(".mewu-transfer-{id}"));
        fs::write(work.join("surprise.partial"), b"private file").unwrap();
        assert_eq!(
            resolve_startup(&f.paths).unwrap_err().code,
            "target_unowned_files"
        );
        assert_eq!(
            fs::read(work.join("surprise.partial")).unwrap(),
            b"private file"
        );
        let f = Fixture::new();
        let id = f.request();
        assert_eq!(
            resolve_startup_with_hook(&f.paths, &mut fail_once(Checkpoint::CopyChunk))
                .unwrap_err()
                .code,
            "interrupted"
        );
        let work = f.target.join(format!(".mewu-transfer-{id}"));
        let temp = fs::read_dir(&work)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "partial"))
            .unwrap();
        fs::rename(&temp, work.join("old-kept.bin")).unwrap();
        fs::write(&temp, b"replacement").unwrap();
        assert!(resolve_startup(&f.paths).is_err());
        assert_eq!(fs::read(temp).unwrap(), b"replacement");
    }
    #[test]
    fn running_run_plugin_intent_and_rollback_journal_require_separate_recovery() {
        for case in 0..3 {
            let f = Fixture::new();
            if case == 0 {
                let db = Connection::open(f.paths.legacy_root.join("spaces.db")).unwrap();
                db.execute(
                    "UPDATE app_state SET payload=?1",
                    ["{\"scenes\":[{\"run\":{\"status\":\"running\"}}]}"],
                )
                .unwrap();
            } else if case == 1 {
                let db = Connection::open(f.paths.legacy_root.join("plugins/plugins.db")).unwrap();
                db.execute("INSERT INTO file_intent VALUES(1,'{}')", [])
                    .unwrap();
            } else {
                fs::write(
                    f.paths.legacy_root.join("spaces.db-journal"),
                    b"hot journal evidence",
                )
                .unwrap();
            }
            let before = f.source_plan();
            let id = f.request();
            assert_eq!(
                resolve_startup(&f.paths).unwrap_err().code,
                "needs_recovery"
            );
            assert_eq!(before, f.source_plan());
            assert_eq!(
                readonly_state(&f.paths).unwrap().unwrap().phase,
                StoragePhase::NeedsRecovery
            );
            assert!(abandon_failed_migration(&f.paths, &id).unwrap());
        }
    }
    #[test]
    fn wal_pair_is_read_logically_only_on_scratch_and_raw_copied() {
        let f = Fixture::new();
        let db = Connection::open(f.paths.legacy_root.join("spaces.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=0;INSERT INTO history VALUES(29,X'AABBCC');").unwrap();
        // Snapshot test-owned ordinary bytes while SQLite keeps its WAL live,
        // then close the writer and restore that synthetic cold WAL pair.
        let root = &f.paths.legacy_root;
        let files = ["spaces.db", "spaces.db-wal", "spaces.db-shm"]
            .into_iter()
            .map(|name| (name, fs::read(root.join(name)).unwrap()))
            .collect::<Vec<_>>();
        drop(db);
        for (name, bytes) in files {
            fs::write(root.join(name), bytes).unwrap();
        }
        let before = f.source_plan();
        let id = f.request();
        let r = resolve_startup(&f.paths).unwrap();
        assert_eq!(
            inventory(
                &f.target,
                Some(&format!(".mewu-transfer-{id}")),
                &mut |_| Ok(())
            )
            .unwrap(),
            before
        );
        assert_eq!(
            inventory(
                root,
                Some(&format!("{SOURCE_MARKER_PREFIX}{id}.json")),
                &mut |_| Ok(())
            )
            .unwrap(),
            before
        );
        commit_startup(&f.paths, r.trial.as_ref().unwrap()).unwrap();
    }
    #[test]
    fn activated_rollback_and_committed_never_fall_back() {
        let f = Fixture::new();
        f.request();
        let r = f.initial();
        let trial = r.trial.clone().unwrap();
        rollback_startup(&f.paths, &trial).unwrap();
        let old = f.initial();
        assert_eq!(old.root, f.paths.legacy_root);
        assert_eq!(old.generation, 2);
        assert!(commit_startup(&f.paths, &trial).is_err());
        let f = Fixture::new();
        f.request();
        let r = f.initial();
        let trial = r.trial.clone().unwrap();
        commit_startup(&f.paths, &trial).unwrap();
        commit_startup(&f.paths, &trial).unwrap();
        fs::write(f.target.join("new-after-commit.bin"), b"new user data").unwrap();
        assert!(rollback_startup(&f.paths, &trial).is_err());
        assert_eq!(f.initial().root, f.target);
        drop(r);
        fs::rename(&f.target, f.base.join("target-missing")).unwrap();
        assert!(resolve_startup(&f.paths).is_err());
    }
    #[test]
    fn missing_locator_never_resurrects_migrated_old_default() {
        let f = Fixture::new();
        f.request();
        let r = f.initial();
        commit_startup(&f.paths, r.trial.as_ref().unwrap()).unwrap();
        fs::remove_dir_all(&f.paths.bootstrap).unwrap();
        assert_eq!(
            resolve_startup(&f.paths).unwrap_err().code,
            "locator_missing_migrated_source"
        );
        assert!(!ledger_path(&f.paths).exists());
    }
    #[test]
    fn chained_migration_reads_only_current_files_even_if_both_old_roots_gone() {
        let f = Fixture::new();
        f.request();
        let r = f.initial();
        commit_startup(&f.paths, r.trial.as_ref().unwrap()).unwrap();
        let next = f.base.join("third");
        fs::create_dir(&next).unwrap();
        request_migration(&f.paths, &f.target, r.generation, &next).unwrap();
        let third = f.initial();
        commit_startup(&f.paths, third.trial.as_ref().unwrap()).unwrap();
        drop(r);
        fs::rename(&f.paths.legacy_root, f.base.join("legacy-gone")).unwrap();
        fs::rename(&f.target, f.base.join("target-gone")).unwrap();
        for old in [&f.paths.legacy_root, &f.target] {
            let path = third
                .locations
                .resolve_path(&old.join("assets/picture.bin"), &next.join("assets"))
                .unwrap();
            assert_eq!(fs::read(path).unwrap(), b"original image bytes");
        }
        fs::remove_file(next.join("assets/picture.bin")).unwrap();
        assert!(third
            .locations
            .resolve_path(
                &f.paths.legacy_root.join("assets/picture.bin"),
                &next.join("assets")
            )
            .is_err());
    }
    #[test]
    fn readonly_query_does_not_create_profile_or_touch_missing_root() {
        let f = Fixture::new();
        assert!(readonly_state(&f.paths).unwrap().is_none());
        assert!(!f.paths.bootstrap.exists());
        f.initial();
        let before = fs::read(ledger_path(&f.paths)).unwrap();
        assert!(readonly_state(&f.paths).unwrap().is_some());
        assert_eq!(fs::read(ledger_path(&f.paths)).unwrap(), before);
    }
    #[test]
    fn nonempty_target_and_ancestor_targets_are_rejected_without_mutation() {
        let f = Fixture::new();
        let r = f.initial();
        fs::write(f.target.join("existing.bin"), b"keep").unwrap();
        assert_eq!(
            request_migration(&f.paths, &r.root, 0, &f.target)
                .unwrap_err()
                .code,
            "target_not_empty"
        );
        assert_eq!(
            request_migration(&f.paths, &r.root, 0, &f.base)
                .unwrap_err()
                .code,
            "target_overlaps_data"
        );
        assert_eq!(
            request_migration(&f.paths, &r.root, 0, &r.root.join("assets"))
                .unwrap_err()
                .code,
            "target_overlaps_data"
        );
        assert!(readonly_state(&f.paths).unwrap().unwrap().phase == StoragePhase::Idle);
    }
    #[test]
    fn target_directory_replaced_after_request_cannot_receive_migration() {
        let f = Fixture::new();
        let id = f.request();
        fs::rename(&f.target, f.base.join("original-target-kept")).unwrap();
        fs::create_dir(&f.target).unwrap();
        assert_eq!(
            resolve_startup(&f.paths).unwrap_err().code,
            "root_identity_changed"
        );
        assert!(fs::read_dir(&f.target).unwrap().next().is_none());
        assert!(abandon_failed_migration(&f.paths, &id).unwrap());
    }
    #[test]
    fn target_conflict_after_copy_checkpoint_keeps_source_and_conflicting_bytes() {
        let f = Fixture::new();
        f.request();
        assert_eq!(
            resolve_startup_with_hook(&f.paths, &mut fail_once(Checkpoint::FileDurable))
                .unwrap_err()
                .code,
            "interrupted"
        );
        let output = f.target.join("assets/picture.bin");
        assert!(output.exists());
        fs::write(&output, b"other person's bytes").unwrap();
        assert_eq!(
            resolve_startup(&f.paths).unwrap_err().code,
            "target_file_conflict"
        );
        assert_eq!(fs::read(output).unwrap(), b"other person's bytes");
        assert_eq!(
            fs::read(f.paths.legacy_root.join("assets/picture.bin")).unwrap(),
            b"original image bytes"
        );
    }
    #[test]
    fn committed_recreated_empty_root_and_removed_database_are_rejected() {
        for case in 0..3 {
            let f = Fixture::new();
            f.request();
            let r = f.initial();
            commit_startup(&f.paths, r.trial.as_ref().unwrap()).unwrap();
            drop(r);
            let code = if case == 0 {
                fs::rename(&f.target, f.base.join("committed-kept")).unwrap();
                fs::create_dir(&f.target).unwrap();
                "active_root_identity_changed"
            } else {
                let path = if case == 1 {
                    f.target.join("spaces.db")
                } else {
                    f.target.join("plugins/plugins.db")
                };
                fs::rename(path, f.base.join(format!("database-kept-{case}"))).unwrap();
                "required_database_missing"
            };
            assert_eq!(resolve_startup(&f.paths).unwrap_err().code, code);
            assert_eq!(readonly_state(&f.paths).unwrap_err().code, code);
            assert!(f.target.exists());
        }
    }
    #[test]
    fn fresh_profile_exception_ends_after_explicit_initialization_and_allows_database_growth() {
        let f = Fixture::new();
        fs::remove_dir_all(&f.paths.legacy_root).unwrap();
        let r = f.initial();
        assert_eq!(r.root, f.paths.default_root);
        assert_eq!(
            request_migration(&f.paths, &r.root, 0, &f.target)
                .unwrap_err()
                .code,
            "dataset_not_initialized"
        );
        fs::create_dir(r.root.join("plugins")).unwrap();
        let spaces = Connection::open(r.root.join("spaces.db")).unwrap();
        spaces.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,payload TEXT);INSERT INTO app_state VALUES(1,'{\"scenes\":[]}');").unwrap();
        let plugins = Connection::open(r.root.join("plugins/plugins.db")).unwrap();
        plugins
            .execute_batch("CREATE TABLE file_intent(slot INTEGER PRIMARY KEY,payload TEXT);")
            .unwrap();
        mark_initialized(&f.paths, &r).unwrap();
        spaces
            .execute(
                "UPDATE app_state SET payload=?1",
                ["{\"scenes\":[],\"newUserData\":\"normal growth\"}"],
            )
            .unwrap();
        drop(spaces);
        drop(plugins);
        drop(r);
        let current = f.initial();
        assert_eq!(current.root, f.paths.default_root);
        drop(current);
        fs::rename(
            f.paths.default_root.join("spaces.db"),
            f.base.join("db-kept"),
        )
        .unwrap();
        assert_eq!(
            resolve_startup(&f.paths).unwrap_err().code,
            "required_database_missing"
        );
    }
    #[test]
    fn final_control_unknown_file_is_preserved_and_blocks_activation() {
        let f = Fixture::new();
        let id = f.request();
        let mut injected = false;
        let error = resolve_startup_with_hook(&f.paths, &mut |p| {
            if p == Checkpoint::FileDurable && !injected {
                injected = true;
                fs::write(
                    f.target.join(format!(".mewu-transfer-{id}/surprise.bin")),
                    b"unknown",
                )
                .unwrap();
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "target_unowned_files");
        assert_eq!(
            readonly_state(&f.paths).unwrap().unwrap().root,
            f.paths.legacy_root
        );
        assert_eq!(
            fs::read(f.target.join(format!(".mewu-transfer-{id}/surprise.bin"))).unwrap(),
            b"unknown"
        );
    }
    #[test]
    fn cleanup_rejects_same_named_replaced_scratch_file_and_preserves_it() {
        let f = Fixture::new();
        let work = f.target.join("own-test-scratch");
        fs::create_dir(&work).unwrap();
        let guard = pin_owned_directory(&work).unwrap();
        let original = work.join("check.db");
        write_new(&original, b"original scratch").unwrap();
        let stamp = identity(&asset_locations::open_regular(&original).unwrap()).unwrap();
        fs::rename(&original, work.join("original-kept.bin")).unwrap();
        write_new(&original, b"different owner file").unwrap();
        // Put the kept original outside this exact scratch directory so the error
        // proves identity rejection, rather than merely the unknown-name oracle.
        fs::rename(work.join("original-kept.bin"), f.base.join("kept-original")).unwrap();
        assert_eq!(
            cleanup_scratch(&work, guard, &[(original.clone(), stamp)])
                .unwrap_err()
                .code,
            "temporary_identity_changed"
        );
        assert_eq!(fs::read(original).unwrap(), b"different owner file");
    }
    #[test]
    fn runtime_lock_is_send_exclusive_and_release_is_observed_before_next_start() {
        fn assert_send<T: Send>() {}
        assert_send::<RuntimeLock>();
        let f = Fixture::new();
        let lock = acquire_runtime_lock(&f.paths, Duration::ZERO).unwrap();
        f.initial();
        assert!(readonly_state(&f.paths).unwrap().is_some());
        assert_eq!(
            acquire_runtime_lock(&f.paths, Duration::from_millis(80))
                .unwrap_err()
                .code,
            "runtime_lock_busy"
        );
        drop(lock);
        let next = acquire_runtime_lock(&f.paths, Duration::from_millis(80)).unwrap();
        drop(next);
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_synthetic_ciphertext_remains_identical_and_decryptable_at_new_path() {
        #[repr(C)]
        struct Blob {
            len: u32,
            data: *mut u8,
        }
        #[link(name = "crypt32")]
        extern "system" {
            fn CryptProtectData(
                input: *const Blob,
                description: *const u16,
                entropy: *const Blob,
                reserved: *mut std::ffi::c_void,
                prompt: *const std::ffi::c_void,
                flags: u32,
                output: *mut Blob,
            ) -> i32;
            fn CryptUnprotectData(
                input: *const Blob,
                description: *mut *mut u16,
                entropy: *const Blob,
                reserved: *mut std::ffi::c_void,
                prompt: *const std::ffi::c_void,
                flags: u32,
                output: *mut Blob,
            ) -> i32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn LocalFree(value: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        }
        let mut plain = b"test-created synthetic value, no API credential".to_vec();
        let input = Blob {
            len: plain.len() as u32,
            data: plain.as_mut_ptr(),
        };
        let mut protected = Blob {
            len: 0,
            data: std::ptr::null_mut(),
        };
        assert_ne!(
            unsafe {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    1,
                    &mut protected,
                )
            },
            0
        );
        let bytes =
            unsafe { std::slice::from_raw_parts(protected.data, protected.len as usize) }.to_vec();
        unsafe {
            LocalFree(protected.data.cast());
        }
        let f = Fixture::new();
        fs::write(f.paths.legacy_root.join("connections/id/key.bin"), &bytes).unwrap();
        f.request();
        let r = f.initial();
        let mut copied = fs::read(f.target.join("connections/id/key.bin")).unwrap();
        assert_eq!(copied, bytes);
        let input = Blob {
            len: copied.len() as u32,
            data: copied.as_mut_ptr(),
        };
        let mut output = Blob {
            len: 0,
            data: std::ptr::null_mut(),
        };
        assert_ne!(
            unsafe {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    1,
                    &mut output,
                )
            },
            0
        );
        assert_eq!(
            unsafe { std::slice::from_raw_parts(output.data, output.len as usize) },
            plain
        );
        unsafe {
            LocalFree(output.data.cast());
        }
        commit_startup(&f.paths, r.trial.as_ref().unwrap()).unwrap();
    }
}
