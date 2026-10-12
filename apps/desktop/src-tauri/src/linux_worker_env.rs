// SPDX-License-Identifier: MPL-2.0
//! Resource environment derived from the actual loaded AppDir, never inherited loader paths.
#![cfg(target_os = "linux")]
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
};

const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: usize = 16;
const MAX_PATH: usize = 4096;
const RESERVED: &[&str] = &[
    "APPDIR",
    "APPIMAGE",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "LD_AUDIT",
    "LD_ORIGIN_PATH",
    "LD_DEBUG",
    "LD_DEBUG_OUTPUT",
    "LD_PROFILE",
    "LD_PROFILE_OUTPUT",
    "LD_TRACE_LOADED_OBJECTS",
    "LD_BIND_NOT",
    "LD_BIND_NOW",
    "LD_DYNAMIC_WEAK",
    "LD_HWCAP_MASK",
    "LD_ASSUME_KERNEL",
    "LD_USE_LOAD_BIAS",
    "GLIBC_TUNABLES",
    "GCONV_PATH",
    "GTK_MODULES",
    "GTK3_MODULES",
    "GTK_DATA_PREFIX",
    "GTK_EXE_PREFIX",
    "GTK_PATH",
    "GTK_THEME",
    "GTK_IM_MODULE_FILE",
    "GDK_PIXBUF_MODULE_FILE",
    "GDK_PIXBUF_MODULEDIR",
    "GI_TYPELIB_PATH",
    "GIO_MODULE_DIR",
    "GIO_EXTRA_MODULES",
    "GSETTINGS_SCHEMA_DIR",
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_PATH_1_0",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GST_PLUGIN_SCANNER",
    "GST_PLUGIN_SCANNER_1_0",
    "GST_PTP_HELPER_1_0",
    "GST_REGISTRY_REUSE_PLUGIN_SCANNER",
    "PIPEWIRE_MODULE_DIR",
    "SPA_PLUGIN_DIR",
    "PIPEWIRE_CONFIG_DIR",
    "PIPEWIRE_CONFIG_NAME",
    "PYTHONHOME",
    "PYTHONPATH",
    "PERLLIB",
    "QT_PLUGIN_PATH",
];

pub(super) struct ResourceEnvironment {
    root: Option<(PathBuf, File)>,
    values: Vec<(&'static str, OsString)>,
}
impl ResourceEnvironment {
    pub fn capture(source: &File) -> io::Result<Self> {
        let Some(appdir) = std::env::var_os("APPDIR") else {
            return Ok(Self {
                root: None,
                values: Vec::new(),
            });
        };
        let provided = PathBuf::from(appdir);
        bounded(&provided)?;
        if !provided.is_absolute() {
            return Err(invalid());
        }
        let root = fs::canonicalize(provided)?;
        bounded(&root)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&root)?;
        validate(&root, &directory, source)?;
        let mut values = vec![("APPDIR", root.clone().into_os_string())];
        for (name, relative, directory_required) in [
            ("GTK_DATA_PREFIX", "", true),
            ("GTK_EXE_PREFIX", "usr", true),
            ("GTK_PATH", "usr/lib/gtk-3.0", true),
            (
                "GTK_IM_MODULE_FILE",
                "usr/lib/gtk-3.0/3.0.0/immodules.cache",
                false,
            ),
            (
                "GDK_PIXBUF_MODULE_FILE",
                "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache",
                false,
            ),
            ("GI_TYPELIB_PATH", "usr/lib/girepository-1.0", true),
            ("GIO_MODULE_DIR", "usr/lib/gio/modules", true),
            ("GSETTINGS_SCHEMA_DIR", "usr/share/glib-2.0/schemas", true),
            ("GST_PLUGIN_PATH_1_0", "usr/lib/gstreamer-1.0", true),
            ("GST_PLUGIN_SYSTEM_PATH_1_0", "usr/lib/gstreamer-1.0", true),
            ("PIPEWIRE_MODULE_DIR", "usr/lib/pipewire-0.3", true),
            ("SPA_PLUGIN_DIR", "usr/lib/spa-0.2", true),
            ("PIPEWIRE_CONFIG_DIR", "usr/share/pipewire", true),
            (
                "GST_PLUGIN_SCANNER_1_0",
                "usr/lib/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner",
                false,
            ),
            (
                "GST_PTP_HELPER_1_0",
                "usr/lib/gstreamer1.0/gstreamer-1.0/gst-ptp-helper",
                false,
            ),
        ] {
            let path = root.join(relative);
            let canonical = fs::canonicalize(&path)?;
            let metadata = fs::metadata(&canonical)?;
            if !canonical.starts_with(&root)
                || metadata.dev() != directory.metadata()?.dev()
                || (directory_required && !metadata.is_dir())
                || (!directory_required && !metadata.is_file())
                || (name.starts_with("GST_") && !directory_required && metadata.mode() & 0o111 == 0)
            {
                return Err(invalid());
            }
            // Fixed AppDir paths retain the normal relative SONAME/module layout.
            values.push((name, path.into_os_string()));
        }
        values.push(("GST_REGISTRY_REUSE_PLUGIN_SCANNER", OsString::from("no")));
        values.push(("PIPEWIRE_CONFIG_NAME", OsString::from("client.conf")));
        values.push(("GTK_THEME", OsString::from("Adwaita")));
        let mut data = root.join("usr/share").into_os_string();
        data.push(":/usr/local/share:/usr/share");
        values.push(("XDG_DATA_DIRS", data));
        Ok(Self {
            root: Some((root, directory)),
            values,
        })
    }
    pub fn apply(&self, command: &mut Command, source: &File) -> io::Result<()> {
        if let Some((root, pinned)) = &self.root {
            validate(root, pinned, source)?;
        }
        // Runs after every caller env/env_clear call, before any exec. Even a
        // non-AppImage worker must not inherit or reintroduce arbitrary loaders.
        use std::os::unix::ffi::OsStrExt;
        let loader_names: Vec<_> = std::env::vars_os()
            .map(|(name, _)| name)
            .chain(command.get_envs().map(|(name, _)| name.to_owned()))
            .filter(|name| name.as_bytes().starts_with(b"LD_"))
            .collect();
        for name in loader_names {
            command.env_remove(name);
        }
        for name in RESERVED {
            command.env_remove(name);
        }
        for (name, value) in &self.values {
            command.env(name, value);
        }
        Ok(())
    }
}

fn invalid() -> io::Error {
    io::Error::from(io::ErrorKind::PermissionDenied)
}
fn bounded(path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH || bytes.contains(&0) || bytes.contains(&b':') {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn validate(root: &Path, pinned: &File, source: &File) -> io::Result<()> {
    let original = pinned.metadata()?;
    let current = fs::symlink_metadata(root)?;
    if !current.is_dir() || (current.dev(), current.ino()) != (original.dev(), original.ino()) {
        return Err(invalid());
    }
    let binary = fs::metadata(root.join("usr/bin/MewuAI"))?;
    let source = source.metadata()?;
    if !binary.is_file() || (binary.dev(), binary.ino()) != (source.dev(), source.ino()) {
        return Err(invalid());
    }
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: valid live directory FD and writable statvfs storage.
    if unsafe { libc::fstatvfs(pinned.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // A real read-only filesystem can safely contain historical writable mode
    // bits. An extracted tree receives no such exception and is never chmod'ed.
    let readonly = unsafe { stat.assume_init() }.f_flag & libc::ST_RDONLY != 0;
    let mut remaining = MAX_ENTRIES;
    let mut visited = HashSet::new();
    inspect(
        root,
        root,
        original.dev(),
        readonly,
        0,
        &mut remaining,
        &mut visited,
    )
}
fn inspect(
    root: &Path,
    path: &Path,
    device: u64,
    readonly: bool,
    depth: usize,
    remaining: &mut usize,
    visited: &mut HashSet<(u64, u64)>,
) -> io::Result<()> {
    if depth > MAX_DEPTH || *remaining == 0 {
        return Err(invalid());
    }
    *remaining -= 1;
    bounded(path)?;
    let canonical = fs::canonicalize(path)?;
    bounded(&canonical)?;
    if !canonical.starts_with(root) {
        return Err(invalid());
    }
    let metadata = fs::metadata(&canonical)?;
    if metadata.dev() != device || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(invalid());
    }
    if !readonly {
        // SAFETY: geteuid is a side-effect-free libc query, outside pre_exec.
        let euid = unsafe { libc::geteuid() };
        if metadata.mode() & 0o022 != 0 || (metadata.uid() != 0 && metadata.uid() != euid) {
            return Err(invalid());
        }
    }
    if metadata.is_dir() && visited.insert((metadata.dev(), metadata.ino())) {
        for entry in fs::read_dir(&canonical)? {
            inspect(
                root,
                &entry?.path(),
                device,
                readonly,
                depth + 1,
                remaining,
                visited,
            )?;
        }
    }
    Ok(())
}
