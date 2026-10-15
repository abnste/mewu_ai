// SPDX-License-Identifier: MPL-2.0
//! Identify Mewu's UI process before any window or startup-error dialog exists.
//! https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-setcurrentprocessexplicitappusermodelid
use windows::{core::PCWSTR, Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID};

const APP_ID: &str = "dev.mewu.remake";

pub(crate) fn initialize() -> windows::core::Result<()> {
    let wide: Vec<u16> = APP_ID.encode_utf16().chain(Some(0)).collect();
    // This sets only the calling process. It does not change another app's
    // windows, installer identity, shortcuts, or durable user configuration.
    unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(wide.as_ptr())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    #[test]
    fn process_identity_uses_the_existing_bundle_identifier() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"].as_str(), Some(APP_ID));
        assert!(APP_ID.encode_utf16().count() <= 128);
        assert!(!APP_ID.chars().any(char::is_whitespace));
        assert!(!APP_ID.contains('\0'));
    }

    #[test]
    fn native_process_identity_roundtrips_in_an_isolated_child_without_ui() {
        const PROBE: &str = "MEWU_IDENTITY_NATIVE_TEST_CHILD";
        if std::env::var_os(PROBE).as_deref() == Some(std::ffi::OsStr::new("1")) {
            initialize().unwrap();
            unsafe {
                let id = windows::Win32::UI::Shell::GetCurrentProcessExplicitAppUserModelID()
                    .expect("read the actual process AppUserModelID");
                let actual = id.to_string();
                windows::Win32::System::Com::CoTaskMemFree(Some(id.0.cast()));
                assert_eq!(actual.unwrap(), APP_ID);
            }
            println!("MEWU_IDENTITY_NATIVE_ROUNDTRIP_OK");
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .args([
                "--exact",
                "windows_identity::tests::native_process_identity_roundtrips_in_an_isolated_child_without_ui",
                "--nocapture",
            ])
            .env(PROBE, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated identity probe failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "MEWU_IDENTITY_NATIVE_ROUNDTRIP_OK"));
    }
}
