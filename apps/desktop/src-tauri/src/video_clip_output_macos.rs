// SPDX-License-Identifier: MPL-2.0
//! Private AVFoundation staging and exclusive publication in the held output directory.
use crate::video_clip_contract::ClipError;
use std::{
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
};

fn name(value: &std::ffi::OsStr) -> Result<CString, ClipError> {
    CString::new(value.as_bytes()).map_err(|_| ClipError::UnsupportedPath)
}
fn open_at(parent: &File, name: &CStr, directory: bool) -> Result<File, ClipError> {
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(ClipError::FileUnavailable);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn identity(file: &File) -> Result<(u64, u64), ClipError> {
    let meta = file.metadata().map_err(|_| ClipError::FileUnavailable)?;
    Ok((meta.dev(), meta.ino()))
}
pub(crate) struct Destination {
    parent_path: PathBuf,
    parent: File,
    directory: File,
    directory_name: CString,
    final_name: CString,
    staging_path: PathBuf,
    published: Option<(u64, u64)>,
    committed: bool,
}
impl Destination {
    pub(crate) fn new(output: &Path) -> Result<Self, ClipError> {
        if !output.is_absolute() || output.extension().is_none_or(|v| v != "mp4") {
            return Err(ClipError::UnsupportedPath);
        }
        let parent_path = output.parent().ok_or(ClipError::UnsupportedPath)?;
        if parent_path
            .canonicalize()
            .map_err(|_| ClipError::FileUnavailable)?
            != parent_path
        {
            return Err(ClipError::UnsupportedPath);
        }
        match std::fs::symlink_metadata(output) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(ClipError::FileUnavailable),
        }
        let final_name = name(output.file_name().ok_or(ClipError::UnsupportedPath)?)?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent_path)
            .map_err(|_| ClipError::FileUnavailable)?;
        let directory_name = CString::new(format!(".mewu-video-{}", uuid::Uuid::new_v4())).unwrap();
        if unsafe { libc::mkdirat(parent.as_raw_fd(), directory_name.as_ptr(), 0o700) } != 0 {
            return Err(ClipError::FileUnavailable);
        }
        let directory = match open_at(&parent, &directory_name, true) {
            Ok(value) => value,
            Err(error) => {
                unsafe {
                    libc::unlinkat(
                        parent.as_raw_fd(),
                        directory_name.as_ptr(),
                        libc::AT_REMOVEDIR,
                    );
                }
                return Err(error);
            }
        };
        let staging_path = parent_path
            .join(directory_name.to_str().unwrap())
            .join("output.mp4");
        let result = Self {
            parent_path: parent_path.to_owned(),
            parent,
            directory,
            directory_name,
            final_name,
            staging_path,
            published: None,
            committed: false,
        };
        result.check()?;
        Ok(result)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.staging_path
    }
    pub(crate) fn check(&self) -> Result<(), ClipError> {
        for (path, file) in [
            (self.parent_path.as_path(), &self.parent),
            (self.staging_path.parent().unwrap(), &self.directory),
        ] {
            let meta = std::fs::symlink_metadata(path).map_err(|_| ClipError::FileUnavailable)?;
            if !meta.is_dir()
                || meta.file_type().is_symlink()
                || (meta.dev(), meta.ino()) != identity(file)?
            {
                return Err(ClipError::FileUnavailable);
            }
        }
        Ok(())
    }
    pub(crate) fn publish(&mut self, checked_file: &File) -> Result<(), ClipError> {
        self.check()?;
        let staging = c"output.mp4";
        let actual = open_at(&self.directory, staging, false)?;
        if identity(&actual)? != identity(checked_file)? {
            return Err(ClipError::OutputInvalid);
        }
        actual.sync_all().map_err(|_| ClipError::FileUnavailable)?;
        // linkat is an atomic no-replace publication on this same filesystem.
        if unsafe {
            libc::linkat(
                self.directory.as_raw_fd(),
                staging.as_ptr(),
                self.parent.as_raw_fd(),
                self.final_name.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(ClipError::FileUnavailable);
        }
        self.published = Some(identity(&actual)?);
        let final_file = open_at(&self.parent, &self.final_name, false)?;
        if identity(&final_file)? != self.published.unwrap() {
            return Err(ClipError::OutputInvalid);
        }
        self.parent
            .sync_all()
            .map_err(|_| ClipError::FileUnavailable)?;
        self.check()
    }
    pub(crate) fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for Destination {
    fn drop(&mut self) {
        if !self.committed {
            if let Some(published) = self.published {
                if open_at(&self.parent, &self.final_name, false)
                    .is_ok_and(|file| identity(&file) == Ok(published))
                {
                    unsafe {
                        libc::unlinkat(self.parent.as_raw_fd(), self.final_name.as_ptr(), 0);
                    }
                }
            }
        }
        unsafe {
            libc::unlinkat(self.directory.as_raw_fd(), c"output.mp4".as_ptr(), 0);
        }
        if identity(&self.directory).is_ok_and(|held| {
            open_at(&self.parent, &self.directory_name, true)
                .is_ok_and(|file| identity(&file) == Ok(held))
        }) {
            unsafe {
                libc::unlinkat(
                    self.parent.as_raw_fd(),
                    self.directory_name.as_ptr(),
                    libc::AT_REMOVEDIR,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!("mewu-mac-output-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }
    #[test]
    fn publication_is_exclusive_and_failed_publication_preserves_existing_bytes() {
        let root = root();
        let output = root.join("视频.mp4");
        let mut destination = Destination::new(&output).unwrap();
        std::fs::write(destination.path(), b"decoded checked media").unwrap();
        let file = File::open(destination.path()).unwrap();
        std::fs::write(&output, b"existing").unwrap();
        assert_eq!(destination.publish(&file), Err(ClipError::FileUnavailable));
        drop(destination);
        assert_eq!(std::fs::read(&output).unwrap(), b"existing");
        assert!(Destination::new(&output).is_err());
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn commit_retains_checked_bytes_but_uncommitted_publication_is_removed() {
        let root = root();
        let output = root.join("result.mp4");
        for committed in [false, true] {
            let mut destination = Destination::new(&output).unwrap();
            std::fs::write(destination.path(), b"checked media").unwrap();
            let file = File::open(destination.path()).unwrap();
            destination.publish(&file).unwrap();
            if committed {
                destination.commit();
            }
            drop(destination);
            assert_eq!(output.exists(), committed);
            assert_eq!(
                std::fs::read_dir(&root).unwrap().count(),
                usize::from(committed)
            );
        }
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn replaced_parent_is_refused_and_cleanup_uses_held_directory() {
        let root = root();
        let parent = root.join("parent");
        std::fs::create_dir(&parent).unwrap();
        let destination = Destination::new(&parent.join("result.mp4")).unwrap();
        std::fs::write(destination.path(), b"private media").unwrap();
        let old = root.join("old");
        std::fs::rename(&parent, &old).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("keep"), b"unrelated").unwrap();
        assert_eq!(destination.check(), Err(ClipError::FileUnavailable));
        drop(destination);
        assert_eq!(std::fs::read_dir(&old).unwrap().count(), 0);
        assert_eq!(std::fs::read(parent.join("keep")).unwrap(), b"unrelated");
        std::fs::remove_file(parent.join("keep")).unwrap();
        std::fs::remove_dir(parent).unwrap();
        std::fs::remove_dir(old).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
