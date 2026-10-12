// SPDX-License-Identifier: MPL-2.0
//! A local X11 socket with one deadline for connect, setup and all replies.
//! x11rb's DefaultStream is nonblocking, but its default poll has no timeout.
//! Keep its FD transport and replace the poll; no detached timeout worker.

use rustix::{
    event::{poll, PollFd, PollFlags, Timespec},
    fd::AsFd,
    io::Errno,
    net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType},
};
use std::{
    io::{self, IoSlice},
    os::unix::net::UnixStream,
    sync::Mutex,
    time::{Duration, Instant},
};
use x11rb::{
    reexports::x11rb_protocol::{parse_display::ParsedDisplay, xauth},
    rust_connection::{DefaultStream, PollMode, RustConnection, Stream},
    utils::RawFdContainer,
};

const SOCKET_BUDGET: Duration = Duration::from_secs(10);
pub(super) type Connection = RustConnection<DeadlineStream>;

fn timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "X11 socket deadline exceeded")
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(timeout)
}

fn wait<F: AsFd>(socket: &F, flags: PollFlags, deadline: Instant) -> io::Result<()> {
    loop {
        let budget = remaining(deadline)?;
        let ts = Timespec {
            tv_sec: budget.as_secs() as _,
            tv_nsec: budget.subsec_nanos() as _,
        };
        let mut fds = [PollFd::new(socket, flags)];
        match poll(&mut fds, Some(&ts)) {
            Ok(0) => return Err(timeout()),
            Ok(_) => {
                remaining(deadline)?;
                if fds[0].revents().contains(PollFlags::NVAL) {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "X11 socket closed",
                    ));
                }
                return Ok(());
            }
            Err(Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn connect_socket(address: &SocketAddrUnix, deadline: Instant) -> io::Result<UnixStream> {
    remaining(deadline)?;
    let socket = net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )?;
    match net::connect(&socket, address) {
        Ok(()) => {}
        Err(Errno::INPROGRESS) => {
            wait(&socket, PollFlags::OUT, deadline)?;
            net::sockopt::socket_error(&socket)??;
        }
        // A full Unix listen queue returns EAGAIN without a pending connect.
        // Drop the socket and fail rather than entering a blocking retry.
        Err(error) => return Err(error.into()),
    }
    remaining(deadline)?;
    Ok(socket.into())
}

fn local_socket(display: &ParsedDisplay) -> io::Result<String> {
    if display.protocol.as_deref().is_some_and(|p| p != "unix") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "local X11 required",
        ));
    }
    if display.protocol.as_deref() == Some("unix") && display.host.starts_with('/') {
        return Ok(display.host.clone());
    }
    if !display.host.is_empty() && display.host != "unix" {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "local X11 required",
        ));
    }
    // Derive from the real parsed DISPLAY; do not guess :0 or try TCP/DNS.
    Ok(format!("/tmp/.X11-unix/X{}", display.display))
}

pub(super) fn connect() -> Result<(Connection, usize), String> {
    connect_display(None)
}

pub(super) fn connect_display(display_name: Option<&str>) -> Result<(Connection, usize), String> {
    let deadline = Instant::now() + SOCKET_BUDGET;
    connect_display_until(display_name, deadline)
}

// Recording retains this same subscribed connection across frames and pauses.
// Per-request deadlines may be renewed only within its original hard deadline;
// ordinary snapshots still have their original immutable ten-second ceiling.
pub(super) fn connect_display_until(
    display_name: Option<&str>,
    expires: Instant,
) -> Result<(Connection, usize), String> {
    let deadline = (Instant::now() + SOCKET_BUDGET).min(expires);
    remaining(deadline).map_err(|_| "X11 图形连接预算已结束")?;
    let display = x11rb::reexports::x11rb_protocol::parse_display::parse_display(display_name)
        .map_err(|_| "X11 图形会话地址无效")?;
    let path = local_socket(&display).map_err(|_| "截图需要本机 X11 桌面会话")?;
    let socket = SocketAddrUnix::new_abstract_name(path.as_bytes())
        .map_err(io::Error::from)
        .and_then(|address| connect_socket(&address, deadline))
        .or_else(|_| {
            let address = SocketAddrUnix::new(path.as_str())?;
            connect_socket(&address, deadline)
        })
        .map_err(|_| "无法连接当前本机 X11 图形会话")?;
    let poll_socket = socket.try_clone().map_err(|_| "无法准备 X11 图形连接")?;
    let (inner, (family, address)) =
        DefaultStream::from_unix_stream(socket).map_err(|_| "无法准备 X11 图形连接")?;
    // Follow x11rb's normal Xauthority lookup; never log authorization bytes.
    let (auth_name, auth_data) = xauth::get_auth(family, &address, display.display)
        .unwrap_or(None)
        .unwrap_or_default();
    let stream = DeadlineStream {
        inner,
        poll_socket,
        deadline: Mutex::new(deadline),
        expires,
    };
    let connection = Connection::connect_to_stream_with_auth_info(
        stream,
        usize::from(display.screen),
        auth_name,
        auth_data,
    )
    .map_err(|_| "X11 图形连接失败或等待超时")?;
    Ok((connection, usize::from(display.screen)))
}

pub(super) struct DeadlineStream {
    inner: DefaultStream,
    // This duplicate owns the same nonblocking socket, allowing safe borrowed
    // descriptor polling. Dropping the stream closes both descriptors.
    poll_socket: UnixStream,
    deadline: Mutex<Instant>,
    expires: Instant,
}

impl DeadlineStream {
    fn deadline(&self) -> io::Result<Instant> {
        self.deadline
            .lock()
            .map(|deadline| *deadline)
            .map_err(|_| io::Error::other("X11 deadline lock unavailable"))
    }
    pub(super) fn begin_request(&self, budget: Duration) -> io::Result<()> {
        let deadline = (Instant::now() + budget).min(self.expires);
        remaining(deadline)?;
        *self
            .deadline
            .lock()
            .map_err(|_| io::Error::other("X11 deadline lock unavailable"))? = deadline;
        Ok(())
    }
}

impl Stream for DeadlineStream {
    fn poll(&self, mode: PollMode) -> io::Result<()> {
        let mut flags = PollFlags::empty();
        if mode.readable() {
            flags |= PollFlags::IN;
        }
        if mode.writable() {
            flags |= PollFlags::OUT;
        }
        wait(&self.poll_socket, flags, self.deadline()?)
    }

    fn read(&self, buf: &mut [u8], fds: &mut Vec<RawFdContainer>) -> io::Result<usize> {
        remaining(self.deadline()?)?;
        self.inner.read(buf, fds)
    }

    fn write(&self, buf: &[u8], fds: &mut Vec<RawFdContainer>) -> io::Result<usize> {
        remaining(self.deadline()?)?;
        self.inner.write(buf, fds)
    }

    fn write_vectored(
        &self,
        bufs: &[IoSlice<'_>],
        fds: &mut Vec<RawFdContainer>,
    ) -> io::Result<usize> {
        remaining(self.deadline()?)?;
        self.inner.write_vectored(bufs, fds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn stream(socket: UnixStream, budget: Duration) -> DeadlineStream {
        let poll_socket = socket.try_clone().unwrap();
        let (inner, _) = DefaultStream::from_unix_stream(socket).unwrap();
        let expires = Instant::now() + budget;
        DeadlineStream {
            inner,
            poll_socket,
            deadline: Mutex::new(expires),
            expires,
        }
    }

    #[test]
    fn request_renewal_never_extends_the_original_hard_deadline() {
        let (client, _server) = UnixStream::pair().unwrap();
        let stream = stream(client, Duration::from_secs(1));
        let expires = stream.expires;
        stream.begin_request(Duration::from_secs(60)).unwrap();
        assert_eq!(stream.deadline().unwrap(), expires);
        stream.begin_request(Duration::from_millis(10)).unwrap();
        assert!(stream.deadline().unwrap() < expires);
        stream.begin_request(Duration::from_secs(60)).unwrap();
        assert_eq!(stream.deadline().unwrap(), expires);
        let (client, _server) = UnixStream::pair().unwrap();
        let expired = stream_for_expired_deadline(client);
        assert_eq!(
            expired
                .begin_request(Duration::from_secs(60))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }

    fn stream_for_expired_deadline(socket: UnixStream) -> DeadlineStream {
        let mut stream = stream(socket, Duration::from_secs(1));
        stream.expires = Instant::now() - Duration::from_secs(1);
        stream
    }

    #[test]
    fn silent_setup_times_out_and_closes_actual_socket() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let start = Instant::now();
        let result = Connection::connect_to_stream(stream(client, Duration::from_millis(80)), 0);
        assert!(
            matches!(result, Err(x11rb::errors::ConnectError::IoError(ref e))
            if e.kind() == io::ErrorKind::TimedOut)
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        server
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut setup = Vec::new();
        server.read_to_end(&mut setup).unwrap(); // EOF proves both client FDs closed.
        assert!(setup.len() >= 12);
    }

    #[test]
    fn write_backpressure_uses_the_same_deadline() {
        let (client, _server) = UnixStream::pair().unwrap();
        let stream = stream(client, Duration::from_millis(80));
        let bytes = [0_u8; 65536];
        loop {
            match stream.write(&bytes, &mut Vec::new()) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("unexpected fill error: {error}"),
            }
        }
        assert_eq!(
            stream.poll(PollMode::Writable).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            stream
                .read(&mut [0_u8; 1], &mut Vec::new())
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn nonblocking_abstract_connect_and_ready_io() {
        let name = format!("mewu-x11-test-{}", uuid::Uuid::new_v4());
        let address = SocketAddrUnix::new_abstract_name(name.as_bytes()).unwrap();
        let listener = net::socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        net::bind(&listener, &address).unwrap();
        net::listen(&listener, 1).unwrap();
        let socket = connect_socket(&address, Instant::now() + Duration::from_secs(1)).unwrap();
        let mut server: UnixStream = net::accept_with(&listener, SocketFlags::CLOEXEC)
            .unwrap()
            .into();
        let stream = stream(socket, Duration::from_secs(1));
        server.write_all(b"x").unwrap();
        stream.poll(PollMode::Readable).unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(stream.read(&mut byte, &mut Vec::new()).unwrap(), 1);
        assert_eq!(&byte, b"x");
    }

    #[test]
    fn local_display_is_derived_without_tcp_or_dns() {
        use x11rb::reexports::x11rb_protocol::parse_display::parse_display_with_file_exists_callback;
        let parse = |text| parse_display_with_file_exists_callback(text, |_| false).unwrap();
        assert_eq!(local_socket(&parse(":7.2")).unwrap(), "/tmp/.X11-unix/X7");
        assert_eq!(
            local_socket(&parse("unix/:65535")).unwrap(),
            "/tmp/.X11-unix/X65535"
        );
        assert!(local_socket(&parse("localhost:10")).is_err());
        assert!(local_socket(&parse("tcp/:0")).is_err());
    }
}
