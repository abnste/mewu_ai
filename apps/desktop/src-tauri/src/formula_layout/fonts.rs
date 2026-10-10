// SPDX-License-Identifier: MPL-2.0
use super::{
    protocol::{FontFaceIdentity, FontIdentity},
    render::digest,
};
use serde::Deserialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

pub(super) const INVENTORY_SHA256: &str =
    "0c9de5b0f1f11ed337cb534fb3432fba94c69c68df5301139fdb289949f07798";
const MAX_FONT: u64 = 64 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    source: String,
    sha256: String,
    license: String,
    bytes: u64,
}

fn ordinary(metadata: &fs::Metadata, directory: bool) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    !metadata.file_type().is_symlink()
        && if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        }
}
pub(super) fn pin_directory(path: &Path) -> Result<File, String> {
    if !path.is_absolute() {
        return Err("formula_directory_invalid".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "formula_directory_io")?;
    if !ordinary(&metadata, true) {
        return Err("formula_directory_invalid".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(1 | 2)
            .custom_flags(0x0200_0000 | 0x0020_0000);
    }
    let file = options.open(path).map_err(|_| "formula_directory_pin")?;
    if !ordinary(&file.metadata().map_err(|_| "formula_directory_io")?, true) {
        return Err("formula_directory_invalid".into());
    }
    Ok(file)
}
pub(super) fn pin_file(path: &Path, maximum: u64) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let file = options.open(path).map_err(|_| "formula_file_io")?;
    let metadata = file.metadata().map_err(|_| "formula_file_io")?;
    if !ordinary(&metadata, false) || metadata.len() > maximum {
        return Err("formula_file_budget".into());
    }
    Ok(file)
}
pub(super) fn read_bounded(mut file: &File, maximum: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    (&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "formula_file_io")?;
    if bytes.len() as u64 > maximum {
        return Err("formula_file_budget".into());
    }
    Ok(bytes)
}

/// These real handles must remain owned by the process permit until child+Job+IO
/// have exited. Files cannot be replaced while the renderer reopens their paths.
pub(super) struct PinnedFonts {
    _directory: File,
    _files: Vec<File>,
    pub directory: PathBuf,
    pub unicode_font: PathBuf,
    pub primary_sha256: String,
}
pub(super) fn pin_fonts(directory: &Path, unicode_font: &Path) -> Result<PinnedFonts, String> {
    let directory_pin = pin_directory(directory)?;
    let directory = directory
        .canonicalize()
        .map_err(|_| "formula_font_directory")?;
    let inventory = include_bytes!("../../resources/formula/fonts/inventory.json");
    if digest(inventory) != INVENTORY_SHA256 {
        return Err("formula_compiled_inventory".into());
    }
    let entries: Vec<Entry> =
        serde_json::from_slice(inventory).map_err(|_| "formula_compiled_inventory")?;
    if entries.len() != 19 {
        return Err("formula_compiled_inventory".into());
    }
    let mut files = Vec::with_capacity(entries.len() + 1);
    for entry in &entries {
        if Path::new(&entry.name).file_name().and_then(|v| v.to_str()) != Some(entry.name.as_str())
            || entry.license != "OFL-1.1"
            || !entry
                .source
                .contains("776c1d37bafa3bf445a0ab9377c55fe77f7a0133")
        {
            return Err("formula_compiled_inventory".into());
        }
        let path = directory.join(&entry.name);
        let file = pin_file(&path, entry.bytes)?;
        let bytes = read_bounded(&file, entry.bytes)?;
        if bytes.len() as u64 != entry.bytes || digest(&bytes) != entry.sha256 {
            return Err("formula_font_pin_mismatch".into());
        }
        if path
            .canonicalize()
            .map_err(|_| "formula_font_path")?
            .parent()
            != Some(directory.as_path())
        {
            return Err("formula_font_path".into());
        }
        files.push(file);
    }
    // Extra TTFs must not silently influence a supposed pinned directory.
    let actual = fs::read_dir(&directory)
        .map_err(|_| "formula_font_directory")?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "formula_font_directory")?;
    let count = actual
        .iter()
        .filter(|v| {
            Path::new(v)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ttf"))
        })
        .count();
    if count != entries.len() {
        return Err("formula_font_inventory_mismatch".into());
    }
    if !unicode_font.is_absolute() {
        return Err("formula_primary_font_path".into());
    }
    let unicode = pin_file(unicode_font, MAX_FONT)?;
    let primary_sha256 = digest(&read_bounded(&unicode, MAX_FONT)?);
    let unicode_font = unicode_font
        .canonicalize()
        .map_err(|_| "formula_primary_font_path")?;
    files.push(unicode);
    Ok(PinnedFonts {
        _directory: directory_pin,
        _files: files,
        directory,
        unicode_font,
        primary_sha256,
    })
}

fn face(
    data: Option<&ratex_unicode_font::FontData>,
    index: Option<u32>,
) -> Result<Option<FontFaceIdentity>, String> {
    match (data, index) {
        (None, None) => Ok(None),
        (Some(data), Some(index)) if !data.is_empty() && data.len() as u64 <= MAX_FONT => {
            Ok(Some(FontFaceIdentity {
                bytes: data.len() as u64,
                sha256: digest(data.as_slice()),
                face_index: index,
            }))
        }
        _ => Err("formula_font_identity".into()),
    }
}
/// Capture actual OnceLock-owned bytes, not a family name or installed path.
/// Discovery/load happens inside the worker deadline. This is not an OS RAM cap
/// or a redistribution approval for installed fonts.
pub(super) fn snapshot(primary_sha256: &str) -> Result<FontIdentity, String> {
    let primary = face(
        ratex_unicode_font::unicode_font_data_ref(),
        ratex_unicode_font::unicode_font_face_index(),
    )?;
    let secondary = face(
        ratex_unicode_font::fallback_font_data_ref(),
        ratex_unicode_font::fallback_font_face_index(),
    )?;
    let emoji = face(
        ratex_unicode_font::emoji_font_data_ref(),
        ratex_unicode_font::emoji_font_face_index(),
    )?;
    if primary
        .as_ref()
        .is_none_or(|face| face.sha256 != primary_sha256)
    {
        return Err("formula_primary_font_mismatch".into());
    }
    let bytes = [&primary, &secondary, &emoji]
        .iter()
        .filter_map(|face| face.as_ref())
        .map(|face| face.bytes)
        .sum::<u64>();
    if bytes > 128 * 1024 * 1024 {
        return Err("formula_font_budget".into());
    }
    Ok(FontIdentity {
        katex_inventory_sha256: INVENTORY_SHA256.into(),
        primary,
        secondary,
        emoji,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_bytes_and_unique_filenames_are_pinned() {
        let bytes = include_bytes!("../../resources/formula/fonts/inventory.json");
        assert_eq!(digest(bytes), INVENTORY_SHA256);
        let entries: Vec<Entry> = serde_json::from_slice(bytes).unwrap();
        let names: std::collections::BTreeSet<_> =
            entries.iter().map(|entry| &entry.name).collect();
        assert_eq!(entries.len(), 19);
        assert_eq!(names.len(), 19);
    }
}
