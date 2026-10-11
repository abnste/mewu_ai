// SPDX-License-Identifier: MPL-2.0
//! Native AppImage installation identity; the official updater verifies and
//! installs signed raw bytes. No arbitrary IPC destination or shell command.
use super::appimage;
use rustix::fs::{open, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use tauri::Manager;

const LOCATION: &str = "update_appimage_location";
const LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Identity {
    fn file(meta: &fs::Metadata) -> Result<Self, &'static str> {
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.nlink() != 1
            || !(1..=LIMIT).contains(&meta.len())
        {
            return Err(LOCATION);
        }
        Ok(Self {
            dev: meta.dev(),
            ino: meta.ino(),
            uid: meta.uid(),
            mode: meta.mode(),
            size: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Parent {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
}
fn parent(path: &Path) -> Result<Parent, &'static str> {
    let meta = fs::symlink_metadata(path).map_err(|_| LOCATION)?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o300 != 0o300
        || meta.mode() & 0o022 != 0
    {
        return Err(LOCATION);
    }
    Ok(Parent {
        dev: meta.dev(),
        ino: meta.ino(),
        uid: meta.uid(),
        mode: meta.mode(),
    })
}
fn canonical(path: PathBuf) -> Result<PathBuf, &'static str> {
    if !path.is_absolute() || path.canonicalize().map_err(|_| LOCATION)? != path {
        return Err(LOCATION);
    }
    Ok(path)
}
fn open_image(path: &Path) -> Result<(File, Identity), &'static str> {
    let before = Identity::file(&fs::symlink_metadata(path).map_err(|_| LOCATION)?)?;
    if before.uid != rustix::process::geteuid().as_raw()
        || before.mode & 0o300 != 0o300
        || before.mode & 0o022 != 0
    {
        return Err(LOCATION);
    }
    let mut file = File::from(
        open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| LOCATION)?,
    );
    if Identity::file(&file.metadata().map_err(|_| LOCATION)?)? != before {
        return Err(LOCATION);
    }
    appimage::validate(&mut file, before.size).map_err(|_| LOCATION)?;
    file.seek(SeekFrom::Start(0)).map_err(|_| LOCATION)?;
    Ok((file, before))
}
struct Admission {
    path: PathBuf,
    appdir: PathBuf,
    executable: PathBuf,
    parent: Parent,
    image: Identity,
    binary: Identity,
    file: File,
}
fn admit_paths(
    path: PathBuf,
    appdir: PathBuf,
    executable: PathBuf,
) -> Result<Admission, &'static str> {
    if !cfg!(target_arch = "x86_64") {
        return Err(LOCATION);
    }
    let path = canonical(path)?;
    let appdir = canonical(appdir)?;
    let executable = canonical(executable)?;
    if path.extension().and_then(|value| value.to_str()) != Some("AppImage")
        || executable != appdir.join("usr/bin/MewuAI")
        || path.starts_with(&appdir)
    {
        return Err(LOCATION);
    }
    let parent = parent(path.parent().ok_or(LOCATION)?)?;
    let (file, image) = open_image(&path)?;
    let binary = Identity::file(&fs::symlink_metadata(&executable).map_err(|_| LOCATION)?)?;
    Ok(Admission {
        path,
        appdir,
        executable,
        parent,
        image,
        binary,
        file,
    })
}
fn admit() -> Result<Admission, &'static str> {
    admit_paths(
        std::env::var_os("APPIMAGE").ok_or(LOCATION)?.into(),
        std::env::var_os("APPDIR").ok_or(LOCATION)?.into(),
        std::env::current_exe().map_err(|_| LOCATION)?,
    )
}
fn hash(file: &mut File, expected: &Identity) -> Result<[u8; 32], &'static str> {
    file.seek(SeekFrom::Start(0)).map_err(|_| LOCATION)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| LOCATION)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .filter(|total| *total <= LIMIT)
            .ok_or(LOCATION)?;
        digest.update(&buffer[..count]);
    }
    if total != expected.size
        || Identity::file(&file.metadata().map_err(|_| LOCATION)?)? != *expected
    {
        return Err(LOCATION);
    }
    Ok(digest.finalize().into())
}
pub fn supported() -> bool {
    admit().is_ok()
}
/// Tauri caches APPIMAGE for relaunch. Refuse a destination that differs from
/// that original launch path, even if process environment is changed later.
pub fn appimage_path(app: &tauri::AppHandle) -> Result<PathBuf, &'static str> {
    let admission = admit()?;
    let environment = app.env();
    let cached = environment.appimage.ok_or(LOCATION)?;
    let cached_directory = environment.appdir.ok_or(LOCATION)?;
    if PathBuf::from(cached) != admission.path
        || PathBuf::from(cached_directory) != admission.appdir
    {
        return Err(LOCATION);
    }
    Ok(admission.path)
}
pub struct Installation {
    path: PathBuf,
    appdir: PathBuf,
    executable: PathBuf,
    parent: Parent,
    image: Identity,
    binary: Identity,
    digest: [u8; 32],
}
impl Installation {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn capture(expected: PathBuf) -> Result<Self, &'static str> {
        let admission = admit()?;
        if admission.path != expected {
            return Err(LOCATION);
        }
        Self::from_admission(admission)
    }
    fn from_admission(mut admission: Admission) -> Result<Self, &'static str> {
        let digest = hash(&mut admission.file, &admission.image)?;
        let result = Self {
            path: admission.path,
            appdir: admission.appdir,
            executable: admission.executable,
            parent: admission.parent,
            image: admission.image,
            binary: admission.binary,
            digest,
        };
        result.check_admission(admit_paths(
            result.path.clone(),
            result.appdir.clone(),
            result.executable.clone(),
        )?)?;
        Ok(result)
    }
    pub fn check(&self) -> Result<(), &'static str> {
        let admission = admit()?;
        self.check_admission(admission)
    }
    fn check_admission(&self, mut admission: Admission) -> Result<(), &'static str> {
        if admission.path != self.path
            || admission.appdir != self.appdir
            || admission.executable != self.executable
            || admission.parent != self.parent
            || admission.image != self.image
            || admission.binary != self.binary
            || hash(&mut admission.file, &admission.image)? != self.digest
        {
            return Err(LOCATION);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    struct Fixture {
        root: PathBuf,
        image: PathBuf,
        appdir: PathBuf,
        executable: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("mewu-appimage-preflight-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let root = root.canonicalize().unwrap();
            let appdir = root.join("mounted-fixture");
            fs::create_dir_all(appdir.join("usr/bin")).unwrap();
            let executable = appdir.join("usr/bin/MewuAI");
            fs::write(&executable, b"synthetic non-executed app binary").unwrap();
            let image = root.join("MewuAI.AppImage");
            fs::write(&image, appimage::tests::fixture()).unwrap();
            fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
            Self {
                root,
                image,
                appdir,
                executable,
            }
        }
        fn admission(&self) -> Result<Admission, &'static str> {
            admit_paths(
                self.image.clone(),
                self.appdir.clone(),
                self.executable.clone(),
            )
        }
        fn capture(&self) -> Installation {
            Installation::from_admission(self.admission().unwrap()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
    #[test]
    fn original_owned_appimage_identity_is_stable_without_writing_user_data() {
        let fixture = Fixture::new();
        let guard = fixture.capture();
        assert_eq!(guard.path(), fixture.image);
        let before = fs::read(&fixture.image).unwrap();
        guard.check_admission(fixture.admission().unwrap()).unwrap();
        assert_eq!(fs::read(&fixture.image).unwrap(), before);
    }
    #[test]
    fn same_bytes_new_inode_or_modified_original_image_never_pass_preflight() {
        let fixture = Fixture::new();
        let guard = fixture.capture();
        let bytes = fs::read(&fixture.image).unwrap();
        fs::rename(&fixture.image, fixture.root.join("original.AppImage")).unwrap();
        fs::write(&fixture.image, &bytes).unwrap();
        fs::set_permissions(&fixture.image, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(guard.check_admission(fixture.admission().unwrap()).is_err());
        let guard = fixture.capture();
        let mut modified = bytes;
        modified[400] ^= 1;
        fs::write(&fixture.image, modified).unwrap();
        assert!(guard.check_admission(fixture.admission().unwrap()).is_err());
    }
    #[test]
    fn symlink_hardlink_parent_and_binary_changes_are_refused() {
        let fixture = Fixture::new();
        let guard = fixture.capture();
        let link = fixture.root.join("alias.AppImage");
        symlink(&fixture.image, &link).unwrap();
        assert!(admit_paths(
            link.clone(),
            fixture.appdir.clone(),
            fixture.executable.clone()
        )
        .is_err());
        fs::remove_file(&link).unwrap();
        fs::hard_link(&fixture.image, &link).unwrap();
        assert!(fixture.admission().is_err());
        fs::remove_file(&link).unwrap();
        fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(fixture.admission().is_err());
        fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&fixture.executable, b"replacement binary").unwrap();
        assert!(guard.check_admission(fixture.admission().unwrap()).is_err());
    }
    #[test]
    fn wrong_binary_location_read_only_image_and_plain_binary_are_manual_only() {
        let fixture = Fixture::new();
        assert!(admit_paths(
            fixture.image.clone(),
            fixture.appdir.clone(),
            std::env::current_exe().unwrap()
        )
        .is_err());
        fs::set_permissions(&fixture.image, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(fixture.admission().is_err());
        fs::set_permissions(&fixture.image, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&fixture.image, b"ordinary development binary").unwrap();
        assert!(fixture.admission().is_err());
    }
}
