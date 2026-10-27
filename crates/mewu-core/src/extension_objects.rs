// SPDX-License-Identifier: MPL-2.0
//! Plugin-owned state plus durable, host-generated object templates.
//! Layout, grouping, folders and trash semantics belong to the plugin.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectLibrary {
    pub revision: u64,
    pub state: serde_json::Value,
    pub objects: BTreeMap<String, ObjectTemplate>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn template()->ObjectTemplate {
        ObjectTemplate { item: SpaceItem {
            id:id(), asset:Asset{id:id(),name:"reference.png".into(),path:"/host/reference.png".into(),kind:AssetKind::Image,width:Some(120),height:Some(80),origin_x:None,origin_y:None,scale_factor:None,capture_coordinates:None},
            x:0.2,y:0.2,width:0.3,height:0.2,state:None,video_edit:None,video_annotations:None,image_annotations:None
        },board:None }
    }
    #[test]
    fn plugin_state_is_arbitrary_isolated_and_reopens_with_objects() {
        let root=std::env::temp_dir().join(format!("mewu-object-library-{}",id()));
        std::fs::create_dir(&root).unwrap();let path=root.join("store.db");
        let expected;
        {
            let mut store=Store::open(&path).unwrap();
            let object=template();let asset_id=object.item.asset.id.clone();
            expected=store.commit_objects("example.one",0,serde_json::json!({"layout":"free","folders":[{"name":"任意结构"}]}),&[],Some(("shape".into(),object))).unwrap();
            store.commit_objects("example.two",0,serde_json::json!({"counter":42,"done":false}),&[],None).unwrap();
            assert_eq!(store.object_library("example.two").objects.len(),0);
            assert_eq!(store.asset(&asset_id).unwrap().name,"reference.png");
            assert!(store.storage_assets().iter().any(|asset|asset.id==asset_id));
            assert!(!expected.catalogue().to_string().contains("/host/"));
            assert!(store.restore_object("example.two","shape",1,&store.snapshot().active_scene_id,0.2,0.2).is_err());
        }
        {
            let mut store=Store::open(&path).unwrap();
            assert_eq!(store.object_library("example.one"),expected);
            assert_eq!(store.object_library("example.two").state["counter"],42);
            let scene=store.snapshot().active_scene_id;
            store.restore_object("example.one","shape",1,&scene,0.4,0.5).unwrap();
            let restored=&store.active_scene().unwrap().items[0];
            assert_ne!(restored.id,expected.objects["shape"].item.id);
            assert_eq!(restored.x,0.4);
        }
        std::fs::remove_file(path).unwrap();
        for name in ["store.db-wal","store.db-shm"]{let p=root.join(name);if p.exists(){std::fs::remove_file(p).unwrap();}}
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn stale_and_failed_commits_do_not_change_state_or_retained_assets() {
        let mut store=Store::open_in_memory().unwrap();
        let before=store.commit_objects("example.one",0,serde_json::json!({"phase":1}),&[],Some(("one".into(),template()))).unwrap();
        assert!(store.commit_objects("example.one",0,serde_json::json!({"phase":2}),&[],None).is_err());
        assert!(store.commit_objects("example.one",1,serde_json::Value::Null,&["missing".into()],None).is_err());
        assert!(store.commit_objects("example.one",1,serde_json::Value::Null,&[],Some(("one".into(),template()))).is_err());
        assert_eq!(store.object_library("example.one"),before);
        store.db.execute_batch("CREATE TRIGGER reject_objects BEFORE UPDATE ON extension_objects BEGIN SELECT RAISE(ABORT,'test');END;").unwrap();
        assert!(store.commit_objects("example.one",1,serde_json::Value::Null,&["one".into()],None).is_err());
        assert_eq!(store.object_library("example.one"),before);
    }
    #[test]
    fn membership_and_plugin_state_change_in_one_transaction() {
        let mut store=Store::open_in_memory().unwrap();
        let old=store.commit_objects("example.one",0,serde_json::json!({"selected":"one"}),&[],Some(("one".into(),template()))).unwrap();
        let old_asset=old.objects["one"].item.asset.id.clone();
        let next=store.commit_objects("example.one",1,serde_json::json!({"selected":"two"}),&["one".into()],Some(("two".into(),template()))).unwrap();
        assert_eq!(next.revision,2);assert_eq!(next.objects.len(),1);
        assert_eq!(next.state["selected"],"two");assert!(store.asset(&old_asset).is_none());
        assert!(store.commit_objects("../foreign",0,serde_json::Value::Null,&[],None).is_err());
        assert!(store.commit_objects("example.one",2,serde_json::json!("x".repeat(256*1024)),&[],None).is_err());
        assert_eq!(store.object_library("example.one"),next);
    }
    #[test]
    fn schema_migration_preserves_document_bytes_and_missing_object_table_is_rejected() {
        let root=std::env::temp_dir().join(format!("mewu-object-schema-{}",id()));
        std::fs::create_dir(&root).unwrap();let path=root.join("store.db");
        drop(Store::open(&path).unwrap());
        let before:String;
        {
            let db=SqlConnection::open(&path).unwrap();
            before=db.query_row("SELECT payload FROM app_state WHERE id=1",[],|r|r.get(0)).unwrap();
            db.execute_batch("DROP TABLE extension_objects; PRAGMA user_version=10;").unwrap();
        }
        drop(Store::open(&path).unwrap());
        {
            let db=SqlConnection::open(&path).unwrap();
            assert_eq!(db.query_row::<u32,_,_>("PRAGMA user_version",[],|r|r.get(0)).unwrap(),DB_SCHEMA_VERSION);
            assert_eq!(db.query_row::<String,_,_>("SELECT payload FROM app_state WHERE id=1",[],|r|r.get(0)).unwrap(),before);
            db.execute_batch("DROP TABLE extension_objects;").unwrap();
        }
        assert!(Store::open(&path).is_err());
        std::fs::remove_file(path).unwrap();
        for name in ["store.db-wal","store.db-shm"]{let p=root.join(name);if p.exists(){std::fs::remove_file(p).unwrap();}}
        std::fs::remove_dir(root).unwrap();
    }
}
impl Default for ObjectLibrary {
    fn default() -> Self { Self { revision: 0, state: serde_json::Value::Null, objects: BTreeMap::new() } }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectTemplate { pub item: SpaceItem, pub board: Option<Box<Scene>> }
impl ObjectTemplate {
    fn assets(&self) -> Vec<&Asset> {
        let mut assets=vec![&self.item.asset];
        if let Some(board)=&self.board {
            assets.extend(board.background.iter());
            assets.extend(board.items.iter().map(|item|&item.asset));
            assets.extend(board.regions.iter().flat_map(|r|r.image_override.iter().chain(r.translation.iter().map(|t|&t.overlay))));
        }
        assets
    }
    fn validate(&self)->Result<()> {
        for asset in self.assets() { validate_asset(asset)?; }
        crate::video_edit::validate(&self.item)?;
        crate::video_annotations::validate_item(&self.item)?;
        crate::image_annotations::validate(&self.item)?;
        if ![self.item.x,self.item.y,self.item.width,self.item.height].iter().all(|n|n.is_finite())
            || self.item.width<=0. || self.item.height<=0. || self.item.width>8. || self.item.height>8. {
            return Err(invalid("对象尺寸无效"));
        }
        if self.board.as_ref().is_some_and(|b|!b.messages.is_empty()||b.run.is_some()||!b.draft.is_empty()) {
            return Err(invalid("对象模板不能包含会话内容"));
        }
        Ok(())
    }
}
impl ObjectLibrary {
    pub fn assets(&self)->Vec<Asset> { self.objects.values().flat_map(ObjectTemplate::assets).cloned().collect() }
    pub fn asset(&self,id:&str)->Option<&Asset> { self.objects.values().flat_map(ObjectTemplate::assets).find(|a|a.id==id) }
    /// This is display metadata, never a capability or a template accepted on input.
    pub fn catalogue(&self)->serde_json::Value {
        let objects:BTreeMap<_,_>=self.objects.iter().map(|(key,value)|(key,serde_json::json!({
            "name":value.item.asset.name,"kind":value.item.asset.kind,
            "width":value.item.asset.width,"height":value.item.asset.height,
            "board":value.board.is_some()
        }))).collect();
        serde_json::json!({"revision":self.revision,"state":self.state,"objects":objects})
    }
    fn validate(&self)->Result<()> {
        if self.objects.len()>4096 || serde_json::to_vec(&self.state)?.len()>256*1024
            || serde_json::to_vec(self)?.len()>32*1024*1024 {return Err(invalid("插件对象数据过大"));}
        for (key,value) in &self.objects { validate_key(key)?; value.validate()?; }
        Ok(())
    }
}
fn validate_key(key:&str)->Result<()> {
    if key.is_empty()||key.len()>128||!key.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b)) {
        return Err(invalid("对象命名空间或标识无效"));
    }
    Ok(())
}
pub(super) fn migrate(db:&SqlConnection)->Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS extension_objects(plugin_id TEXT PRIMARY KEY,revision INTEGER NOT NULL,payload TEXT NOT NULL)")?;
    Ok(())
}
pub(super) fn load(db:&SqlConnection)->Result<BTreeMap<String,ObjectLibrary>> {
    let mut result=BTreeMap::new();
    let mut query=db.prepare("SELECT plugin_id,revision,payload FROM extension_objects")?;
    for row in query.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))? {
        let (owner,revision,payload)=row?; validate_key(&owner)?;
        let value:ObjectLibrary=serde_json::from_str(&payload)?;
        if u64::try_from(revision).ok()!=Some(value.revision) {return Err(invalid("对象版本无效"));}
        value.validate()?; result.insert(owner,value);
    }
    Ok(result)
}
fn detached(item: &mut SpaceItem) {
    if let Some(state) = &mut item.state {
        for key in [
            "coordinateSpace",
            "backgroundId",
            "blackboardSceneId",
            "__mewuBoardSource",
        ] {
            state.remove(key);
        }
    }
    detach_ink(item);
}
fn detach_ink(item: &mut SpaceItem) {
    // This is an explicit user-created template, not a replay of the original
    // model operation in a different scene. Keep pixels/shapes, start fresh undo
    // history, and never transplant a run's authorization/provenance identity.
    if let Some(document) = &mut item.image_annotations {
        for drawing in &mut document.drawings {
            drawing.origin = None;
        }
        document.history = Default::default();
    }
    if let Some(document) = &mut item.video_annotations {
        for object in &mut document.objects {
            object.origin = None;
        }
        document.undo.clear();
        document.redo.clear();
    }
    if let Some(edit) = &mut item.video_edit {
        edit.undo.clear();
        edit.redo.clear();
    }
}
impl Store {
    pub fn object_library(&self,owner:&str)->ObjectLibrary {self.object_libraries.get(owner).cloned().unwrap_or_default()}
    /// State and retained-object membership commit together. Plugin code never
    /// supplies native paths or a serialized scene to this public boundary.
    pub fn commit_objects(&mut self,owner:&str,expected:u64,state:serde_json::Value,remove:&[String],capture:Option<(String,ObjectTemplate)>)->Result<ObjectLibrary> {
        validate_key(owner)?;
        let mut next=self.object_library(owner);
        if next.revision!=expected {return Err(CoreError::ConcurrentModification);}
        if remove.len()>4096 {return Err(invalid("对象操作过大"));}
        for key in remove {
            validate_key(key)?;
            if next.objects.remove(key).is_none(){return Err(invalid("对象已移除"));}
        }
        if let Some((key,mut template))=capture {
            validate_key(&key)?;
            if next.objects.contains_key(&key){return Err(invalid("对象标识已存在"));}
            detached(&mut template.item); template.validate()?;
            next.objects.insert(key,template);
        }
        next.state=state;
        next.revision=expected.checked_add(1).ok_or_else(||invalid("对象版本无效"))?;
        next.validate()?;
        let revision=i64::try_from(next.revision).map_err(|_|invalid("对象版本无效"))?;
        let previous=i64::try_from(expected).map_err(|_|invalid("对象版本无效"))?;
        let payload=serde_json::to_string(&next)?;
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current:i64=tx.query_row("SELECT revision FROM app_state WHERE id=1",[],|r|r.get(0))?;
        if current!=self.revision {return Err(CoreError::ConcurrentModification);}
        let changed=if expected==0 {
            tx.execute("INSERT OR IGNORE INTO extension_objects(plugin_id,revision,payload) VALUES(?1,?2,?3)",params![owner,revision,payload])?
        } else {
            tx.execute("UPDATE extension_objects SET revision=?2,payload=?3 WHERE plugin_id=?1 AND revision=?4",params![owner,revision,payload,previous])?
        };
        if changed!=1 {return Err(CoreError::ConcurrentModification);}
        tx.commit()?;
        self.object_libraries.insert(owner.into(),next.clone());
        self.notify_checkpoint_maintenance();
        Ok(next)
    }
    pub fn capture_object(&self,scene_id:&str,item_id:&str)->Result<ObjectTemplate> {
        let source = self.active_scene()?;
        if source.id != scene_id || source.closed || source.frozen || is_running(source) {
            return Err(invalid("会话已变化"));
        }
        let mut item = source
            .items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .ok_or_else(|| invalid("对象已移除"))?;
        let board =
            self.state
                .scenes
                .iter()
                .find(|board| {
                    board.blackboard_link.as_ref().is_some_and(|link| {
                        link.parent_scene_id == scene_id && link.item_id == item_id
                    })
                })
                .cloned()
                .map(|mut board| {
                    board.messages.clear();
                    board.draft.clear();
                    board.refs.clear();
                    board.run = None;
                    board.conversation_start = None;
                    for item in &mut board.items {
                        detach_ink(item);
                        if let Some(state) = &mut item.state {
                            state.remove("__mewuBoardSource");
                        }
                    }
                    for region in &mut board.regions {
                        for drawing in &mut region.drawings {
                            drawing.origin = None;
                        }
                        region.drawing_history = Default::default();
                    }
                    board.geometry_history = Default::default();
                    Box::new(board)
                });
        detached(&mut item);
        Ok(ObjectTemplate { item, board })
    }
    pub fn restore_object(&mut self,owner:&str,entry_id:&str,expected:u64,scene_id:&str,x:f64,y:f64)->Result<Snapshot> {
        let library=self.object_libraries.get(owner).ok_or_else(||invalid("对象库不存在"))?;
        if expected!=library.revision {return Err(CoreError::ConcurrentModification);}
        if !x.is_finite() || !y.is_finite() || !(0. ..=1.).contains(&x) || !(0. ..=1.).contains(&y)
        {
            return Err(invalid("放置位置无效"));
        }
        let source = self.active_scene()?;
        if source.id != scene_id || source.closed || source.frozen || is_running(source) {
            return Err(invalid("会话已变化"));
        }
        let ObjectTemplate { item, board } = library.objects.get(entry_id)
            .ok_or_else(|| invalid("对象已移除"))?;
        let mut item = item.clone();
        detached(&mut item);
        item.id = id();
        item.x = x;
        item.y = y;
        let mut next = self.state.clone();
        let parent = scene_mut(&mut next, scene_id)?;
        let agent = parent.agent_id.clone();
        if let Some(board) = board {
            if parent.blackboard_link.is_some() {
                return Err(invalid("请回到截图空间添加黑板"));
            }
            let mut board = (**board).clone();
            board.id = id();
            board.agent_id = agent;
            board.connection_id = parent.connection_id.clone();
            board.blackboard_link = Some(BlackboardLink {
                parent_scene_id: scene_id.into(),
                item_id: item.id.clone(),
            });
            board.frozen = true;
            board.closed = false;
            board.minimized = false;
            item.state.get_or_insert_with(Default::default).insert(
                "blackboardSceneId".into(),
                serde_json::Value::String(board.id.clone()),
            );
            parent.items.push(item);
            touch(parent);
            next.scenes.push(board);
        } else {
            parent.items.push(item);
            touch(parent);
        }
        self.persist(next)
    }
}
