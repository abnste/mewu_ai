// SPDX-License-Identifier: MPL-2.0
//! Protocol-neutral, invocation-owned local JSON-lines processes.
//! Agent names, handshakes, session policies and UI belong to the plugin.
use std::{collections::{HashMap,BTreeMap}, path::PathBuf, sync::{Arc,atomic::{AtomicBool,Ordering}}, time::Duration};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json,Value};
use mewu_core::McpCommand;

#[derive(Default)]
pub(crate) struct Scope(HashMap<String,crate::mcp_runtime::JsonLinesProcess>);
async fn canceled(flag:&AtomicBool){loop{if flag.load(Ordering::Acquire){return;}tokio::time::sleep(Duration::from_millis(20)).await;}}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Open { command:Program }
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Program {executable:String,args:Vec<String>,#[serde(default)]cwd:Option<String>,#[serde(default)]env:BTreeMap<String,String>}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Message { id:String, #[serde(default)] message:Option<Value>, #[serde(default)] timeout_ms:Option<u64> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resolve { candidates:Vec<String>, #[serde(default)]files:Vec<String> }

#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct FrameBinding { frame_id:String,pointer:String,encoding:String }
pub(crate) fn bind_frames(params:&mut Value,bindings:Value,frames:&HashMap<String,crate::extension_region::RegionFrame>)->Result<(),String>{
    let bindings:Vec<FrameBinding>=serde_json::from_value(bindings).map_err(|_|"图片绑定无效")?;
    if bindings.is_empty()||bindings.len()>8{return Err("图片绑定数量无效".into());}
    let message=params.get_mut("message").ok_or("缺少进程消息")?;
    for binding in bindings {
        if binding.pointer.len()>512||!binding.pointer.starts_with('/') {return Err("图片绑定位置无效".into());}
        let frame=frames.get(&binding.frame_id).ok_or("图片不属于本次会话输入")?;
        let value=message.pointer_mut(&binding.pointer).filter(|v|v.is_null()).ok_or("图片绑定必须指向空占位")?;
        let base64=base64::engine::general_purpose::STANDARD.encode(&frame.png);
        *value=Value::String(match binding.encoding.as_str(){"base64"=>base64,"dataUrl"=>format!("data:image/png;base64,{base64}"),_=>return Err("图片编码不支持".into())});
    }
    Ok(())
}

fn expand(value:&str)->Option<PathBuf> {
    if value.is_empty()||value.len()>4096||value.chars().any(char::is_control){return None;}
    let mut path=value.to_owned();
    for (marker,name) in [("${home}","USERPROFILE"),("${localAppData}","LOCALAPPDATA"),("${appData}","APPDATA"),("${programFiles}","ProgramFiles")] {
        if path.starts_with(marker) {
            let value=if name=="USERPROFILE" {std::env::var_os(name).or_else(||std::env::var_os("HOME"))}else{std::env::var_os(name)}?;
            path=format!("{}{}",value.to_string_lossy(),&path[marker.len()..]);
            break;
        }
    }
    Some(PathBuf::from(path))
}
fn candidate(value:&str)->Result<Option<PathBuf>,String>{
    if value.is_empty()||value.len()>4096||value.chars().any(char::is_control)
        ||value.contains("${")&&! ["${home}","${appData}","${localAppData}","${programFiles}"].iter().any(|prefix|value.starts_with(prefix)) {
        return Err("程序候选路径无效".into());
    }
    Ok(expand(value))
}
fn resolve(candidates:Vec<String>)->Result<Value,String> {
    if candidates.len()>64{return Err("程序候选过多".into());}
    let mut matches=Vec::new();
    for candidate in candidates {
        let Some(path)=self::candidate(&candidate)? else {continue};
        let paths=if path.is_absolute(){vec![path]}else{
            if path.components().any(|part|!matches!(part,std::path::Component::Normal(_))){return Err("程序候选必须是名称或绝对路径".into());}
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|p|p.is_absolute()).map(|p|p.join(&path)).collect()
        };
        for path in paths {
            #[cfg(windows)]
            let path=if path.extension().is_none(){path.with_extension("exe")}else{path};
            if let Ok(executable)=path.canonicalize() {
                let command=McpCommand{executable:executable.to_string_lossy().into_owned(),args:vec![],cwd:None};
                if crate::mcp_runtime::validate_command(&command).is_ok()&&!matches.contains(&command.executable){matches.push(command.executable);}
            }
        }
    }
    Ok(json!({"platform":std::env::consts::OS,"executables":matches}))
}
impl Scope {
    pub(crate) fn call(&mut self,method:&str,params:Value,canceled:Arc<AtomicBool>)->Result<Value,String> {
        if method=="process.resolve" {
            let input:Resolve=serde_json::from_value(params).map_err(|_|"程序候选参数无效")?;
            if input.files.len()>64{return Err("文件候选过多".into());}
            let mut result=resolve(input.candidates)?;
            let mut files=Vec::new();
            for value in input.files {
                let Some(path)=candidate(&value)? else {continue};
                if !path.is_absolute(){return Err("文件候选需要绝对路径".into());}
                if let Ok(path)=path.canonicalize(){if path.is_file(){files.push(path);}}
            }
            result["files"]=serde_json::to_value(files).map_err(|_|"文件候选无效")?;
            return Ok(result);
        }
        if method=="process.open" {
            if self.0.len()>=2{return Err("单次插件调用最多打开两个进程".into());}
            let input:Open=serde_json::from_value(params).map_err(|_|"进程配置无效")?;
            let config=McpCommand{executable:input.command.executable,args:input.command.args,cwd:input.command.cwd};
            let mut env=input.command.env;
            if env.len()>64||env.iter().any(|(key,value)| key.is_empty()||key.len()>128||!key.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')||value.len()>16384||value.contains('\0'))
                ||env.values().map(String::len).sum::<usize>()>65536{return Err("进程环境变量无效或过大".into());}
            for value in env.values_mut(){if value.starts_with("${home}")||value.starts_with("${localAppData}")||value.starts_with("${appData}") {*value=expand(value).ok_or("环境路径不可用")?.to_string_lossy().into_owned();}}
            let (_,cwd)=crate::mcp_runtime::validate_command(&config)?;
            let process=tauri::async_runtime::block_on(async {
                tokio::select! {
                    biased;
                    _=self::canceled(&canceled)=>Err("外部进程启动已取消".to_string()),
                    result=crate::mcp_runtime::JsonLinesProcess::open(&config,&env)=>result,
                }
            })?;
            let id=uuid::Uuid::new_v4().to_string();self.0.insert(id.clone(),process);
            return Ok(json!({"id":id,"cwd":cwd}));
        }
        let input:Message=serde_json::from_value(params).map_err(|_|"进程消息参数无效")?;
        if method=="process.close" {self.0.remove(&input.id);return Ok(Value::Null);}
        let process=self.0.get_mut(&input.id).ok_or("进程不属于本次插件调用")?;
        match method {
            "process.write"=>{
                let message=input.message.ok_or("缺少进程消息")?;
                tauri::async_runtime::block_on(async{tokio::select!{biased;_=self::canceled(&canceled)=>Err("进程写入已取消".to_string()),result=process.write(message)=>result}})?;Ok(Value::Null)
            },
            "process.read"=>tauri::async_runtime::block_on(async{tokio::select!{biased;_=self::canceled(&canceled)=>Err("进程读取已取消".to_string()),result=process.read(input.timeout_ms.unwrap_or(250))=>result}}),
            _=>Err("不支持的进程方法".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore="explicit local client configuration required; chat mode sends the supplied test prompt"]
    fn independent_agent_settings_probe_uses_real_quickjs_and_local_json_lines(){
        use mewu_extension::{ExtensionRuntime,RuntimeLimits,WorkerResponse,ProtocolError};
        let file=PathBuf::from(std::env::var_os("MEWU_AGENT_CONNECTOR_PROBE").expect("explicit probe JSON required"));
        let probe:Value=serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let chat=probe["mode"]=="chat";
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap();
        // Authoring projects may contain README/tests; runtime packages contain
        // only the explicit manifest/module allowlist, exactly like CLI pack.
        let files=["mewu-plugin.json","index.mjs","profiles.mjs","rpc.mjs","adapters.mjs","view.mjs"];
        let stage=file.parent().unwrap().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&stage).unwrap();
        for file in files {std::fs::copy(root.join("extensions/official/agent-connectors").join(file),stage.join(file)).unwrap();}
        let package=mewu_extension::load_directory_package(&stage).unwrap();
        for file in files {std::fs::remove_file(stage.join(file)).unwrap();}std::fs::remove_dir(&stage).unwrap();
        let output=Arc::new(std::sync::Mutex::new(Vec::new()));let emitted=output.clone();
        let cancel=Arc::new(AtomicBool::new(false));
        let limits=RuntimeLimits{request_timeout:Duration::from_secs(300),max_host_calls:4096,..RuntimeLimits::default()};
        let mut runtime=ExtensionRuntime::new(package,limits,cancel.clone(),Arc::new(move|value|{emitted.lock().unwrap().push(value);Ok(())})).unwrap();
        let mut scope=Scope::default();let mut initialized=false;let mut storage=HashMap::<String,Value>::new();let mut methods=Vec::new();
        let mut frames=HashMap::new();let mut image_bindings=0;let mut round=1;
        if probe["image"]==true {
            let mut png=std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64,64,image::Rgb([255,0,0]))).write_to(&mut png,image::ImageFormat::Png).unwrap();
            frames.insert("probe-frame".into(),crate::extension_region::RegionFrame{png:png.into_inner(),width:64,height:64,sha256:"synthetic-test-only".into()});
        }
        let until=std::time::Instant::now()+Duration::from_secs(if chat{300}else{80});
        let result=loop{
            assert!(std::time::Instant::now()<until,"native Agent probe exceeded deadline after {methods:?}");
            let calls=std::mem::take(&mut *output.lock().unwrap());
            for call in calls {if let WorkerResponse::HostCall{id,method,mut params}=call {
                if method=="process.write" {if let Some(method)=params["message"]["method"].as_str(){methods.push(method.to_owned());}}
                if method=="process.write" {if let Some(bindings)=params.as_object_mut().unwrap().remove("bindings"){crate::extension_process::bind_frames(&mut params,bindings,&frames).unwrap();image_bindings+=1;}}
                let result=match method.as_str(){
                    "runtime.describe"=>Ok(crate::extension_host::runtime_capabilities()),
                    "configuration.get"=>Ok(json!({probe["provider"].as_str().unwrap():probe["config"]})),
                    "storage.get"=>Ok(storage.get(params["key"].as_str().unwrap()).cloned().unwrap_or(Value::Null)),
                    "storage.set"=>{storage.insert(params["key"].as_str().unwrap().into(),params["value"].clone());Ok(Value::Null)},
                    "events.emit"=>Ok(Value::Null),
                    _ if method.starts_with("process.")=>scope.call(&method,params,cancel.clone()),
                    _=>Err("probe did not grant this method".into()),
                };
                match result{Ok(value)=>runtime.host_result(&id,Some(value),None).unwrap(),Err(_)=>runtime.host_result(&id,None,Some(ProtocolError{code:"PROBE_FAILED".into(),message:"local protocol operation failed".into()})).unwrap()}
            }}
            if !initialized {
                if let Some(registrations)=runtime.poll_initialization().unwrap(){
                    assert_eq!(registrations.len(),5);
                    let input=if chat {probe["input"].clone()}else{json!({"type":"test","provider":probe["provider"],"config":probe["config"]})};
                    runtime.begin_invoke(if chat {probe["provider"].as_str().unwrap()}else{"settings"},&input,&json!({"agentId":"probe","sceneId":"probe"})).unwrap();initialized=true;
                }
            }else {match runtime.poll_result(){
                Ok(Some(value))=>{
                    if chat && round==1 && probe["rounds"]==2 {
                        assert!(value["content"].as_str().is_some_and(|text|text.contains("MEWU_CONNECTOR_OK")));
                        round=2;
                        runtime.begin_invoke(probe["provider"].as_str().unwrap(),&probe["input"],&json!({"agentId":"probe","sceneId":"probe"})).unwrap();
                    }else{break value;}
                },Ok(None)=>{},Err(_)=>panic!("local Agent probe failed after protocol methods {methods:?}")}}
            std::thread::sleep(Duration::from_millis(1));
        };
        // Only a reduced receipt is written for model turns, never transcript
        // or login material. Probe storage is synthetic, not a user database.
        let receipt=std::env::var_os("MEWU_AGENT_CONNECTOR_RECEIPT").expect("receipt path required");
        if chat {
            let valid=result["content"].as_str().is_some_and(|text|text.contains("MEWU_CONNECTOR_OK"));
            let resumed=methods.iter().any(|method|matches!(method.as_str(),"thread/resume"|"session/load"));
            std::fs::write(receipt,serde_json::to_vec_pretty(&json!({"provider":probe["provider"],"ok":valid,"sessionCheckpointed":!storage.is_empty(),"rounds":round,"resumed":resumed,"imageBindings":image_bindings})).unwrap()).unwrap();
            assert!(valid,"local Agent test prompt failed");assert!(!storage.is_empty());
            if round==2{assert!(resumed,"client did not restore persisted session");}
            if probe["image"]==true{assert!(image_bindings>0);}
        }else{
            std::fs::write(receipt,serde_json::to_vec_pretty(&result).unwrap()).unwrap();
            assert_eq!(result["ok"],true,"local Agent connection failed; see private receipt");
            assert!(result["models"].as_array().is_some_and(|v|!v.is_empty()));
        }
        runtime.deactivate().unwrap();drop(runtime);drop(scope);
        assert!(tauri::async_runtime::block_on(crate::mcp_runtime::wait_until_idle(Duration::from_secs(5))));
    }
    #[test]fn paths_never_expand_arbitrary_environment_or_relative_directories(){
        assert!(resolve(vec!["../unknown".into()]).is_err());
        assert!(resolve(vec!["bad\nname".into()]).is_err());
        assert!(resolve(vec!["${SECRET}/run".into()]).is_err());
        assert!(resolve(vec!["name".into();65]).is_err());
    }
    #[test]fn handles_are_invocation_owned_and_close_is_idempotent(){
        let mut scope=Scope::default();
        let cancel=Arc::new(AtomicBool::new(false));
        assert!(scope.call("process.read",json!({"id":"another-invocation"}),cancel.clone()).is_err());
        assert!(scope.call("process.write",json!({"id":"another-invocation","message":{}}),cancel.clone()).is_err());
        assert_eq!(scope.call("process.close",json!({"id":"gone"}),cancel.clone()).unwrap(),Value::Null);
        assert!(scope.call("process.open",json!({"command":{},"extra":1}),cancel).is_err());
    }
    #[test]fn image_bindings_can_only_use_this_runs_native_frames_and_null_placeholders(){
        let mut frames=HashMap::new();
        frames.insert("own".into(),crate::extension_region::RegionFrame{png:vec![1,2,3],width:1,height:1,sha256:"test".into()});
        let mut message=json!({"message":{"params":{"image":null}}});
        assert!(bind_frames(&mut message,json!([{"frameId":"foreign","pointer":"/params/image","encoding":"base64"}]),&frames).is_err());
        bind_frames(&mut message,json!([{"frameId":"own","pointer":"/params/image","encoding":"base64"}]),&frames).unwrap();
        assert_eq!(message["message"]["params"]["image"],"AQID");
        assert!(bind_frames(&mut message,json!([{"frameId":"own","pointer":"/params/image","encoding":"base64"}]),&frames).is_err());
    }
}
