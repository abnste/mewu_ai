// SPDX-License-Identifier: MPL-2.0
//! Fixed host resources only; no URL, arbitrary file or shell arguments.
//! The settings host exposes only these fixed resources.
use crate::capture_preferences::ControlledDirectory;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
// Resolve from the existing native Cargo package, not from this private draft's directory.
const LICENSE_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../LICENSE"));
const NOTICES_TEXT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../THIRD-PARTY-NOTICES"
));

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    Development,
    Alpha,
    Stable,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateSupport {
    SignedInstaller,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInfo {
    pub application: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub channel: ReleaseChannel,
    pub platform: String,
    pub update_support: UpdateSupport,
    pub data_directory: String,
    pub license: String,
    pub notices_available: bool,
}
/// Passed by the host from package/build metadata. Never take these fields from a renderer.
pub struct BuildInfo {
    version: String,
    commit: Option<String>,
    channel: ReleaseChannel,
}
impl BuildInfo {
    pub fn new(
        version: &str,
        commit: Option<&str>,
        channel: ReleaseChannel,
    ) -> Result<Self, Error> {
        if version.len() > 64 {
            return Err(Error::BuildMetadata);
        }
        let parsed = semver::Version::parse(version).map_err(|_| Error::BuildMetadata)?;
        if channel == ReleaseChannel::Stable && !parsed.pre.is_empty() {
            return Err(Error::BuildMetadata);
        }
        if commit.is_some_and(|value| {
            !(7..=64).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            return Err(Error::BuildMetadata);
        }
        Ok(Self {
            version: version.into(),
            commit: commit.map(str::to_owned),
            channel,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub enum LicenseDocumentKind {
    License,
    Notices,
}
impl TryFrom<String> for LicenseDocumentKind {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "license" => Ok(Self::License),
            "notices" => Ok(Self::Notices),
            _ => Err("没有此许可文档"),
        }
    }
}
impl From<LicenseDocumentKind> for String {
    fn from(value: LicenseDocumentKind) -> Self {
        match value {
            LicenseDocumentKind::License => "license",
            LicenseDocumentKind::Notices => "notices",
        }
        .into()
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseDocument {
    pub title: &'static str,
    pub text: &'static str,
}
pub fn read_license_document(kind: LicenseDocumentKind) -> Result<LicenseDocument, Error> {
    let (title, text) = match kind {
        LicenseDocumentKind::License => ("MPL-2.0", LICENSE_TEXT),
        LicenseDocumentKind::Notices => ("第三方许可", NOTICES_TEXT),
    };
    validate_document(text)?;
    Ok(LicenseDocument { title, text })
}
fn validate_document(text: &str) -> Result<(), Error> {
    if text.is_empty() || text.len() > MAX_DOCUMENT_BYTES || text.contains('\0') {
        Err(Error::DocumentUnavailable)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    DataDirectory,
    BuildMetadata,
    DocumentUnavailable,
    Busy,
    Canceled,
    OpenFailed,
    Unsupported,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DataDirectory => "数据目录不可用",
            Self::BuildMetadata => "版本信息不可用",
            Self::DocumentUnavailable => "许可文档不可用",
            Self::Busy => "正在打开数据目录",
            Self::Canceled => "操作已取消",
            Self::OpenFailed => "无法打开数据目录",
            Self::Unsupported => "此平台尚未接入打开数据目录",
        })
    }
}
impl std::error::Error for Error {}

struct Inner {
    directory: ControlledDirectory,
    build: BuildInfo,
    busy: AtomicBool,
    generation: AtomicU64,
}
#[derive(Clone)]
pub struct SettingsResources(Arc<Inner>);
impl SettingsResources {
    pub fn open(root: &Path, build: BuildInfo) -> Result<Self, Error> {
        let directory = ControlledDirectory::open(root).map_err(|_| Error::DataDirectory)?;
        directory.path().to_str().ok_or(Error::DataDirectory)?;
        Ok(Self(Arc::new(Inner {
            directory,
            build,
            busy: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        })))
    }
    /// Only return through the settings capability. Does not enumerate anything in the directory.
    pub fn info(&self) -> Result<SettingsInfo, Error> {
        self.0.directory.check().map_err(|_| Error::DataDirectory)?;
        let build = &self.0.build;
        Ok(SettingsInfo {
            application: "Mewu".into(),
            version: build.version.clone(),
            commit: build.commit.clone(),
            channel: build.channel,
            platform: std::env::consts::OS.into(),
            update_support: UpdateSupport::SignedInstaller,
            data_directory: self
                .0
                .directory
                .path()
                .to_str()
                .ok_or(Error::DataDirectory)?
                .into(),
            license: "MPL-2.0".into(),
            notices_available: validate_document(NOTICES_TEXT).is_ok(),
        })
    }
    /// Take the permit before the blocking task is spawned, after the host settings/exit gate.
    pub fn begin_open_data_directory(&self) -> Result<OpenDataDirectory, Error> {
        if self
            .0
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::Busy);
        }
        Ok(OpenDataDirectory {
            permit: OpenPermit(self.0.clone()),
            generation: self.0.generation.load(Ordering::Acquire),
        })
    }
    pub fn busy(&self) -> bool {
        self.0.busy.load(Ordering::Acquire)
    }
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
struct OpenPermit(Arc<Inner>);
impl Drop for OpenPermit {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}
pub struct OpenDataDirectory {
    permit: OpenPermit,
    generation: u64,
}
impl OpenDataDirectory {
    fn allowed(&self, allowed: &impl Fn() -> bool) -> Result<(), Error> {
        if self.permit.0.generation.load(Ordering::Acquire) != self.generation || !allowed() {
            return Err(Error::Canceled);
        }
        self.permit
            .0
            .directory
            .check()
            .map_err(|_| Error::DataDirectory)
    }
    /// Run on a blocking worker and await actual completion. A timeout/drop cancels by generation,
    /// but cannot forcibly interrupt Shell extensions; the permit stays with the actual STA thread.
    pub fn open(self, allowed: impl Fn() -> bool + Send + 'static) -> Result<(), Error> {
        #[cfg(windows)]
        {
            std::thread::Builder::new()
                .name("mewu-data-directory".into())
                .spawn(move || {
                    use std::os::windows::ffi::OsStrExt;
                    use windows::{
                        core::{w, PCWSTR},
                        Win32::{
                            Foundation::HWND,
                            System::Com::{
                                CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
                                COINIT_DISABLE_OLE1DDE,
                            },
                            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
                        },
                    };
                    self.allowed(&allowed)?;
                    unsafe {
                        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()
                    }
                    .map_err(|_| Error::OpenFailed)?;
                    struct Apartment;
                    impl Drop for Apartment {
                        fn drop(&mut self) {
                            unsafe {
                                CoUninitialize();
                            }
                        }
                    }
                    let _apartment = Apartment;
                    let wide: Vec<u16> = self
                        .permit
                        .0
                        .directory
                        .path()
                        .as_os_str()
                        .encode_wide()
                        .chain(Some(0))
                        .collect();
                    if wide.len() > 32_768 || wide[..wide.len() - 1].contains(&0) {
                        return Err(Error::DataDirectory);
                    }
                    self.allowed(&allowed)?;
                    // Fixed verb, directory, no parameters, no command interpreter.
                    let result = unsafe {
                        ShellExecuteW(
                            Some(HWND::default()),
                            w!("explore"),
                            PCWSTR(wide.as_ptr()),
                            PCWSTR::null(),
                            PCWSTR::null(),
                            SW_SHOWNORMAL,
                        )
                    };
                    // Do not recheck cancellation after successful Shell acceptance or falsely report it undone.
                    if result.0 as isize > 32 {
                        Ok(())
                    } else {
                        Err(Error::OpenFailed)
                    }
                })
                .map_err(|_| Error::OpenFailed)?
                .join()
                .map_err(|_| Error::OpenFailed)?
        }
        #[cfg(not(windows))]
        {
            let _ = (self, allowed);
            Err(Error::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("mewu-settings-info-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn build() -> BuildInfo {
        BuildInfo::new("1.0.0-alpha.1", None, ReleaseChannel::Alpha).unwrap()
    }

    #[test]
    fn fixed_documents_are_complete_plain_text_and_kind_cannot_be_a_path() {
        for (kind, expected) in [
            (LicenseDocumentKind::License, LICENSE_TEXT),
            (LicenseDocumentKind::Notices, NOTICES_TEXT),
        ] {
            let result = read_license_document(kind).unwrap();
            assert_eq!(result.text, expected);
            assert!(result.text.len() <= MAX_DOCUMENT_BYTES);
        }
        for json in [
            r#""../../credentials""#,
            r#"{"license":null}"#,
            r#""file:///C:/data""#,
            r#"1"#,
        ] {
            assert!(serde_json::from_str::<LicenseDocumentKind>(json).is_err());
        }
        assert!(validate_document("").is_err());
        assert!(validate_document("a\0b").is_err());
        assert!(validate_document(&"a".repeat(MAX_DOCUMENT_BYTES + 1)).is_err());
    }
    #[test]
    fn info_has_real_fixed_root_signed_update_and_no_inferred_commit() {
        let directory = Directory::new();
        let resources = SettingsResources::open(&directory.0, build()).unwrap();
        let info = resources.info().unwrap();
        assert_eq!(
            std::path::PathBuf::from(&info.data_directory),
            std::fs::canonicalize(&directory.0).unwrap()
        );
        assert_eq!(info.version, "1.0.0-alpha.1");
        assert_eq!(info.update_support, UpdateSupport::SignedInstaller);
        assert!(info.commit.is_none());
        let wire = serde_json::to_value(info).unwrap();
        assert_eq!(wire["updateSupport"], "signed_installer");
        assert!(wire.get("commit").is_none());
        assert!(wire.get("latestVersion").is_none());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
    #[test]
    fn fixed_directory_is_required_and_bad_build_facts_are_not_invented() {
        let directory = Directory::new();
        let missing = directory.0.join("missing");
        assert!(SettingsResources::open(&missing, build()).is_err());
        assert!(!missing.exists());
        assert!(BuildInfo::new("latest", None, ReleaseChannel::Stable).is_err());
        assert!(BuildInfo::new("1.0.0-alpha.1", None, ReleaseChannel::Stable).is_err());
        assert!(BuildInfo::new("1.0.0", Some("$(secret)"), ReleaseChannel::Stable).is_err());
        assert!(BuildInfo::new("1.0.0", Some("1234567"), ReleaseChannel::Stable).is_ok());
    }
    #[test]
    fn canceled_open_holds_slot_until_actual_task_drop_without_launching_shell() {
        let directory = Directory::new();
        let resources = SettingsResources::open(&directory.0, build()).unwrap();
        let task = resources.begin_open_data_directory().unwrap();
        resources.invalidate();
        assert_eq!(task.allowed(&|| true).unwrap_err(), Error::Canceled);
        assert!(resources.busy());
        assert!(matches!(
            resources.begin_open_data_directory(),
            Err(Error::Busy)
        ));
        drop(task);
        assert!(!resources.busy());
    }
}
