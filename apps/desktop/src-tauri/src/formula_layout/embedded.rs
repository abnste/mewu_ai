// SPDX-License-Identifier: MPL-2.0
//! First-use immutable font expansion. No startup cache or external font bundle.
use super::{fonts, render::digest, Resources};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
static PREPARATION: Mutex<()> = Mutex::new(());
struct EmbeddedFont {
    name: &'static str,
    sha256: &'static str,
    bytes: &'static [u8],
}
include!("embedded-fonts.in");

fn directory(parent: &Path, name: &str) -> Result<(PathBuf, std::fs::File), String> {
    let path = parent.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("formula_cache_directory".into()),
    }
    let pin = fonts::pin_directory(&path)?;
    let canonical = path.canonicalize().map_err(|_| "formula_cache_directory")?;
    if canonical.parent() != Some(parent) {
        return Err("formula_cache_directory".into());
    }
    Ok((canonical, pin))
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[cfg(windows)]
fn move_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
    }
    let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<_> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Exact same-directory generated regular files only, never a directory.
    // Unlike replacing rename, MoveFileW rejects an existing destination.
    // https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefilew
    if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn move_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(source, destination)?;
    fs::remove_file(source)
}
fn verify(path: &Path, font: &EmbeddedFont) -> Result<(), String> {
    let file = fonts::pin_file(path, font.bytes.len() as u64)?;
    let bytes = fonts::read_bounded(&file, font.bytes.len() as u64)?;
    if bytes.len() != font.bytes.len() || digest(&bytes) != font.sha256 {
        return Err("formula_font_pin_mismatch".into());
    }
    Ok(())
}
fn expand(directory: &Path, font: &EmbeddedFont) -> Result<(), String> {
    if digest(font.bytes) != font.sha256 {
        return Err("formula_embedded_font_pin".into());
    }
    let destination = directory.join(font.name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return verify(&destination, font),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("formula_font_cache_io".into()),
    }
    let temporary =
        Temporary(directory.join(format!(".formula-font-{}.tmp", uuid::Uuid::new_v4())));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let mut file = options
        .open(&temporary.0)
        .map_err(|_| "formula_font_cache_io")?;
    file.write_all(font.bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "formula_font_cache_io")?;
    drop(file);
    // Concurrent old/new hosts may race. Never overwrite either host's font;
    // accept a winner only if the exact bytes validate against this pin.
    match move_no_replace(&temporary.0, &destination) {
        Ok(()) => {}
        Err(_) => {
            verify(&destination, font)?;
        }
    }
    verify(&destination, font)
}

pub(super) fn prepare(app_owned_root: PathBuf) -> Result<Resources, String> {
    if !cfg!(windows) {
        return Err("formula_worker_platform_unsupported".into());
    }
    let _serial = PREPARATION.lock().map_err(|_| "formula_font_preparation")?;
    let _root_pin = fonts::pin_directory(&app_owned_root)?;
    let root = app_owned_root
        .canonicalize()
        .map_err(|_| "formula_cache_root")?;
    let (cache, _cache_pin) = directory(&root, "formula-cache")?;
    let (fonts, _font_pin) = directory(&cache, "fonts-776c1d3")?;
    for font in EMBEDDED {
        expand(&fonts, font)?;
    }
    let (jobs, _jobs_pin) = directory(&cache, "jobs")?;
    let windows =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:/Windows".into()));
    if !windows.is_absolute() {
        return Err("formula_system_font_path".into());
    }
    let system_fonts = windows.join("Fonts");
    let _system_pin = fonts::pin_directory(&system_fonts)?;
    // Primary is host-installed only. Missing glyphs remain explicit; fallback
    // outlines do not imply that we distribute, approve or copy system fonts.
    let unicode = [
        "msyh.ttc",
        "msyh.ttf",
        "simhei.ttf",
        "simsun.ttc",
        "segoeui.ttf",
    ]
    .iter()
    .map(|name| system_fonts.join(name))
    .find(|path| fonts::pin_file(path, 64 * 1024 * 1024).is_ok())
    .ok_or("formula_system_font_missing")?;
    Resources::new(fonts, unicode, jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_embedded_file_matches_inventory_and_license_pin() {
        assert_eq!(EMBEDDED.len(), 19);
        for font in EMBEDDED {
            assert_eq!(digest(font.bytes), font.sha256);
        }
        let inventory: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(
            "../../resources/formula/fonts/inventory.json"
        ))
        .unwrap();
        for item in &inventory {
            let found = EMBEDDED
                .iter()
                .find(|font| Some(font.name) == item["name"].as_str())
                .unwrap();
            assert_eq!(found.bytes.len() as u64, item["bytes"].as_u64().unwrap());
            assert_eq!(found.sha256, item["sha256"].as_str().unwrap());
        }
    }
}
