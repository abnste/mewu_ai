// SPDX-License-Identifier: MPL-2.0
//! Read-only AT-SPI focus geometry. No text contents, clipboard, accessibility
//! actions or application input is read or written through this connection.
use crate::extension_region_desktop::Rect;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use zbus::{zvariant::OwnedObjectPath, Connection, Proxy};

type Reference = (String, OwnedObjectPath);
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
fn error() -> String {
    "无法确认授权区域内的输入焦点".into()
}
async fn bounded<T>(
    future: impl std::future::Future<Output = Result<T, zbus::Error>>,
) -> Result<T, String> {
    futures_lite::future::race(async { future.await.map_err(|_| error()) }, async {
        async_io::Timer::after(Duration::from_millis(300)).await;
        Err(error())
    })
    .await
}
async fn proxy<'a>(
    connection: &'a Connection,
    peer: &'a str,
    path: &'a str,
    interface: &'a str,
) -> Result<Proxy<'a>, String> {
    bounded(Proxy::new(connection, peer, path, interface)).await
}
async fn children(
    connection: &Connection,
    reference: &Reference,
) -> Result<Vec<Reference>, String> {
    let p = proxy(connection, &reference.0, reference.1.as_str(), ACCESSIBLE).await?;
    let children: Vec<Reference> = bounded(p.call("GetChildren", &())).await?;
    if children.len() > 512 {
        return Err(error());
    }
    Ok(children)
}
fn state(words: &[u32], bit: usize) -> bool {
    words
        .get(bit / 32)
        .is_some_and(|v| v & (1 << (bit % 32)) != 0)
}
async fn extent(connection: &Connection, reference: &Reference) -> Result<Rect, String> {
    let p = proxy(
        connection,
        &reference.0,
        reference.1.as_str(),
        "org.a11y.atspi.Component",
    )
    .await?;
    // SCREEN coordinates are not global on native Wayland GTK. Use WINDOW
    // coordinates and demand an exact compositor frame/buffer size match.
    let value: (i32, i32, i32, i32) = bounded(p.call("GetExtents", &(1u32,))).await?;
    Ok(Rect([value.0, value.1, value.2, value.3]))
}

pub(crate) struct Focus {
    pub(crate) toplevel: Rect,
    pub(crate) bounds: Rect,
    pub(crate) label_digest: String,
    pub(crate) editable: bool,
    pub(crate) secure: bool,
    pub(crate) keyboard_supported: bool,
}
pub(crate) struct Accessibility {
    connection: Connection,
}
impl Accessibility {
    pub(crate) fn open() -> Result<Self, String> {
        futures_lite::future::block_on(async {
            let address = zbus::Address::session().map_err(|_| error())?;
            if !matches!(
                address.transport(),
                zbus::address::transport::Transport::Unix(_)
            ) {
                return Err(error());
            }
            let session = bounded(
                zbus::connection::Builder::address(address)
                    .map_err(|_| error())?
                    .method_timeout(Duration::from_millis(300))
                    .max_queued(8)
                    .build(),
            )
            .await?;
            let p = proxy(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus").await?;
            let address: String = bounded(p.call("GetAddress", &())).await?;
            if address.len() > 4096 {
                return Err(error());
            }
            let address = zbus::Address::try_from(address.as_str()).map_err(|_| error())?;
            if !matches!(
                address.transport(),
                zbus::address::transport::Transport::Unix(_)
            ) {
                return Err(error());
            }
            let connection = bounded(
                zbus::connection::Builder::address(address)
                    .map_err(|_| error())?
                    .method_timeout(Duration::from_millis(300))
                    .max_queued(8)
                    .build(),
            )
            .await?;
            Ok(Self { connection })
        })
    }
    pub(crate) fn focus(&self, pid: u32) -> Result<Focus, String> {
        futures_lite::future::block_on(self.focus_async(pid))
    }
    async fn peer(&self, reference: &Reference, pid: u32) -> Result<(), String> {
        if zbus::names::UniqueName::try_from(reference.0.as_str()).is_err() {
            return Err(error());
        }
        let bus = proxy(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await?;
        let actual: u32 =
            bounded(bus.call("GetConnectionUnixProcessID", &(reference.0.as_str(),))).await?;
        let uid: u32 = bounded(bus.call("GetConnectionUnixUser", &(reference.0.as_str(),))).await?;
        if actual != pid || uid != unsafe { libc::geteuid() } {
            return Err(error());
        }
        Ok(())
    }
    async fn focus_async(&self, pid: u32) -> Result<Focus, String> {
        let expires = Instant::now() + Duration::from_secs(3);
        let root = (
            "org.a11y.atspi.Registry".into(),
            OwnedObjectPath::try_from(ROOT).map_err(|_| error())?,
        );
        let apps = children(&self.connection, &root).await?;
        if apps.len() > 128 {
            return Err(error());
        }
        let mut matching = Vec::new();
        for app in apps {
            if Instant::now() > expires {
                return Err(error());
            }
            if self.peer(&app, pid).await.is_ok() {
                matching.push(app)
            }
        }
        if matching.len() != 1 {
            return Err(error());
        }
        let application = matching.pop().ok_or_else(error)?;
        let app_proxy = proxy(
            &self.connection,
            &application.0,
            application.1.as_str(),
            "org.a11y.atspi.Application",
        )
        .await?;
        let toolkit: String = bounded(app_proxy.get_property("ToolkitName")).await?;
        let version: String = bounded(app_proxy.get_property("Version")).await?;
        // GTK4 (including legacy visibility=false Entry) currently reports
        // password entries as ordinary TEXT and invisible=false. Never infer
        // safety from those false negatives or inspect the field's contents.
        let supported = keyboard_toolkit(&toolkit, &version);
        let mut stack = vec![(application.clone(), 0usize, None::<Reference>, false)];
        let mut count = 0;
        let mut found = None;
        while let Some((node, depth, parent_window, secure_parent)) = stack.pop() {
            count += 1;
            if count > 512 || depth > 32 || Instant::now() > expires || node.0 != application.0 {
                return Err(error());
            }
            let p = proxy(&self.connection, &node.0, node.1.as_str(), ACCESSIBLE).await?;
            let role: u32 = bounded(p.call("GetRole", &())).await?;
            let states: Vec<u32> = bounded(p.call("GetState", &())).await?;
            if states.len() > 4 || state(&states, 6) {
                continue;
            }
            let secure = secure_parent || role == 40;
            let window = if matches!(role, 23 | 69) {
                if !state(&states, 1) || !state(&states, 25) {
                    continue;
                }
                Some(node.clone())
            } else {
                parent_window
            };
            if state(&states, 12) {
                if found.is_some() {
                    return Err(error());
                }
                let window = window.clone().ok_or_else(error)?;
                if !state(&states, 25) || !state(&states, 30) {
                    return Err(error());
                }
                let title_proxy =
                    proxy(&self.connection, &window.0, window.1.as_str(), ACCESSIBLE).await?;
                let title: String = bounded(title_proxy.get_property("Name")).await?;
                if title.is_empty() || title.len() > 8192 {
                    return Err(error());
                }
                let attributes: HashMap<String, String> =
                    bounded(p.call("GetAttributes", &())).await?;
                if attributes.len() > 128 {
                    return Err(error());
                }
                let protected = attributes.iter().any(|(k, v)| {
                    matches!(k.as_str(), "text-input-type" | "input-type")
                        && matches!(v.as_str(), "password" | "pin")
                });
                found = Some(Focus {
                    toplevel: extent(&self.connection, &window).await?,
                    bounds: extent(&self.connection, &node).await?,
                    label_digest: format!("{:x}", Sha256::digest(title.as_bytes())),
                    editable: state(&states, 7),
                    secure: secure || protected,
                    keyboard_supported: supported && matches!(role, 61 | 79 | 7 | 43),
                });
            }
            for child in children(&self.connection, &node).await?.into_iter().rev() {
                stack.push((child, depth + 1, window.clone(), secure))
            }
        }
        self.peer(&application, pid).await?;
        found.ok_or_else(error)
    }
}
fn keyboard_toolkit(toolkit: &str, version: &str) -> bool {
    match toolkit.to_ascii_lowercase().as_str() {
        "gtk" => version.split('.').next() == Some("3"),
        "gecko" | "qt" | "qt5" | "qt6" | "chromium" | "webkitgtk" | "aura" => true,
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn password_ambiguous_toolkits_do_not_enable_keyboard() {
        assert!(!keyboard_toolkit("GTK", "4.14.5"));
        assert!(!keyboard_toolkit("GTK", ""));
        assert!(!keyboard_toolkit("unknown", "1"));
        assert!(keyboard_toolkit("GTK", "3.24.41"));
        assert!(keyboard_toolkit("Gecko", "128"));
    }
    #[test]
    fn state_words_are_bounded_and_not_confused_with_values() {
        assert!(state(&[1 << 12], 12));
        assert!(!state(&[], 12));
        assert!(!state(&[1 << 7], 12));
        assert!(state(&[0, 1 << 10], 42));
    }
}
