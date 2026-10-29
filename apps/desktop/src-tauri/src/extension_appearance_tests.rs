// SPDX-License-Identifier: MPL-2.0
//! Real archives/parser/registry/QuickJS; no production data and no mock SDK.
use crate::extension_appearance::projection;
use mewu_extension::{ExtensionRuntime,RuntimeLimits,WorkerResponse};
use serde_json::json;
use std::{io::Write,sync::{Arc,atomic::AtomicBool,mpsc}};
fn archive(aurora:bool)->Vec<u8>{
    let files=if aurora{[("mewu-plugin.json",include_str!("../../../../extensions/examples/aurora-theme/mewu-plugin.json")),("index.mjs",include_str!("../../../../extensions/examples/aurora-theme/index.mjs"))]}else{[("mewu-plugin.json",include_str!("../../../../extensions/examples/paper-theme/mewu-plugin.json")),("index.mjs",include_str!("../../../../extensions/examples/paper-theme/index.mjs"))]};
    let mut zip=zip::ZipWriter::new(std::io::Cursor::new(vec![]));for(name,text)in files{zip.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(text.as_bytes()).unwrap();}zip.finish().unwrap().into_inner()
}
fn initialize(package:mewu_extension::ExtensionPackage)->Result<Vec<mewu_extension::ServiceRegistration>,String>{
    let(tx,rx)=mpsc::channel();
    let mut runtime=ExtensionRuntime::new(package,RuntimeLimits::default(),Arc::new(AtomicBool::new(false)),Arc::new(move|value|{tx.send(value).unwrap();Ok(())})).unwrap();
    for _ in 0..500{
        match runtime.poll_initialization(){Ok(Some(services))=>{runtime.deactivate().unwrap();return Ok(services);},Err(error)=>return Err(error.to_string()),_=>{}}
        while let Ok(WorkerResponse::HostCall{id,method,..})=rx.try_recv(){
            let result=match method.as_str(){"runtime.describe"=>crate::extension_host::runtime_capabilities(),"ui.appearance.describe"=>mewu_extension::appearance_contract(),_=>panic!("theme attempted an unrelated host call")};runtime.host_result(&id,Some(result),None).unwrap();
        }
    }Err("activation did not complete".into())
}
#[test]
fn independent_appearance_archives_install_replace_reopen_disable_remove_and_reinstall(){
    let root=std::env::temp_dir().join(format!("mewu-appearance-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
    let mut store=crate::extension_store::Store::open(&root).unwrap();
    let source=crate::extension_store::Source::Local{path:"independent-theme.zip".into()};
    let one=archive(true);let two=archive(false);
    let mut ids=vec![];
    for bytes in [&one,&two]{let package=crate::extension_market::decode_package(bytes).unwrap();let services=initialize(package.clone()).unwrap();let record=store.install(&package,bytes,&source,None).unwrap();store.set_services(&record.manifest.id,record.revision,&services).unwrap();store.set_value(&record.manifest.id,"plugin-configuration","retained",&json!({"private":true})).unwrap();ids.push(record.manifest.id);}
    let before=projection(&store.list().unwrap());assert_eq!(before.iter().map(|r|r.themes.len()).sum::<usize>(),2);
    let wire=serde_json::to_string(&before).unwrap();assert!(!wire.contains("independent-theme.zip")&&!wire.contains("retained")&&!wire.contains("private"));
    drop(store);let mut store=crate::extension_store::Store::open(&root).unwrap();assert_eq!(serde_json::to_value(projection(&store.list().unwrap())).unwrap(),serde_json::to_value(before).unwrap());
    let initial=store.get(&ids[0]).unwrap();let disabled=store.set_state(&ids[0],initial.revision,"disabled").unwrap();assert!(projection(&[disabled.clone()])[0].themes.is_empty());
    let enabled=store.set_state(&ids[0],disabled.revision,"enabled").unwrap();assert_eq!(projection(&[enabled.clone()])[0].themes.len(),1);
    let removed=store.set_state(&ids[0],enabled.revision,"removed").unwrap();assert!(projection(&[removed.clone()])[0].themes.is_empty());
    assert!(store.set_state(&ids[0],initial.revision,"enabled").is_err());
    let package=crate::extension_market::decode_package(&one).unwrap();let installed=store.install(&package,&one,&source,Some(removed.revision)).unwrap();let services=initialize(package).unwrap();store.set_services(&ids[0],installed.revision,&services).unwrap();
    assert_eq!(store.value(&ids[0],"plugin-configuration","retained").unwrap(),json!({"private":true}));
    assert_eq!(projection(&store.list().unwrap()).iter().map(|r|r.themes.len()).sum::<usize>(),2);
    let mut denied=crate::extension_market::decode_package(&one).unwrap();denied.manifest.permissions.clear();assert!(initialize(denied).is_err());
    drop(store);std::fs::remove_dir_all(root).unwrap();
}
