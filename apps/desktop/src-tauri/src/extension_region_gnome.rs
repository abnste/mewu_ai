// SPDX-License-Identifier: MPL-2.0
//! Private GNOME compositor witness client. This carries no input authority;
//! Portal owns that separately. Calls are pinned to one bus owner and a live
//! root-owned GNOME Shell executable, never to a replaceable well-known name.
use crate::extension_region_desktop::Snapshot;
use std::{
    fs::File,
    os::{fd::OwnedFd, unix::fs::MetadataExt},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use zbus::{Connection, Proxy};

const SHELL: &str = "org.gnome.Shell";
const PATH: &str = "/org/gnome/Shell/Extensions/MewuWitness";
const INTERFACE: &str = "dev.mewu.DesktopWitness";
fn unavailable() -> String {
    "GNOME 桌面连接不可用，请确认 Mewu Desktop Witness 已启用".into()
}
async fn bounded<T>(
    future: impl std::future::Future<Output = Result<T, zbus::Error>>,
    ms: u64,
) -> Result<T, String> {
    futures_lite::future::race(async { future.await.map_err(|_| unavailable()) }, async {
        async_io::Timer::after(Duration::from_millis(ms)).await;
        Err(unavailable())
    })
    .await
}
struct Process {
    pid: u32,
    pidfd: OwnedFd,
    executable: File,
    expected_path: PathBuf,
}
impl Process {
    fn pin(pid: u32) -> Result<Self, String> {
        let id = rustix::process::Pid::from_raw(i32::try_from(pid).map_err(|_| unavailable())?)
            .ok_or_else(unavailable)?;
        let pidfd = rustix::process::pidfd_open(id, rustix::process::PidfdFlags::empty())
            .map_err(|_| unavailable())?;
        let expected_path =
            std::fs::canonicalize("/usr/bin/gnome-shell").map_err(|_| unavailable())?;
        let executable = File::open(&expected_path).map_err(|_| unavailable())?;
        let identity = Self {
            pid,
            pidfd,
            executable,
            expected_path,
        };
        identity.check()?;
        Ok(identity)
    }
    fn check(&self) -> Result<(), String> {
        let mut poll = [rustix::event::PollFd::new(
            &self.pidfd,
            rustix::event::PollFlags::IN,
        )];
        let ready = rustix::event::poll(
            &mut poll,
            Some(&rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        )
        .map_err(|_| unavailable())?;
        let path = format!("/proc/{}/exe", self.pid);
        let link = std::fs::read_link(&path).map_err(|_| unavailable())?;
        let actual = std::fs::metadata(&path).map_err(|_| unavailable())?;
        let expected = self.executable.metadata().map_err(|_| unavailable())?;
        if ready != 0
            || link != self.expected_path
            || expected.uid() != 0
            || expected.mode() & 0o022 != 0
            || actual.dev() != expected.dev()
            || actual.ino() != expected.ino()
        {
            return Err(unavailable());
        }
        Ok(())
    }
}

pub(crate) struct Witness {
    connection: Connection,
    owner: String,
    process: Process,
    token: String,
    epoch: String,
    closed: AtomicBool,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Armed {
    protocol: u32,
    epoch: String,
}

async fn owner(connection: &Connection) -> Result<String, String> {
    let bus = bounded(
        Proxy::new(
            connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        ),
        400,
    )
    .await?;
    bounded(bus.call("GetNameOwner", &(SHELL,)), 400).await
}
impl Witness {
    pub(crate) fn open() -> Result<Self, String> {
        futures_lite::future::block_on(async {
            let address = zbus::Address::session().map_err(|_| unavailable())?;
            if !matches!(
                address.transport(),
                zbus::address::transport::Transport::Unix(_)
            ) {
                return Err(unavailable());
            }
            let builder = zbus::connection::Builder::address(address)
                .map_err(|_| unavailable())?
                .method_timeout(Duration::from_millis(1200))
                .max_queued(16);
            let connection = bounded(builder.build(), 2000).await?;
            let name = owner(&connection).await?;
            if zbus::names::UniqueName::try_from(name.as_str()).is_err() {
                return Err(unavailable());
            }
            let bus = bounded(
                Proxy::new(
                    &connection,
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                ),
                400,
            )
            .await?;
            let pid: u32 = bounded(
                bus.call("GetConnectionUnixProcessID", &(name.as_str(),)),
                400,
            )
            .await?;
            let uid: u32 =
                bounded(bus.call("GetConnectionUnixUser", &(name.as_str(),)), 400).await?;
            if uid != unsafe { libc::geteuid() } {
                return Err(unavailable());
            }
            let process = Process::pin(pid)?;
            drop(bus);
            // Construct the cleanup owner before sending Arm. An incomplete
            // reply must close this exact lease, never resend the command.
            let mut result = Self {
                connection,
                owner: name,
                process,
                token: uuid::Uuid::new_v4().to_string(),
                epoch: String::new(),
                closed: AtomicBool::new(false),
            };
            let proxy = bounded(
                Proxy::new(&result.connection, result.owner.as_str(), PATH, INTERFACE),
                400,
            )
            .await?;
            let text: String = bounded(proxy.call("Arm", &(result.token.as_str(),)), 1500).await?;
            if text.len() > 4096 {
                return Err(unavailable());
            }
            let armed: Armed = serde_json::from_str(&text).map_err(|_| unavailable())?;
            if armed.protocol != 1 || uuid::Uuid::parse_str(&armed.epoch).is_err() {
                return Err(unavailable());
            }
            result.epoch = armed.epoch;
            drop(proxy);
            result.check().await?;
            Ok(result)
        })
    }
    async fn check(&self) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(unavailable());
        }
        self.process.check()?;
        if owner(&self.connection).await? != self.owner {
            return Err(unavailable());
        }
        Ok(())
    }
    pub(crate) fn snapshot(&self, points: &[[f64; 2]]) -> Result<Snapshot, String> {
        if points.len() > 16
            || points
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
        {
            return Err(unavailable());
        }
        futures_lite::future::block_on(async {
            self.check().await?;
            let proxy = bounded(
                Proxy::new(&self.connection, self.owner.as_str(), PATH, INTERFACE),
                400,
            )
            .await?;
            let points = serde_json::to_string(points).map_err(|_| unavailable())?;
            let text: String =
                bounded(proxy.call("Snapshot", &(self.token.as_str(), points)), 1200).await?;
            if text.len() > 128 * 1024 {
                return Err(unavailable());
            }
            let snapshot: Snapshot = serde_json::from_str(&text).map_err(|_| unavailable())?;
            if snapshot.epoch != self.epoch || snapshot.protocol != 1 {
                return Err(unavailable());
            }
            self.check().await?;
            Ok(snapshot)
        })
    }
    pub(crate) fn keep_alive(&self) -> Result<bool, String> {
        futures_lite::future::block_on(async {
            self.check().await?;
            let proxy = bounded(
                Proxy::new(&self.connection, self.owner.as_str(), PATH, INTERFACE),
                400,
            )
            .await?;
            let text: String =
                bounded(proxy.call("KeepAlive", &(self.token.as_str(),)), 400).await?;
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Alive {
                stopped: bool,
            }
            if text.len() > 128 {
                return Err(unavailable());
            }
            let alive: Alive = serde_json::from_str(&text).map_err(|_| unavailable())?;
            Ok(!alive.stopped)
        })
    }
    pub(crate) fn request_frame(&self) -> Result<(), String> {
        futures_lite::future::block_on(async {
            self.check().await?;
            let proxy = bounded(
                Proxy::new(&self.connection, self.owner.as_str(), PATH, INTERFACE),
                400,
            ).await?;
            let text: String = bounded(proxy.call("RequestFrame", &(self.token.as_str(),)), 400).await?;
            if text.len() > 4096 { return Err(unavailable()); }
            let response: Armed = serde_json::from_str(&text).map_err(|_| unavailable())?;
            if response.protocol != 1 || response.epoch != self.epoch { return Err(unavailable()); }
            self.check().await
        })
    }
    pub(crate) fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        futures_lite::future::block_on(async {
            let close = async {
                if let Ok(proxy) =
                    Proxy::new(&self.connection, self.owner.as_str(), PATH, INTERFACE).await
                {
                    let _: Result<(), _> = proxy.call("Disarm", &(self.token.as_str(),)).await;
                }
            };
            futures_lite::future::race(close, async {
                async_io::Timer::after(Duration::from_millis(200)).await;
            })
            .await;
            // Closing the client connection also revokes the Shell stop lease
            // when Arm or Disarm had an unknown transport outcome.
            let _ = bounded(self.connection.clone().close(), 200).await;
        });
    }
}
impl Drop for Witness {
    fn drop(&mut self) {
        self.close()
    }
}
