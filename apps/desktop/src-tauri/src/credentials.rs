// SPDX-License-Identifier: MPL-2.0
use std::path::Path;
use zeroize::Zeroizing;

/// Credentials are immutable. The database points to a fresh, opaque UUID only
/// after its encrypted file is durable; failed commits cannot overwrite a key
/// still in use by another request or by the previous database revision.
pub fn validate_key(key: &str) -> Result<(), String> {
    if key.len() > 16 * 1024
        || (!key.is_empty() && reqwest::header::HeaderValue::from_str(key).is_err())
        || key.chars().any(char::is_control)
    {
        return Err("连接密钥格式无效".into());
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id)
        .map(|v| v.to_string() != id)
        .unwrap_or(true)
    {
        return Err("连接凭据身份无效".into());
    }
    Ok(())
}

fn ordinary(path: &Path, directory: bool) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "无法访问连接凭据")?;
    if metadata.file_type().is_symlink() || metadata.is_dir() != directory {
        return Err("连接凭据路径无效".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("连接凭据路径无效".into());
        }
    }
    if !directory && (!metadata.is_file() || metadata.len() > 65536) {
        return Err("连接凭据文件无效".into());
    }
    Ok(())
}

#[cfg(windows)]
fn blob_path(
    root: &Path,
    namespace: &str,
    connection_id: &str,
    credential_id: &str,
    create: bool,
) -> Result<std::path::PathBuf, String> {
    validate_id(connection_id)?;
    validate_id(credential_id)?;
    ordinary(root, true)?;
    let canonical_root = root.canonicalize().map_err(|_| "无法访问连接凭据")?;
    let mut directory = root.to_path_buf();
    for segment in [namespace, connection_id] {
        directory.push(segment);
        if create {
            match std::fs::create_dir(&directory) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(_) => return Err("无法创建连接凭据目录".into()),
            }
        }
        ordinary(&directory, true)?;
        if !directory
            .canonicalize()
            .map_err(|_| "无法访问连接凭据")?
            .starts_with(&canonical_root)
        {
            return Err("连接凭据路径越界".into());
        }
    }
    Ok(directory.join(format!("{credential_id}.bin")))
}

pub fn read_profile(
    root: &Path,
    profile: &mewu_core::ConnectionProfile,
) -> Result<Zeroizing<String>, String> {
    validate_id(&profile.id)?;
    let Some(credential_id) = profile.credential_id.as_deref() else {
        return Ok(Zeroizing::new(String::new()));
    };
    validate_id(credential_id)?;
    // Only this exact migration reference may read the old shared key. Clearing
    // or replacing it permanently stops fallback, including after a restart.
    if profile.id == mewu_core::LEGACY_CONNECTION_ID
        && credential_id == mewu_core::LEGACY_CREDENTIAL_ID
    {
        let key = read(root)?;
        if key.is_empty() {
            return Err("原连接密钥不存在，请重新填写".into());
        }
        validate_key(&key)?;
        return Ok(key);
    }
    if credential_id == mewu_core::LEGACY_CREDENTIAL_ID {
        return Err("连接凭据身份无效".into());
    }
    read_scoped(root, "connections", &profile.id, credential_id)
}

/// Original WPF CredentialService used current-user DPAPI without entropy.
/// This is a migration-only reader: a missing/broken referenced blob is an
/// error, never an empty-key fallback. The caller must not log the returned key.
#[cfg(windows)]
pub(crate) fn read_legacy(
    settings_directory: &Path,
    credential_id: &str,
) -> Result<Zeroizing<String>, String> {
    read_legacy_detailed(settings_directory, credential_id)
        .map_err(|error| format!("旧版连接凭据读取失败（{:?}）", error.stage))
}

/// Diagnostic metadata is deliberately independent of paths and key content.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LegacyCredentialStage {
    Id,
    RootDirectory,
    CredentialDirectory,
    FileMetadata,
    OpenFile,
    FileType,
    FileAttributes,
    FileLength,
    CanonicalFile,
    ParentIdentity,
    FileIdentity,
    ReadFile,
    FileChanged,
    Dpapi,
    Utf8,
    KeyFormat,
    Empty,
}
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct LegacyCredentialError {
    pub stage: LegacyCredentialStage,
    pub win32: Option<i32>,
}
#[cfg(windows)]
impl LegacyCredentialError {
    fn at(stage: LegacyCredentialStage) -> Self {
        Self { stage, win32: None }
    }
    fn io(stage: LegacyCredentialStage, error: std::io::Error) -> Self {
        Self {
            stage,
            win32: error.raw_os_error(),
        }
    }
}

#[cfg(windows)]
pub(crate) fn read_legacy_detailed(
    settings_directory: &Path,
    credential_id: &str,
) -> Result<Zeroizing<String>, LegacyCredentialError> {
    use std::{fs::OpenOptions, io::Read, os::windows::fs::OpenOptionsExt};
    use LegacyCredentialStage as Stage;
    if credential_id.is_empty()
        || credential_id.len() > 128
        || !credential_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(LegacyCredentialError::at(Stage::Id));
    }
    fn pin(
        path: &Path,
        stage: LegacyCredentialStage,
    ) -> Result<std::fs::File, LegacyCredentialError> {
        ordinary(path, true).map_err(|_| LegacyCredentialError::at(stage))?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(path)
            .map_err(|error| LegacyCredentialError::io(stage, error))?;
        use std::os::windows::fs::MetadataExt;
        let metadata = file
            .metadata()
            .map_err(|error| LegacyCredentialError::io(stage, error))?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err(LegacyCredentialError::at(stage));
        }
        Ok(file)
    }
    let _parent = pin(settings_directory, Stage::RootDirectory)?;
    let directory = settings_directory.join("Credentials");
    let directory_pin = pin(&directory, Stage::CredentialDirectory)?;
    let path = directory.join(format!("{credential_id}.bin"));
    ordinary(&path, false).map_err(|_| LegacyCredentialError::at(Stage::FileMetadata))?;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000)
        .open(&path)
        .map_err(|error| LegacyCredentialError::io(Stage::OpenFile, error))?;
    use std::os::windows::fs::MetadataExt;
    let before = file
        .metadata()
        .map_err(|error| LegacyCredentialError::io(Stage::FileMetadata, error))?;
    if !before.is_file() {
        return Err(LegacyCredentialError::at(Stage::FileType));
    }
    if before.file_attributes() & 0x400 != 0 {
        return Err(LegacyCredentialError::at(Stage::FileAttributes));
    }
    if before.len() == 0 || before.len() > 65536 {
        return Err(LegacyCredentialError::at(Stage::FileLength));
    }
    let canonical_file = path
        .canonicalize()
        .map_err(|error| LegacyCredentialError::io(Stage::CanonicalFile, error))?;
    let parent = canonical_file
        .parent()
        .ok_or_else(|| LegacyCredentialError::at(Stage::ParentIdentity))?;
    // Windows canonical strings can differ in case while identifying the same
    // directory. Compare the pinned volume/file identity, not path spelling.
    // Both handles still reject reparse points and deny FILE_SHARE_DELETE.
    let actual_parent = same_file::Handle::from_file(pin(parent, Stage::ParentIdentity)?)
        .map_err(|error| LegacyCredentialError::io(Stage::FileIdentity, error))?;
    let expected_parent = same_file::Handle::from_file(directory_pin)
        .map_err(|error| LegacyCredentialError::io(Stage::FileIdentity, error))?;
    if actual_parent != expected_parent {
        return Err(LegacyCredentialError::at(Stage::ParentIdentity));
    }
    let mut encrypted = Zeroizing::new(Vec::new());
    (&file)
        .take(65537)
        .read_to_end(&mut encrypted)
        .map_err(|error| LegacyCredentialError::io(Stage::ReadFile, error))?;
    let after = file
        .metadata()
        .map_err(|error| LegacyCredentialError::io(Stage::FileMetadata, error))?;
    if encrypted.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(LegacyCredentialError::at(Stage::FileChanged));
    }
    let plain = transform_detailed(&encrypted, false).map_err(|win32| LegacyCredentialError {
        stage: Stage::Dpapi,
        win32,
    })?;
    let text = std::str::from_utf8(&plain).map_err(|_| LegacyCredentialError::at(Stage::Utf8))?;
    validate_key(text).map_err(|_| LegacyCredentialError::at(Stage::KeyFormat))?;
    if text.trim().is_empty() {
        return Err(LegacyCredentialError::at(Stage::Empty));
    }
    Ok(Zeroizing::new(text.to_owned()))
}

#[cfg(not(windows))]
pub(crate) fn read_legacy(_: &Path, _: &str) -> Result<Zeroizing<String>, String> {
    Err("旧版连接导入仅支持 Windows".into())
}

pub fn read_memory(
    root: &Path,
    binding_id: &str,
    credential_id: Option<&str>,
) -> Result<Zeroizing<String>, String> {
    validate_id(binding_id)?;
    match credential_id {
        Some(id) => read_scoped(root, "memory", binding_id, id),
        None => Ok(Zeroizing::new(String::new())),
    }
}

fn read_scoped(
    root: &Path,
    namespace: &str,
    owner_id: &str,
    credential_id: &str,
) -> Result<Zeroizing<String>, String> {
    validate_id(owner_id)?;
    validate_id(credential_id)?;
    #[cfg(windows)]
    let key = {
        use std::io::Read;
        let path = blob_path(root, namespace, owner_id, credential_id, false)?;
        ordinary(&path, false)?;
        let mut data = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "无法读取连接密钥")?
            .take(65537)
            .read_to_end(&mut data)
            .map_err(|_| "无法读取连接密钥")?;
        let bytes = transform(&data, false)?;
        String::from_utf8(bytes.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| "连接密钥损坏")?
    };
    #[cfg(target_os = "macos")]
    let key = {
        let prefix = if namespace == "connections" {
            "connection"
        } else {
            "memory"
        };
        let account = format!("{prefix}/{owner_id}/{credential_id}");
        let bytes =
            security_framework::passwords::get_generic_password("dev.mewu.remake", &account)
                .map_err(|_| "无法读取连接钥匙串")?;
        String::from_utf8(bytes)
            .map(Zeroizing::new)
            .map_err(|_| "连接密钥损坏")?
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let key: Zeroizing<String> = {
        return Err("此平台尚未接入凭据存储".into());
    };
    validate_key(&key)?;
    Ok(key)
}

pub fn create(root: &Path, connection_id: &str, key: &str) -> Result<String, String> {
    create_scoped(root, "connections", connection_id, key)
}

pub fn create_memory(root: &Path, binding_id: &str, key: &str) -> Result<String, String> {
    create_scoped(root, "memory", binding_id, key)
}

fn create_scoped(
    root: &Path,
    namespace: &str,
    connection_id: &str,
    key: &str,
) -> Result<String, String> {
    validate_id(connection_id)?;
    validate_key(key)?;
    if key.is_empty() {
        return Err("连接密钥为空".into());
    }
    let credential_id = uuid::Uuid::new_v4().to_string();
    #[cfg(windows)]
    {
        use std::io::Write;
        let path = blob_path(root, namespace, connection_id, &credential_id, true)?;
        let data = transform(key.as_bytes(), true)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "无法保存连接密钥")?;
        if file.write_all(&data).and_then(|_| file.sync_all()).is_err() {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err("无法保存连接密钥".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let prefix = if namespace == "connections" {
            "connection"
        } else {
            "memory"
        };
        security_framework::passwords::set_generic_password(
            "dev.mewu.remake",
            &format!("{prefix}/{connection_id}/{credential_id}"),
            key.as_bytes(),
        )
        .map_err(|_| "无法写入连接钥匙串")?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    return Err("此平台尚未接入凭据存储".into());
    Ok(credential_id)
}

/// Only for a newly-created blob whose database commit failed. Existing blobs
/// are retained: another operation may still hold their immutable reference.
pub fn discard_uncommitted(root: &Path, connection_id: &str, credential_id: &str) {
    discard_scoped(root, "connections", connection_id, credential_id);
}

pub fn discard_uncommitted_memory(root: &Path, binding_id: &str, credential_id: &str) {
    discard_scoped(root, "memory", binding_id, credential_id);
}

fn discard_scoped(root: &Path, namespace: &str, connection_id: &str, credential_id: &str) {
    if validate_id(connection_id).is_err()
        || validate_id(credential_id).is_err()
        || credential_id == mewu_core::LEGACY_CREDENTIAL_ID
    {
        return;
    }
    #[cfg(windows)]
    if let Ok(path) = blob_path(root, namespace, connection_id, credential_id, false) {
        if ordinary(&path, false).is_ok() {
            let _ = std::fs::remove_file(path);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let prefix = if namespace == "connections" {
            "connection"
        } else {
            "memory"
        };
        let _ = security_framework::passwords::delete_generic_password(
            "dev.mewu.remake",
            &format!("{prefix}/{connection_id}/{credential_id}"),
        );
    }
}

#[cfg(windows)]
pub fn read(root: &Path) -> Result<Zeroizing<String>, String> {
    let path = root.join("connection.bin");
    if !path.exists() {
        return Ok(Zeroizing::new(String::new()));
    }
    ordinary(root, true)?;
    ordinary(&path, false)?;
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "无法读取连接密钥")?
        .take(65537)
        .read_to_end(&mut data)
        .map_err(|_| "无法读取连接密钥")?;
    let bytes = transform(&data, false)?;
    String::from_utf8(bytes.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| "连接密钥损坏".into())
}

#[cfg(all(windows, test))]
pub fn write(root: &Path, key: &str) -> Result<(), String> {
    let encrypted = transform(key.as_bytes(), true)?;
    let temporary = root.join("connection.bin.tmp");
    std::fs::write(&temporary, &encrypted[..]).map_err(|_| "无法保存连接密钥")?;
    std::fs::rename(&temporary, root.join("connection.bin")).map_err(|_| "无法替换连接密钥".into())
}

#[cfg(windows)]
fn transform(data: &[u8], protect: bool) -> Result<Zeroizing<Vec<u8>>, String> {
    transform_detailed(data, protect).map_err(|_| "无法访问本机加密密钥".into())
}

#[cfg(windows)]
fn transform_detailed(data: &[u8], protect: bool) -> Result<Zeroizing<Vec<u8>>, Option<i32>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    if data.len() > 65536 {
        return Err(None);
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI allocates output with LocalAlloc; every success frees exactly that buffer.
    let success = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error().raw_os_error());
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        if !protect {
            std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        }
        LocalFree(output.pbData.cast());
    }
    Ok(Zeroizing::new(bytes))
}

#[cfg(target_os = "macos")]
pub fn read(_: &Path) -> Result<Zeroizing<String>, String> {
    match security_framework::passwords::get_generic_password("dev.mewu.remake", "connection") {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Zeroizing::new)
            .map_err(|_| "连接密钥损坏".into()),
        Err(e) if e.code() == -25300 => Ok(Zeroizing::new(String::new())),
        Err(_) => Err("无法读取钥匙串".into()),
    }
}
#[cfg(all(target_os = "macos", test))]
pub fn write(_: &Path, key: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(
        "dev.mewu.remake",
        "connection",
        key.as_bytes(),
    )
    .map_err(|_| "无法写入钥匙串".into())
}
#[cfg(not(any(windows, target_os = "macos")))]
pub fn read(_: &Path) -> Result<Zeroizing<String>, String> {
    Err("此平台尚未接入凭据存储".into())
}
#[cfg(all(not(any(windows, target_os = "macos")), test))]
pub fn write(_: &Path, _: &str) -> Result<(), String> {
    Err("此平台尚未接入凭据存储".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn memory_keys_are_encrypted_and_never_fall_back_to_connection_keys() {
        let root = std::env::temp_dir().join(format!("mewu-memory-key-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let chat = create(&root, &owner, "fixture-chat-only").unwrap();
        let memory = create_memory(&root, &owner, "fixture-memory-only").unwrap();
        assert_eq!(
            &**read_memory(&root, &owner, Some(&memory)).unwrap(),
            "fixture-memory-only"
        );
        assert!(read_memory(&root, &owner, Some(&chat)).is_err());
        assert!(read_scoped(&root, "connections", &owner, &memory).is_err());
        assert!(read_memory(&root, &owner, None).unwrap().is_empty());
        assert!(read_memory(&root, "../connections", Some(&memory)).is_err());
        let bytes =
            std::fs::read(blob_path(&root, "memory", &owner, &memory, false).unwrap()).unwrap();
        assert!(!bytes.windows(19).any(|part| part == b"fixture-memory-only"));
        discard_uncommitted_memory(&root, &owner, &memory);
        assert!(read_memory(&root, &owner, Some(&memory)).is_err());
        assert_eq!(
            &**read_scoped(&root, "connections", &owner, &chat).unwrap(),
            "fixture-chat-only"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dpapi_roundtrip_is_ciphertext_and_supports_key_rotation() {
        let root = std::env::temp_dir().join(format!("mewu-key-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        write(&root, "fixture-key-never-a-real-credential").unwrap();
        let blob = std::fs::read(root.join("connection.bin")).unwrap();
        assert!(!blob.windows(11).any(|v| v == b"fixture-key"));
        assert_eq!(
            &**read(&root).unwrap(),
            "fixture-key-never-a-real-credential"
        );
        write(&root, "replacement").unwrap();
        assert_eq!(&**read(&root).unwrap(), "replacement");
        write(&root, "").unwrap();
        assert!(read(&root).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
