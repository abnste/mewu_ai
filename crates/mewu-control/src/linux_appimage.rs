// SPDX-License-Identifier: MPL-2.0
//! Explicit AppImage-only peer proof. Environment never substitutes for the
//! authenticated socket peer and exact bytes of the caller's own running image.
use crate::endpoint::denied;
use interprocess::local_socket::{prelude::*, Stream};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, BufReader, Read, Seek, SeekFrom},
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{
            ffi::OsStringExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    time::Instant,
};

const MAX_EXECUTABLE: u64 = 512 * 1024 * 1024;
const MAX_ENVIRONMENT: usize = 1024 * 1024;
const MAX_IMAGE_PATH: usize = 32 * 1024;

// SO_PEERCRED keeps the PID observed at listen/connect time. A pidfd obtained
// from this socket, rather than pidfd_open(a numeric PID), closes the gap before
// the first /proc open. This explicit API fails closed on pre-6.5 kernels.
struct PeerLifetime(OwnedFd);
impl PeerLifetime {
    fn socket(socket: RawFd) -> io::Result<Self> {
        let mut descriptor: libc::c_int = -1;
        let mut length = std::mem::size_of_val(&descriptor) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                socket,
                libc::SOL_SOCKET,
                libc::SO_PEERPIDFD,
                (&mut descriptor as *mut libc::c_int).cast(),
                &mut length,
            )
        } != 0
        {
            let error = io::Error::last_os_error();
            return Err(
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOPROTOOPT | libc::EOPNOTSUPP)
                ) {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "AppImage console requires SO_PEERPIDFD",
                    )
                } else {
                    error
                },
            );
        }
        if descriptor < 0 {
            return Err(denied());
        }
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        if length as usize != std::mem::size_of::<libc::c_int>() {
            return Err(denied());
        }
        if unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }
    fn alive(&self, deadline: Instant) -> io::Result<()> {
        loop {
            before_deadline(deadline)?;
            let mut watch = libc::pollfd {
                fd: self.0.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            match unsafe { libc::poll(&mut watch, 1, 0) } {
                0 => return Ok(()),
                -1 => {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                }
                _ => return Err(denied()), // Exit, reap, error and invalid fd all reject.
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    owner: u32,
    group: u32,
    mode: u32,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    links: u64,
}
impl Stamp {
    fn read(metadata: Metadata) -> io::Result<Self> {
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(denied());
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            group: metadata.gid(),
            mode: metadata.mode(),
            bytes: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            links: metadata.nlink(),
        })
    }
    fn file(file: &File) -> io::Result<Self> {
        Self::read(file.metadata()?)
    }
    fn unchanged(&self, file: &File) -> io::Result<()> {
        if Self::file(file)? == *self {
            Ok(())
        } else {
            Err(denied())
        }
    }
}

struct Process {
    directory: File,
}
impl Process {
    fn open(pid: libc::pid_t, uid: libc::uid_t) -> io::Result<Self> {
        if pid <= 0 {
            return Err(denied());
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(format!("/proc/{pid}"))?;
        if directory.metadata()?.uid() != uid {
            return Err(denied());
        }
        Ok(Self { directory })
    }
    fn member(&self, name: &'static [u8], follow_exe: bool) -> io::Result<File> {
        let flags =
            libc::O_RDONLY | libc::O_CLOEXEC | if follow_exe { 0 } else { libc::O_NOFOLLOW };
        // /proc/PID/exe is a kernel magic link; following it opens the actual
        // running executable inode. Keep the proc directory open to avoid
        // resolving a subsequently reused numeric PID in later checks.
        let fd = unsafe { libc::openat(self.directory.as_raw_fd(), name.as_ptr().cast(), flags) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn executable(&self) -> io::Result<File> {
        self.member(b"exe\0", true)
    }
    fn appimage(&self) -> io::Result<PathBuf> {
        appimage_environment(self.member(b"environ\0", false)?)
    }
    fn executable_unchanged(&self, original: &Stamp) -> io::Result<()> {
        original.unchanged(&self.executable()?)
    }
}

// Bound the proc read, retaining only APPIMAGE bytes. Other environment entries
// are discarded as they stream past and are never returned, logged or parsed.
fn appimage_environment(reader: impl Read) -> io::Result<PathBuf> {
    const KEY: &[u8] = b"APPIMAGE=";
    let mut found = None;
    let (mut position, mut matches, mut total) = (0usize, true, 0usize);
    let mut value = Vec::new();
    for byte in BufReader::new(reader).bytes() {
        let byte = byte?;
        total += 1;
        if total > MAX_ENVIRONMENT {
            return Err(denied());
        }
        if byte == 0 {
            if matches && position >= KEY.len() {
                if found.is_some() || value.is_empty() {
                    return Err(denied());
                }
                let path = PathBuf::from(OsString::from_vec(std::mem::take(&mut value)));
                if !path.is_absolute() {
                    return Err(denied());
                }
                found = Some(path);
            }
            position = 0;
            matches = true;
            value.clear();
        } else {
            if position < KEY.len() {
                matches &= byte == KEY[position];
            } else if matches {
                if value.len() >= MAX_IMAGE_PATH {
                    return Err(denied());
                }
                value.push(byte);
            }
            position = position.checked_add(1).ok_or_else(denied)?;
        }
    }
    if position != 0 {
        return Err(denied());
    }
    found.ok_or_else(denied)
}

struct Image {
    canonical: PathBuf,
    file: File,
    stamp: Stamp,
}
impl Image {
    fn open(path: &Path) -> io::Result<Self> {
        let canonical = fs::canonicalize(path)?;
        if !fs::symlink_metadata(&canonical)?.is_file() {
            return Err(denied());
        }
        // Even a replacement FIFO must not block before its fstat rejection.
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(&canonical)?;
        let stamp = Stamp::file(&file)?;
        let mut magic = [0u8; 11];
        file.read_exact(&mut magic)?;
        if magic[..4] != *b"\x7fELF" || magic[8..] != *b"AI\x02" {
            return Err(denied());
        }
        stamp.unchanged(&file)?;
        Ok(Self {
            canonical,
            file,
            stamp,
        })
    }
    fn unchanged(&self) -> io::Result<()> {
        self.stamp.unchanged(&self.file)?;
        let metadata = fs::symlink_metadata(&self.canonical)?;
        if metadata.file_type().is_symlink() || Stamp::read(metadata)? != self.stamp {
            return Err(denied());
        }
        Ok(())
    }
}

fn before_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(io::ErrorKind::TimedOut.into())
    }
}
fn hash(file: &mut File, stamp: &Stamp, deadline: Instant) -> io::Result<[u8; 32]> {
    if stamp.bytes > MAX_EXECUTABLE {
        return Err(denied());
    }
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        before_deadline(deadline)?;
        let read = match file.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if read == 0 {
            break;
        }
        bytes = bytes.checked_add(read as u64).ok_or_else(denied)?;
        if bytes > stamp.bytes {
            return Err(denied());
        }
        hash.update(&buffer[..read]);
    }
    if bytes != stamp.bytes {
        return Err(denied());
    }
    stamp.unchanged(file)?;
    Ok(hash.finalize().into())
}

fn same_images(self_image: &Image, peer_image: &Image) -> io::Result<()> {
    // Both canonical ordinary-file handles must identify the same original
    // AppImage inode, including owner/size/timestamps. Hardlink aliases are the
    // same file; another equally named or byte-identical image is not.
    if self_image.stamp != peer_image.stamp {
        return Err(denied());
    }
    self_image.unchanged()?;
    peer_image.unchanged()
}
fn same_executables(
    own: &mut File,
    peer: &mut File,
    deadline: Instant,
    after_hash: impl FnOnce(),
) -> io::Result<()> {
    let own_stamp = Stamp::file(own)?;
    let peer_stamp = Stamp::file(peer)?;
    if own_stamp.bytes != peer_stamp.bytes
        || own_stamp.mode & 0o111 == 0
        || peer_stamp.mode & 0o111 == 0
    {
        return Err(denied());
    }
    let own_hash = hash(own, &own_stamp, deadline)?;
    let peer_hash = hash(peer, &peer_stamp, deadline)?;
    after_hash();
    own_stamp.unchanged(own)?;
    peer_stamp.unchanged(peer)?;
    if own_hash == peer_hash {
        Ok(())
    } else {
        Err(denied())
    }
}

pub(crate) fn verify_server(stream: &Stream, deadline: Instant) -> io::Result<()> {
    before_deadline(deadline)?;
    let credentials = stream.peer_creds()?;
    let uid = unsafe { libc::geteuid() };
    let pid = peer_identity(credentials.euid(), credentials.pid(), uid)?;
    let Stream::UdSocket(socket) = stream;
    let lifetime = PeerLifetime::socket(socket.as_fd().as_raw_fd())?;
    lifetime.alive(deadline)?;
    let own = Process::open(unsafe { libc::getpid() }, uid)?;
    let peer = Process::open(pid, uid)?;
    let mut own_exe = own.executable()?;
    let mut peer_exe = peer.executable()?;
    let own_stamp = Stamp::file(&own_exe)?;
    let peer_stamp = Stamp::file(&peer_exe)?;
    let own_path = own.appimage()?;
    let peer_path = peer.appimage()?;
    let own_image = Image::open(&own_path)?;
    let peer_image = Image::open(&peer_path)?;
    same_images(&own_image, &peer_image)?;
    same_executables(&mut own_exe, &mut peer_exe, deadline, || {})?;
    own.executable_unchanged(&own_stamp)?;
    peer.executable_unchanged(&peer_stamp)?;
    if own.appimage()? != own_path || peer.appimage()? != peer_path {
        return Err(denied());
    }
    same_images(&own_image, &peer_image)?;
    let after = stream.peer_creds()?;
    if after.euid() != Some(uid) || after.pid() != Some(pid) {
        return Err(denied());
    }
    lifetime.alive(deadline)
}

fn peer_identity(
    peer_uid: Option<libc::uid_t>,
    peer_pid: Option<libc::pid_t>,
    own_uid: libc::uid_t,
) -> io::Result<libc::pid_t> {
    if peer_uid != Some(own_uid) {
        return Err(denied());
    }
    peer_pid.filter(|pid| *pid > 0).ok_or_else(denied)
}

#[cfg(test)]
#[path = "linux_appimage_tests.rs"]
mod tests;
