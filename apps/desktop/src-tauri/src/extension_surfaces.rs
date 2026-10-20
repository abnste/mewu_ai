// SPDX-License-Identifier: MPL-2.0
//! Ephemeral plugin client documents. Durable state belongs to plugin storage.
use std::{collections::HashMap, sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}}};
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
struct Document { owner: Owner, html: Vec<u8>,generation:Arc<Generation>,width:u32,height:u32,window:Option<bool>,prepared:bool }
#[derive(Default)]
pub(crate) struct Surfaces(Mutex<HashMap<String, Document>>);
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct DocumentInput { pub html: String, pub width: u32, pub height: u32 }
#[derive(Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct Mounted { pub surface_id: String, pub url: String, pub width: u32, pub height: u32 }
impl Surfaces {
    #[cfg(test)]
    pub(crate) fn test_html(&self,id:&str)->String {
        String::from_utf8(self.0.lock().unwrap().get(id).unwrap().html.clone()).unwrap()
    }
    pub(crate) fn mount(&self, owner: Owner, document: DocumentInput, origin: &str) -> Result<Mounted,String> {
        if document.html.len()>256*1024 || document.html.is_empty()
            || !(80..=1600).contains(&document.width) || !(40..=1200).contains(&document.height) {
            return Err("插件界面尺寸或内容无效".into());
        }
        let mut entries=self.0.lock().map_err(|_|"插件界面不可用")?;
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
        let html=format!("<!doctype html><meta charset=\"utf-8\"><script>{}</script>{}",
            include_str!("../../../../packages/sdk/client.js"),document.html).into_bytes();
        entries.insert(id.clone(),Document{owner,html,generation:Arc::new(Generation{live:AtomicBool::new(true),commit:Mutex::new(() )}),width:document.width,height:document.height,window:None,prepared:false});
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
    let host=app.state::<crate::Host>();
    let engine=host.lock().ok()?;
    let scene=engine.store.active_scene().ok()?;
    let settings=document.owner.scene.is_empty() && record.services.iter().any(|service|
        service.id==document.owner.service && crate::extension_objects_host::is_settings_surface(service));
    if !settings && document.window.is_none()&&(scene.id!=document.owner.scene || scene.closed) {return None;}
    Some(document.html.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(plugin:&str)->Owner {Owner{plugin:plugin.into(),revision:1,service:"view".into(),scene:"scene".into()}}
    fn document()->DocumentInput {DocumentInput{html:"<button>Test</button>".into(),width:300,height:200}}
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
