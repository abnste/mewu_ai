// SPDX-License-Identifier: MPL-2.0
//! A single user-approved ScreenCast session. Never restores permission or
//! connects to the unrestricted PipeWire daemon; the remote FD is session-owned.
use futures_util::StreamExt;
use std::{collections::HashMap, future::Future, os::fd::OwnedFd, time::{Duration, Instant}};
use zbus::{zvariant::{OwnedObjectPath, OwnedValue, Value}, Connection, Proxy};

type Options<'a> = HashMap<&'a str, Value<'a>>;
type Results = HashMap<String, OwnedValue>;
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const SESSION: &str = "org.freedesktop.portal.Session";
const REQUEST: &str = "org.freedesktop.portal.Request";
const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(5);

fn token() -> String { format!("mewu_{}", uuid::Uuid::new_v4().simple()) }
fn error() -> String { "系统屏幕共享服务不可用".into() }

// zbus method_timeout bounds method replies, not connection authentication or
// every proxy/subscription setup await. Poll the same future without resending
// it, and apply both Work cancellation and the operation's overall deadline.
async fn checked<T, E>(
    future: impl Future<Output = Result<T, E>>,
    expires: Instant,
    allowed: &mut impl FnMut() -> bool,
) -> Result<T, String> {
    futures_lite::pin!(future);
    loop {
        if !allowed() { return Err("屏幕共享已取消".into()); }
        let now = Instant::now();
        if now >= expires { return Err("屏幕共享授权超时".into()); }
        let tick = (now + Duration::from_millis(20)).min(expires);
        let result = futures_lite::future::race(async { Some(future.as_mut().await) }, async {
            async_io::Timer::at(tick).await; None
        }).await;
        if let Some(result) = result {
            if !allowed() { return Err("屏幕共享已取消".into()); }
            if Instant::now() >= expires { return Err("屏幕共享授权超时".into()); }
            return result.map_err(|_| error());
        }
    }
}
async fn transport<T, E>(
    future: impl Future<Output = Result<T, E>>,
    expires: Instant,
    allowed: &mut impl FnMut() -> bool,
) -> Result<T, String> {
    checked(future, expires.min(Instant::now() + TRANSPORT_TIMEOUT), allowed).await
}

async fn bounded_close(connection: &Connection, owner: &str, path: &str, interface: &str) {
    let closing = async {
        if let Ok(proxy) = Proxy::new(connection, owner, path, interface).await {
            let _: Result<(), _> = proxy.call("Close", &()).await;
        }
    };
    futures_lite::future::race(closing, async {
        async_io::Timer::after(Duration::from_millis(200)).await;
    }).await;
}

#[derive(Clone, Copy)]
enum RequestCleanup { CloseRequest, ViaSession }
struct RequestGuard<'a> { connection: &'a Connection, owner: &'a str, path: String, ended: bool, cleanup: RequestCleanup }
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.ended && matches!(self.cleanup, RequestCleanup::CloseRequest) {
            futures_lite::future::block_on(bounded_close(self.connection, self.owner, &self.path, REQUEST));
        }
    }
}

async fn request<B: serde::Serialize + zbus::zvariant::DynamicType>(
    connection: &Connection, owner: &str, method: &str, token: &str,
    body: &B, cleanup: RequestCleanup, expires: Instant, allowed: &mut impl FnMut() -> bool,
) -> Result<Results, String> {
    if !allowed() || Instant::now() >= expires { return Err("录屏已取消".into()); }
    let sender = connection.unique_name().ok_or_else(error)?.as_str()
        .trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    // Subscribe before sending: the portal may emit Response before the method
    // reply. Pin the actual bus owner as well as the exact unpredictable path.
    let response = transport(Proxy::new(connection, owner, path.as_str(), REQUEST), expires, allowed).await?;
    let mut signals = transport(response.receive_signal("Response"), expires, allowed).await?;
    // Once a session exists, Session.Close ends its pending chooser too. Do not
    // send Request.Close followed by Session.Close: GNOME portal 46.2 frees the
    // chooser in the former without clearing its session pointer (upstream
    // 9a4dffa), so the latter would free that same handle twice. The enclosing
    // setup error path awaits one bounded Session.Close before returning.
    let mut guard = RequestGuard { connection, owner, path: path.clone(), ended: false, cleanup };
    let desktop = transport(Proxy::new(connection, owner, DESKTOP, SCREENCAST), expires, allowed).await?;
    let returned = transport(desktop.call::<_, _, OwnedObjectPath>(method, body), expires, allowed).await?;
    if returned.as_str() != guard.path { return Err("屏幕共享请求身份不匹配".into()); }
    let signal = checked(async { signals.next().await.ok_or_else(error) }, expires, allowed).await?;
    let (code, results): (u32, Results) = signal.body().deserialize().map_err(|_| error())?;
    guard.ended = true;
    match code { 0 => Ok(results), 1 => Err("已取消屏幕共享".into()), _ => Err(error()) }
}

pub(crate) struct PortalSession {
    connection: Connection,
    owner: String,
    path: OwnedObjectPath,
    pub(crate) node: u32,
    pub(crate) remote: OwnedFd,
    pub(crate) logical_size: Option<(i32, i32)>,
    pub(crate) logical_position: Option<(i32, i32)>,
}
impl Drop for PortalSession {
    fn drop(&mut self) {
        futures_lite::future::block_on(bounded_close(&self.connection, &self.owner, self.path.as_str(), SESSION));
    }
}
impl PortalSession {
    pub(crate) fn open(expires: Instant, allowed: &mut impl FnMut() -> bool) -> Result<Self, String> {
        let address = zbus::Address::session().map_err(|_| error())?;
        futures_lite::future::block_on(Self::open_at(address, expires, allowed))
    }
    async fn open_at(address: zbus::Address, expires: Instant, allowed: &mut impl FnMut() -> bool) -> Result<Self, String> {
            if !matches!(address.transport(), zbus::address::transport::Transport::Unix(_)) { return Err(error()); }
            let builder = zbus::connection::Builder::address(address).map_err(|_| error())?
                .max_queued(16).method_timeout(TRANSPORT_TIMEOUT);
            let connection = transport(builder.build(), expires, allowed).await?;
            let bus = transport(Proxy::new(&connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus"), expires, allowed).await?;
            // GetNameOwner alone never activates an idle desktop portal. This
            // explicit user operation may start the installed portal service,
            // then pin all permission calls/signals to its unique bus owner.
            let _: u32 = transport(bus.call("StartServiceByName", &("org.freedesktop.portal.Desktop", 0u32)), expires, allowed).await?;
            let owner: String = transport(bus.call("GetNameOwner", &("org.freedesktop.portal.Desktop",)), expires, allowed).await?;
            if !owner.starts_with(':') { return Err(error()); }
            drop(bus);
            let desktop = transport(Proxy::new(&connection, owner.as_str(), DESKTOP, SCREENCAST), expires, allowed).await?;
            let version: u32 = transport(desktop.get_property("version"), expires, allowed).await?;
            let types: u32 = transport(desktop.get_property("AvailableSourceTypes"), expires, allowed).await?;
            let cursors: u32 = transport(desktop.get_property("AvailableCursorModes"), expires, allowed).await?;
            if version < 3 || types & 1 == 0 || cursors & 1 == 0 { return Err(error()); }
            let create_token = token();
            let session_token = token();
            let mut options = Options::new();
            options.insert("handle_token", Value::from(create_token.as_str()));
            options.insert("session_handle_token", Value::from(session_token.as_str()));
            let result = request(&connection, &owner, "CreateSession", &create_token, &(options,), RequestCleanup::CloseRequest, expires, allowed).await?;
            let path = result.get("session_handle").and_then(|v| <&str>::try_from(v).ok()).ok_or_else(error)?;
            let sender = connection.unique_name().ok_or_else(error)?.as_str().trim_start_matches(':').replace('.', "_");
            let expected = format!("/org/freedesktop/portal/desktop/session/{sender}/{session_token}");
            if path != expected { return Err("屏幕共享会话身份不匹配".into()); }
            let path = OwnedObjectPath::try_from(path.to_string()).map_err(|_| error())?;
            let setup = async {
                let select_token = token();
                let mut options = Options::new();
                options.insert("handle_token", Value::from(select_token.as_str()));
                options.insert("types", Value::from(1u32));
                options.insert("multiple", Value::from(false));
                options.insert("cursor_mode", Value::from(1u32));
                if version >= 4 { options.insert("persist_mode", Value::from(0u32)); }
                request(&connection, &owner, "SelectSources", &select_token, &(&path, options), RequestCleanup::ViaSession, expires, allowed).await?;
                let start_token = token();
                let mut options = Options::new();
                options.insert("handle_token", Value::from(start_token.as_str()));
                let mut result = request(&connection, &owner, "Start", &start_token, &(&path, "", options), RequestCleanup::ViaSession, expires, allowed).await?;
                let streams: Vec<(u32, HashMap<String, OwnedValue>)> = result.remove("streams")
                    .ok_or_else(error)?.try_into().map_err(|_| error())?;
                let (node, properties) = single_monitor(streams)?;
                if !allowed() || Instant::now() >= expires { return Err("录屏已取消".into()); }
                let empty = Options::new();
                let remote: zbus::zvariant::OwnedFd = transport(desktop.call("OpenPipeWireRemote", &(&path, empty)), expires, allowed).await?;
                let size = properties.get("size").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok());
                let position = properties.get("position").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok());
                Ok((node, remote.into(), size, position))
            }.await;
            drop(desktop);
            match setup {
                Ok((node, remote, logical_size, logical_position)) => Ok(Self { connection, owner, path, node, remote, logical_size, logical_position }),
                Err(error) => { bounded_close(&connection, &owner, path.as_str(), SESSION).await; Err(error) }
            }
    }
}

fn single_monitor(mut streams: Vec<(u32, HashMap<String, OwnedValue>)>) -> Result<(u32, HashMap<String, OwnedValue>), String> {
    if streams.len() != 1 { return Err("请选择一个屏幕".into()); }
    let (node, properties) = streams.pop().unwrap();
    if node == 0 || properties.get("source_type").and_then(|v| u32::try_from(v).ok()) != Some(1) {
        return Err("请选择屏幕而不是应用窗口".into());
    }
    Ok((node, properties))
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn stream_admission_requires_exact_single_monitor() {
        assert!(single_monitor(vec![]).is_err());
        let mut props = HashMap::new(); props.insert("source_type".into(), OwnedValue::from(1u32));
        assert!(single_monitor(vec![(0, props.clone())]).is_err());
        assert!(single_monitor(vec![(12, props.clone()), (13, props.clone())]).is_err());
        assert!(single_monitor(vec![(12, props)]).is_ok());
        let mut props = HashMap::new(); props.insert("source_type".into(), OwnedValue::from(2u32));
        assert!(single_monitor(vec![(12, props)]).is_err());
    }
}
