// SPDX-License-Identifier: MPL-2.0
use super::extension_objects_host::{describe, Commit};
use mewu_extension::{ExtensionRuntime,RuntimeLimits,WorkerResponse};
use serde_json::{json,Value};
use std::{io::Write,sync::{Arc,atomic::AtomicBool,mpsc}};

fn archive(notes:bool)->Vec<u8> {
    let files=if notes {[
        ("mewu-plugin.json",include_str!("../../../../extensions/examples/research-notes/mewu-plugin.json")),
        ("index.mjs",include_str!("../../../../extensions/examples/research-notes/index.mjs")),
        ("view.mjs",include_str!("../../../../extensions/examples/research-notes/view.mjs"))
    ]}else{[
        ("mewu-plugin.json",include_str!("../../../../extensions/official/object-toolbox/mewu-plugin.json")),
        ("index.mjs",include_str!("../../../../extensions/official/object-toolbox/index.mjs")),
        ("view.mjs",include_str!("../../../../extensions/official/object-toolbox/view.mjs"))
    ]};
    let mut writer=zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for(name,source)in files {writer.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();writer.write_all(source.as_bytes()).unwrap();}
    writer.finish().unwrap().into_inner()
}
fn registrations(package:mewu_extension::ExtensionPackage)->Vec<mewu_extension::ServiceRegistration>{
    let(tx,rx)=mpsc::channel();
    let mut runtime=ExtensionRuntime::new(package,RuntimeLimits::default(),Arc::new(AtomicBool::new(false)),Arc::new(move|v|{tx.send(v).unwrap();Ok(())})).unwrap();
    for _ in 0..400 {
        if let Some(services)=runtime.poll_initialization().unwrap(){runtime.deactivate().unwrap();return services;}
        while let Ok(WorkerResponse::HostCall{id,method,..})=rx.try_recv(){
            assert_eq!(method,"objects.library.describe");runtime.host_result(&id,Some(describe()),None).unwrap();
        }
    }
    panic!("initialization timed out");
}
fn invoke(registry:&crate::extension_store::Store,store:&mut mewu_core::Store,owner:&str,revision:u64,service:&str,input:Value,mut drop:Option<mewu_core::ObjectTemplate>)->Result<Value,String>{
    let record=registry.get(owner)?;
    if record.state!="enabled"||record.revision!=revision{return Err("revoked".into());}
    let package=registry.package(owner,revision)?;
    let(tx,rx)=mpsc::channel();
    let mut runtime=ExtensionRuntime::new(package,RuntimeLimits::default(),Arc::new(AtomicBool::new(false)),Arc::new(move|v|{tx.send(v).map_err(|_|mewu_extension::ExtensionError::InvalidPackage("closed"))?;Ok(())})).map_err(|e|e.to_string())?;
    let mut registered=false;
    for _ in 0..400 {
        if !registered {
            if let Some(services)=runtime.poll_initialization().map_err(|e|e.to_string())? {
                assert!(services.iter().any(|s|super::extension_objects_host::is_surface(s)));
                runtime.begin_invoke(service,&input,&json!({})).map_err(|e|e.to_string())?;registered=true;
            }
        }else if let Some(value)=runtime.poll_result().map_err(|e|e.to_string())? {runtime.deactivate().unwrap();return Ok(value);}
        while let Ok(WorkerResponse::HostCall{id,method,params})=rx.try_recv(){
            let result:Result<Value,String>=(||match method.as_str(){
                "objects.library.describe"=>Ok(describe()),
                "objects.library.get"=>Ok(store.object_library(owner).catalogue()),
                "objects.library.commit"=>{
                    let c:Commit=serde_json::from_value(params).map_err(|e|e.to_string())?;
                    let capture=if let Some(key)=c.capture_key {Some((key,drop.take().ok_or("no user drop")?))}else{None};
                    Ok(store.commit_objects(owner,c.expected_revision,c.state,&c.remove,capture).map_err(|e|e.to_string())?.catalogue())
                },
                "objects.library.restore"=>{
                    let scene=store.snapshot().active_scene_id;
                    store.restore_object(owner,params["id"].as_str().unwrap(),params["expectedRevision"].as_u64().unwrap(),&scene,params["x"].as_f64().unwrap(),params["y"].as_f64().unwrap()).map_err(|e|e.to_string())?;
                    Ok(json!({"restored":true}))
                },
                _=>Err("unexpected host call".into())
            })();
            match result{Ok(value)=>runtime.host_result(&id,Some(value),None),Err(_)=>runtime.host_result(&id,None,Some(mewu_extension::ProtocolError{code:"DENIED".into(),message:"denied".into()}))}.map_err(|e|e.to_string())?;
        }
    }
    Err("runtime did not settle".into())
}
#[test]
fn two_independent_archives_install_interact_reopen_restore_and_uninstall() {
    let root=std::env::temp_dir().join(format!("mewu-independent-surfaces-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let mut registry=crate::extension_store::Store::open(&root).unwrap();
    let mut store=mewu_core::Store::open(root.join("objects.db")).unwrap();
    let a=archive(false);let b=archive(true);
    let origin=crate::extension_store::Source::Local{path:"independent-test.zip".into()};
    let toolbox=registry.install(&crate::extension_market::decode_package(&a).unwrap(),&a,&origin,None).unwrap();
    let notes=registry.install(&crate::extension_market::decode_package(&b).unwrap(),&b,&origin,None).unwrap();
    let one=&toolbox.manifest.id;let two=&notes.manifest.id;
    let scene=store.snapshot().active_scene_id;
    store.add_asset(&scene,mewu_core::Asset{id:uuid::Uuid::new_v4().to_string(),name:"drawing.svg".into(),kind:mewu_core::AssetKind::Svg,path:root.join("drawing.svg").to_string_lossy().into_owned(),width:Some(200),height:Some(120),origin_x:None,origin_y:None,scale_factor:None,capture_coordinates:None}).unwrap();
    let item=store.active_scene().unwrap().items[0].id.clone();
    let template=store.capture_object(&scene,&item).unwrap();
    let mount=invoke(&registry,&mut store,one,toolbox.revision,"toolbox",json!({"type":"mount"}),None).unwrap();
    assert!(mount["html"].as_str().unwrap().contains("grid-template-columns"));
    let other=invoke(&registry,&mut store,two,notes.revision,"notes",json!({"type":"mount"}),None).unwrap();
    assert!(other["html"].as_str().unwrap().contains("textarea"));
    invoke(&registry,&mut store,one,toolbox.revision,"toolbox",json!({"type":"drop","target":"root/0"}),Some(template.clone())).unwrap();
    let folder=invoke(&registry,&mut store,one,toolbox.revision,"toolbox",json!({"type":"drop","target":"root/0"}),Some(template.clone())).unwrap();
    assert_eq!(folder["state"]["slots"][0]["children"].as_array().unwrap().len(),2);
    assert!(invoke(&registry,&mut store,one,toolbox.revision,"toolbox",json!({"type":"drop","target":"root/1"}),None).is_err());
    invoke(&registry,&mut store,two,notes.revision,"notes",json!({"type":"save","text":"独立便笺"}),None).unwrap();
    invoke(&registry,&mut store,two,notes.revision,"notes",json!({"type":"drop","target":"references"}),Some(template)).unwrap();
    let saved_one=store.object_library(one);let saved_two=store.object_library(two);
    assert_eq!(saved_one.objects.len(),2);assert_eq!(saved_two.objects.len(),1);
    drop(store);drop(registry);
    let mut registry=crate::extension_store::Store::open(&root).unwrap();
    let mut store=mewu_core::Store::open(root.join("objects.db")).unwrap();
    assert_eq!(store.object_library(one),saved_one);assert_eq!(store.object_library(two),saved_two);
    store.apply(mewu_core::SceneCommand::NewScene).unwrap();
    invoke(&registry,&mut store,one,toolbox.revision,"toolbox",json!({"type":"restore","id":"object-1","x":0.2,"y":0.3}),None).unwrap();
    invoke(&registry,&mut store,two,notes.revision,"notes",json!({"type":"restore","id":"reference-2","x":0.4,"y":0.3}),None).unwrap();
    assert_eq!(store.active_scene().unwrap().items.len(),2);
    let disabled=registry.set_state(one,toolbox.revision,"disabled").unwrap();
    assert!(invoke(&registry,&mut store,one,toolbox.revision,"toolbox",json!({"type":"get"}),None).is_err());
    assert!(invoke(&registry,&mut store,two,notes.revision,"notes",json!({"type":"get"}),None).is_ok());
    let removed=registry.set_state(one,disabled.revision,"removed").unwrap();
    assert!(invoke(&registry,&mut store,one,removed.revision,"toolbox",json!({"type":"get"}),None).is_err());
    let installed=registry.install(&crate::extension_market::decode_package(&a).unwrap(),&a,&origin,Some(removed.revision)).unwrap();
    let again=invoke(&registry,&mut store,one,installed.revision,"toolbox",json!({"type":"get"}),None).unwrap();
    assert_eq!(again,saved_one.catalogue());
    let removed_two=registry.set_state(two,notes.revision,"removed").unwrap();
    assert!(invoke(&registry,&mut store,two,removed_two.revision,"notes",json!({"type":"get"}),None).is_err());
    assert_eq!(store.object_library(two),saved_two);
    drop(store);drop(registry);
    // The private UUID tree is the only test-owned tree eligible for cleanup.
    let resolved=root.canonicalize().unwrap();let parent=std::env::temp_dir().canonicalize().unwrap();
    assert!(resolved.starts_with(parent)&&root.file_name().unwrap().to_string_lossy().starts_with("mewu-independent-surfaces-"));
    std::fs::remove_dir_all(resolved).unwrap();
}

/// Opt-in native integration transport for the browser UI harness. Uses a
/// disposable Store and real archive/parser/QuickJS, never the user's database.
#[test]
#[ignore = "interactive browser integration harness"]
fn browser_transport() {
    use std::io::BufRead;
    let root=std::env::temp_dir().join(format!("mewu-surface-browser-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let mut registry=crate::extension_store::Store::open(&root).unwrap();
    let mut store=mewu_core::Store::open(root.join("objects.db")).unwrap();
    let surfaces=crate::extension_surfaces::Surfaces::default();
    let scene=store.snapshot().active_scene_id;
    store.add_asset(&scene,mewu_core::Asset{id:uuid::Uuid::new_v4().to_string(),name:"测试选区.svg".into(),kind:mewu_core::AssetKind::Svg,path:root.join("selection.svg").to_string_lossy().into_owned(),width:Some(200),height:Some(120),origin_x:None,origin_y:None,scale_factor:None,capture_coordinates:None}).unwrap();
    let template=store.capture_object(&scene,&store.active_scene().unwrap().items[0].id).unwrap();
    let source=crate::extension_store::Source::Local{path:"integration.zip".into()};
    for line in std::io::stdin().lock().lines(){
        let request:Value=serde_json::from_str(&line.unwrap()).unwrap();
        let result:Result<Value,String>=(||{
            match request["method"].as_str().ok_or("method")? {
                "install"=>{
                    for notes in [false,true]{
                        let bytes=archive(notes);let package=crate::extension_market::decode_package(&bytes)?;
                        let expected=registry.get_optional(&package.manifest.id)?.map(|r|r.revision);
                        let record=registry.install(&package,&bytes,&source,expected)?;
                        let services=registrations(package);
                        registry.set_services(&record.manifest.id,record.revision,&services)?;
                    }
                    serde_json::to_value(registry.list()?).map_err(|e|e.to_string())
                },
                "list"=>serde_json::to_value(registry.list()?).map_err(|e|e.to_string()),
                "invoke"=>{
                    let owner=crate::extension_surfaces::Owner{
                        plugin:request["pluginId"].as_str().ok_or("plugin")?.into(),revision:request["revision"].as_u64().ok_or("revision")?,
                        service:request["serviceId"].as_str().ok_or("service")?.into(),scene:request["sceneId"].as_str().ok_or("scene")?.into()
                    };
                    if request["surface"]["action"]=="close"{
                        surfaces.close(request["surface"]["id"].as_str().ok_or("surface")?,&owner)?;return Ok(json!({"closed":true}));
                    }
                    if request["surface"]["action"]=="call"{surfaces.check(request["surface"]["id"].as_str().ok_or("surface")?,&owner)?;}
                    let value=invoke(&registry,&mut store,&owner.plugin,owner.revision,&owner.service,request["input"].clone(),request["drop"].as_bool().filter(|v|*v).map(|_|template.clone()))?;
                    if request["surface"]["action"]=="mount"{
                        let document=serde_json::from_value(value).map_err(|e|e.to_string())?;
                        let mounted=surfaces.mount(owner,document,"http://127.0.0.1:49618")?;
                        let html=surfaces.test_html(&mounted.surface_id);
                        let mut result=serde_json::to_value(mounted).unwrap();result["document"]=json!(html);Ok(result)
                    }else{Ok(value)}
                },
                "control"=>{
                    surfaces.revoke(request["pluginId"].as_str());
                    registry.set_state(request["pluginId"].as_str().ok_or("plugin")?,request["revision"].as_u64().ok_or("revision")?,request["state"].as_str().ok_or("state")?)?;
                    serde_json::to_value(registry.list()?).map_err(|e|e.to_string())
                },
                "reopen"=>{
                    surfaces.revoke(None);
                    registry=crate::extension_store::Store::open(&root)?;
                    store=mewu_core::Store::open(root.join("objects.db")).map_err(|e|e.to_string())?;
                    Ok(json!({"reopened":true}))
                },
                "shutdown"=>Ok(json!({"closed":true})),
                _=>Err("unknown method".into())
            }
        })();
        println!("MEWU_SURFACE_RESULT {}",match result{Ok(v)=>json!({"result":v}),Err(e)=>json!({"error":e})});
        std::io::stdout().flush().unwrap();
        if request["method"]=="shutdown"{break;}
    }
    drop(store);drop(registry);
    let resolved=root.canonicalize().unwrap();let parent=std::env::temp_dir().canonicalize().unwrap();
    assert!(resolved.starts_with(parent)&&root.file_name().unwrap().to_string_lossy().starts_with("mewu-surface-browser-"));
    std::fs::remove_dir_all(resolved).unwrap();
}
