// SPDX-License-Identifier: MPL-2.0
//! User-initiated Screenshot portal capture. A returned bitmap has no desktop
//! coordinate claim. Call on the real capture worker, retaining Work/transition.
use crate::capture::{CaptureCheck, CaptureOptions};
use futures_lite::StreamExt;
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ImageEncoder,
};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    future::Future,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawFd, FromRawFd},
    },
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SCREENSHOT: &str = "org.freedesktop.portal.Screenshot";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const MAX_ENCODED: u64 = 128 * 1024 * 1024;
const MAX_SIDE: u32 = 16_384;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(750);
const UNAVAILABLE: &str = "无法访问桌面截图授权服务";
const INVALID: &str = "桌面截图结果无效";

/// Only the generated file belongs to Mewu. The portal's input file and its
/// document grant are never deleted. Store adoption is the sole retain point.
pub(crate) struct PendingPortalCapture {
    pub(crate) asset_id: String,
    pub(crate) path: PathBuf,
    pub(crate) width: u32,
    pub(crate) height: u32,
    retained: bool,
}
impl PendingPortalCapture {
    pub(crate) fn retain(&mut self) {
        self.retained = true;
    }
}
impl Drop for PendingPortalCapture {
    fn drop(&mut self) {
        if !self.retained {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Cancellation polling belongs to the worker: dropping an invoke cancels Work,
/// rather than aborting this future and abandoning the desktop's request UI.
pub(crate) fn capture(
    asset_dir: &Path,
    options: CaptureOptions,
    allowed: CaptureCheck,
) -> Result<PendingPortalCapture, String> {
    if options.include_cursor {
        return Err("当前平台尚不支持截图包含光标".into());
    }
    if !asset_dir.is_absolute() {
        return Err("截图素材目录必须是绝对路径".into());
    }
    allowed()?;
    let uri = async_io::block_on(request_screenshot(&allowed))?;
    allowed()?;
    import_portal_image(&uri, asset_dir, &allowed)
}

/// Keep the same future pinned between ticks: never resend a method while
/// checking cancellation. Every setup call has a separate small transport
/// budget; permission interaction has an explicit longer overall budget.
async fn checked<T>(
    future: impl Future<Output = Result<T, String>>,
    deadline: Instant,
    allowed: &CaptureCheck,
) -> Result<T, String> {
    futures_lite::pin!(future);
    loop {
        allowed()?;
        let now = Instant::now();
        if now >= deadline {
            return Err("桌面截图授权超时".into());
        }
        let tick = (now + Duration::from_millis(25)).min(deadline);
        let result = futures_lite::future::race(async { Some(future.as_mut().await) }, async {
            async_io::Timer::at(tick).await;
            None
        })
        .await;
        if let Some(result) = result {
            allowed()?;
            return result;
        }
    }
}

async fn bounded<T>(future: impl Future<Output = T>, timeout: Duration) -> Option<T> {
    futures_lite::future::race(async { Some(future.await) }, async {
        async_io::Timer::after(timeout).await;
        None
    })
    .await
}

/// The connection is private to this capture. Pin all calls and signals to the
/// activated portal's unique owner; a replacement service cannot complete it.
async fn request_screenshot(allowed: &CaptureCheck) -> Result<String, String> {
    let address = zbus::Address::session().map_err(|_| UNAVAILABLE)?;
    request_at(address, allowed).await
}
async fn request_at(address: zbus::Address, allowed: &CaptureCheck) -> Result<String, String> {
    if !matches!(
        address.transport(),
        zbus::address::transport::Transport::Unix(_)
    ) {
        return Err(UNAVAILABLE.into());
    }
    let connection = checked(
        async {
            zbus::connection::Builder::address(address)
                .map_err(|_| UNAVAILABLE)?
                .max_queued(16)
                .method_timeout(TRANSPORT_TIMEOUT)
                .build()
                .await
                .map_err(|_| UNAVAILABLE.into())
        },
        Instant::now() + TRANSPORT_TIMEOUT,
        allowed,
    )
    .await?;
    let result = screenshot_on(&connection, allowed).await;
    // Name disappearance also closes a still-pending portal request. Never keep
    // an owned bus socket or screenshot reader running beyond this Work.
    let _ = bounded(connection.close(), Duration::from_millis(150)).await;
    result
}

async fn screenshot_on(connection: &Connection, allowed: &CaptureCheck) -> Result<String, String> {
    let bus = checked(
        async {
            Proxy::new(connection, BUS, BUS_PATH, BUS)
                .await
                .map_err(|_| UNAVAILABLE.into())
        },
        Instant::now() + TRANSPORT_TIMEOUT,
        allowed,
    )
    .await?;
    let mut owner_changes = checked(
        async {
            bus.receive_signal_with_args("NameOwnerChanged", &[(0, SERVICE)])
                .await
                .map_err(|_| UNAVAILABLE.into())
        },
        Instant::now() + TRANSPORT_TIMEOUT,
        allowed,
    )
    .await?;
    // Activation is allowed only for this explicit user capture, never startup.
    let _: u32 = checked(
        async {
            bus.call("StartServiceByName", &(SERVICE, 0u32))
                .await
                .map_err(|_| UNAVAILABLE.into())
        },
        Instant::now() + TRANSPORT_TIMEOUT,
        allowed,
    )
    .await?;
    let owner: String = checked(
        async {
            bus.call("GetNameOwner", &(SERVICE,))
                .await
                .map_err(|_| UNAVAILABLE.into())
        },
        Instant::now() + TRANSPORT_TIMEOUT,
        allowed,
    )
    .await?;
    if !owner.starts_with(':') {
        return Err(UNAVAILABLE.into());
    }
    let proxy = checked(
        async {
            Proxy::new(connection, owner.as_str(), DESKTOP, SCREENSHOT)
                .await
                .map_err(|_| UNAVAILABLE.into())
        },
        Instant::now() + TRANSPORT_TIMEOUT,
        allowed,
    )
    .await?;
    let version: u32 = checked(
        async {
            proxy
                .get_property("version")
                .await
                .map_err(|_| UNAVAILABLE.into())
        },
        Instant::now() + TRANSPORT_TIMEOUT,
        allowed,
    )
    .await?;
    if version < 1 {
        return Err(UNAVAILABLE.into());
    }
    let sender = connection
        .unique_name()
        .ok_or(UNAVAILABLE)?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = format!("mewu_{}", Uuid::new_v4().simple());
    let expected = OwnedObjectPath::try_from(format!("{DESKTOP}/request/{sender}/{token}"))
        .map_err(|_| INVALID)?;
    let mut request = PortalRequest {
        connection: connection.clone(),
        owner: owner.clone(),
        path: expected.clone(),
        armed: true,
    };
    let outcome = async {
        let request_proxy = checked(
            async {
                Proxy::new(connection, owner.as_str(), expected.as_str(), REQUEST)
                    .await
                    .map_err(|_| UNAVAILABLE.into())
            },
            Instant::now() + TRANSPORT_TIMEOUT,
            allowed,
        )
        .await?;
        // Subscribe BEFORE Screenshot, as required to avoid fast-response loss.
        let mut response = checked(
            async {
                request_proxy
                    .receive_signal("Response")
                    .await
                    .map_err(|_| UNAVAILABLE.into())
            },
            Instant::now() + TRANSPORT_TIMEOUT,
            allowed,
        )
        .await?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("modal", Value::from(false));
        if version >= 2 {
            options.insert("interactive", Value::from(false));
        }
        let returned: OwnedObjectPath = checked(
            async {
                proxy
                    .call("Screenshot", &("", options))
                    .await
                    .map_err(|_| UNAVAILABLE.into())
            },
            Instant::now() + TRANSPORT_TIMEOUT,
            allowed,
        )
        .await?;
        if returned != expected {
            // Do not accept arbitrary Request paths supplied by a service. A
            // modern portal must honor our token; close any legitimate alternate
            // caller-owned handle then reject this unobserved response channel.
            if returned
                .as_str()
                .starts_with(&format!("{DESKTOP}/request/{sender}/"))
            {
                close_request(connection, &owner, returned.as_str()).await;
            }
            return Err(INVALID.into());
        }
        let reply = checked(
            async {
                loop {
                    let event = futures_lite::future::race(
                        async { response.next().await.map(|message| (true, message)) },
                        async { owner_changes.next().await.map(|message| (false, message)) },
                    )
                    .await
                    .ok_or(UNAVAILABLE)?;
                    if event.0 {
                        return Ok(event.1);
                    }
                    let (name, old, new): (String, String, String) =
                        event.1.body().deserialize().map_err(|_| INVALID)?;
                    if name == SERVICE && old == owner && new != owner {
                        return Err(UNAVAILABLE.into());
                    }
                }
            },
            Instant::now() + REQUEST_TIMEOUT,
            allowed,
        )
        .await?;
        if reply.data().bytes().len() > 16 * 1024 {
            return Err(INVALID.into());
        }
        let (status, results): (u32, HashMap<String, OwnedValue>) =
            reply.body().deserialize().map_err(|_| INVALID)?;
        match status {
            0 => {
                let uri = results.get("uri").ok_or(INVALID)?;
                let uri: &str = uri.try_into().map_err(|_| INVALID)?;
                if uri.len() > 16 * 1024 {
                    return Err(INVALID.into());
                }
                Ok(uri.to_owned())
            }
            1 => Err(crate::capture_delay_work::CANCELED.into()),
            _ => Err("桌面截图未完成".into()),
        }
    }
    .await;
    // Close even after rejection, bad reply, cancellation or timeout. Completed
    // requests may already be gone; a failed close never revives the operation.
    close_request(connection, &owner, request.path.as_str()).await;
    request.armed = false;
    outcome
}

struct PortalRequest {
    connection: Connection,
    owner: String,
    path: OwnedObjectPath,
    armed: bool,
}
impl Drop for PortalRequest {
    fn drop(&mut self) {
        if self.armed {
            // Panic/unexpected future drop still closes on the owning worker;
            // no detached cleanup task outlives Work or its shutdown drain.
            async_io::block_on(close_request(
                &self.connection,
                &self.owner,
                self.path.as_str(),
            ));
        }
    }
}
async fn close_request(connection: &Connection, owner: &str, path: &str) {
    let _ = bounded(
        connection.call_method(Some(owner), path, Some(REQUEST), "Close", &()),
        CLOSE_TIMEOUT,
    )
    .await;
}

fn local_file_path(uri: &str) -> Result<PathBuf, String> {
    let parsed = url::Url::parse(uri).map_err(|_| INVALID)?;
    if parsed.scheme() != "file"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.host_str().is_some_and(|host| host != "localhost")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return Err(INVALID.into());
    }
    let path = parsed.to_file_path().map_err(|_| INVALID)?;
    if !path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(INVALID.into());
    }
    Ok(path)
}

/// Walk using openat and directory FDs: no symlink in any component and no
/// check-then-open race. Document portal/FUSE directories remain legitimate.
fn open_portal_file(path: &Path) -> Result<File, String> {
    use std::ffi::CString;
    let root = CString::new("/").unwrap();
    let fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(INVALID.into());
    }
    let mut directory = unsafe { File::from_raw_fd(fd) };
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect::<Vec<_>>();
    if components.is_empty() {
        return Err(INVALID.into());
    }
    for (index, component) in components.iter().enumerate() {
        let name = CString::new(component.as_bytes()).map_err(|_| INVALID)?;
        let final_component = index + 1 == components.len();
        let flags = if final_component {
            libc::O_RDONLY | libc::O_NONBLOCK
        } else {
            libc::O_PATH | libc::O_DIRECTORY
        };
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(INVALID.into());
        }
        let opened = unsafe { File::from_raw_fd(fd) };
        if final_component {
            let metadata = opened.metadata().map_err(|_| INVALID)?;
            if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ENCODED {
                return Err(INVALID.into());
            }
            return Ok(opened);
        }
        directory = opened;
    }
    Err(INVALID.into())
}

fn valid_dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}
fn fingerprint(file: &File) -> Result<(u64, u64, u64, i64, i64, i64, i64), String> {
    let m = file.metadata().map_err(|_| INVALID)?;
    if !m.is_file() || m.len() == 0 || m.len() > MAX_ENCODED {
        return Err(INVALID.into());
    }
    Ok((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}

fn import_portal_image(
    uri: &str,
    asset_dir: &Path,
    allowed: &CaptureCheck,
) -> Result<PendingPortalCapture, String> {
    allowed()?;
    let source = local_file_path(uri)?;
    let mut file = open_portal_file(&source)?;
    let identity = fingerprint(&file)?;
    let dimensions = image::ImageReader::new(BufReader::new(CheckedReader {
        file: &mut file,
        allowed,
    }))
    .with_guessed_format()
    .map_err(|_| INVALID)?
    .into_dimensions()
    .map_err(|_| INVALID)?;
    if !valid_dimensions(dimensions.0, dimensions.1) {
        return Err("截图像素超过限制".into());
    }
    allowed()?;
    file.seek(SeekFrom::Start(0)).map_err(|_| INVALID)?;
    let mut reader = image::ImageReader::new(BufReader::new(CheckedReader {
        file: &mut file,
        allowed,
    }))
    .with_guessed_format()
    .map_err(|_| INVALID)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let pixels = reader.decode().map_err(|_| INVALID)?.into_rgba8();
    if pixels.dimensions() != dimensions || fingerprint(&file)? != identity {
        return Err(INVALID.into());
    }
    allowed()?;
    let asset_id = Uuid::new_v4().to_string();
    let path = asset_dir.join(format!("{asset_id}.png"));
    let destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| "无法保存截图")?;
    let pending = PendingPortalCapture {
        asset_id,
        path,
        width: dimensions.0,
        height: dimensions.1,
        retained: false,
    };
    let mut writer = LimitedWriter {
        inner: BufWriter::new(destination),
        written: 0,
        allowed,
    };
    PngEncoder::new_with_quality(&mut writer, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            pixels.as_raw(),
            dimensions.0,
            dimensions.1,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "无法保存截图")?;
    writer.flush().map_err(|_| "无法保存截图")?;
    allowed()?;
    Ok(pending)
}

struct LimitedWriter<'a, W> {
    inner: W,
    written: u64,
    allowed: &'a CaptureCheck,
}
impl<W: Write> Write for LimitedWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        (self.allowed)().map_err(std::io::Error::other)?;
        let length = self
            .written
            .checked_add(buffer.len() as u64)
            .filter(|length| *length <= MAX_ENCODED)
            .ok_or_else(|| std::io::Error::other("截图文件超过限制"))?;
        let written = self.inner.write(buffer)?;
        self.written = length - (buffer.len() - written) as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Bound reads on the leased FD too: an in-place growing file cannot bypass
/// its initial metadata limit. Cancellation is checked between decoder reads.
struct CheckedReader<'a> {
    file: &'a mut File,
    allowed: &'a CaptureCheck,
}
impl Read for CheckedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        (self.allowed)().map_err(std::io::Error::other)?;
        let position = self.file.stream_position()?;
        if position >= MAX_ENCODED {
            return Ok(0);
        }
        let count = buffer.len().min((MAX_ENCODED - position) as usize);
        self.file.read(&mut buffer[..count])
    }
}
impl Seek for CheckedReader<'_> {
    fn seek(&mut self, seek: SeekFrom) -> std::io::Result<u64> {
        (self.allowed)().map_err(std::io::Error::other)?;
        let position = self.file.seek(seek)?;
        if position > MAX_ENCODED {
            return Err(std::io::Error::other("截图文件超过限制"));
        }
        Ok(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portal_uris_are_local_files_only() {
        assert_eq!(
            local_file_path("file:///tmp/a%20b%E4%B8%AD.png").unwrap(),
            PathBuf::from("/tmp/a b中.png")
        );
        for uri in [
            "https://example.com/a.png",
            "file://other-host/tmp/a.png",
            "file:///tmp/a.png?anything",
            "file:///tmp/a.png#anything",
            "file:///tmp/a%00.png",
        ] {
            assert!(local_file_path(uri)
                .and_then(|path| open_portal_file(&path))
                .is_err());
        }
    }
    #[test]
    fn dimensions_enforce_actual_pixel_budget() {
        assert!(valid_dimensions(7680, 4320));
        assert!(!valid_dimensions(0, 1));
        assert!(!valid_dimensions(16385, 1));
        assert!(!valid_dimensions(8192, 8192));
    }
    #[test]
    fn portal_fd_rejects_symlinks_fifo_and_directory_without_blocking() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = std::env::temp_dir().join(format!("mewu-portal-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.join("regular"), b"known").unwrap();
        symlink(root.join("regular"), root.join("link")).unwrap();
        symlink(&root, root.join("parent-link")).unwrap();
        let fifo = std::ffi::CString::new(root.join("fifo").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(open_portal_file(&root.join("regular")).is_ok());
        for path in [
            root.join("link"),
            root.join("parent-link/regular"),
            root.join("fifo"),
            root.clone(),
        ] {
            assert!(open_portal_file(&path).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[derive(Clone)]
    struct TestRequest(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[zbus::interface(name = "org.freedesktop.portal.Request")]
    impl TestRequest {
        fn close(&self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    struct TestScreenshot {
        started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        closed: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    #[zbus::interface(name = "org.freedesktop.portal.Screenshot")]
    impl TestScreenshot {
        #[zbus(property)]
        fn version(&self) -> u32 {
            2
        }
        async fn screenshot(
            &self,
            parent_window: &str,
            options: HashMap<String, OwnedValue>,
            #[zbus(connection)] connection: &Connection,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            assert!(parent_window.is_empty());
            assert_eq!(
                bool::try_from(options.get("interactive").unwrap()).unwrap(),
                false
            );
            let token = <&str>::try_from(options.get("handle_token").unwrap()).unwrap();
            assert!(token.starts_with("mewu_"));
            let sender = header
                .sender()
                .unwrap()
                .as_str()
                .trim_start_matches(':')
                .replace('.', "_");
            let path =
                OwnedObjectPath::try_from(format!("{DESKTOP}/request/{sender}/{token}")).unwrap();
            connection
                .object_server()
                .at(path.clone(), TestRequest(self.closed.clone()))
                .await
                .unwrap();
            self.started
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(path)
        }
    }
    struct TestBus(std::process::Child);
    impl Drop for TestBus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    fn cancellation_closes_the_real_request_before_worker_releases_its_slot() {
        use std::{
            io::BufRead,
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
        };
        // A dedicated bus, not DBUS_SESSION_BUS_ADDRESS, prevents impersonating
        // or reading any real desktop service during protocol regression.
        let mut bus = TestBus(
            std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = zbus::Address::try_from(address.trim()).unwrap();
        let started = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let server = async_io::block_on(
            zbus::connection::Builder::address(address.clone())
                .unwrap()
                .name(SERVICE)
                .unwrap()
                .serve_at(
                    DESKTOP,
                    TestScreenshot {
                        started: started.clone(),
                        closed: closed.clone(),
                    },
                )
                .unwrap()
                .build(),
        )
        .unwrap();
        let jobs = Arc::new(crate::capture_delay_work::CaptureJobs::default());
        let work = Arc::new(jobs.begin().unwrap().unwrap());
        let checked_work = work.clone();
        let allowed: CaptureCheck = Arc::new(move || checked_work.check());
        let cancel_jobs = jobs.clone();
        let started_probe = started.clone();
        let cancel = std::thread::spawn(move || {
            let deadline = Instant::now() + TRANSPORT_TIMEOUT;
            while started_probe.load(Ordering::SeqCst) == 0 {
                assert!(
                    Instant::now() < deadline,
                    "test request never reached its actual bus service"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(100));
            cancel_jobs.cancel();
        });
        let error = async_io::block_on(request_at(address, &allowed)).unwrap_err();
        cancel.join().unwrap();
        assert_eq!(error, crate::capture_delay_work::CANCELED);
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        assert!(jobs.begin().unwrap().is_none());
        drop(allowed);
        drop(work);
        assert!(jobs.begin().unwrap().is_some());
        async_io::block_on(server.close()).unwrap();
    }
}
