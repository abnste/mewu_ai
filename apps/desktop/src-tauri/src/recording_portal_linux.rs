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
const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";
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
    request_on(connection,owner,SCREENCAST,method,token,body,cleanup,expires,allowed).await
}
async fn request_on<B: serde::Serialize + zbus::zvariant::DynamicType>(
    connection:&Connection,owner:&str,interface:&str,method:&str,token:&str,
    body:&B,cleanup:RequestCleanup,expires:Instant,allowed:&mut impl FnMut()->bool,
)->Result<Results,String> {
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
    let desktop = transport(Proxy::new(connection, owner, DESKTOP, interface), expires, allowed).await?;
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
    pub(crate) pipewire_serial: Option<u64>,
    pub(crate) remote: OwnedFd,
    pub(crate) logical_size: Option<(i32, i32)>,
    pub(crate) logical_position: Option<(i32, i32)>,
    control: bool,
}
impl Drop for PortalSession {
    fn drop(&mut self) {
        futures_lite::future::block_on(bounded_close(&self.connection, &self.owner, self.path.as_str(), SESSION));
    }
}
impl PortalSession {
    pub(crate) fn open(expires: Instant, allowed: &mut impl FnMut() -> bool) -> Result<Self, String> {
        let address = zbus::Address::session().map_err(|_| error())?;
        futures_lite::future::block_on(Self::open_at(address, "", false, expires, allowed))
    }
    pub(crate) fn open_parent(parent:&str,expires:Instant,allowed:&mut impl FnMut()->bool)->Result<Self,String>{
        if !valid_recording_parent(parent){return Err("录屏父窗口标识无效".into())}
        let address=zbus::Address::session().map_err(|_|error())?;
        futures_lite::future::block_on(Self::open_at(address,parent,false,expires,allowed))
    }
    /// A fresh user-approved combined input/screen session. Never reuse a
    /// screenshot grant, remembered restore token or unrestricted PipeWire FD.
    pub(crate) fn open_control(expires:Instant,allowed:&mut impl FnMut()->bool)->Result<Self,String>{
        let address=zbus::Address::session().map_err(|_|error())?;
        futures_lite::future::block_on(Self::open_at(address,"",true,expires,allowed))
    }
    async fn open_at(address: zbus::Address,parent:&str,control:bool, expires: Instant, allowed: &mut impl FnMut() -> bool) -> Result<Self, String> {
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
            let session_interface=if control {REMOTE_DESKTOP}else{SCREENCAST};
            if control {
                let remote=transport(Proxy::new(&connection,owner.as_str(),DESKTOP,REMOTE_DESKTOP),expires,allowed).await?;
                let devices:u32=transport(remote.get_property("AvailableDeviceTypes"),expires,allowed).await?;
                if devices & 3 != 3 {return Err("系统未提供鼠标和键盘控制".into());}
            }
            let create_token = token();
            let session_token = token();
            let mut options = Options::new();
            options.insert("handle_token", Value::from(create_token.as_str()));
            options.insert("session_handle_token", Value::from(session_token.as_str()));
            let result = request_on(&connection, &owner, session_interface,"CreateSession", &create_token, &(options,), RequestCleanup::CloseRequest, expires, allowed).await?;
            let path = result.get("session_handle").and_then(|v| <&str>::try_from(v).ok()).ok_or_else(error)?;
            let sender = connection.unique_name().ok_or_else(error)?.as_str().trim_start_matches(':').replace('.', "_");
            let expected = format!("/org/freedesktop/portal/desktop/session/{sender}/{session_token}");
            if path != expected { return Err("屏幕共享会话身份不匹配".into()); }
            let path = OwnedObjectPath::try_from(path.to_string()).map_err(|_| error())?;
            let setup = async {
                if control {
                    let devices_token=token();let mut options=Options::new();
                    options.insert("handle_token",Value::from(devices_token.as_str()));
                    options.insert("types",Value::from(3u32));
                    request_on(&connection,&owner,REMOTE_DESKTOP,"SelectDevices",&devices_token,&(&path,options),RequestCleanup::ViaSession,expires,allowed).await?;
                }
                let select_token = token();
                let mut options = Options::new();
                options.insert("handle_token", Value::from(select_token.as_str()));
                options.insert("types", Value::from(1u32));
                options.insert("multiple", Value::from(false));
                options.insert("cursor_mode", Value::from(1u32));
                if version >= 4 && !control { options.insert("persist_mode", Value::from(0u32)); }
                request(&connection, &owner, "SelectSources", &select_token, &(&path, options), RequestCleanup::ViaSession, expires, allowed).await?;
                let start_token = token();
                let mut options = Options::new();
                options.insert("handle_token", Value::from(start_token.as_str()));
                let mut result = request_on(&connection, &owner, session_interface,"Start", &start_token, &(&path, parent, options), RequestCleanup::ViaSession, expires, allowed).await?;
                if control && result.get("devices").and_then(|v|u32::try_from(v).ok()).is_none_or(|devices|devices&3!=3) {
                    return Err("未授权鼠标和键盘控制".into());
                }
                let streams: Vec<(u32, HashMap<String, OwnedValue>)> = result.remove("streams")
                    .ok_or_else(error)?.try_into().map_err(|_| error())?;
                let (node, properties) = single_monitor(streams)?;
                let pipewire_serial = stream_serial(&properties, version)?;
                if !allowed() || Instant::now() >= expires { return Err("录屏已取消".into()); }
                let empty = Options::new();
                let remote: zbus::zvariant::OwnedFd = transport(desktop.call("OpenPipeWireRemote", &(&path, empty)), expires, allowed).await?;
                let size = properties.get("logical_size").or_else(||properties.get("size")).and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok());
                let position = properties.get("position").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok());
                Ok((node, remote.into(), size, position, pipewire_serial))
            }.await;
            drop(desktop);
            match setup {
                Ok((node, remote, logical_size, logical_position, pipewire_serial)) => Ok(Self { connection, owner, path, node, pipewire_serial, remote, logical_size, logical_position, control }),
                Err(error) => { bounded_close(&connection, &owner, path.as_str(), SESSION).await; Err(error) }
            }
    }

    /// These methods are native backend primitives, never exposed directly to
    /// a plugin. The region owner must validate its source and focus per input.
    pub(crate) fn input<B:serde::Serialize+zbus::zvariant::DynamicType>(&self,method:&str,body:&B,allowed:&mut impl FnMut()->bool)->Result<(),String>{
        if !self.control || !matches!(method,"NotifyPointerMotionAbsolute"|"NotifyPointerButton"|"NotifyPointerAxisDiscrete"|"NotifyKeyboardKeysym") {return Err("区域输入会话未授权".into());}
        futures_lite::future::block_on(async{
            let expires=Instant::now()+TRANSPORT_TIMEOUT;
            let desktop=transport(Proxy::new(&self.connection,self.owner.as_str(),DESKTOP,REMOTE_DESKTOP),expires,allowed).await?;
            transport(desktop.call::<_,_,()>(method,body),expires,allowed).await
        })
    }
    pub(crate) fn pointer(&self,x:f64,y:f64,allowed:&mut impl FnMut()->bool)->Result<(),String>{
        let Some((w,h))=self.logical_size else{return Err("屏幕坐标信息缺失".into())};
        if !x.is_finite()||!y.is_finite()||x<0.||y<0.||x>=f64::from(w)||y>=f64::from(h){return Err("输入坐标超出授权屏幕".into());}
        self.input("NotifyPointerMotionAbsolute",&(&self.path,Options::new(),self.node,x,y),allowed)
    }
    pub(crate) fn button(&self,pressed:bool,allowed:&mut impl FnMut()->bool)->Result<(),String>{
        self.input("NotifyPointerButton",&(&self.path,Options::new(),272i32,u32::from(pressed)),allowed)
    }
    pub(crate) fn scroll(&self,steps:i32,allowed:&mut impl FnMut()->bool)->Result<(),String>{
        self.input("NotifyPointerAxisDiscrete",&(&self.path,Options::new(),0u32,steps),allowed)
    }
    pub(crate) fn keysym(&self,key:i32,pressed:bool,allowed:&mut impl FnMut()->bool)->Result<(),String>{
        self.input("NotifyKeyboardKeysym",&(&self.path,Options::new(),key,u32::from(pressed)),allowed)
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

/// ScreenCast v6 supplies object.serial because a removed node's numerical ID
/// can be reused. A malformed or missing v6 identity must not silently select
/// the old ID; earlier portal versions retain their session-restricted FD path.
fn stream_serial(properties: &HashMap<String, OwnedValue>, version: u32) -> Result<Option<u64>, String> {
    match properties.get("pipewire-serial") {
        Some(value) => u64::try_from(value).ok().filter(|value| *value != 0)
            .map(Some).ok_or_else(|| "屏幕共享流标识无效".into()),
        None if version >= 6 => Err("屏幕共享流标识缺失".into()),
        None => Ok(None),
    }
}

fn valid_recording_parent(value:&str)->bool{
    // The desktop session may be Wayland while the owned GTK window is XWayland.
    // Its identifier must match that actual window, not the session backend.
    // Only the host's exported parent enters this worker; this is wire-format
    // validation, not permission to accept a renderer-provided window ID.
    if let Some(xid)=value.strip_prefix("x11:") {
        return !xid.is_empty() && xid.len()<=8 && xid.bytes().all(|b|b.is_ascii_hexdigit())
            && u32::from_str_radix(xid,16).is_ok_and(|id|id!=0);
    }
    value.strip_prefix("wayland:").is_some_and(|handle|!handle.is_empty()&&handle.len()<=512&&!handle.chars().any(char::is_control))
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn stream_identity_does_not_downgrade_to_a_reused_node_id() {
        let mut properties = HashMap::new();
        assert_eq!(stream_serial(&properties, 5).unwrap(), None);
        assert!(stream_serial(&properties, 6).is_err());
        for value in [1u64, u64::from(u32::MAX) + 1, u64::MAX] {
            properties.insert("pipewire-serial".into(), OwnedValue::from(value));
            assert_eq!(stream_serial(&properties, 6).unwrap(), Some(value));
            assert_eq!(stream_serial(&properties, 5).unwrap(), Some(value));
        }
        properties.insert("pipewire-serial".into(), OwnedValue::from(0u64));
        assert!(stream_serial(&properties, 6).is_err());
        assert!(stream_serial(&properties, 5).is_err());
        properties.insert("pipewire-serial".into(), OwnedValue::from(42u32));
        assert!(stream_serial(&properties, 6).is_err());
    }
    #[test] fn recording_parent_accepts_both_owned_portal_window_formats(){
        assert!(valid_recording_parent("wayland:~12l9jdl.-a"));
        for value in ["x11:1234","x11:1","x11:ffffffff","x11:ABCDEF12"]{assert!(valid_recording_parent(value));}
        for value in ["","wayland:","x11:","x11:0","x11:00000000","x11:100000000","x11:-1","x11:0x1234","x11:12\n34","x11: 1234","x11:xyz","https://host/","wayland:bad\nhandle"]{assert!(!valid_recording_parent(value),"{value:?}");}
        assert!(!valid_recording_parent(&format!("wayland:{}","a".repeat(513))));
    }
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
