// SPDX-License-Identifier: MPL-2.0
//! Query installed fonts through Fontconfig; no subprocess, downloads or
//! assumptions about distribution-specific font paths.
//! https://fontconfig.pages.freedesktop.org/fontconfig/fontconfig-devel/
use std::{
    ffi::{c_char, c_int, c_void, CStr},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

#[link(name = "fontconfig")]
extern "C" {
    fn FcInitLoadConfigAndFonts() -> *mut c_void;
    fn FcConfigDestroy(config: *mut c_void);
    fn FcNameParse(name: *const u8) -> *mut c_void;
    fn FcPatternDestroy(pattern: *mut c_void);
    fn FcConfigSubstitute(config: *mut c_void, pattern: *mut c_void, kind: c_int) -> c_int;
    fn FcDefaultSubstitute(pattern: *mut c_void);
    fn FcFontMatch(config: *mut c_void, pattern: *mut c_void, result: *mut c_int) -> *mut c_void;
    fn FcPatternGetString(
        pattern: *const c_void,
        object: *const c_char,
        n: c_int,
        value: *mut *mut u8,
    ) -> c_int;
}
struct Config(*mut c_void);
impl Drop for Config {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { FcConfigDestroy(self.0) }
        }
    }
}
struct Pattern(*mut c_void);
impl Drop for Pattern {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { FcPatternDestroy(self.0) }
        }
    }
}

pub(super) fn paths() -> Result<Vec<PathBuf>, String> {
    let config = Config(unsafe { FcInitLoadConfigAndFonts() });
    if config.0.is_null() {
        return Err("formula_system_font_config".into());
    }
    let mut paths = Vec::new();
    for name in [c"sans-serif:lang=zh-cn", c"sans-serif"] {
        let query = Pattern(unsafe { FcNameParse(name.as_ptr().cast()) });
        if query.0.is_null() {
            continue;
        }
        // FcMatchPattern=0; FcResultMatch=0, as declared by fontconfig.h.
        if unsafe { FcConfigSubstitute(config.0, query.0, 0) } == 0 {
            continue;
        }
        unsafe { FcDefaultSubstitute(query.0) };
        let mut result = -1;
        let matched = Pattern(unsafe { FcFontMatch(config.0, query.0, &mut result) });
        if matched.0.is_null() || result != 0 {
            continue;
        }
        let mut file = std::ptr::null_mut();
        if unsafe { FcPatternGetString(matched.0, c"file".as_ptr(), 0, &mut file) } != 0
            || file.is_null()
        {
            continue;
        }
        let bytes = unsafe { CStr::from_ptr(file.cast()) }.to_bytes();
        if bytes.len() > 4096 {
            continue;
        }
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
        if path.is_absolute() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        Err("formula_system_font_missing".into())
    } else {
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resolves_installed_font_files_without_a_shell() {
        let paths = super::paths().unwrap();
        assert!(!paths.is_empty() && paths.len() <= 2);
        assert!(paths.iter().all(|p| p.is_absolute() && p.is_file()));
    }
}
