// SPDX-License-Identifier: MPL-2.0
use super::denied;
use interprocess::{
    local_socket::{prelude::*, ConnectOptions, GenericFilePath, ListenerOptions, Name, Stream},
    ConnectWaitMode,
};
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::PathBuf,
    time::Duration,
};

#[derive(Clone, Debug)]
pub(super) struct Endpoint {
    directory: PathBuf,
    socket: PathBuf,
    uid: libc::uid_t,
}
pub(crate) struct Guard {
    _lock: File,
}
impl Endpoint {
    pub(super) fn new(namespace: &str) -> io::Result<Self> {
        let uid = unsafe { libc::geteuid() };
        #[cfg(target_os = "linux")]
        let base = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(path) if !path.is_empty() => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(denied());
                }
                secure_directory(&path, uid)?;
                path
            }
            _ => PathBuf::from("/tmp"),
        };
        #[cfg(not(target_os = "linux"))]
        let base = PathBuf::from("/tmp");
        let directory = base.join(format!("mewu-control-{uid}"));
        let socket = directory.join(format!("{namespace}.sock"));
        // Darwin's sockaddr_un is shorter than Linux's. Never truncate names.
        use std::os::unix::ffi::OsStrExt;
        if socket.as_os_str().as_bytes().len() >= 104 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Local control endpoint path is too long",
            ));
        }
        Ok(Self {
            directory,
            socket,
            uid,
        })
    }
    pub(super) fn name(&self) -> io::Result<Name<'_>> {
        self.socket.as_path().to_fs_name::<GenericFilePath>()
    }
    pub(super) fn prepare(&self) -> io::Result<Guard> {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&self.directory) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        secure_directory(&self.directory, self.uid)?;
        let path = self.socket.with_extension("lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let metadata = lock.metadata()?;
        if !metadata.is_file() || metadata.uid() != self.uid || metadata.mode() & 0o777 != 0o600 {
            return Err(denied());
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Local control host is already running",
            ));
        }
        // The per-endpoint flock is held by every server worker until it exits.
        // Only a verified stale socket may be reclaimed after a crash.
        match fs::symlink_metadata(&self.socket) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() || metadata.uid() != self.uid {
                    return Err(denied());
                }
                match ConnectOptions::new()
                    .name(self.name()?)
                    .nonblocking_stream(true)
                    .wait_mode(ConnectWaitMode::Timeout(Duration::from_millis(20)))
                    .connect_sync()
                {
                    Ok(stream) => {
                        super::before_close(&stream);
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "Local control host is already running",
                        ));
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                        ) =>
                    {
                        fs::remove_file(&self.socket)?
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        Ok(Guard { _lock: lock })
    }
    pub(super) fn options<'a>(
        &self,
        options: ListenerOptions<'a>,
    ) -> io::Result<ListenerOptions<'a>> {
        // On macOS socket fchmod before bind is unsupported. Its parent is
        // already 0700, and after_bind applies 0600 before starting admission.
        #[cfg(target_os = "linux")]
        {
            use interprocess::os::unix::local_socket::ListenerOptionsExt;
            Ok(options.mode(0o600))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(options)
        }
    }
    pub(super) fn after_bind(&self) -> io::Result<()> {
        fs::set_permissions(&self.socket, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&self.socket)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != self.uid
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(super) fn verify_peer(&self, stream: &Stream, _: bool) -> io::Result<()> {
        secure_directory(&self.directory, self.uid)?;
        if stream.peer_creds()?.euid() != Some(self.uid) {
            return Err(denied());
        }
        Ok(())
    }
    pub(super) fn verify_server_image(
        &self,
        stream: &Stream,
        expected: &std::path::Path,
    ) -> io::Result<()> {
        self.verify_peer(stream, false)?;
        let expected = fs::canonicalize(expected)?;
        let pid = peer_pid(stream)?;
        let actual = peer_image(pid)?;
        if fs::canonicalize(actual)? != expected || peer_pid(stream)? != pid {
            return Err(denied());
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn peer_pid(stream: &Stream) -> io::Result<libc::pid_t> {
    stream
        .peer_creds()?
        .pid()
        .filter(|pid| *pid > 0)
        .ok_or_else(denied)
}
#[cfg(target_os = "linux")]
fn peer_image(pid: libc::pid_t) -> io::Result<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe"))
}

#[cfg(target_os = "macos")]
fn peer_pid(stream: &Stream) -> io::Result<libc::pid_t> {
    use std::os::fd::AsFd;
    let Stream::UdSocket(stream) = stream;
    let mut pid: libc::pid_t = 0;
    let mut size = std::mem::size_of_val(&pid) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_fd().as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut size,
        )
    };
    if result != 0 || size as usize != std::mem::size_of_val(&pid) || pid <= 0 {
        return Err(denied());
    }
    Ok(pid)
}
#[cfg(target_os = "macos")]
fn peer_image(pid: libc::pid_t) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let count = unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if count <= 0 {
        return Err(denied());
    }
    let end = bytes.iter().position(|b| *b == 0).ok_or_else(denied)?;
    bytes.truncate(end);
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peer_pid(_: &Stream) -> io::Result<libc::pid_t> {
    Err(io::ErrorKind::Unsupported.into())
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peer_image(_: libc::pid_t) -> io::Result<PathBuf> {
    Err(io::ErrorKind::Unsupported.into())
}
fn secure_directory(path: &std::path::Path, uid: libc::uid_t) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != uid
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(denied());
    }
    Ok(())
}
