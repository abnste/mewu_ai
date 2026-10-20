// SPDX-License-Identifier: MPL-2.0
use interprocess::local_socket::{
    Listener, ListenerNonblockingMode, ListenerOptions, Name, Stream,
};
use std::io;

#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

/// One well-known endpoint per OS user/logon. The client does not create it.
#[derive(Clone, Debug)]
pub struct Endpoint {
    inner: platform::Endpoint,
}
impl Endpoint {
    pub fn for_current_user() -> io::Result<Self> {
        Self::named("v1")
    }
    fn named(namespace: &str) -> io::Result<Self> {
        Ok(Self {
            inner: platform::Endpoint::new(namespace)?,
        })
    }
    pub(crate) fn name(&self) -> io::Result<Name<'_>> {
        self.inner.name()
    }
    pub(crate) fn listen(&self) -> io::Result<(Listener, platform::Guard)> {
        let guard = self.inner.prepare()?;
        let options = ListenerOptions::new()
            .name(self.name()?)
            .nonblocking(ListenerNonblockingMode::Both)
            .try_overwrite(false);
        let listener = self.inner.options(options)?.create_sync()?;
        self.inner.after_bind()?;
        Ok((listener, guard))
    }
    pub(crate) fn verify_peer(&self, stream: &Stream, server: bool) -> io::Result<()> {
        self.inner.verify_peer(stream, server)
    }
    pub(crate) fn verify_server_image(
        &self,
        stream: &Stream,
        expected: &std::path::Path,
    ) -> io::Result<()> {
        if !expected.is_absolute() {
            return Err(denied());
        }
        self.inner.verify_server_image(stream, expected)
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn verify_appimage_server(
        &self,
        stream: &Stream,
        deadline: std::time::Instant,
    ) -> io::Result<()> {
        self.verify_peer(stream, false)?;
        crate::linux_appimage::verify_server(stream, deadline)
    }
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn for_test_namespace(namespace: &str) -> io::Result<Self> {
        if namespace.is_empty() || namespace.len() > 48 || !namespace.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(denied());
        }
        Self::named(namespace)
    }
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self::named(&format!(
            "test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
        .unwrap()
    }
}

pub(crate) fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Local control identity or endpoint permissions rejected",
    )
}

/// Interprocess deliberately drains Windows pipes on Drop in a background pool.
/// Our framed protocol treats a broken/partial response as Unknown, so explicitly
/// opt out of that unbounded peer-dependent drain on every stream close.
pub(crate) fn before_close(stream: &Stream) {
    #[cfg(windows)]
    {
        let Stream::NamedPipe(stream) = stream;
        stream.inner().assume_flushed();
    }
    #[cfg(unix)]
    let _ = stream;
}
