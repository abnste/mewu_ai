// SPDX-License-Identifier: MPL-2.0
//! Current-user Run registration. The original MewuAI entry is never touched.
use std::path::Path;

pub fn command(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("应用程序路径不可用")?;
    if !Path::new(path).is_absolute() || path.contains(['"', '\0', '\r', '\n']) {
        return Err("应用程序路径不可用".into());
    }
    let command = format!("\"{path}\" --background");
    // Run/RunOnce values have a documented 260-character command limit.
    if command.encode_utf16().count() > 260 {
        return Err("应用程序路径过长，无法注册开机启动".into());
    }
    Ok(command)
}

fn owned_command(path: &Path, owner: &str) -> Result<String, String> {
    crate::system_preferences::validate_startup_owner(Some(owner))
        .map_err(|_| "开机启动归属无法确认")?;
    let value = format!("{} --startup-owner={owner}", command(path)?);
    if value.encode_utf16().count() > 260 {
        return Err("应用程序路径过长，无法注册开机启动".into());
    }
    Ok(value)
}
fn matches_owned(input: &str, owner: &str) -> bool {
    let suffix = format!("\" --background --startup-owner={owner}");
    let Some(path) = input
        .strip_prefix('"')
        .and_then(|input| input.strip_suffix(&suffix))
    else {
        return false;
    };
    owned_command(Path::new(path), owner).is_ok_and(|expected| expected == input)
}
fn authorize_change(current: Option<&str>, owner: Option<&str>) -> Result<(), String> {
    if let Some(current) = current {
        if !owner.is_some_and(|owner| matches_owned(current, owner)) {
            return Err("开机启动归属无法确认，未修改现有启动项".into());
        }
    }
    Ok(())
}
fn refresh_command(
    value: &crate::system_preferences::SystemPreferences,
    current: Option<&str>,
    executable: &Path,
) -> Result<Option<String>, String> {
    if !value.launch_at_startup || current.is_none() {
        return Ok(None);
    }
    let owner = value
        .startup_owner
        .as_deref()
        .ok_or("开机启动未注册到当前版本，请重新保存设置")?;
    let current = current.ok_or("开机启动配置不可用")?;
    if !matches_owned(current, owner) {
        return Err("开机启动归属无法确认，请检查设置".into());
    }
    let next = owned_command(executable, owner)?;
    Ok((current != next).then_some(next))
}
// StartupApproved is not a public Win32 contract. Accept only the enabled
// record used by auto-launch's Windows implementation; any unfamiliar record
// is left untouched and must not authorize a silent Run path replacement.
fn startup_approved(bytes: &[u8]) -> bool {
    bytes == [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
}
const APPROVAL_WARNING: &str = "开机启动已被 Windows 停用或无法确认，请检查启动应用";
static REFRESH_WARNING: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
pub fn status(owner: Option<&str>) -> (bool, Option<String>) {
    match registered(owner) {
        Ok(true) => (true, None),
        Ok(false) => (
            false,
            REFRESH_WARNING
                .lock()
                .ok()
                .and_then(|warning| warning.clone()),
        ),
        Err(error) => (false, Some(error)),
    }
}
pub fn initialize(value: &crate::system_preferences::SystemPreferences) {
    let result = refresh(value);
    if let Ok(mut warning) = REFRESH_WARNING.lock() {
        *warning = result.err();
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::*,
    };
    const SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const APPROVAL_SUBKEY: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
    const NAME: &str = "Mewu1.0";
    fn wide(input: &str) -> Vec<u16> {
        input.encode_utf16().chain(Some(0)).collect()
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn open(write: bool) -> Result<Option<Key>, String> {
        open_at(SUBKEY, write)
    }
    fn open_at(subkey: &str, write: bool) -> Result<Option<Key>, String> {
        let mut handle = std::ptr::null_mut();
        let name = wide(subkey);
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                if write {
                    KEY_QUERY_VALUE | KEY_SET_VALUE
                } else {
                    KEY_QUERY_VALUE
                },
                &mut handle,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err("无法读取当前用户的开机启动配置".into());
        }
        Ok(Some(Key(handle)))
    }
    fn create() -> Result<Key, String> {
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(SUBKEY).as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err("无法保存当前用户的开机启动配置".into());
        }
        Ok(Key(handle))
    }
    #[derive(Clone, PartialEq, Eq)]
    struct Value {
        kind: u32,
        bytes: Vec<u8>,
    }
    fn read(key: &Key) -> Result<Option<Value>, String> {
        let mut kind = 0;
        let mut length = 0;
        let name = wide(NAME);
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut length,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS || length > 4096 {
            return Err("开机启动配置不可用".into());
        }
        let mut bytes = vec![0; length as usize];
        let second = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                bytes.as_mut_ptr(),
                &mut length,
            )
        };
        if second != ERROR_SUCCESS || length as usize > bytes.len() {
            return Err("开机启动配置已变化，请重试".into());
        }
        bytes.truncate(length as usize);
        Ok(Some(Value { kind, bytes }))
    }
    fn ensure_approved() -> Result<(), String> {
        let Some(key) = open_at(APPROVAL_SUBKEY, false)? else {
            return Ok(());
        };
        let Some(value) = read(&key)? else {
            return Ok(());
        };
        if value.kind != REG_BINARY || !startup_approved(&value.bytes) {
            return Err(APPROVAL_WARNING.into());
        }
        Ok(())
    }
    fn write(key: &Key, value: &Option<Value>) -> Result<(), String> {
        let name = wide(NAME);
        let status = unsafe {
            if let Some(value) = value {
                RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    value.kind,
                    value.bytes.as_ptr(),
                    value.bytes.len() as u32,
                )
            } else {
                RegDeleteValueW(key.0, name.as_ptr())
            }
        };
        if status == ERROR_SUCCESS || value.is_none() && status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err("无法保存开机启动配置".into())
        }
    }
    fn desired(enabled: bool, owner: Option<&str>) -> Result<Option<Value>, String> {
        if !enabled {
            return Ok(None);
        }
        let exe = std::env::current_exe().map_err(|_| "无法确定当前应用程序路径")?;
        if !exe.is_file() {
            return Err("无法确定当前应用程序路径".into());
        }
        let bytes = wide(&owned_command(&exe, owner.ok_or("开机启动归属无法确认")?)?)
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        Ok(Some(Value {
            kind: REG_SZ,
            bytes,
        }))
    }
    fn text(value: &Value) -> Result<String, String> {
        if value.kind != REG_SZ || value.bytes.len() % 2 != 0 {
            return Err("开机启动配置不可用".into());
        }
        let units: Vec<_> = value
            .bytes
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        let Some((0, data)) = units.split_last().map(|(last, data)| (*last, data)) else {
            return Err("开机启动配置不可用".into());
        };
        String::from_utf16(data).map_err(|_| "开机启动配置不可用".into())
    }
    pub fn registered(owner: Option<&str>) -> Result<bool, String> {
        if owner.is_none() {
            return Ok(false);
        }
        let Some(key) = open(false)? else {
            return Ok(false);
        };
        let Some(value) = read(&key)? else {
            return Ok(false);
        };
        if Some(value) != desired(true, owner)? {
            return Ok(false);
        }
        ensure_approved()?;
        Ok(true)
    }
    pub fn refresh(value: &crate::system_preferences::SystemPreferences) -> Result<(), String> {
        if !value.launch_at_startup {
            return Ok(());
        }
        // Read first. A deleted/disabled registration is never re-created here.
        let Some(key) = open(false)? else {
            return Ok(());
        };
        let Some(before) = read(&key)? else {
            return Ok(());
        };
        let executable = std::env::current_exe().map_err(|_| "无法确定当前应用程序路径")?;
        if !executable.is_file() {
            return Err("无法确定当前应用程序路径".into());
        }
        let Some(next) = refresh_command(value, Some(&text(&before)?), &executable)? else {
            return Ok(());
        };
        ensure_approved()?;
        let after = Some(Value {
            kind: REG_SZ,
            bytes: wide(&next).into_iter().flat_map(u16::to_le_bytes).collect(),
        });
        drop(key);
        let Some(key) = open(true)? else {
            return Ok(());
        };
        if read(&key)? != Some(before.clone()) {
            return Err("开机启动配置已变化，未更新路径".into());
        }
        ensure_approved()?;
        let change = Change {
            key: Some(key),
            before: Some(before),
            after,
            changed: true,
            finished: false,
        };
        let key = change.key.as_ref().ok_or("开机启动配置不可用")?;
        write(key, &change.after)?;
        if read(key)? != change.after {
            return Err("无法确认开机启动路径的更新状态".into());
        }
        ensure_approved()?;
        // StartupApproved is read-only. A system-disabled or unknown record
        // never triggers refresh and is never overwritten to enable startup.
        change.commit();
        Ok(())
    }
    pub struct Change {
        key: Option<Key>,
        before: Option<Value>,
        after: Option<Value>,
        changed: bool,
        finished: bool,
    }
    impl Change {
        pub fn apply(enabled: bool, owner: Option<&str>) -> Result<Self, String> {
            // A fresh default must not even open another installation's Run entry.
            if !enabled && owner.is_none() {
                return Ok(Self {
                    key: None,
                    before: None,
                    after: None,
                    changed: false,
                    finished: false,
                });
            }
            if enabled {
                ensure_approved()?;
            }
            let after = desired(enabled, owner)?;
            let key = match open(true)? {
                Some(key) => Some(key),
                None if enabled => Some(create()?),
                None => None,
            };
            let before = key.as_ref().map(read).transpose()?.flatten();
            let current = before.as_ref().map(text).transpose()?;
            authorize_change(current.as_deref(), owner)?;
            let changed = before != after;
            let change = Self {
                key,
                before,
                after,
                changed,
                finished: false,
            };
            if changed {
                let key = change.key.as_ref().ok_or("开机启动配置不可用")?;
                if read(key)? != change.before {
                    return Err("开机启动配置已变化，未保存".into());
                }
                write(key, &change.after)?;
                if read(key)? != change.after {
                    return Err("无法确认开机启动配置的保存状态".into());
                }
            }
            if enabled {
                ensure_approved()?;
            }
            Ok(change)
        }
        pub fn commit(mut self) {
            self.finished = true;
            if let Ok(mut warning) = REFRESH_WARNING.lock() {
                *warning = None;
            }
        }
        pub fn rollback(mut self) -> Result<(), String> {
            if !self.changed {
                return Ok(());
            }
            let key = self.key.as_ref().ok_or("开机启动配置不可用")?;
            if read(key)? != self.after {
                return Err("开机启动配置在保存期间改变，请检查设置".into());
            }
            write(key, &self.before)?;
            if read(key)? != self.before {
                return Err("无法确认开机启动配置的恢复状态".into());
            }
            self.finished = true;
            Ok(())
        }
    }
    impl Drop for Change {
        fn drop(&mut self) {
            if !self.finished && self.changed {
                if let Some(key) = self.key.as_ref() {
                    if read(key).ok().as_ref() == Some(&self.after) {
                        let _ = write(key, &self.before);
                    }
                }
            }
        }
    }
}
#[cfg(windows)]
pub use platform::{refresh, registered, Change};
#[cfg(not(windows))]
pub fn registered(_: Option<&str>) -> Result<bool, String> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn refresh(_: &crate::system_preferences::SystemPreferences) -> Result<(), String> {
    Ok(())
}
#[cfg(not(windows))]
pub struct Change;
#[cfg(not(windows))]
impl Change {
    pub fn apply(enabled: bool, _: Option<&str>) -> Result<Self, String> {
        if enabled {
            Err("此平台暂不支持登录后自动启动".into())
        } else {
            Ok(Self)
        }
    }
    pub fn commit(self) {}
    pub fn rollback(self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_save_preserves_foreign_entries_and_allows_missing_owned_repair() {
        let owner = "04f2cc75-dd56-4891-8d8c-430d285773de";
        #[cfg(windows)]
        let path = Path::new("D:/old/Mewu.exe");
        #[cfg(not(windows))]
        let path = Path::new("/old/Mewu");
        let owned = owned_command(path, owner).unwrap();
        assert!(authorize_change(Some(&owned), Some(owner)).is_ok());
        assert!(authorize_change(None, Some(owner)).is_ok());
        assert!(authorize_change(None, None).is_ok());
        assert!(authorize_change(Some(&owned), None).is_err());
        assert!(authorize_change(Some(&command(path).unwrap()), Some(owner)).is_err());
        assert!(authorize_change(
            Some(&owned.replace(owner, "98c1f045-34b6-4926-9c30-6934a31bb2a6")),
            Some(owner)
        )
        .is_err());
    }
    #[test]
    fn only_known_enabled_startup_approval_authorizes_registration() {
        assert!(startup_approved(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        for record in [
            vec![],
            vec![2],
            vec![0; 12],
            vec![3; 12],
            vec![6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        ] {
            assert!(!startup_approved(&record));
        }
    }
    #[test]
    fn current_executable_is_quoted_and_startup_has_no_capture_activation() {
        #[cfg(windows)]
        let path = Path::new("D:/Programs/Mewu 1.0/Mewu.exe");
        #[cfg(not(windows))]
        let path = Path::new("/opt/Mewu 1.0/Mewu");
        let value = command(path).unwrap();
        assert!(value.starts_with('"'));
        assert!(value.ends_with("\" --background"));
        assert_eq!(
            crate::lifecycle::launch_action(&["Mewu".into(), "--background".into()]),
            crate::lifecycle::LaunchAction::Ignore
        );
        assert!(command(Path::new("relative.exe")).is_err());
        assert!(command(Path::new("D:/bad\"name.exe")).is_err());
    }
    #[test]
    fn refresh_only_accepts_a_saved_opt_in_with_exact_owned_command_and_never_recreates_missing() {
        let owner = "04f2cc75-dd56-4891-8d8c-430d285773de";
        #[cfg(windows)]
        let old = Path::new("D:/old/Mewu.exe");
        #[cfg(windows)]
        let next = Path::new("D:/new/Mewu.exe");
        #[cfg(not(windows))]
        let old = Path::new("/old/Mewu");
        #[cfg(not(windows))]
        let next = Path::new("/new/Mewu");
        let old_command = owned_command(old, owner).unwrap();
        let mut value = crate::system_preferences::SystemPreferences::defaults();
        value.startup_owner = Some(owner.into());
        assert_eq!(
            refresh_command(&value, Some(&old_command), next).unwrap(),
            None
        );
        value.launch_at_startup = true;
        assert_eq!(refresh_command(&value, None, next).unwrap(), None);
        assert_eq!(
            refresh_command(&value, Some(&old_command), next).unwrap(),
            Some(owned_command(next, owner).unwrap())
        );
        assert_eq!(
            refresh_command(&value, Some(&old_command), old).unwrap(),
            None
        );
        for foreign in [
            command(old).unwrap(),
            format!("{old_command} extra"),
            old_command.replace(owner, "98c1f045-34b6-4926-9c30-6934a31bb2a6"),
            old_command.replace("--background", "--settings"),
        ] {
            assert!(refresh_command(&value, Some(&foreign), next).is_err());
        }
        value.startup_owner = None;
        assert!(refresh_command(&value, Some(&old_command), next).is_err());
        assert!(owned_command(next, "not-uuid").is_err());
        assert!(owned_command(next, "00000000-0000-0000-0000-000000000000").is_err());
    }
}
