// SPDX-License-Identifier: MPL-2.0
use crate::contract::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPackage {
    pub manifest: ExtensionManifest,
    pub modules: BTreeMap<String, String>,
}

pub(crate) fn module_path(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        && name.split('/').all(|p| {
            let stem = p.split('.').next().unwrap_or("").to_ascii_uppercase();
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with('.')
                && !p.ends_with(' ')
                && !matches!(
                    stem.as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
        && (name.ends_with(".js") || name.ends_with(".mjs"))
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

pub fn validate_package(
    manifest: &ExtensionManifest,
    modules: &BTreeMap<String, String>,
) -> Result<(), ExtensionError> {
    let invalid = ExtensionError::InvalidPackage;
    if manifest.schema_version != 2
        || manifest.host_api != HOST_API
        || manifest.runtime != "javascript"
    {
        return Err(invalid("unsupported extension API"));
    }
    if !identifier(&manifest.id)
        || semver::Version::parse(&manifest.version).is_err()
        || !bounded_text(&manifest.name, 128, false)
        || !bounded_text(&manifest.description, 4000, true)
        || !bounded_text(&manifest.license, 128, false)
        || manifest
            .publisher
            .as_ref()
            .is_some_and(|p| !bounded_text(p, 128, false))
    {
        return Err(invalid("invalid identity or metadata"));
    }
    if serde_json::to_vec(manifest)
        .map_err(|_| invalid("invalid manifest"))?
        .len()
        > MAX_MANIFEST_BYTES
    {
        return Err(invalid("manifest exceeds limit"));
    }
    if !module_path(&manifest.entry) || !modules.contains_key(&manifest.entry) {
        return Err(invalid("entry module is missing or unsafe"));
    }
    for list in [&manifest.permissions, &manifest.tags] {
        if list.len() > 64
            || list.iter().any(|p| !identifier(p))
            || list.iter().collect::<BTreeSet<_>>().len() != list.len()
        {
            return Err(invalid("invalid or duplicate permissions or tags"));
        }
    }
    if manifest.dependencies.len() > 32
        || manifest.dependencies.iter().any(|(id, version)| {
            !identifier(id) || id == &manifest.id || semver::VersionReq::parse(version).is_err()
        })
    {
        return Err(invalid("invalid extension dependencies"));
    }
    if modules.is_empty() || modules.len() > MAX_MODULES {
        return Err(invalid("module count exceeds limit"));
    }
    let mut total = 0usize;
    let mut names = BTreeSet::new();
    for (name, source) in modules {
        if !module_path(name)
            || source.len() > MAX_MODULE_BYTES
            || source.starts_with('\u{feff}')
            || source.contains('\0')
            || !names.insert(name.to_ascii_lowercase())
        {
            return Err(invalid("unsafe or oversized module"));
        }
        total = total
            .checked_add(source.len())
            .ok_or(invalid("package exceeds limit"))?;
    }
    if total > MAX_PACKAGE_BYTES {
        return Err(invalid("package exceeds limit"));
    }
    Ok(())
}

fn bounded_read(path: &Path, max: usize) -> Result<Vec<u8>, ExtensionError> {
    let file = fs::File::open(path)
        .map_err(|_| ExtensionError::InvalidPackage("cannot read package file"))?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ExtensionError::InvalidPackage("cannot read package file"))?;
    if bytes.len() > max {
        return Err(ExtensionError::InvalidPackage("package file exceeds limit"));
    }
    Ok(bytes)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// The caller must stage packages in an application-owned immutable directory.
/// Reject links/reparse points rather than following an author-controlled path.
pub fn load_directory_package(directory: &Path) -> Result<ExtensionPackage, ExtensionError> {
    let metadata = fs::symlink_metadata(directory)
        .map_err(|_| ExtensionError::InvalidPackage("package directory is missing"))?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(ExtensionError::InvalidPackage(
            "package directory is unsafe",
        ));
    }
    let mut modules = BTreeMap::new();
    let mut manifest = None;
    let mut total = 0usize;
    fn visit(
        base: &Path,
        directory: &Path,
        depth: usize,
        modules: &mut BTreeMap<String, String>,
        manifest: &mut Option<ExtensionManifest>,
        total: &mut usize,
    ) -> Result<(), ExtensionError> {
        if depth > 16 {
            return Err(ExtensionError::InvalidPackage(
                "package directory depth exceeds limit",
            ));
        }
        for entry in fs::read_dir(directory)
            .map_err(|_| ExtensionError::InvalidPackage("cannot read package directory"))?
        {
            let entry =
                entry.map_err(|_| ExtensionError::InvalidPackage("cannot read package entry"))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| ExtensionError::InvalidPackage("cannot inspect package entry"))?;
            if is_link(&metadata) {
                return Err(ExtensionError::InvalidPackage(
                    "package links are forbidden",
                ));
            }
            if metadata.is_dir() {
                visit(base, &path, depth + 1, modules, manifest, total)?;
                continue;
            }
            if !metadata.is_file() {
                return Err(ExtensionError::InvalidPackage("unsupported package entry"));
            }
            let name = path
                .strip_prefix(base)
                .map_err(|_| ExtensionError::InvalidPackage("package path escaped"))?
                .to_str()
                .ok_or(ExtensionError::InvalidPackage("non UTF-8 package path"))?
                .replace('\\', "/");
            if name == "mewu-plugin.json" {
                let bytes = bounded_read(&path, MAX_MANIFEST_BYTES)?;
                *manifest = Some(
                    serde_json::from_slice(&bytes)
                        .map_err(|_| ExtensionError::InvalidPackage("invalid manifest JSON"))?,
                );
            } else if module_path(&name) {
                if modules.len() >= MAX_MODULES {
                    return Err(ExtensionError::InvalidPackage("module count exceeds limit"));
                }
                let bytes = bounded_read(&path, MAX_MODULE_BYTES)?;
                *total = total
                    .checked_add(bytes.len())
                    .ok_or(ExtensionError::InvalidPackage("package exceeds limit"))?;
                if *total > MAX_PACKAGE_BYTES {
                    return Err(ExtensionError::InvalidPackage("package exceeds limit"));
                }
                modules.insert(
                    name,
                    String::from_utf8(bytes)
                        .map_err(|_| ExtensionError::InvalidPackage("module is not UTF-8"))?,
                );
            } else {
                return Err(ExtensionError::InvalidPackage("unsupported package file"));
            }
        }
        Ok(())
    }
    visit(
        directory,
        directory,
        0,
        &mut modules,
        &mut manifest,
        &mut total,
    )?;
    let manifest = manifest.ok_or(ExtensionError::InvalidPackage("manifest is missing"))?;
    validate_package(&manifest, &modules)?;
    Ok(ExtensionPackage { manifest, modules })
}
