// SPDX-License-Identifier: MPL-2.0
//! A GTK/WebKit window tree belongs to one display for its lifetime. Let the
//! target login create its own tree after the previous owner has flushed.
use crate::capture_shortcut_desktop_linux as desktop;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        linux::net::SocketAddrExt,
        unix::{
            fs::OpenOptionsExt,
            net::{SocketAddr, UnixListener, UnixStream},
        },
    },
    path::PathBuf,
    sync::{atomic::Ordering, OnceLock},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const ARG: &str = "--resume-desktop-session";
const FOLLOW: &str = "--follow-desktop-session";
const WATCH: &str = "--observe-desktop-session";
const LOGIN: &str = "org.freedesktop.login1";
const MANAGER: &str = "/org/freedesktop/login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const ENTRY: &str = "dev.mewu.remake-session.desktop";
static OWNER: OnceLock<(File, Record)> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Record {
    boot: Uuid,
    instance: Uuid,
    portal: bool,
}
fn boot() -> io::Result<Uuid> {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .parse()
        .map_err(io::Error::other)
}
fn state() -> io::Result<PathBuf> {
    let path = dirs::runtime_dir()
        .ok_or_else(|| io::Error::other("缺少用户运行时目录"))?
        .join("mewu-session");
    desktop::safe_path(&path).map_err(io::Error::other)?;
    fs::create_dir_all(&path)?;
    Ok(path)
}
fn active_path() -> io::Result<PathBuf> {
    // The runtime directory may be removed between logout and the next login.
    // Keep only the resume intent across logout; boot_id expires it on reboot.
    let dir = dirs::state_dir()
        .ok_or_else(|| io::Error::other("缺少用户状态目录"))?
        .join("dev.mewu.remake");
    desktop::safe_path(&dir).map_err(io::Error::other)?;
    fs::create_dir_all(&dir)?;
    Ok(dir.join("desktop-session.json"))
}
fn record() -> io::Result<Option<Record>> {
    let bytes = desktop::read(&active_path()?).map_err(io::Error::other)?;
    bytes
        .map(|v| serde_json::from_slice(&v).map_err(io::Error::other))
        .transpose()
}
fn lock(name: &str) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(state()?.join(name))
}
fn address(instance: Uuid) -> io::Result<SocketAddr> {
    SocketAddr::from_abstract_name(format!("mewu-session-{instance}"))
}
fn action() -> Option<String> {
    std::env::args().nth(1)
}
pub(crate) fn is_activation() -> bool {
    matches!(action().as_deref(), Some(ARG | FOLLOW | WATCH))
}

async fn session_path(manager: &zbus::Proxy<'_>) -> zbus::Result<zbus::zvariant::OwnedObjectPath> {
    let id = std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "auto".into());
    manager.call("GetSession", &(id,)).await
}

// logind drives reactivation of an already logged-in desktop. This observer
// never creates a GTK window, holds user storage, or polls the seat.
async fn observe() -> Result<(), Box<dyn std::error::Error>> {
    use futures_util::{FutureExt, StreamExt};
    let bus = zbus::Connection::system().await?;
    let manager = zbus::Proxy::new(&bus, LOGIN, MANAGER, "org.freedesktop.login1.Manager").await?;
    let path = session_path(&manager).await?;
    let session = zbus::Proxy::new(&bus, LOGIN, path.clone(), SESSION).await?;
    let kind: String = session.get_property("Type").await?;
    let expected = if crate::desktop_session_linux::uses_portal() {
        "wayland"
    } else {
        "x11"
    };
    if kind != expected {
        return Err("缺少对应的图形登录会话".into());
    }
    let id: String = session.get_property("Id").await?;
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("无法识别登录会话".into());
    }
    let observer = lock(&format!("seat-{id}.lock"))?;
    match observer.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    let launcher = desktop::current_launcher()?;
    let mut removed = manager.receive_signal("SessionRemoved").await?;
    let mut changes = session.receive_property_changed::<bool>("Active").await;
    let mut previous = false;
    let mut current: bool = session.get_property("Active").await?;
    loop {
        if current && !previous {
            let mut child = std::process::Command::new(&launcher)
                .arg(ARG)
                .env("XDG_SESSION_ID", &id)
                .spawn()?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        previous = current;
        futures_util::select! {
            change = changes.next().fuse() => match change {
                Some(v) => current = v.get().await?,
                None => return Ok(()),
            },
            signal = removed.next().fuse() => match signal {
                Some(m) => {
                    let (removed_id, _): (String, zbus::zvariant::OwnedObjectPath) = m.body().deserialize()?;
                    if removed_id == id { return Ok(()); }
                }
                None => return Ok(()),
            },
        }
    }
}
async fn seat_active() -> zbus::Result<bool> {
    let bus = zbus::Connection::system().await?;
    let manager = zbus::Proxy::new(&bus, LOGIN, MANAGER, "org.freedesktop.login1.Manager").await?;
    let session = zbus::Proxy::new(&bus, LOGIN, session_path(&manager).await?, SESSION).await?;
    session.get_property("Active").await
}

fn valid_display(portal: bool) -> bool {
    use gtk::prelude::*;
    if gtk::init().is_err() {
        return false;
    }
    let Some(display) = gtk::gdk::Display::default() else {
        return false;
    };
    match display.type_().name() {
        "GdkWaylandDisplay" => portal,
        "GdkX11Display" if portal => std::env::var("DISPLAY")
            .is_ok_and(|d| crate::capture_wayland_host::is_xwayland_server(&d)),
        "GdkX11Display" => true,
        _ => false,
    }
}
/// Serialize competing login notifications until the new owner has published
/// its generation. The owner lock is held to process exit, including teardown.
pub(crate) fn admit() -> io::Result<Option<File>> {
    if !is_activation() {
        return Ok(None);
    }
    // Standalone compositors (including WSL) may have no graphical logind
    // seat. Their XDG activation still receives the correct desktop environment.
    if matches!(action().as_deref(), Some(WATCH | FOLLOW)) {
        if futures_lite::future::block_on(observe()).is_ok() || action().as_deref() == Some(WATCH) {
            return Ok(None);
        }
    }
    let claim = lock("activation.lock")?;
    claim.lock()?;
    let Some(old) = record()? else {
        return Ok(None);
    };
    if old.boot != boot()? {
        return Ok(None);
    }
    let portal = crate::desktop_session_linux::uses_portal();
    if !valid_display(portal) {
        return Err(io::Error::other("目标桌面连接不可用"));
    }
    let lifetime = lock("owner.lock")?;
    let mut rejected = 0;
    loop {
        if action().as_deref() == Some(ARG)
            && std::env::var_os("XDG_SESSION_ID").is_some()
            && !futures_lite::future::block_on(seat_active()).unwrap_or(false)
        {
            return Ok(None);
        }
        if record()?.as_ref() != Some(&old) {
            return Ok(None);
        }
        match lifetime.try_lock() {
            Ok(()) => {
                // Quit clears the record before releasing the owner lock.
                return Ok((record()?.as_ref() == Some(&old)).then_some(claim));
            }
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
        if old.portal == portal {
            return Ok(None);
        }
        match ask_owner(old.instance, portal) {
            Ok(Reply::Busy | Reply::Saved) => {}
            Ok(Reply::Rejected) if rejected < 2 => {
                // A just-finished recording can still be publishing its edit.
                // A failed prepare restores admission; retry only after checking
                // the seat, owner generation and lifetime lock again.
                rejected += 1;
                std::thread::sleep(Duration::from_secs(rejected));
            }
            Ok(Reply::Rejected) => return Err(io::Error::other("旧桌面保存失败")),
            Err(e) if owner_departing(e.kind()) => {}
            Err(e) => return Err(e),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// A compositor may tear down the old process between connect, send and
// acknowledgement. Its lifetime lock and generation, not EOF, authorize entry.
fn owner_departing(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::UnexpectedEof
    )
}
#[derive(Debug, PartialEq)]
enum Reply {
    Busy,
    Saved,
    Rejected,
}
fn ask_owner(instance: Uuid, portal: bool) -> io::Result<Reply> {
    let mut stream = UnixStream::connect_addr(&address(instance)?)?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.write_all(&[u8::from(portal)])?;
    let mut reply = [0];
    stream.read_exact(&mut reply)?;
    match reply[0] {
        0 => Ok(Reply::Busy),
        1 => Ok(Reply::Saved),
        2 => Ok(Reply::Rejected),
        _ => Err(io::Error::other("无法识别桌面接手应答")),
    }
}

fn install_entry() -> Result<(), String> {
    let directory = dirs::config_dir()
        .ok_or("缺少用户配置目录")?
        .join("autostart");
    desktop::safe_path(&directory)?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(ENTRY);
    let previous = desktop::read(&path)?;
    let prefix = "[Desktop Entry]\nType=Application\nName=MewuAI\nX-Mewu-Session-Activation=1\n";
    let exec = desktop::quote_launcher(&desktop::current_launcher()?)?;
    let entry = format!("{prefix}Exec={exec} {FOLLOW}\nTerminal=false\nNoDisplay=true\n");
    if previous.as_deref() == Some(entry.as_bytes()) {
        return Ok(());
    }
    if let Some(bytes) = previous.as_ref() {
        // Honour user/desktop overrides (including Hidden=true).
        let lines = std::str::from_utf8(bytes).unwrap_or("");
        if !lines.starts_with(prefix)
            || !lines.ends_with("Terminal=false\nNoDisplay=true\n")
            || lines.lines().count() != entry.lines().count()
        {
            return Ok(());
        }
    }
    desktop::replace(&path, previous.as_deref(), entry.as_bytes(), &|| true)
}

pub(crate) fn register(app: &AppHandle) -> Result<(), String> {
    let lifetime = lock("owner.lock").map_err(|e| e.to_string())?;
    lifetime.try_lock().map_err(|e| e.to_string())?;
    let owner = Record {
        boot: boot().map_err(|e| e.to_string())?,
        instance: Uuid::new_v4(),
        portal: crate::desktop_session_linux::uses_portal(),
    };
    let listener = UnixListener::bind_addr(&address(owner.instance).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    install_entry()?;
    let path = active_path().map_err(|e| e.to_string())?;
    let previous = desktop::read(&path)?;
    desktop::replace(
        &path,
        previous.as_deref(),
        &serde_json::to_vec(&owner).map_err(|e| e.to_string())?,
        &|| true,
    )?;
    let portal = owner.portal;
    OWNER.set((lifetime, owner)).map_err(|_| "重复注册桌面")?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("mewu-session".into())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                if !rustix::net::sockopt::socket_peercred(&stream)
                    .is_ok_and(|peer| peer.uid == rustix::process::geteuid())
                {
                    continue;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut target = [0];
                if stream.read_exact(&mut target).is_err() || target != [u8::from(!portal)] {
                    continue;
                }
                let host = app.state::<crate::Host>();
                let active = host.capturing.load(Ordering::Acquire)
                    || crate::recording_wayland_host::has_lease()
                    || crate::recording::busy(&app)
                    || crate::scroll_host::busy(&app)
                    || host.lock().map_or(true, |h| {
                        !h.runs.is_empty()
                            || !h.agent_runs.is_empty()
                            || !h.plugin_runs.is_empty()
                            || !h.visual_runs.is_empty()
                    });
                let result = if active || !host.exit.accepts_new_operations() {
                    0
                } else {
                    match tauri::async_runtime::block_on(crate::lifecycle::yield_desktop_session(
                        &app,
                    )) {
                        Ok(()) => 1,
                        Err(error) => {
                            eprintln!("[desktop-session] 无法交接旧桌面: {error}");
                            2
                        }
                    }
                };
                let _ = stream.write_all(&[result]);
            }
        })
        .map_err(|e| e.to_string())?;
    if !is_activation() {
        // The application can be installed or started after this login's
        // autostart phase. Observe its original seat as well as later logins.
        let mut child = std::process::Command::new(desktop::current_launcher()?)
            .arg(WATCH)
            .spawn()
            .map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(())
}
pub(crate) fn disarm() {
    if let Some((_, own)) = OWNER.get() {
        if record().ok().flatten().as_ref() == Some(own) {
            if let Ok(path) = active_path() {
                let _ = fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(reply: Option<u8>) -> (Uuid, std::thread::JoinHandle<()>) {
        let id = Uuid::new_v4();
        let listener = UnixListener::bind_addr(&address(id).unwrap()).unwrap();
        (
            id,
            std::thread::spawn(move || {
                let (mut client, _) = listener.accept().unwrap();
                let mut request = [0];
                client.read_exact(&mut request).unwrap();
                assert_eq!(request, [1]);
                if let Some(value) = reply {
                    client.write_all(&[value]).unwrap();
                }
            }),
        )
    }
    #[test]
    fn lost_owner_is_rechecked_but_unknown_replies_are_rejected() {
        for response in [None, Some(3)] {
            let (id, worker) = owner(response);
            let error = ask_owner(id, true).unwrap_err();
            assert_eq!(owner_departing(error.kind()), response.is_none());
            worker.join().unwrap();
        }
    }
    #[test]
    fn distinguish_save_rejection_from_busy_and_saved() {
        for (reply, expected) in [(0, Reply::Busy), (1, Reply::Saved), (2, Reply::Rejected)] {
            let (id, worker) = owner(Some(reply));
            assert_eq!(ask_owner(id, true).unwrap(), expected);
            worker.join().unwrap();
        }
    }
}
