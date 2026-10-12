// SPDX-License-Identifier: MPL-2.0
//! A single user-approved ScreenCast session. Never restores permission or
//! connects to the unrestricted PipeWire daemon; the remote FD is session-owned.
use futures_util::StreamExt;
use std::{collections::HashMap, os::fd::OwnedFd, time::{Duration, Instant}};
use zbus::{zvariant::{OwnedObjectPath, OwnedValue, Value}, Connection, Proxy};

type Options<'a> = HashMap<&'a str, Value<'a>>;
type Results = HashMap<String, OwnedValue>;
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const SESSION: &str = "org.freedesktop.portal.Session";
const REQUEST: &str = "org.freedesktop.portal.Request";

fn token() -> String { format!("mewu_{}", uuid::Uuid::new_v4().simple()) }
fn error() -> String { "系统屏幕共享服务不可用".into() }

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

struct RequestGuard<'a> { connection: &'a Connection, owner: &'a str, path: String, ended: bool }
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.ended {
            futures_lite::future::block_on(bounded_close(self.connection, self.owner, &self.path, REQUEST));
        }
    }
}

async fn request<B: serde::Serialize + zbus::zvariant::DynamicType>(
    connection: &Connection, owner: &str, method: &str, token: &str,
    body: &B, expires: Instant, allowed: &mut impl FnMut() -> bool,
) -> Result<Results, String> {
    if !allowed() || Instant::now() >= expires { return Err("录屏已取消".into()); }
    let sender = connection.unique_name().ok_or_else(error)?.as_str()
        .trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    // Subscribe before sending: the portal may emit Response before the method
    // reply. Pin the actual bus owner as well as the exact unpredictable path.
    let response = Proxy::new(connection, owner, path.as_str(), REQUEST).await.map_err(|_| error())?;
    let mut signals = response.receive_signal("Response").await.map_err(|_| error())?;
    let mut guard = RequestGuard { connection, owner, path, ended: false };
    let desktop = Proxy::new(connection, owner, DESKTOP, SCREENCAST).await.map_err(|_| error())?;
    let reply = desktop.call::<_, _, OwnedObjectPath>(method, body);
    futures_lite::pin!(reply);
    let returned = loop {
        if !allowed() || Instant::now() >= expires { return Err("录屏授权已取消或超时".into()); }
        let next = futures_lite::future::race(async { Some(reply.as_mut().await) }, async {
            async_io::Timer::after(Duration::from_millis(20)).await; None
        }).await;
        if let Some(result) = next { break result.map_err(|_| error())?; }
    };
    if returned.as_str() != guard.path { return Err("屏幕共享请求身份不匹配".into()); }
    loop {
        if !allowed() || Instant::now() >= expires { return Err("录屏授权已取消或超时".into()); }
        let next = futures_lite::future::race(signals.next(), async {
            async_io::Timer::after(Duration::from_millis(20)).await; None
        }).await;
        let Some(signal) = next else { continue; };
        let (code, results): (u32, Results) = signal.body().deserialize().map_err(|_| error())?;
        guard.ended = true;
        return match code { 0 => Ok(results), 1 => Err("已取消屏幕共享".into()), _ => Err(error()) };
    }
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
        futures_lite::future::block_on(async {
            let connection = Connection::session().await.map_err(|_| error())?;
            let bus = zbus::fdo::DBusProxy::new(&connection).await.map_err(|_| error())?;
            let owner = bus.get_name_owner("org.freedesktop.portal.Desktop".try_into().map_err(|_| error())?)
                .await.map_err(|_| error())?.to_string();
            let desktop = Proxy::new(&connection, owner.as_str(), DESKTOP, SCREENCAST).await.map_err(|_| error())?;
            let version: u32 = desktop.get_property("version").await.map_err(|_| error())?;
            let types: u32 = desktop.get_property("AvailableSourceTypes").await.map_err(|_| error())?;
            let cursors: u32 = desktop.get_property("AvailableCursorModes").await.map_err(|_| error())?;
            if version < 3 || types & 1 == 0 || cursors & 1 == 0 { return Err(error()); }
            let create_token = token();
            let session_token = token();
            let mut options = Options::new();
            options.insert("handle_token", Value::from(create_token.as_str()));
            options.insert("session_handle_token", Value::from(session_token.as_str()));
            let result = request(&connection, &owner, "CreateSession", &create_token, &(options,), expires, allowed).await?;
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
                request(&connection, &owner, "SelectSources", &select_token, &(&path, options), expires, allowed).await?;
                let start_token = token();
                let mut options = Options::new();
                options.insert("handle_token", Value::from(start_token.as_str()));
                let mut result = request(&connection, &owner, "Start", &start_token, &(&path, "", options), expires, allowed).await?;
                let streams: Vec<(u32, HashMap<String, OwnedValue>)> = result.remove("streams")
                    .ok_or_else(error)?.try_into().map_err(|_| error())?;
                let (node, properties) = single_monitor(streams)?;
                if !allowed() || Instant::now() >= expires { return Err("录屏已取消".into()); }
                let empty = Options::new();
                let remote: zbus::zvariant::OwnedFd = desktop.call("OpenPipeWireRemote", &(&path, empty)).await.map_err(|_| error())?;
                let size = properties.get("size").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok());
                let position = properties.get("position").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok());
                Ok((node, remote.into(), size, position))
            }.await;
            drop(desktop);
            match setup {
                Ok((node, remote, logical_size, logical_position)) => Ok(Self { connection, owner, path, node, remote, logical_size, logical_position }),
                Err(error) => { bounded_close(&connection, &owner, path.as_str(), SESSION).await; Err(error) }
            }
        })
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
