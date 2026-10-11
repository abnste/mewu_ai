// SPDX-License-Identifier: MPL-2.0
//! Explicit navigation for a registered decoded value. No probing or fetching.
use url::Url;

pub fn http_url(text: &str) -> Option<Url> {
    if text.len() > 8 * 1024
        || text.chars().any(char::is_control)
        || text.trim() != text
        || !text
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
            && !text
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
    {
        return None;
    }
    let parsed = Url::parse(text).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.has_host()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(parsed)
}

/// Runs only after an explicit registered-code action. The caller must retain
/// its action permit until this returns and provide a final source/exit fence.
pub fn open(
    address: Url,
    allowed: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    // Revalidate the canonical URL even if another caller is added later.
    http_url(address.as_str()).ok_or("此内容不能作为网页打开")?;
    dispatch(address.to_string(), allowed)
}

pub fn registered_address(
    format: crate::code_decode::CodeFormat,
    text: &str,
) -> Result<String, String> {
    use crate::code_decode::CodeFormat;
    let address = match format {
        CodeFormat::Email => crate::screen_entities::mailto(text),
        CodeFormat::Phone => {
            crate::screen_entities::phone_number(text).map(|number| format!("tel:{number}"))
        }
        _ => http_url(text).map(|address| address.to_string()),
    };
    address.ok_or_else(|| "此识别内容不能打开".into())
}
pub fn open_registered(
    format: crate::code_decode::CodeFormat,
    text: &str,
    allowed: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    dispatch(registered_address(format, text)?, allowed)
}
// Private: every caller constructs a validated HTTP URL, encoded recipient,
// or global phone number. Never accepts caller-supplied schemes or arguments.
fn dispatch(
    address: String,
    allowed: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("mewu-code-open".into())
            .spawn(move || {
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
                unsafe {
                    CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()
                }
                .map_err(|_| "无法打开应用".to_string())?;
                struct Apartment;
                impl Drop for Apartment {
                    fn drop(&mut self) {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                }
                let _apartment = Apartment;
                let wide: Vec<u16> = address.encode_utf16().chain(Some(0)).collect();
                allowed()?;
                let result = unsafe {
                    ShellExecuteW(
                        Some(HWND::default()),
                        w!("open"),
                        PCWSTR(wide.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    )
                };
                if result.0 as isize > 32 {
                    Ok(())
                } else {
                    Err("无法打开应用，可复制内容后重试".into())
                }
            })
            .map_err(|_| "无法打开应用".to_string())?
            .join()
            .map_err(|_| "打开应用的任务已中断".to_string())?
    }
    #[cfg(target_os = "macos")]
    {
        crate::code_open_macos::dispatch(address, allowed)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (address, allowed);
        Err("此平台尚未接入打开应用".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_entities_cannot_inject_headers_arguments_or_other_protocols() {
        use crate::code_decode::CodeFormat as F;
        assert_eq!(
            registered_address(F::Email, "a?b&c=d@example.org").unwrap(),
            "mailto:a%3Fb%26c%3Dd@example.org"
        );
        assert_eq!(
            registered_address(F::Phone, "+8613800138000").unwrap(),
            "tel:+8613800138000"
        );
        assert!(registered_address(F::QrCode, "tel:+8613800138000").is_err());
        assert!(registered_address(F::Email, "a@example.com?bcc=x@example.org").is_err());
        assert!(registered_address(F::Phone, "123456;ext=1").is_err());
        assert!(registered_address(F::Url, "javascript:alert(1)").is_err());
    }
    #[test]
    fn accepts_only_explicit_http_urls_without_credentials_or_controls() {
        for text in [
            "https://example.com/a?q=hello#one",
            "HTTP://example.com/",
            "https://例子.测试/你好",
            "http://127.0.0.1:1234/",
        ] {
            assert!(http_url(text).is_some(), "{text}");
        }
        for text in [
            "",
            "example.com",
            "//example.com",
            "http:example.com",
            "https:/example.com",
            "file:///C:/data.txt",
            "javascript:alert(1)",
            "mailto:a@example.com",
            "https://user:pass@example.com",
            "https://user@example.com",
            " https://example.com",
            "https://example.com\n",
            "https://exam\tple.com",
            "https://",
            "https://example.com\0/a",
        ] {
            assert!(http_url(text).is_none(), "{text}");
        }
        assert!(http_url(&format!("https://example.com/{}", "a".repeat(8192))).is_none());
    }
}
