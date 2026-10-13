// SPDX-License-Identifier: MPL-2.0
//! Real isolated D-Bus protocol gates. No actual desktop, user bus or permission UI.
use super::*;
use std::{io::BufRead, path::PathBuf, sync::atomic::AtomicUsize};

#[derive(Default)] struct State { registered:AtomicBool,binds:AtomicUsize,events:Mutex<Vec<&'static str>>,hang:bool,fail_close:bool,bind_code:u32,fail_session_close:bool }
struct Registry(Arc<State>);
#[zbus::interface(name="org.freedesktop.host.portal.Registry")]
impl Registry {
    fn register(&self, app_id:&str, _options:Results)->zbus::fdo::Result<()> {
        assert_eq!(app_id,crate::capture_shortcut_desktop_linux::APP_ID);
        assert!(!self.0.registered.swap(true,Ordering::AcqRel));
        self.0.events.lock().unwrap().push("register");Ok(())
    }
}
struct Request(Arc<State>);
#[zbus::interface(name="org.freedesktop.portal.Request")]
impl Request {
    fn close(&self)->zbus::fdo::Result<()> {
        self.0.events.lock().unwrap().push("request-close");
        if self.0.fail_close{Err(zbus::fdo::Error::Failed("fixture unconfirmed close".into()))}else{Ok(())}
    }
}
struct TestSession(Arc<State>);
#[zbus::interface(name="org.freedesktop.portal.Session")]
impl TestSession {
    fn close(&self)->zbus::fdo::Result<()>{self.0.events.lock().unwrap().push("session-close");if self.0.fail_session_close{Err(zbus::fdo::Error::Failed("fixture unconfirmed session close".into()))}else{Ok(())}}
}
struct Global(Arc<State>);
fn sender(header:&zbus::message::Header<'_>)->String{header.sender().unwrap().as_str().trim_start_matches(':').replace('.',"_")}
fn request_path(header:&zbus::message::Header<'_>,options:&Results)->OwnedObjectPath{
    let token=<&str>::try_from(options.get("handle_token").unwrap()).unwrap();
    assert!(token.starts_with("mewu_"));
    OwnedObjectPath::try_from(format!("{DESKTOP}/request/{}/{token}",sender(header))).unwrap()
}
#[zbus::interface(name="org.freedesktop.portal.GlobalShortcuts")]
impl Global {
    #[zbus(property,name="version")]
    fn version(&self)->u32{
        assert!(self.0.registered.load(Ordering::Acquire),"Registry must precede portal access");
        self.0.events.lock().unwrap().push("version");1
    }
    async fn create_session(&self,options:Results,#[zbus(connection)]connection:&Connection,#[zbus(header)]header:zbus::message::Header<'_>)->OwnedObjectPath{
        assert!(self.0.registered.load(Ordering::Acquire));
        let token=<&str>::try_from(options.get("session_handle_token").unwrap()).unwrap();
        let path=OwnedObjectPath::try_from(format!("{DESKTOP}/session/{}/{token}",sender(&header))).unwrap();
        connection.object_server().at(path.clone(),TestSession(self.0.clone())).await.unwrap();
        let request=request_path(&header,&options);
        connection.object_server().at(request.clone(),Request(self.0.clone())).await.unwrap();
        let mut result=Results::new();result.insert("session_handle".into(),OwnedValue::from(zbus::zvariant::Str::from(path.as_str())));
        connection.emit_signal(header.sender().map(|v|v.as_str()),request.clone(),REQUEST,"Response",&(0u32,result)).await.unwrap();
        self.0.events.lock().unwrap().push("create");request
    }
    async fn bind_shortcuts(&self,session:OwnedObjectPath,shortcuts:Shortcuts,parent:&str,options:Results,#[zbus(connection)]connection:&Connection,#[zbus(header)]header:zbus::message::Header<'_>)->OwnedObjectPath{
        assert!(session.as_str().contains("/session/"));assert_eq!(shortcuts.len(),1);assert_eq!(shortcuts[0].0,ACTION);
        assert_eq!(<&str>::try_from(shortcuts[0].1.get("preferred_trigger").unwrap()).unwrap(),"ALT+SHIFT+s");
        assert_eq!(parent,"wayland:fixture");
        let request=request_path(&header,&options);connection.object_server().at(request.clone(),Request(self.0.clone())).await.unwrap();
        self.0.binds.fetch_add(1,Ordering::AcqRel);self.0.events.lock().unwrap().push("bind");
        if !self.0.hang {
            let mut props=Results::new();props.insert("trigger_description".into(),OwnedValue::from(zbus::zvariant::Str::from("Alt + Shift + S")));
            let mut result=Results::new();result.insert("shortcuts".into(),OwnedValue::try_from(Value::from(vec![(ACTION,props)])).unwrap());
            // The real signal is emitted BEFORE the method reply; production
            // must already have subscribed to the exact path and owner.
            connection.emit_signal(header.sender().map(|v|v.as_str()),request.clone(),REQUEST,"Response",&(self.0.bind_code,result)).await.unwrap();
        }
        request
    }
    async fn list_shortcuts(&self,_session:OwnedObjectPath,options:Results,#[zbus(connection)]connection:&Connection,#[zbus(header)]header:zbus::message::Header<'_>)->OwnedObjectPath{
        let request=request_path(&header,&options);connection.object_server().at(request.clone(),Request(self.0.clone())).await.unwrap();
        connection.emit_signal(header.sender().map(|v|v.as_str()),request.clone(),REQUEST,"Response",&(2u32,Results::new())).await.unwrap();request
    }
}
struct Bus { child:std::process::Child,root:PathBuf }
impl Drop for Bus {fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();let _=std::fs::remove_dir_all(&self.root);}}
fn bus()->(Bus,zbus::Address){
    let root=std::env::temp_dir().join(format!("mewu-shortcut-bus-{}",uuid::Uuid::new_v4()));let services=root.join("dbus-1/services");std::fs::create_dir_all(&services).unwrap();
    std::fs::write(services.join(format!("{SERVICE}.service")),format!("[D-BUS Service]\nName={SERVICE}\nExec=/bin/false\n")).unwrap();
    let mut child=std::process::Command::new("dbus-daemon").args(["--session","--nofork","--print-address=1"]).env("XDG_DATA_HOME",&root).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().unwrap();
    let mut address=String::new();std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut address).unwrap();
    (Bus{child,root},zbus::Address::try_from(address.trim()).unwrap())
}
fn runtime()->tokio::runtime::Runtime {tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap()}
async fn open(address:zbus::Address,state:Arc<State>)->(Connection,Portal){
    let server=zbus::connection::Builder::address(address.clone()).unwrap().name(SERVICE).unwrap().serve_at(DESKTOP,Registry(state.clone())).unwrap().serve_at(DESKTOP,Global(state)).unwrap().build().await.unwrap();
    let until=tokio::time::Instant::now()+METHOD;
    let (connection,owner)=registered_connection(address,until,&||true).await.unwrap();
    let session_token=token();let sender=connection.unique_name().unwrap().as_str().trim_start_matches(':').replace('.',"_");
    let path=OwnedObjectPath::try_from(format!("{DESKTOP}/session/{sender}/{session_token}")).unwrap();
    let mut portal=Portal{connection,owner,path,session_token,pending:None,live:Arc::new(AtomicBool::new(true)),owner_gone:Arc::new(AtomicBool::new(false)),closing:Arc::new(AtomicBool::new(false)),wake:Arc::new(Notify::new()),monitor:None,generation:1};
    portal.create(until,&||true).await.unwrap();(server,portal)
}
#[test]fn registry_identity_precedes_portal_and_early_response_is_not_lost(){
    let (_bus,address)=bus();let state=Arc::new(State::default());runtime().block_on(async{
        let (server,mut portal)=open(address,state.clone()).await;
        assert_eq!(portal.bind("wayland:fixture",tokio::time::Instant::now()+METHOD,&||true).await.unwrap(),"Alt + Shift + S");
        assert!(portal.pending.is_none());assert!(portal.close().await);
        let events=state.events.lock().unwrap().clone();assert_eq!(&events[..2],&["register","version"]);assert_eq!(state.binds.load(Ordering::Acquire),1);assert_eq!(events.iter().filter(|x|**x=="request-close").count(),0);assert_eq!(events.last(),Some(&"session-close"));
        drop(portal);server.close().await.unwrap();
    });
}
fn pending(cancel:bool,fail_close:bool){
    let (_bus,address)=bus();let state=Arc::new(State{hang:true,fail_close,..Default::default()});let admitted=Arc::new(AtomicBool::new(true));runtime().block_on(async{
        let (server,mut portal)=open(address,state.clone()).await;
        let canceler=if cancel{let state=state.clone();let admitted=admitted.clone();let wake=portal.wake.clone();Some(tokio::spawn(async move{
            let until=tokio::time::Instant::now()+METHOD;
            while state.binds.load(Ordering::Acquire)==0 {assert!(tokio::time::Instant::now()<until);tokio::time::sleep(Duration::from_millis(2)).await;}
            admitted.store(false,Ordering::Release);wake.notify_one();
        }))}else{None};
        let error=portal.bind("wayland:fixture",tokio::time::Instant::now()+if cancel{METHOD}else{Duration::from_millis(90)},&||admitted.load(Ordering::Acquire)).await.unwrap_err();
        assert_eq!(error,if cancel{"快捷键设置已取消"}else{"系统快捷键授权超时"});if let Some(task)=canceler{task.await.unwrap();}
        assert_eq!(state.binds.load(Ordering::Acquire),1);assert_eq!(portal.close().await,!fail_close);
        let events=state.events.lock().unwrap().clone();assert_eq!(events.iter().filter(|x|**x=="request-close").count(),1);
        assert_eq!(events.iter().filter(|x|**x=="session-close").count(),usize::from(!fail_close));
        if fail_close{assert!(portal.pending.is_some());}else{assert_eq!(&events[events.len()-2..],&["request-close","session-close"]);}
        drop(portal);server.close().await.unwrap();
    });
}
#[test]fn pending_bind_cancel_closes_request_before_session(){pending(true,false)}
#[test]fn pending_bind_deadline_closes_request_before_session(){pending(false,false)}
#[test]fn request_close_failure_retains_session_and_does_not_claim_cleanup(){pending(true,true)}

fn unbound_state(clean:bool)->CaptureShortcutState{
    CaptureShortcutState{revision:7,sequence:1,configured:None,active:None,status:if clean{ShortcutStatus::NeedsSetup}else{ShortcutStatus::CleanupPending},editable:clean,message:if clean{None}else{Some("系统快捷键清理尚未确认".into())},backend:Some("wayland_portal"),actual_trigger_description:None,configuration_pending:Some(false),system_settings_available:Some(false),configuration_request_id:None,portal_enabled:Some(false)}
}
fn completed_nonbinding(code:u32,fail_session_close:bool){
    let (_bus,address)=bus();let state=Arc::new(State{bind_code:code,fail_session_close,..Default::default()});runtime().block_on(async{
        let (server,mut portal)=open(address,state.clone()).await;
        let error=portal.bind("wayland:fixture",tokio::time::Instant::now()+METHOD,&||true).await.unwrap_err();
        assert_eq!(error,if code==1{CANCELED}else if code==2{NOT_BOUND}else{UNAVAILABLE});
        assert!(portal.pending.is_none(),"a completed Response owns no pending Request");
        let clean=portal.close().await;assert_eq!(clean,!fail_session_close);
        let reply=configuration_reply(Err(error),unbound_state(clean));
        if code<=2&&clean{
            let reply=reply.unwrap();assert_eq!(reply.status,ShortcutStatus::NeedsSetup);assert_eq!(reply.revision,7);assert_eq!(reply.portal_enabled,Some(false));assert!(reply.message.is_none());assert!(reply.active.is_none()&&reply.actual_trigger_description.is_none());
        }else{assert_eq!(reply.unwrap_err(),if fail_session_close{"系统快捷键清理尚未确认"}else{UNAVAILABLE});}
        let events=state.events.lock().unwrap();assert_eq!(state.binds.load(Ordering::Acquire),1);assert_eq!(events.iter().filter(|x|**x=="request-close").count(),0);assert_eq!(events.iter().filter(|x|**x=="session-close").count(),1);drop(events);
        drop(portal);server.close().await.unwrap();
    });
}
#[test]fn response_one_is_quiet_unbound_only_after_confirmed_cleanup(){completed_nonbinding(1,false)}
#[test]fn gnome_response_two_is_quiet_unbound_not_service_failure(){completed_nonbinding(2,false)}
#[test]fn unconfirmed_session_cleanup_is_not_quiet_or_retryable(){completed_nonbinding(2,true)}
#[test]fn unknown_response_remains_a_real_service_error(){completed_nonbinding(3,false)}
#[test]fn list_response_two_is_not_misclassified_as_normal_bind_end(){
    let (_bus,address)=bus();let state=Arc::new(State::default());runtime().block_on(async{
        let (server,mut portal)=open(address,state.clone()).await;
        let error=portal.list(tokio::time::Instant::now()+METHOD,&||true).await.unwrap_err();assert_eq!(error,UNAVAILABLE);
        assert!(portal.pending.is_none());assert!(portal.close().await);
        assert_eq!(configuration_reply(Err(error),unbound_state(true)).unwrap_err(),UNAVAILABLE);
        assert_eq!(state.binds.load(Ordering::Acquire),0);drop(portal);server.close().await.unwrap();
    });
}
#[test]fn real_service_owner_loss_during_pending_bind_is_not_a_quiet_cancel(){
    let (_bus,address)=bus();let state=Arc::new(State{hang:true,..Default::default()});runtime().block_on(async{
        let (server,mut portal)=open(address,state.clone()).await;
        let bus=Proxy::new_owned(portal.connection.clone(),"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus").await.unwrap();
        let mut changes=bus.receive_signal_with_args("NameOwnerChanged",&[(0,SERVICE)]).await.unwrap();
        let live=portal.live.clone();let gone=portal.owner_gone.clone();let wake=portal.wake.clone();let owner=portal.owner.clone();let observed=state.clone();
        let watcher=tokio::spawn(async move{
            let until=tokio::time::Instant::now()+METHOD;
            while observed.binds.load(Ordering::Acquire)==0{assert!(tokio::time::Instant::now()<until);tokio::time::sleep(Duration::from_millis(2)).await;}
            server.close().await.unwrap();
            let signal=tokio::time::timeout_at(until,changes.next()).await.unwrap().unwrap();
            assert!(service_owner_changed(&signal.body(),&owner));gone.store(true,Ordering::Release);live.store(false,Ordering::Release);wake.notify_one();
        });
        let error=portal.bind("wayland:fixture",tokio::time::Instant::now()+METHOD,&||true).await.unwrap_err();watcher.await.unwrap();
        assert!(!portal.live.load(Ordering::Acquire));assert!(portal.owner_gone.load(Ordering::Acquire));assert_eq!(state.binds.load(Ordering::Acquire),1);
        let result=service_result(Err(error),portal.live.load(Ordering::Acquire));assert!(portal.close().await);
        assert_eq!(configuration_reply(result,unbound_state(true)).unwrap_err(),UNAVAILABLE);
    });
}
