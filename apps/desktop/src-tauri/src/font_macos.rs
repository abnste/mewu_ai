// SPDX-License-Identifier: MPL-2.0
//! Resolve a bounded set of installed system fonts through Core Text instead
//! of assuming a fixed location for Apple's versioned font assets. This module
//! only returns existing local files; it never downloads or copies a font.
//!
//! https://developer.apple.com/documentation/coretext/ctfontcreatewithnameandoptions(_:_:_:_:)
//! https://developer.apple.com/documentation/coretext/ctfontcopypostscriptname(_:)
//! https://developer.apple.com/documentation/coretext/kctfonturlattribute

use core_foundation::{
    base::{CFOptionFlags, CFType, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
    url::{CFURLCopyHostName, CFURLCopyScheme, CFURL},
};
use std::{ffi::c_void, path::PathBuf};

// Public CTFontOptions values from Apple's headers and the pinned
// objc2-core-text 0.3.2 generated source. No activation/download/matching
// progress API is used. These options are available before macOS 12.3.
const PREVENT_AUTO_ACTIVATION: CFOptionFlags = 1 << 0;
const PREVENT_AUTO_DOWNLOAD: CFOptionFlags = 1 << 1;
const PREFER_SYSTEM_FONT: CFOptionFlags = 1 << 2;

#[link(name = "CoreText", kind = "framework")]
extern "C" {
    // CTFontRef is an opaque Core Foundation object, with the same pointer ABI
    // as CFTypeRef. Only a null transform is passed. CGFloat is f64 on both
    // supported 64-bit macOS architectures. Create/Copy results are +1 owned.
    fn CTFontCreateWithNameAndOptions(
        name: CFStringRef,
        size: f64,
        matrix: *const c_void,
        options: CFOptionFlags,
    ) -> CFTypeRef;
    fn CTFontCopyPostScriptName(font: CFTypeRef) -> CFStringRef;
    fn CTFontCopyAttribute(font: CFTypeRef, attribute: CFStringRef) -> CFTypeRef;
    static kCTFontURLAttribute: CFStringRef;
}

fn installed_font_path(postscript_name: &str) -> Option<PathBuf> {
    let name = CFString::new(postscript_name);
    // Core Text can return a different font when the requested name is absent.
    // Verify the actual PostScript name before accepting its URL. CFType and
    // CFString wrappers release all retained objects on every early return.
    let font = unsafe {
        let raw = CTFontCreateWithNameAndOptions(
            name.as_concrete_TypeRef(),
            12.,
            std::ptr::null(),
            PREVENT_AUTO_ACTIVATION | PREVENT_AUTO_DOWNLOAD | PREFER_SYSTEM_FONT,
        );
        if raw.is_null() {
            return None;
        }
        CFType::wrap_under_create_rule(raw)
    };
    let actual_name = unsafe {
        let raw = CTFontCopyPostScriptName(font.as_CFTypeRef());
        if raw.is_null() {
            return None;
        }
        CFString::wrap_under_create_rule(raw)
    };
    if actual_name.to_string() != postscript_name {
        return None;
    }
    let attribute = unsafe {
        if kCTFontURLAttribute.is_null() {
            return None;
        }
        let raw = CTFontCopyAttribute(font.as_CFTypeRef(), kCTFontURLAttribute);
        if raw.is_null() {
            return None;
        }
        CFType::wrap_under_create_rule(raw)
    };
    // Apple's documented attribute type is CFURL. Check the runtime type
    // before using URL APIs rather than casting arbitrary attribute data.
    let url = attribute.downcast::<CFURL>()?;
    let scheme = unsafe {
        let raw = CFURLCopyScheme(url.as_concrete_TypeRef());
        if raw.is_null() {
            return None;
        }
        CFString::wrap_under_create_rule(raw)
    };
    if scheme.to_string() != "file" {
        return None;
    }
    let host = unsafe {
        let raw = CFURLCopyHostName(url.as_concrete_TypeRef());
        if raw.is_null() {
            None
        } else {
            Some(CFString::wrap_under_create_rule(raw))
        }
    };
    if host.is_some_and(|host| {
        let value = host.to_string();
        !value.is_empty() && !value.eq_ignore_ascii_case("localhost")
    }) {
        return None;
    }
    // CFURL performs native file-system conversion; percent escaping, Unicode,
    // and filesystem bytes are not decoded by hand or lossy string parsing.
    let path = url.to_path()?;
    if !path.is_absolute() || !path.starts_with("/System/Library") {
        return None;
    }
    let path = path.canonicalize().ok()?;
    // Exact names and PreferSystemFont alone do not prove provenance. Reject
    // user-installed duplicates and any symlink that leaves Apple's system
    // library. Modern reserved fonts can live in PrivateFrameworks here.
    (path.starts_with("/System/Library") && path.is_file()).then_some(path)
}

/// At most three distinct, real local files for exact installed font names.
/// The caller retains its existing total/per-file byte budgets when reading.
/// An unavailable font is skipped without discarding other usable fonts.
pub fn font_paths() -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::with_capacity(3);
    for name in ["PingFangSC-Regular", "Helvetica", "ArialUnicodeMS"] {
        if let Some(path) = installed_font_path(name) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    if paths.is_empty() {
        Err("未找到可用于绘制文字的本机系统字体".into())
    } else {
        Ok(paths)
    }
}

/// Resolved only for bold drawings; do not grow the ordinary screenshot cache.
pub fn bold_font_paths() -> Vec<PathBuf> {
    // Recent macOS ships PingFang as a >32MiB variable collection. Keep the
    // caller's budgets: try the installed static CJK semibold collection first
    // instead of growing the ordinary font cache or activating/downloading it.
    [
        "HiraginoSansGB-W6",
        "Helvetica-Bold",
        "PingFangSC-Semibold",
        "Arial-BoldMT",
    ]
    .iter()
    .filter_map(|name| installed_font_path(name))
    .collect()
}
