//! Startup-only logical asset identity -> current physical file location.
//! No legacy directory is opened, canonicalized, or used as a fallback.
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Debug)]
pub struct AssetLocations {
    pub physical_assets_root: PathBuf,
    pub legacy_asset_roots: Vec<PathBuf>,
}
pub struct AssetLease {
    pub path: PathBuf,
    pub file: File,
    _parents: Vec<File>,
}
static CONTEXT: OnceLock<AssetLocations> = OnceLock::new();

pub fn install_context(locations: AssetLocations) -> Result<(), String> {
    CONTEXT
        .set(locations)
        .map_err(|_| "asset_context_already_installed".into())
}
pub fn resolve_path(stored: &Path, supplied_root: &Path) -> Result<PathBuf, String> {
    match CONTEXT.get() {
        Some(context) => context.resolve_path(stored, supplied_root),
        None => AssetLocations::new(supplied_root.to_path_buf(), Vec::new())?
            .resolve_path(stored, supplied_root),
    }
}
impl AssetLocations {
    pub fn new(
        physical_assets_root: PathBuf,
        legacy_asset_roots: Vec<PathBuf>,
    ) -> Result<Self, String> {
        lexical(&physical_assets_root)?;
        for root in &legacy_asset_roots {
            lexical(root)?;
        }
        Ok(Self {
            physical_assets_root,
            legacy_asset_roots,
        })
    }
    /// Compatibility adapter for existing path-returning readers. Existing source
    /// owners must retain their second open/identity check; this drops its lease.
    pub fn resolve_path(&self, stored: &Path, supplied_root: &Path) -> Result<PathBuf, String> {
        Ok(self.open_asset(stored, supplied_root)?.path)
    }
    pub fn open_asset(&self, stored: &Path, supplied_root: &Path) -> Result<AssetLease, String> {
        let logical = lexical(stored)?;
        let supplied = lexical(supplied_root)?;
        let configured = lexical(&self.physical_assets_root)?;
        let roots = if components_eq(&supplied, &configured) {
            std::iter::once(self.physical_assets_root.as_path())
                .chain(self.legacy_asset_roots.iter().map(PathBuf::as_path))
                .collect::<Vec<_>>()
        } else {
            vec![supplied_root]
        };
        let mut suffix = None;
        for root in roots {
            let prefix = lexical(root)?;
            if logical.len() > prefix.len()
                && components_eq(&logical[..prefix.len().min(logical.len())], &prefix)
            {
                suffix = Some(&logical[prefix.len()..]);
                break;
            }
        }
        let mut current = supplied_root.to_path_buf();
        let mut parents = pin_ancestors(&current).map_err(|_| "asset_directory_unsafe")?;
        let canonical_root = current
            .canonicalize()
            .map_err(|_| "asset_directory_missing")?;
        // Writers may have stored Windows' canonical spelling of this same
        // pinned root (including package virtualization). Never resolve an
        // arbitrary stored path or search other directories for a basename.
        if suffix.is_none() {
            let prefix = lexical(&canonical_root)?;
            if logical.len() > prefix.len() && components_eq(&logical[..prefix.len()], &prefix) {
                suffix = Some(&logical[prefix.len()..]);
            }
        }
        let suffix = suffix.ok_or("asset_path_outside_registered_roots")?;
        for (index, part) in suffix.iter().enumerate() {
            current.push(part);
            if index + 1 < suffix.len() {
                parents.push(pin_directory(&current).map_err(|_| "asset_directory_unsafe")?);
            }
        }
        let file = open_regular(&current).map_err(|_| "asset_file_missing_or_unsafe")?;
        let canonical = current
            .canonicalize()
            .map_err(|_| "asset_file_missing_or_unsafe")?;
        let actual = lexical(&canonical)?;
        let root = lexical(&canonical_root)?;
        if actual.len() <= root.len() || !components_eq(&actual[..root.len()], &root) {
            return Err("asset_path_outside_current_root".into());
        }
        Ok(AssetLease {
            path: canonical,
            file,
            _parents: parents,
        })
    }
}
pub(crate) fn components_eq(a: &[OsString], b: &[OsString]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| ordinal_eq(a, b))
}
fn ordinal_eq(a: &OsString, b: &OsString) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let a: Vec<u16> = a.encode_wide().collect();
        let b: Vec<u16> = b.encode_wide().collect();
        #[link(name = "kernel32")]
        extern "system" {
            fn CompareStringOrdinal(
                a: *const u16,
                an: i32,
                b: *const u16,
                bn: i32,
                ignore: i32,
            ) -> i32;
        }
        if a.len() > i32::MAX as usize || b.len() > i32::MAX as usize {
            return false;
        }
        unsafe {
            CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1) == 2
        }
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
pub(crate) fn lexical(path: &Path) -> Result<Vec<OsString>, String> {
    if !path.is_absolute() {
        return Err("path_must_be_absolute".into());
    }
    let text = path.to_str().ok_or("path_encoding")?;
    if text.chars().any(char::is_control) {
        return Err("path_control".into());
    }
    // Components normalizes embedded '.', so reject it in the original spelling.
    if text.split(['/', '\\']).any(|p| p == "." || p == "..") {
        return Err("path_traversal".into());
    }
    let mut output = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                #[cfg(windows)]
                {
                    use std::path::Prefix;
                    match prefix.kind() {
                        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => output.push(OsString::from(
                            format!("{}:", (d as char).to_ascii_uppercase()),
                        )),
                        _ => return Err("path_device_or_unc".into()),
                    }
                }
                #[cfg(not(windows))]
                {
                    output.push(prefix.as_os_str().to_os_string());
                }
            }
            Component::RootDir => output.push(OsString::from("/")),
            Component::Normal(part) => {
                let value = part.to_str().ok_or("path_encoding")?;
                if value.is_empty()
                    || value.contains(':')
                    || value.contains(['*', '?', '"', '<', '>', '|'])
                    || value.ends_with(['.', ' '])
                {
                    return Err("path_ambiguous_component".into());
                }
                #[cfg(windows)]
                {
                    let name = value.split('.').next().unwrap_or("").to_ascii_uppercase();
                    if matches!(
                        name.as_str(),
                        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                    ) || name.is_ascii()
                        && name.len() == 4
                        && matches!(&name[..3], "COM" | "LPT")
                        && matches!(name.as_bytes()[3], b'1'..=b'9')
                    {
                        return Err("path_reserved_component".into());
                    }
                }
                output.push(part.to_os_string());
            }
            _ => return Err("path_traversal".into()),
        }
    }
    Ok(output)
}
pub(crate) fn ordinary(meta: &fs::Metadata, directory: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("reparse"));
        }
    }
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(io::Error::other("unsafe_type"));
    }
    Ok(())
}
pub(crate) fn pin_directory(path: &Path) -> io::Result<File> {
    ordinary(&fs::symlink_metadata(path)?, true)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x02000000 | 0x00200000);
    }
    let file = options.open(path)?;
    ordinary(&file.metadata()?, true)?;
    Ok(file)
}
pub(crate) fn pin_ancestors(path: &Path) -> io::Result<Vec<File>> {
    lexical(path).map_err(io::Error::other)?;
    let mut paths = path
        .ancestors()
        .filter(|p| p.is_absolute())
        .collect::<Vec<_>>();
    paths.reverse();
    paths.into_iter().map(pin_directory).collect()
}
pub(crate) fn open_regular(path: &Path) -> io::Result<File> {
    ordinary(&fs::symlink_metadata(path)?, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    ordinary(&file.metadata()?, false)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    struct Fixture {
        root: PathBuf,
        current: PathBuf,
        old: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("mewu-locator-synthetic-{}", Uuid::new_v4()));
            assert!(!root
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("c:\\hermes"));
            fs::create_dir(&root).unwrap();
            let current = root.join("current");
            let old = root.join("old");
            fs::create_dir(&current).unwrap();
            fs::create_dir(&old).unwrap();
            fs::write(current.join("asset.bin"), b"current").unwrap();
            fs::write(old.join("asset.bin"), b"old private bytes").unwrap();
            Self { root, current, old }
        }
        fn locations(&self) -> AssetLocations {
            AssetLocations::new(self.current.clone(), vec![self.old.clone()]).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn alias_never_falls_back_to_read_old_directory() {
        let f = Fixture::new();
        let locations = f.locations();
        let p = locations
            .resolve_path(&f.old.join("asset.bin"), &f.current)
            .unwrap();
        assert_eq!(fs::read(p).unwrap(), b"current");
        fs::remove_file(f.current.join("asset.bin")).unwrap();
        assert!(locations
            .resolve_path(&f.old.join("asset.bin"), &f.current)
            .is_err());
        assert_eq!(
            fs::read(f.old.join("asset.bin")).unwrap(),
            b"old private bytes"
        );
    }
    #[test]
    fn canonical_spelling_of_current_root_is_readable_without_allowing_other_canonical_roots() {
        let f = Fixture::new();
        let locations = AssetLocations::new(f.current.clone(), vec![]).unwrap();
        let canonical = f.current.canonicalize().unwrap().join("asset.bin");
        assert_eq!(
            fs::read(locations.resolve_path(&canonical, &f.current).unwrap()).unwrap(),
            b"current"
        );
        assert!(locations
            .resolve_path(&f.old.canonicalize().unwrap().join("asset.bin"), &f.current)
            .is_err());
    }
    #[test]
    fn alias_is_lexical_and_does_not_require_old_root_exists() {
        let f = Fixture::new();
        let locations = f.locations();
        fs::remove_dir_all(&f.old).unwrap();
        assert_eq!(
            fs::read(
                locations
                    .resolve_path(&f.old.join("asset.bin"), &f.current)
                    .unwrap()
            )
            .unwrap(),
            b"current"
        );
    }
    #[test]
    fn aliases_cannot_authorize_temporary_supplied_root() {
        let f = Fixture::new();
        let temporary = f.root.join("temporary");
        fs::create_dir(&temporary).unwrap();
        fs::write(temporary.join("asset.bin"), b"temp").unwrap();
        let locations = f.locations();
        assert!(locations
            .resolve_path(&f.old.join("asset.bin"), &temporary)
            .is_err());
        assert_eq!(
            fs::read(
                locations
                    .resolve_path(&temporary.join("asset.bin"), &temporary)
                    .unwrap()
            )
            .unwrap(),
            b"temp"
        );
    }
    #[test]
    fn traversal_ads_prefix_collision_and_directory_assets_rejected() {
        let f = Fixture::new();
        let locations = f.locations();
        for p in [
            f.old.join("../current/asset.bin"),
            f.old.join("./asset.bin"),
            f.old.join("asset.bin:stream"),
            f.root.join("old-sibling/asset.bin"),
            f.old.clone(),
        ] {
            assert!(locations.resolve_path(&p, &f.current).is_err(), "{p:?}");
        }
        fs::create_dir(f.current.join("folder")).unwrap();
        assert!(locations
            .resolve_path(&f.old.join("folder"), &f.current)
            .is_err());
    }
    #[cfg(windows)]
    #[test]
    fn windows_ordinal_case_and_verbatim_disk_match() {
        let f = Fixture::new();
        let locations = f.locations();
        let old = PathBuf::from(f.old.to_string_lossy().to_uppercase()).join("asset.bin");
        assert_eq!(
            fs::read(locations.resolve_path(&old, &f.current).unwrap()).unwrap(),
            b"current"
        );
        let verbatim = PathBuf::from(format!("\\\\?\\{}", f.old.display())).join("asset.bin");
        assert!(locations.resolve_path(&verbatim, &f.current).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_in_current_tree_is_rejected() {
        let f = Fixture::new();
        std::os::unix::fs::symlink(f.old.join("asset.bin"), f.current.join("linked.bin")).unwrap();
        assert!(f
            .locations()
            .resolve_path(&f.old.join("linked.bin"), &f.current)
            .is_err());
    }
}
