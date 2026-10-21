// SPDX-License-Identifier: MPL-2.0
//! Ephemeral plugin client documents. Durable state belongs to plugin storage.
use std::{collections::HashMap, sync::{Arc,Mutex,atomic::{AtomicBool,AtomicU64,Ordering}}};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct Owner {
    pub plugin: String,
    pub revision: u64,
    pub service: String,
    pub scene: String,
}
struct Generation { live:AtomicBool,commit:Mutex<()> }
#[derive(Clone,Debug)]
pub(crate) struct WindowTicket {pub label:String,epoch:u64}
struct Document { owner: Owner, html: Vec<u8>,generation:Arc<Generation>,width:u32,height:u32,window:Option<bool>,prepared:bool,host_window:Option<WindowTicket> }
#[derive(Default)]
pub(crate) struct Surfaces(Mutex<HashMap<String, Document>>,AtomicU64,AtomicU64);
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct DocumentInput { pub html: String, pub width: u32, pub height: u32 }
#[derive(Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct Mounted { pub surface_id: String, pub url: String, pub width: u32, pub height: u32 }
impl Surfaces {
    fn epoch(&self,label:&str)->Option<&AtomicU64>{match label{"settings"=>Some(&self.1),"space"=>Some(&self.2),_=>None}}
    pub(crate) fn ticket(&self,label:&str)->Option<WindowTicket>{Some(WindowTicket{label:label.into(),epoch:self.epoch(label)?.load(Ordering::Acquire)})}
    /// WebKit also sends iframe loads through the window navigation delegate.
    /// Admit only a live document mounted by this exact host window generation,
    /// never the content server as a whole (which also serves private media).
    pub(crate) fn allows_window_document(&self,url:&tauri::Url,origin:&str,label:&str)->bool {
        self.0.lock().ok().is_some_and(|entries|entries.iter().any(|(id,document)|
            document.window.is_none() && document.generation.live.load(Ordering::SeqCst)
                && document.host_window.as_ref().is_some_and(|ticket|ticket.label==label&&self.window_current(ticket))
                && url.as_str()==format!("{origin}/{id}")))
    }
    pub(crate) fn window_current(&self,ticket:&WindowTicket)->bool{self.epoch(&ticket.label).is_some_and(|epoch|epoch.load(Ordering::Acquire)==ticket.epoch)}
    pub(crate) fn revoke_window(&self,label:&str){
        if let Some(epoch)=self.epoch(label){if let Ok(mut entries)=self.0.lock(){
            epoch.fetch_add(1,Ordering::AcqRel);
            entries.retain(|_,document|{
                let revoke=document.window.is_none()&&document.host_window.as_ref().is_some_and(|ticket|ticket.label==label);
                if revoke{document.generation.live.store(false,Ordering::SeqCst);}!revoke
            });
        }}
    }
    #[cfg(test)]
    pub(crate) fn test_html(&self,id:&str)->String {
        String::from_utf8(self.0.lock().unwrap().get(id).unwrap().html.clone()).unwrap()
    }
    pub(crate) fn mount(&self, owner: Owner, document: DocumentInput, origin: &str) -> Result<Mounted,String> {
        self.mount_scoped(owner,document,origin,None)
    }
    pub(crate) fn mount_scoped(&self, owner: Owner, document: DocumentInput, origin: &str,host_window:Option<&WindowTicket>) -> Result<Mounted,String> {
        if document.html.len()>256*1024 || document.html.is_empty()
            || !(80..=1600).contains(&document.width) || !(40..=1200).contains(&document.height) {
            return Err("插件界面尺寸或内容无效".into());
        }
        let mut entries=self.0.lock().map_err(|_|"插件界面不可用")?;
        if host_window.is_some_and(|ticket|!self.window_current(ticket)){return Err("插件宿主窗口已关闭".into());}
        // A detached object keeps its client and unsaved editor state when the
        // space toolbar is recreated. Never replace a live pinned document.
        if let Some((id,d))=entries.iter().find(|(_,d)|d.owner==owner&&d.window.is_some()&&d.generation.live.load(Ordering::SeqCst)){
            return Ok(Mounted{surface_id:id.clone(),url:format!("{origin}/{id}"),width:d.width,height:d.height});
        }
        // Reopening replaces exactly this service and scene's prior generation.
        entries.retain(|_,v|{
            if v.owner==owner {v.generation.live.store(false,Ordering::SeqCst);false}else{true}
        });
        if entries.len()>=32 {return Err("插件界面数量已达上限".into());}
        let id=uuid::Uuid::new_v4().to_string();
        let html=format!("<!doctype html><meta charset=\"utf-8\"><script>{}\n{}</script>{}",
            include_str!("../../../../packages/sdk/document-runtime.js"),include_str!("../../../../packages/sdk/client.js"),document.html).into_bytes();
        entries.insert(id.clone(),Document{owner,html,generation:Arc::new(Generation{live:AtomicBool::new(true),commit:Mutex::new(() )}),width:document.width,height:document.height,window:None,prepared:false,host_window:host_window.cloned()});
        Ok(Mounted{url:format!("{origin}/{id}"),surface_id:id,width:document.width,height:document.height})
    }
    pub(crate) fn window_owner(&self,id:&str)->Option<Owner>{
        self.0.lock().ok()?.get(id).filter(|d|d.window.is_some()&&d.generation.live.load(Ordering::SeqCst)).map(|d|d.owner.clone())
    }
    pub(crate) fn mark_window_prepared(&self,id:&str)->Result<(),String>{
        let mut entries=self.0.lock().map_err(|_|"插件界面不可用")?;
        let d=entries.get_mut(id).filter(|d|d.window.is_some()&&d.generation.live.load(Ordering::SeqCst)).ok_or("插件对象已关闭")?;
        d.prepared=true;Ok(())
    }
    pub(crate) fn window_prepared(&self,id:&str)->bool{
        self.0.lock().ok().is_some_and(|e|e.get(id).is_some_and(|d|d.prepared&&d.window.is_some()&&d.generation.live.load(Ordering::SeqCst)))
    }
    pub(crate) fn window_state(&self,id:&str,origin:&str)->Option<serde_json::Value>{
        let entries=self.0.lock().ok()?;let d=entries.get(id)?;
        Some(serde_json::json!({"owner":d.owner,"surfaceId":id,"url":format!("{origin}/{id}"),"width":d.width,"height":d.height,"pinned":d.window?}))
    }
    pub(crate) fn attach_window(&self,id:&str)->Result<Owner,String>{
        let mut entries=self.0.lock().map_err(|_|"插件界面不可用")?;let d=entries.get_mut(id).ok_or("插件界面已关闭")?;
        if d.window.is_some(){return Err("插件对象已打开".into());}
        d.window=Some(false);Ok(d.owner.clone())
    }
    pub(crate) fn pin_window(&self,id:&str,pinned:bool)->Result<(),String>{
        let mut entries=self.0.lock().map_err(|_|"插件界面不可用")?;let d=entries.get_mut(id).ok_or("插件界面已关闭")?;
        if d.window.is_none(){return Err("此界面不是独立对象".into());}
        d.window=Some(pinned);Ok(())
    }
    pub(crate) fn pinned(&self,id:&str)->bool{self.0.lock().ok().and_then(|v|v.get(id).and_then(|d|d.window)).unwrap_or(false)}
    pub(crate) fn check(&self,id:&str,owner:&Owner)->Result<(),String> {
        if self.0.lock().map_err(|_|"插件界面不可用")?.get(id).is_some_and(|v|&v.owner==owner&&v.generation.live.load(Ordering::SeqCst)) {Ok(())}
        else {Err("插件界面已关闭或过期".into())}
    }
    /// Serialize the final local commit with close/revoke, rather than checking
    /// and releasing the generation lock before an irreversible write.
    pub(crate) fn with_owner<T>(&self,id:&str,owner:&Owner,operation:impl FnOnce()->Result<T,String>)->Result<T,String> {
        let generation={
            let entries=self.0.lock().map_err(|_|"插件界面不可用")?;
            entries.get(id).filter(|v|&v.owner==owner).map(|v|v.generation.clone()).ok_or("插件界面已关闭或过期")?
        };
        let _commit=generation.commit.lock().map_err(|_|"插件界面不可用")?;
        if !generation.live.load(Ordering::SeqCst){return Err("插件界面已关闭或过期".into());}
        operation()
    }
    pub(crate) fn close(&self,id:&str,owner:&Owner)->Result<(),String> {
        let removed={
            let mut entries=self.0.lock().map_err(|_|"插件界面不可用")?;
            if entries.get(id).is_some_and(|v|&v.owner==owner){entries.remove(id)}else{None}
        };
        if let Some(document)=removed {
            document.generation.live.store(false,Ordering::SeqCst);
            // Close runs outside Engine/registry locks. Drain only this document.
            let _commit=document.generation.commit.lock().map_err(|_|"插件界面不可用")?;
        }
        Ok(())
    }
    pub(crate) fn revoke(&self,plugin:Option<&str>) {
        // Exit preparation requires short revocation only. Existing admitted
        // transactions settle with their worker; ExtensionHost::drain joins it.
        if let Ok(mut entries)=self.0.lock(){entries.retain(|_,v|{
            let keep=plugin.is_some_and(|id|id!=v.owner.plugin);
            if !keep {v.generation.live.store(false,Ordering::SeqCst);}
            keep
        });}
    }
}
/// Called only by the remote-origin read-only server. Revocation and scene
/// ownership are checked before returning bytes; knowing a URL grants no RPC.
pub(crate) fn read(app:&AppHandle,id:&str)->Option<Vec<u8>> {
    let extensions=app.try_state::<crate::extension_host::ExtensionHost>()?;
    let registry=extensions.store().ok()?;
    let entries=extensions.surfaces.0.lock().ok()?;
    let document=entries.get(id)?;
    let record=registry.get(&document.owner.plugin).ok()?;
    if record.state!="enabled" || record.revision!=document.owner.revision {return None;}
    record.ensure_supported().ok()?;
    registry.validate_dependencies(&record.manifest).ok()?;
    let settings=document.owner.scene.is_empty() && record.services.iter().any(|service|
        service.id==document.owner.service && crate::extension_objects_host::is_settings_surface(service));
    if !settings && document.window.is_none(){
        let host=app.state::<crate::Host>();let engine=host.lock().ok()?;let scene=engine.store.active_scene().ok()?;
        if scene.id!=document.owner.scene || scene.closed{return None;}
    }
    Some(document.html.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn navigation_admits_only_live_documents_of_the_exact_host_window() {
        let surfaces=Surfaces::default();
        let origin="http://127.0.0.1:32123";
        let settings=surfaces.ticket("settings").unwrap();
        let space=surfaces.ticket("space").unwrap();
        let mut settings_owner=owner("settings-plugin");settings_owner.scene.clear();
        let mounted=surfaces.mount_scoped(settings_owner.clone(),document(),origin,Some(&settings)).unwrap();
        let url=tauri::Url::parse(&mounted.url).unwrap();
        assert!(surfaces.allows_window_document(&url,origin,"settings"));
        assert!(!surfaces.allows_window_document(&url,origin,"space"));
        for denied in [format!("{}?query=1",mounted.url),format!("{}#fragment",mounted.url),
            mounted.url.replace(":32123",":32124"),format!("{origin}/{}",uuid::Uuid::new_v4()),
            format!("https://example.com/{}",mounted.surface_id)] {
            assert!(!surfaces.allows_window_document(&tauri::Url::parse(&denied).unwrap(),origin,"settings"));
        }
        let other=surfaces.mount_scoped(owner("space-plugin"),document(),origin,Some(&space)).unwrap();
        assert!(!surfaces.allows_window_document(&tauri::Url::parse(&other.url).unwrap(),origin,"settings"));
        let replacement=surfaces.mount_scoped(settings_owner.clone(),document(),origin,Some(&settings)).unwrap();
        assert!(!surfaces.allows_window_document(&url,origin,"settings"));
        let replacement_url=tauri::Url::parse(&replacement.url).unwrap();
        assert!(surfaces.allows_window_document(&replacement_url,origin,"settings"));
        surfaces.revoke_window("settings");
        assert!(!surfaces.allows_window_document(&replacement_url,origin,"settings"));
        let ticket=surfaces.ticket("settings").unwrap();
        let next=surfaces.mount_scoped(settings_owner,document(),origin,Some(&ticket)).unwrap();
        let next_url=tauri::Url::parse(&next.url).unwrap();
        surfaces.attach_window(&next.surface_id).unwrap();
        assert!(!surfaces.allows_window_document(&next_url,origin,"settings"));
    }
    use super::*;
    fn owner(plugin:&str)->Owner {Owner{plugin:plugin.into(),revision:1,service:"view".into(),scene:"scene".into()}}
    fn document()->DocumentInput {DocumentInput{html:"<button>Test</button>".into(),width:300,height:200}}
    #[test]
    fn native_window_close_revokes_only_its_documents_and_rejects_late_mounts_after_recreation(){
        let surfaces=Surfaces::default();let settings=surfaces.ticket("settings").unwrap();let space=surfaces.ticket("space").unwrap();
        let mut settings_owner=owner("settings");settings_owner.scene.clear();
        let a=surfaces.mount_scoped(settings_owner.clone(),document(),"http://localhost:1",Some(&settings)).unwrap();
        let b_owner=owner("space");let b=surfaces.mount_scoped(b_owner.clone(),document(),"http://localhost:1",Some(&space)).unwrap();
        let c_owner=owner("pinned");let c=surfaces.mount_scoped(c_owner.clone(),document(),"http://localhost:1",Some(&space)).unwrap();
        surfaces.attach_window(&c.surface_id).unwrap();surfaces.pin_window(&c.surface_id,true).unwrap();
        surfaces.revoke_window("settings");
        assert!(!surfaces.window_current(&settings));assert!(surfaces.window_current(&space));
        assert!(surfaces.check(&a.surface_id,&settings_owner).is_err());assert!(surfaces.check(&b.surface_id,&b_owner).is_ok());
        assert!(surfaces.with_owner(&a.surface_id,&settings_owner,||Ok(())).is_err());
        assert!(surfaces.mount_scoped(settings_owner.clone(),document(),"http://localhost:1",Some(&settings)).is_err());
        let recreated=surfaces.ticket("settings").unwrap();
        let next=surfaces.mount_scoped(settings_owner.clone(),document(),"http://localhost:1",Some(&recreated)).unwrap();
        surfaces.revoke_window("space");
        assert!(surfaces.check(&b.surface_id,&b_owner).is_err());
        assert!(surfaces.check(&c.surface_id,&c_owner).is_ok());assert!(surfaces.pinned(&c.surface_id));
        assert!(surfaces.check(&next.surface_id,&settings_owner).is_ok());
        assert!(surfaces.ticket("unknown-native-window").is_none());
    }
    #[test]
    fn pinned_windows_keep_the_document_and_storage_lease_until_explicit_close_or_revoke(){
        let surfaces=Surfaces::default();let a=owner("one");
        let first=surfaces.mount(a.clone(),document(),"http://localhost:1").unwrap();
        assert!(surfaces.window_owner(&first.surface_id).is_none());
        surfaces.attach_window(&first.surface_id).unwrap();
        surfaces.pin_window(&first.surface_id,true).unwrap();
        assert!(surfaces.pinned(&first.surface_id));
        let again=surfaces.mount(a.clone(),document(),"http://localhost:1").unwrap();
        assert_eq!(first.surface_id,again.surface_id);
        assert_eq!(surfaces.window_owner(&first.surface_id),Some(a.clone()));
        assert!(surfaces.with_owner(&first.surface_id,&owner("other"),||Ok(())).is_err());
        surfaces.pin_window(&first.surface_id,false).unwrap();
        assert!(!surfaces.pinned(&first.surface_id));
        surfaces.revoke(Some("one"));
        assert!(surfaces.window_owner(&first.surface_id).is_none());
        assert!(surfaces.with_owner(&first.surface_id,&a,||Ok(())).is_err());
    }
    #[test]
    fn generations_are_owned_replaced_and_revoked_independently() {
        let surfaces=Surfaces::default();
        let a=owner("one");let b=owner("two");
        let first=surfaces.mount(a.clone(),document(),"http://localhost:1").unwrap();
        let second=surfaces.mount(b.clone(),document(),"http://localhost:1").unwrap();
        assert!(surfaces.check(&first.surface_id,&b).is_err());
        let new=surfaces.mount(a.clone(),document(),"http://localhost:1").unwrap();
        assert!(surfaces.check(&first.surface_id,&a).is_err());
        surfaces.revoke(Some("one"));
        assert!(surfaces.check(&new.surface_id,&a).is_err());
        assert!(surfaces.check(&second.surface_id,&b).is_ok());
        surfaces.close(&second.surface_id,&a).unwrap();
        assert!(surfaces.check(&second.surface_id,&b).is_ok());
        surfaces.revoke(None);
        assert!(surfaces.check(&second.surface_id,&b).is_err());
    }
    #[test]
    fn closing_serializes_with_a_commit_and_rejects_late_writes() {
        use std::{sync::{Arc,mpsc},time::Duration};
        let surfaces=Arc::new(Surfaces::default());let owner=owner("one");
        let mounted=surfaces.mount(owner.clone(),document(),"http://localhost:1").unwrap();
        let(entered_tx,entered_rx)=mpsc::channel();let(release_tx,release_rx)=mpsc::channel();
        let writer={let surfaces=surfaces.clone();let owner=owner.clone();let id=mounted.surface_id.clone();
            std::thread::spawn(move||surfaces.with_owner(&id,&owner,||{entered_tx.send(()).unwrap();release_rx.recv().unwrap();Ok(())}))};
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let(closed_tx,closed_rx)=mpsc::channel();
        let closer={let surfaces=surfaces.clone();let owner=owner.clone();let id=mounted.surface_id.clone();
            std::thread::spawn(move||{surfaces.close(&id,&owner).unwrap();closed_tx.send(()).unwrap();})};
        assert!(closed_rx.recv_timeout(Duration::from_millis(30)).is_err());
        release_tx.send(()).unwrap();writer.join().unwrap().unwrap();
        closed_rx.recv_timeout(Duration::from_secs(2)).unwrap();closer.join().unwrap();
        let mut wrote=false;
        assert!(surfaces.with_owner(&mounted.surface_id,&owner,||{wrote=true;Ok(())}).is_err());
        assert!(!wrote);
    }
    #[test]
    fn exit_revocation_never_waits_for_a_worker_transaction() {
        use std::{sync::{Arc,mpsc},time::Duration};
        let surfaces=Arc::new(Surfaces::default());let owner=owner("one");
        let mounted=surfaces.mount(owner.clone(),document(),"http://localhost:1").unwrap();
        let(entered_tx,entered_rx)=mpsc::channel();let(release_tx,release_rx)=mpsc::channel();
        let writer={let surfaces=surfaces.clone();let owner=owner.clone();let id=mounted.surface_id.clone();
            std::thread::spawn(move||surfaces.with_owner(&id,&owner,||{entered_tx.send(()).unwrap();release_rx.recv().unwrap();Ok(())}))};
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let(done_tx,done_rx)=mpsc::channel();
        let cancel={let surfaces=surfaces.clone();std::thread::spawn(move||{surfaces.revoke(None);done_tx.send(()).unwrap();})};
        let canceled=done_rx.recv_timeout(Duration::from_secs(2));
        release_tx.send(()).unwrap();writer.join().unwrap().unwrap();cancel.join().unwrap();
        assert!(canceled.is_ok());
        assert!(surfaces.with_owner(&mounted.surface_id,&owner,||Ok(())).is_err());
    }
}
