// SPDX-License-Identifier: MPL-2.0
//! GlobalShortcuts v1, on a separate, registered host D-Bus connection.
//! The actor is the only preference/session owner; cancellation is out of band.
use crate::{capture_shortcut::{CaptureShortcutState, ShortcutStatus}, host_preferences::{LoadedPreferences, PreferencesFile}, Host};
use futures_util::StreamExt;
use std::{collections::{HashMap, VecDeque}, future::Future, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot, Notify};
use zbus::{Connection, Proxy, zvariant::{OwnedObjectPath, OwnedValue, Value}};

const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const SERVICE: &str = "org.freedesktop.portal.Desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const SESSION: &str = "org.freedesktop.portal.Session";
const REQUEST: &str = "org.freedesktop.portal.Request";
const ACTION: &str = "capture";
const METHOD: Duration = Duration::from_secs(5);
const CONSENT: Duration = Duration::from_secs(180);
const UNAVAILABLE: &str = "系统快捷键服务不可用";
type Options<'a> = HashMap<&'a str, Value<'a>>;
type Results = HashMap<String, OwnedValue>;
type Shortcuts = Vec<(String, Results)>;

#[derive(Default)]
struct Nonces { active: Option<String>, canceled: VecDeque<String> }
impl Nonces {
    fn begin(&mut self, id: &str) -> Result<(), String> {
        valid_id(id)?;
        if self.canceled.iter().any(|x| x == id) { return Err("快捷键设置已取消".into()); }
        if self.active.is_some() { return Err("快捷键设置尚未结束".into()); }
        // Do not evict a cancel-before-configure tombstone: exhausting the
        // bounded lifetime budget fails closed rather than resurrecting an ID.
        if self.canceled.len() >= 512 { return Err("快捷键请求过多，请重启后重试".into()); }
        self.active = Some(id.into()); Ok(())
    }
    fn cancel(&mut self, id: &str) -> Result<bool, String> {
        valid_id(id)?;
        if !self.canceled.iter().any(|x| x == id) {
            if self.canceled.len() >= 512 { return Err("快捷键请求过多，请重启后重试".into()); }
            self.canceled.push_back(id.into());
        }
        Ok(self.active.as_deref() == Some(id))
    }
    fn allowed(&self, id: &str) -> bool { self.active.as_deref() == Some(id) && !self.canceled.iter().any(|x|x==id) }
    fn end(&mut self, id: &str) { if self.active.as_deref() == Some(id) { self.active = None; } }
}
fn valid_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id) { Ok(()) } else { Err("快捷键请求标识无效".into()) }
}
fn token() -> String { format!("mewu_{}", uuid::Uuid::new_v4().simple()) }
pub(crate) struct Shared {
    app: AppHandle, root: PathBuf, view: Mutex<CaptureShortcutState>, nonces: Mutex<Nonces>,
    route: AtomicBool, generation: AtomicU64, stop: AtomicBool, completed: AtomicBool,
    wake: Arc<Notify>, settled: Notify, sender: mpsc::Sender<Command>, startup: AtomicBool, restore_due: AtomicBool,
}
#[derive(Clone)] pub(crate) struct Runtime(Arc<Shared>);
enum Command {
    Configure { revision: u64, id: String, owner: usize, parent: crate::capture_shortcut_wayland_host::ExportedParent, reply: oneshot::Sender<Result<CaptureShortcutState,String>> },
    Disable { revision: u64, owner: usize, reply: oneshot::Sender<Result<CaptureShortcutState,String>> },
    Restore,
}
impl Shared {
    fn publish(&self, status: ShortcutStatus, label: Option<String>, pending: bool, message: Option<String>) {
        let state = {
            let mut state = self.view.lock().unwrap_or_else(|e|e.into_inner());
            state.sequence = state.sequence.saturating_add(1);
            state.status = status; state.actual_trigger_description = label;
            state.configuration_pending = Some(pending);
            state.editable = !pending && status != ShortcutStatus::CleanupPending && status != ShortcutStatus::Unavailable;
            state.message = message;
            if !pending { state.configuration_request_id = None; }
            state.clone()
        };
        let _=self.app.emit_to("settings","capture-shortcut-state",&state);
        let _=self.app.emit_to("space","capture-shortcut-state",&state);
    }
    fn host_allowed(&self, owner: Option<usize>) -> bool {
        !self.stop.load(Ordering::Acquire) && self.app.try_state::<Host>().is_some_and(|h|
            h.exit.accepts_new_operations() && h.storage.is_ready() && !h.capturing.load(Ordering::Acquire))
            && crate::recording::ensure_idle(&self.app).is_ok()
            && owner.is_none_or(|v|crate::window_actions_linux::current_for(v,"settings"))
    }
    fn config_allowed(&self, id: &str, owner: usize) -> bool {
        self.host_allowed(Some(owner)) && self.nonces.lock().is_ok_and(|n|n.allowed(id))
    }
}
impl Runtime {
    pub(crate) fn initialize(app: &AppHandle, root: &std::path::Path) -> Result<(), String> {
        let file = PreferencesFile::open(root);
        let loaded = file.as_ref().map_err(|e|*e).and_then(|f|f.load());
        let fault=loaded.as_ref().err().map(ToString::to_string);
        let initial=loaded.as_ref().map(|l|l.preferences.clone()).unwrap_or_else(|_|LoadedPreferences::missing().preferences);
        let (sender, receiver)=mpsc::channel(8);
        let shared=Arc::new(Shared { app:app.clone(),root:root.into(),
            view:Mutex::new(CaptureShortcutState { revision:initial.revision, sequence:0, configured:initial.capture_shortcut.clone(),active:None,
                status:if fault.is_some(){ShortcutStatus::Unavailable}else if initial.capture_shortcut.is_none() && !initial.wayland_shortcut_enabled {ShortcutStatus::Disabled}else{ShortcutStatus::NeedsSetup},
                editable:fault.is_none(),message:fault, backend:Some("wayland_portal"), actual_trigger_description:None,
                configuration_pending:Some(false),system_settings_available:Some(crate::capture_shortcut_wayland_host::system_settings_available()), configuration_request_id:None,portal_enabled:Some(initial.wayland_shortcut_enabled) }),
            nonces:Mutex::new(Nonces::default()),route:AtomicBool::new(false),generation:AtomicU64::new(0),stop:AtomicBool::new(false),completed:AtomicBool::new(false),
            wake:Arc::new(Notify::new()),settled:Notify::new(),sender,startup:AtomicBool::new(false),restore_due:AtomicBool::new(true) });
        app.manage(Runtime(shared.clone()));
        tauri::async_runtime::spawn(actor(shared,receiver,file.ok(),loaded.ok())); Ok(())
    }
    pub(crate) fn view(&self) -> CaptureShortcutState { self.0.view.lock().unwrap_or_else(|e|e.into_inner()).clone() }
    pub(crate) fn restore_once(&self) {
        self.0.startup.store(true,Ordering::Release);
        if self.0.restore_due.swap(false,Ordering::AcqRel) && self.0.sender.try_send(Command::Restore).is_err() { self.0.restore_due.store(true,Ordering::Release); }
    }
    pub(crate) fn signal_cancel(&self, id: &str) -> Result<(), String> {
        let matched=self.0.nonces.lock().map_err(|_|UNAVAILABLE)?.cancel(id)?;
        if matched {self.0.restore_due.store(false,Ordering::Release);}
        if matched || self.view().status==ShortcutStatus::CleanupPending { self.0.wake.notify_one(); } Ok(())
    }
    pub(crate) async fn cancel(&self, id: &str) -> Result<(),String> {
        self.signal_cancel(id)?;
        let until=tokio::time::Instant::now()+Duration::from_secs(11);
        loop {
            let settled=self.0.settled.notified();tokio::pin!(settled);settled.as_mut().enable();
            if self.0.nonces.lock().map_err(|_|UNAVAILABLE)?.active.as_deref()!=Some(id) {
                if self.view().status!=ShortcutStatus::CleanupPending {return Ok(())}
            }
            tokio::time::timeout_at(until,settled).await.map_err(|_|"系统快捷键清理尚未确认")?;
        }
    }
    pub(crate) fn invalidate_settings(&self) {
        let id=self.0.nonces.lock().ok().and_then(|n|n.active.clone());
        if let Some(id)=id { let _=self.signal_cancel(&id); }
    }
    pub(crate) fn cancellation_observer(&self)->impl Fn()+Clone+'static {
        let weak=Arc::downgrade(&self.0);
        move || {if let Some(shared)=weak.upgrade(){Runtime(shared).invalidate_settings();}}
    }
    pub(crate) fn stop(&self) { self.0.stop.store(true,Ordering::Release);self.0.route.store(false,Ordering::Release);self.0.wake.notify_one(); }
    pub(crate) async fn drain(&self, timeout: Duration) -> Result<(), String> {
        self.stop();let until=tokio::time::Instant::now()+timeout;
        loop { let notified=self.0.wake.notified();tokio::pin!(notified);notified.as_mut().enable();
            if self.0.completed.load(Ordering::Acquire){return Ok(())}
            tokio::time::timeout_at(until,notified).await.map_err(|_|"系统快捷键尚未释放，已取消退出")?;
        }
    }
    pub(crate) async fn configure(&self, revision:u64, id:String, owner:usize, parent:crate::capture_shortcut_wayland_host::ExportedParent) -> Result<CaptureShortcutState,String> {
        if !self.0.host_allowed(Some(owner)) {return Err("当前无法设置快捷键".into())}
        self.0.nonces.lock().map_err(|_|UNAVAILABLE)?.begin(&id)?;
        let (reply,receive)=oneshot::channel();
        if self.0.sender.try_send(Command::Configure{revision,id:id.clone(),owner,parent,reply}).is_err() {
            self.0.nonces.lock().unwrap_or_else(|e|e.into_inner()).end(&id);return Err("快捷键设置正在处理".into());
        }
        struct Cancel(Runtime,String);impl Drop for Cancel{fn drop(&mut self){let _=self.0.signal_cancel(&self.1);}}
        let guard=Cancel(self.clone(),id);let result=receive.await.map_err(|_|UNAVAILABLE.to_string())?;drop(guard);result
    }
    pub(crate) async fn disable(&self, revision:u64, owner:usize) -> Result<CaptureShortcutState,String> {
        if !self.0.host_allowed(Some(owner)){return Err("当前无法设置快捷键".into())}
        self.invalidate_settings(); self.0.restore_due.store(false,Ordering::Release); self.0.route.store(false,Ordering::Release);
        let (reply,receive)=oneshot::channel();self.0.sender.try_send(Command::Disable{revision,owner,reply}).map_err(|_|"快捷键设置正在处理")?;
        receive.await.map_err(|_|UNAVAILABLE.to_string())?
    }
}
pub(crate) fn restore_after_ready(app:&AppHandle) { if let Some(r)=app.try_state::<Runtime>(){r.restore_once()} }

async fn checked<T,E>(future:impl Future<Output=Result<T,E>>, until:tokio::time::Instant, allowed:&impl Fn()->bool) -> Result<T,String> {
    tokio::pin!(future);
    loop { if !allowed(){return Err("快捷键设置已取消".into())}
        if tokio::time::Instant::now()>=until{return Err("系统快捷键授权超时".into())}
        tokio::select!{biased;result=&mut future=>{if !allowed(){return Err("快捷键设置已取消".into())}if tokio::time::Instant::now()>=until{return Err("系统快捷键授权超时".into())}return result.map_err(|_|UNAVAILABLE.into())},
            _=tokio::time::sleep_until(until)=>return Err("系统快捷键授权超时".into()),
            _=tokio::time::sleep(Duration::from_millis(20))=>{}}
    }
}
async fn transport<T,E>(future:impl Future<Output=Result<T,E>>, until:tokio::time::Instant, allowed:&impl Fn()->bool)->Result<T,String>{checked(future,until.min(tokio::time::Instant::now()+METHOD),allowed).await}

async fn registered_connection(address:zbus::Address,until:tokio::time::Instant,allowed:&impl Fn()->bool)->Result<(Connection,String),String>{
        if !matches!(address.transport(),zbus::address::transport::Transport::Unix(_)){return Err(UNAVAILABLE.into())}
        let builder=zbus::connection::Builder::address(address).map_err(|_|UNAVAILABLE)?.max_queued(16).method_timeout(METHOD);
        let connection=transport(builder.build(),until,allowed).await?;
        let bus=transport(Proxy::new(&connection,"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus"),until,allowed).await?;
        let _:u32=transport(bus.call("StartServiceByName",&(SERVICE,0u32)),until,allowed).await?;
        let owner:String=transport(bus.call("GetNameOwner",&(SERVICE,)),until,allowed).await?;drop(bus);
        if !owner.starts_with(':'){return Err(UNAVAILABLE.into())}
        // Registry MUST be first on this connection, before properties/Create.
        let registry=transport(Proxy::new(&connection,owner.as_str(),DESKTOP,"org.freedesktop.host.portal.Registry"),until,allowed).await?;
        let _:()=transport(registry.call("Register",&(crate::capture_shortcut_desktop_linux::APP_ID,Options::new())),until,allowed).await?;drop(registry);
        let shortcuts=transport(Proxy::new(&connection,owner.as_str(),DESKTOP,INTERFACE),until,allowed).await?;
        let version:u32=transport(shortcuts.get_property("version"),until,allowed).await?;drop(shortcuts);
        if version<1{return Err(UNAVAILABLE.into())}
        Ok((connection,owner))
}

struct Portal { connection:Connection,owner:String,path:OwnedObjectPath,session_token:String,pending:Option<String>,live:Arc<AtomicBool>,owner_gone:Arc<AtomicBool>,closing:Arc<AtomicBool>,wake:Arc<Notify>,monitor:Option<tauri::async_runtime::JoinHandle<()>>,generation:u64 }
impl Drop for Portal {fn drop(&mut self){if let Some(monitor)=self.monitor.take(){monitor.abort();}}}
impl Portal {
    async fn connect(shared:&Arc<Shared>,until:tokio::time::Instant,allowed:&impl Fn()->bool)->Result<Self,String>{
        let address=zbus::Address::session().map_err(|_|UNAVAILABLE)?;
        let (connection,owner)=registered_connection(address,until,allowed).await?;
        let generation=shared.generation.fetch_update(Ordering::AcqRel,Ordering::Acquire,|v|v.checked_add(1)).map_err(|_|UNAVAILABLE)?+1;
        let session_token=token();let sender=connection.unique_name().ok_or(UNAVAILABLE)?.as_str().trim_start_matches(':').replace('.',"_");
        let path=OwnedObjectPath::try_from(format!("/org/freedesktop/portal/desktop/session/{sender}/{session_token}")).map_err(|_|UNAVAILABLE)?;
        let mut portal=Self{connection,owner,path,session_token,pending:None,live:Arc::new(AtomicBool::new(true)),owner_gone:Arc::new(AtomicBool::new(false)),closing:Arc::new(AtomicBool::new(false)),wake:shared.wake.clone(),monitor:None,generation};
        // Owner observation is established before permission calls; exact
        // session/global subscriptions exist before Bind and its early signals.
        portal.start_monitor(shared,until,allowed).await?;
        Ok(portal)
    }
    async fn create(&mut self,until:tokio::time::Instant,allowed:&impl Fn()->bool)->Result<(),String>{
        let until=until.min(tokio::time::Instant::now()+METHOD);
        let handle=token();let session_token=self.session_token.clone();
        let mut options=Options::new();options.insert("handle_token",Value::from(handle.as_str()));options.insert("session_handle_token",Value::from(session_token.as_str()));
        let result=self.request("CreateSession",&handle,&(options,),until,allowed).await?;
        if result.get("session_handle").and_then(|v|<&str>::try_from(v).ok())!=Some(self.path.as_str()){return Err("系统快捷键会话身份不匹配".into())}Ok(())
    }
    async fn start_monitor(&mut self,shared:&Arc<Shared>,until:tokio::time::Instant,allowed:&impl Fn()->bool)->Result<(),String>{
        let bus=transport(Proxy::new_owned(self.connection.clone(),"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus"),until,allowed).await?;
        let mut owners=transport(bus.receive_signal_with_args("NameOwnerChanged",&[(0,SERVICE)]),until,allowed).await?;
        let globals=transport(Proxy::new_owned(self.connection.clone(),self.owner.clone(),DESKTOP,INTERFACE),until,allowed).await?;
        let mut activated=transport(globals.receive_signal("Activated"),until,allowed).await?;
        let mut changed=transport(globals.receive_signal("ShortcutsChanged"),until,allowed).await?;
        let session=transport(Proxy::new_owned(self.connection.clone(),self.owner.clone(),self.path.clone(),SESSION),until,allowed).await?;
        let mut closed=transport(session.receive_signal("Closed"),until,allowed).await?;
        // Close the subscribe/GetNameOwner race without touching portal methods.
        let current:String=transport(bus.call("GetNameOwner",&(SERVICE,)),until,allowed).await?;
        if current!=self.owner {return Err(UNAVAILABLE.into())}
        let shared=shared.clone();let owner=self.owner.clone();let path=self.path.clone();let live=self.live.clone();let owner_gone=self.owner_gone.clone();let closing=self.closing.clone();let generation=self.generation;
        self.monitor=Some(tauri::async_runtime::spawn(async move {
            loop {tokio::select!{biased;
                signal=owners.next()=> {let Some(signal)=signal else{break};if signal.body().deserialize::<(String,String,String)>().is_ok_and(|(name,old,new)|name==SERVICE&&old==owner&&new!=owner){owner_gone.store(true,Ordering::Release);break}},
                _=closed.next()=>break,
                signal=activated.next()=>{
                    let Some(signal)=signal else{break};
                    if signal.body().deserialize::<(OwnedObjectPath,String,u64,Results)>().is_ok_and(|(session,id,_,_)|session==path&&id==ACTION)
                        && shared.generation.load(Ordering::Acquire)==generation && shared.route.load(Ordering::Acquire) && live.load(Ordering::Acquire)
                        && shared.host_allowed(None) {crate::start_capture(&shared.app)}
                },
                signal=changed.next()=>{
                    let Some(signal)=signal else{break};
                    if let Ok((session,shortcuts))=signal.body().deserialize::<(OwnedObjectPath,Shortcuts)>() {
                        if session==path&&shared.generation.load(Ordering::Acquire)==generation&&shared.route.load(Ordering::Acquire){
                            match label(shortcuts){Ok(label)=>shared.publish(ShortcutStatus::Active,Some(label),false,None),Err(_)=>{shared.route.store(false,Ordering::Release);shared.publish(ShortcutStatus::NeedsSetup,None,false,None)}}
                        }
                    }
                }
            }}
            live.store(false,Ordering::Release);
            if shared.generation.load(Ordering::Acquire)==generation && !closing.load(Ordering::Acquire) {shared.restore_due.store(true,Ordering::Release);shared.route.store(false,Ordering::Release);shared.publish(ShortcutStatus::NeedsSetup,None,false,Some("系统快捷键连接已结束".into()));shared.wake.notify_one();}
        }));Ok(())
    }
    async fn request<B:serde::Serialize+zbus::zvariant::DynamicType>(&mut self,method:&str,handle:&str,body:&B,until:tokio::time::Instant,allowed:&impl Fn()->bool)->Result<Results,String>{
        let sender=self.connection.unique_name().ok_or(UNAVAILABLE)?.as_str().trim_start_matches(':').replace('.',"_");
        let path=format!("/org/freedesktop/portal/desktop/request/{sender}/{handle}");
        let proxy=transport(Proxy::new(&self.connection,self.owner.as_str(),path.as_str(),REQUEST),until,allowed).await?;
        let mut responses=transport(proxy.receive_signal("Response"),until,allowed).await?;
        self.pending=Some(path.clone());
        let live=self.live.clone();let admitted=||allowed()&&live.load(Ordering::Acquire);
        let desktop=transport(Proxy::new(&self.connection,self.owner.as_str(),DESKTOP,INTERFACE),until,&admitted).await?;
        let returned:OwnedObjectPath=transport(desktop.call(method,body),until,&admitted).await?;
        if returned.as_str()!=path{return Err("系统快捷键请求身份不匹配".into())}
        let wake=self.wake.clone();
        let signal=checked(async {
            loop {tokio::select! {biased;
                _=wake.notified()=>{if !admitted(){return Err(UNAVAILABLE)}},
                response=responses.next()=>return response.ok_or(UNAVAILABLE),
            }}
        },until,&admitted).await?;
        if signal.data().bytes().len()>16*1024{return Err("系统快捷键响应过大".into())}
        let (code,result):(u32,Results)=signal.body().deserialize().map_err(|_|UNAVAILABLE)?;self.pending=None;
        if code==0{Ok(result)}else if code==1{Err("快捷键设置已取消".into())}else{Err(UNAVAILABLE.into())}
    }
    async fn list(&mut self,until:tokio::time::Instant,allowed:&impl Fn()->bool)->Result<Option<String>,String>{
        let until=until.min(tokio::time::Instant::now()+METHOD);
        let handle=token();let mut options=Options::new();options.insert("handle_token",Value::from(handle.as_str()));let path=self.path.clone();
        let mut results=self.request("ListShortcuts",&handle,&(path,options),until,allowed).await?;
        let shortcuts:Shortcuts=results.remove("shortcuts").ok_or(UNAVAILABLE)?.try_into().map_err(|_|UNAVAILABLE)?;
        if shortcuts.is_empty(){Ok(None)}else{label(shortcuts).map(Some)}
    }
    async fn bind(&mut self,parent:&str,until:tokio::time::Instant,allowed:&impl Fn()->bool)->Result<String,String>{
        let handle=token();let mut options=Options::new();options.insert("handle_token",Value::from(handle.as_str()));
        let mut shortcut=Options::new();shortcut.insert("description",Value::from("截图"));shortcut.insert("preferred_trigger",Value::from("ALT+SHIFT+s"));
        let path=self.path.clone();let mut result=self.request("BindShortcuts",&handle,&(path,vec![(ACTION,shortcut)],parent,options),until,allowed).await?;
        label(result.remove("shortcuts").ok_or(UNAVAILABLE)?.try_into().map_err(|_|UNAVAILABLE)?)
    }
    async fn close(&mut self)->bool {
        // GNOME GlobalShortcuts v1 cancellation only marks its pending bind
        // canceled. Unlike ScreenCast, Request.Close must precede Session.Close.
        self.closing.store(true,Ordering::Release);
        if self.owner_gone.load(Ordering::Acquire){return true}
        let until=tokio::time::Instant::now()+METHOD;
        if let Some(path)=self.pending.as_ref(){if !close(&self.connection,&self.owner,path,REQUEST,until).await{return false}self.pending=None;}
        close(&self.connection,&self.owner,self.path.as_str(),SESSION,until).await
    }
}
async fn close(connection:&Connection,owner:&str,path:&str,interface:&str,until:tokio::time::Instant)->bool {
    let result=tokio::time::timeout_at(until,async{let p=Proxy::new(connection,owner,path,interface).await?;p.call::<_,_,()>("Close",&()).await}).await;
    matches!(result,Ok(Ok(()))) || matches!(result,Ok(Err(zbus::Error::MethodError(ref name,_,_))) if matches!(name.as_str(),"org.freedesktop.DBus.Error.UnknownObject"))
}
fn label(shortcuts:Shortcuts)->Result<String,String>{
    if shortcuts.len()!=1{return Err("系统未分配截图快捷键".into())}
    let (id,properties)=shortcuts.into_iter().next().unwrap();if id!=ACTION{return Err("系统快捷键身份不匹配".into())}
    let label=properties.get("trigger_description").and_then(|v|<&str>::try_from(v).ok()).ok_or(UNAVAILABLE)?;
    if label.trim().is_empty()||label.len()>512||label.chars().any(char::is_control){return Err("系统未分配截图快捷键".into())}
    Ok(label.into())
}
async fn actor(shared:Arc<Shared>,mut receiver:mpsc::Receiver<Command>,file:Option<PreferencesFile>,mut loaded:Option<LoadedPreferences>){
    let mut session:Option<Portal>=None;
    loop {
        if shared.stop.load(Ordering::Acquire){
            shared.route.store(false,Ordering::Release);
            if let Some(p)=session.as_mut(){if !p.close().await{shared.publish(ShortcutStatus::CleanupPending,None,false,Some("系统快捷键尚未释放".into()));shared.stop.store(false,Ordering::Release);continue}}
            shared.completed.store(true,Ordering::Release);shared.wake.notify_waiters();return;
        }
        let command=tokio::select!{biased;_=shared.wake.notified()=>{
            if shared.view.lock().unwrap_or_else(|e|e.into_inner()).status==ShortcutStatus::CleanupPending || session.as_ref().is_some_and(|p|!p.live.load(Ordering::Acquire)) {
                let clean=match session.as_mut(){Some(p)=>p.close().await,None=>true};
                if clean {
                    session=None;shared.publish(ShortcutStatus::NeedsSetup,None,false,None);shared.settled.notify_waiters();
                    if shared.startup.load(Ordering::Acquire) && loaded.as_ref().is_some_and(|l|l.preferences.wayland_shortcut_enabled) {
                        if shared.restore_due.load(Ordering::Acquire) && shared.host_allowed(None) {Runtime(shared.clone()).restore_once();}
                    }
                } else {shared.publish(ShortcutStatus::CleanupPending,None,false,Some("系统快捷键清理尚未确认".into()));}
            }
            continue;
        },command=receiver.recv()=>{let Some(command)=command else{shared.stop.store(true,Ordering::Release);continue};command}};
        match command {
            Command::Configure{revision,id,owner,parent,reply}=>{
                let allowed=||shared.config_allowed(&id,owner)&&!reply.is_closed();
                let existing=session.is_some();
                let result:Result<CaptureShortcutState,String>=async{
                    let prefs=loaded.as_ref().ok_or(UNAVAILABLE)?;
                    if revision!=prefs.preferences.revision{return Err("快捷键设置已改变，请载入最新设置".into())}
                    if session.is_some(){return Err("请通过系统键盘设置修改快捷键".into())}
                    if !allowed(){return Err("快捷键设置已取消".into())}
                    shared.view.lock().unwrap_or_else(|e|e.into_inner()).configuration_request_id=Some(id.clone());
                    shared.publish(ShortcutStatus::NeedsSetup,None,true,None);
                    crate::capture_shortcut_desktop_linux::ensure(&shared.root,true,&allowed)?;
                    let until=tokio::time::Instant::now()+CONSENT;
                    session=Some(Portal::connect(&shared,until,&allowed).await?);
                    session.as_mut().unwrap().create(until,&allowed).await?;
                    let label=session.as_mut().unwrap().bind(parent.identifier(),until,&allowed).await?;
                    let mut next=prefs.preferences.clone();next.revision=next.revision.checked_add(1).ok_or(UNAVAILABLE)?;next.wayland_shortcut_enabled=true;
                    let live=session.as_ref().unwrap().live.clone();let commit_allowed=||allowed()&&live.load(Ordering::Acquire);
                    let receipt=file.as_ref().ok_or(UNAVAILABLE)?.commit(&prefs.stamp,&next,&commit_allowed).map_err(|e|e.to_string())?;
                    loaded=Some(LoadedPreferences{preferences:next.clone(),stamp:receipt.stamp});
                    shared.restore_due.store(false,Ordering::Release);
                    { let mut state=shared.view.lock().unwrap_or_else(|e|e.into_inner());state.revision=next.revision;state.portal_enabled=Some(true); }
                    // Durable receipt is the commit boundary. A canceled/closed
                    // owner cannot arm routing after it, even if disk committed.
                    let arm=commit_allowed();shared.route.store(arm,Ordering::Release);
                    shared.publish(if arm{ShortcutStatus::Active}else{ShortcutStatus::NeedsSetup},arm.then_some(label),false,receipt.durability_warning.then(||"设置已保存，存储同步状态未确认".into()));
                    Ok(shared.view.lock().unwrap_or_else(|e|e.into_inner()).clone())
                }.await;
                if !existing&&(result.is_err()||!shared.route.load(Ordering::Acquire)){
                    let clean=match session.as_mut(){Some(p)=>p.close().await,None=>true};
                    if clean{session=None;shared.publish(ShortcutStatus::NeedsSetup,None,false,result.as_ref().err().filter(|e|e.as_str()!="快捷键设置已取消").cloned())}else{shared.publish(ShortcutStatus::CleanupPending,None,false,Some("系统快捷键清理尚未确认".into()));}
                }
                shared.nonces.lock().unwrap_or_else(|e|e.into_inner()).end(&id);shared.settled.notify_waiters();let _=reply.send(result);
            }
            Command::Disable{revision,owner,reply}=>{
                let result:Result<(),String>=(||{
                    let prefs=loaded.as_ref().ok_or(UNAVAILABLE)?;if prefs.preferences.revision!=revision{return Err("快捷键设置已改变，请载入最新设置".into())}
                    let mut next=prefs.preferences.clone();next.revision=next.revision.checked_add(1).ok_or(UNAVAILABLE)?;next.wayland_shortcut_enabled=false;next.capture_shortcut=None;
                    let receipt=file.as_ref().ok_or(UNAVAILABLE)?.commit(&prefs.stamp,&next,||shared.host_allowed(Some(owner))&&!reply.is_closed()).map_err(|e|e.to_string())?;
                    loaded=Some(LoadedPreferences{preferences:next.clone(),stamp:receipt.stamp});let mut view=shared.view.lock().unwrap_or_else(|e|e.into_inner());view.revision=next.revision;view.configured=None;view.portal_enabled=Some(false);Ok(())
                })();
                shared.route.store(false,Ordering::Release);
                let clean=match session.as_mut(){Some(p)=>p.close().await,None=>true};
                if clean{session=None;shared.publish(if result.is_ok(){ShortcutStatus::Disabled}else{ShortcutStatus::NeedsSetup},None,false,result.as_ref().err().cloned());}
                else{shared.publish(ShortcutStatus::CleanupPending,None,false,Some("系统快捷键清理尚未确认".into()));}
                let _=reply.send(result.map(|_|shared.view.lock().unwrap_or_else(|e|e.into_inner()).clone()));
            }
            Command::Restore=>{
                if session.is_some()||!loaded.as_ref().is_some_and(|p|p.preferences.wayland_shortcut_enabled){continue}
                if !shared.host_allowed(None){shared.restore_due.store(true,Ordering::Release);continue}
                let id=uuid::Uuid::new_v4().to_string();
                if shared.nonces.lock().unwrap_or_else(|e|e.into_inner()).begin(&id).is_err(){continue}
                let allowed=||shared.host_allowed(None)&&shared.nonces.lock().is_ok_and(|n|n.allowed(&id));
                shared.view.lock().unwrap_or_else(|e|e.into_inner()).configuration_request_id=Some(id.clone());
                let result=async{
                    crate::capture_shortcut_desktop_linux::ensure(&shared.root,false,&allowed)?;
                    let until=tokio::time::Instant::now()+CONSENT;
                    shared.publish(ShortcutStatus::NeedsSetup,None,true,None);
                    session=Some(Portal::connect(&shared,until,&allowed).await?);
                    session.as_mut().unwrap().create(until,&allowed).await?;
                    let _=session.as_mut().unwrap().list(until,&allowed).await?;
                    // Receipt proves a prior explicit user acceptance; the new
                    // session reuses capture, never synthetic capture_v2 IDs.
                    let label=session.as_mut().unwrap().bind("",until,&allowed).await?;
                    if !allowed(){return Err("快捷键恢复已取消".into())}
                    shared.route.store(true,Ordering::Release);shared.publish(ShortcutStatus::Active,Some(label),false,None);Ok::<(),String>(())
                }.await;
                if let Err(error)=result{let clean=match session.as_mut(){Some(p)=>p.close().await,None=>true};if clean{session=None;shared.publish(ShortcutStatus::NeedsSetup,None,false,Some(error))}else{shared.publish(ShortcutStatus::CleanupPending,None,false,Some("系统快捷键清理尚未确认".into()))}}
                shared.nonces.lock().unwrap_or_else(|e|e.into_inner()).end(&id);
                shared.settled.notify_waiters();
            }
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test]fn cancel_before_configure_and_old_cancel_cannot_resurrect_or_clear_new(){let mut n=Nonces::default();let old=uuid::Uuid::new_v4().to_string();let new=uuid::Uuid::new_v4().to_string();n.cancel(&old).unwrap();assert!(n.begin(&old).is_err());n.begin(&new).unwrap();assert!(!n.cancel(&old).unwrap());assert!(n.allowed(&new));assert!(n.cancel(&new).unwrap());assert!(!n.allowed(&new));}
    #[test]fn active_requires_actual_exact_binding_not_preferred_hint(){let mut p=Results::new();p.insert("preferred_trigger".into(),OwnedValue::from(zbus::zvariant::Str::from("ALT+SHIFT+s")));assert!(label(vec![(ACTION.into(),p.clone())]).is_err());p.insert("trigger_description".into(),OwnedValue::from(zbus::zvariant::Str::from("Alt + Shift + S")));assert_eq!(label(vec![(ACTION.into(),p.clone())]).unwrap(),"Alt + Shift + S");assert!(label(vec![("capture_v2".into(),p)]).is_err());}
    #[test]fn tombstones_are_bounded_without_early_cancel_eviction(){let mut n=Nonces::default();for _ in 0..512{n.cancel(&uuid::Uuid::new_v4().to_string()).unwrap();}assert!(n.begin(&uuid::Uuid::new_v4().to_string()).is_err());assert!(n.cancel(&uuid::Uuid::new_v4().to_string()).is_err());}
}

#[cfg(test)]
#[path = "capture_shortcut_portal_tests_linux.rs"]
mod protocol_tests;
