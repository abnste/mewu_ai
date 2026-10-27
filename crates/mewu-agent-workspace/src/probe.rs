// SPDX-License-Identifier: MPL-2.0
//! Isolated authoring checks. Never uses an installed plugin, live configuration,
//! credentials, network, processes, native actions, or a UI permission grant.
use crate::Project;
use mewu_extension::{ExtensionError,ExtensionPackage,ExtensionRuntime,ProtocolError,RuntimeLimits,WorkerResponse};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::{collections::BTreeMap,sync::{atomic::AtomicBool,mpsc,Arc},time::Duration};

#[derive(Debug,Deserialize,Serialize,schemars::JsonSchema)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct ProbeCase {
    pub service_id:String,
    /// Exact JSON text passed to the service, e.g. {"type":"state"}.
    /// Encode arrays/booleans inside this string; the host parses it exactly once.
    pub input_json:String,
    #[schemars(length(min=1,max=32))] pub assertions:Vec<ProbeAssertion>,
}
#[derive(Debug,Deserialize,Serialize,schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all="camelCase")]
pub struct ProbeAssertion {
    pub path:String,
    /// Exact JSON text: true for a boolean, [1,2] for an array, "text" for a string.
    pub equals_json:String,
}
fn value_type(value:Option<&Value>)->&'static str{match value{None=>"missing",Some(Value::Null)=>"null",Some(Value::Bool(_))=>"boolean",Some(Value::Number(_))=>"number",Some(Value::String(_))=>"string",Some(Value::Array(_))=>"array",Some(Value::Object(_))=>"object"}}

/// Every case reactivates a fresh interpreter, while only the fake configuration
/// persists. This deliberately catches relying on module globals across calls.
pub fn probe(project:&Project,cases:&[ProbeCase],mut configuration:BTreeMap<String,Value>,capabilities:Value,catalog:Value)->Result<Value,String>{
    project.validate()?;
    if cases.is_empty()||cases.len()>8||configuration.len()>64||cases.iter().any(|c|c.assertions.is_empty()||c.assertions.len()>32){return Err("测试需要 1–8 个用例，每例必须有 1–32 个行为断言".into());}
    let mut reports=Vec::new();
    for case in cases {
        let input:Value=serde_json::from_str(&case.input_json).map_err(|error|format!("inputJson 必须是合法 JSON：{error}"))?;
        let expected=case.assertions.iter().map(|a|serde_json::from_str::<Value>(&a.equals_json).map_err(|e|format!("断言 {} 的 equalsJson 必须是合法 JSON：{e}",a.path))).collect::<Result<Vec<_>,_>>()?;
        let (tx,rx)=mpsc::channel();
        let limits=RuntimeLimits{initialization_timeout:Duration::from_secs(2),request_timeout:Duration::from_secs(2),max_host_calls:64,..Default::default()};
        let mut runtime=ExtensionRuntime::new(ExtensionPackage{manifest:project.manifest.clone(),modules:project.modules.clone()},limits,Arc::new(AtomicBool::new(false)),Arc::new(move|message|tx.send(message).map_err(|_|ExtensionError::Transport))).map_err(|e|e.to_string())?;
        let mut denied=Vec::new();let mut invoking=false;let mut settings=false;
        let mut drain=|runtime:&ExtensionRuntime,invoking:bool,settings:bool|->Result<(),String>{
            for message in rx.try_iter(){
                if let WorkerResponse::HostCall{id,method,params}=message {
                    let has=|permission:&str|project.manifest.permissions.iter().any(|p|p==permission);
                    let key=||params["key"].as_str().filter(|k|!k.is_empty()&&k.len()<=512&&!k.chars().any(char::is_control)).ok_or("Invalid configuration key");
                    let result:Result<Value,&str>=match method.as_str(){
                        "runtime.describe"=>Ok(capabilities.clone()),
                        "ui.toolbars.describe" if has("ui.render")=>catalog["surfaces"].as_array().and_then(|surfaces|surfaces.iter().find(|s|s["id"]==params["surface"]))
                            .map(|s|json!({"version":1,"surface":s["id"],"items":s["items"],"current":null,"currentScope":"isolated test: no rendered UI"})).ok_or("Unknown toolbar"),
                        "configuration.get" if has("configuration.read")=>key().map(|k|configuration.get(k).cloned().unwrap_or(Value::Null)),
                        "configuration.compareAndSet" if has("configuration.write")&&invoking&&settings=>{
                            key().and_then(|k|{
                                let expected=params.get("expected").ok_or("Missing expected")?;
                                let value=params.get("value").ok_or("Missing value")?;
                                if value.to_string().len()>1024*1024||expected.to_string().len()>1024*1024{return Err("Configuration value limit");}
                                // Production CAS treats expected:null as an insert,
                                // not an update of a key whose stored value is null.
                                let matches=if expected.is_null(){!configuration.contains_key(k)}else{configuration.get(k)==Some(expected)};
                                if !matches{return Ok(Value::Bool(false));}
                                if !configuration.contains_key(k)&&configuration.len()>=64{return Err("Configuration limit");}
                                configuration.insert(k.into(),value.clone());Ok(Value::Bool(true))
                            })
                        },
                        _=>Err("Host capability is unavailable in an isolated test"),
                    };
                    match result {
                        Ok(value)=>runtime.host_result(&id,Some(value),None),
                        Err(reason)=>{denied.push(json!({"method":method,"reason":reason}));runtime.host_result(&id,None,Some(ProtocolError{code:"probe.capability_unavailable".into(),message:reason.into()}))}
                    }.map_err(|e|e.to_string())?;
                }
            }
            Ok(())
        };
        let outcome=(||->Result<Value,String>{
            let services=loop {
                drain(&runtime,invoking,settings)?;
                if let Some(services)=runtime.poll_initialization().map_err(|e|e.to_string())?{break services;}
                std::thread::sleep(Duration::from_millis(1));
            };
            let service=services.iter().find(|s|s.id==case.service_id).ok_or("测试服务未注册")?;
            settings=service.kind==mewu_extension::ServiceKind::Ui&&service.metadata.as_ref().is_some_and(|m|m["surface"]=="settings");
            invoking=true;
            runtime.begin_invoke(&case.service_id,&input,&json!({"testing":true})).map_err(|e|e.to_string())?;
            loop {
                drain(&runtime,invoking,settings)?;
                if let Some(result)=runtime.poll_result().map_err(|e|e.to_string())?{return Ok(result);}
                std::thread::sleep(Duration::from_millis(1));
            }
        })();
        drop(drain);
        let assertions=case.assertions.iter().zip(&expected).map(|(a,expected)|{
            let actual=outcome.as_ref().ok().and_then(|v|v.pointer(&a.path));
            json!({"path":a.path,"expected":expected,"expectedType":value_type(Some(expected)),"actual":actual,"actualType":value_type(actual),"passed":actual==Some(expected)})
        }).collect::<Vec<_>>();
        let passed=outcome.is_ok()&&denied.is_empty()&&assertions.iter().all(|a|a["passed"]==true);
        reports.push(json!({"serviceId":case.service_id,"input":input,"passed":passed,"result":outcome.as_ref().ok(),"error":outcome.err(),"assertions":assertions,"unavailableHostCalls":denied}));
    }
    Ok(json!({"passed":reports.iter().all(|r|r["passed"]==true),"scope":"isolated service execution; fresh interpreter per case; simulated configuration only; NOT a GUI, installation, network or platform test","cases":reports,"configuration":configuration,"installed":false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project(source:&str)->Project{serde_json::from_value(json!({"manifest":{"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"test.probe","version":"1.0.0","name":"Probe","description":"","license":"MPL-2.0","entry":"index.mjs","permissions":["ui.render","configuration.read","configuration.write"]},"modules":{"index.mjs":source}})).unwrap()}
    fn cases(mut value:Value)->Vec<ProbeCase>{
        for case in value.as_array_mut().unwrap(){
            let input=case.as_object_mut().unwrap().remove("input").unwrap();case["inputJson"]=json!(input.to_string());
            for assertion in case["assertions"].as_array_mut().unwrap(){let expected=assertion.as_object_mut().unwrap().remove("equals").unwrap();assertion["equalsJson"]=json!(expected.to_string());}
        }
        serde_json::from_value(value).unwrap()
    }
    #[test]fn json_transport_preserves_nested_values_and_reports_missing_separately_from_null(){
        let p=project("export default ctx=>ctx.services.register({id:'echo',title:'Echo'},input=>input);");
        let typed:Vec<ProbeCase>=serde_json::from_value(json!([{"serviceId":"echo","inputJson":"{\"items\":[1,true,null,[2,3]],\"flag\":false}","assertions":[{"path":"/items","equalsJson":"[1,true,null,[2,3]]"},{"path":"/flag","equalsJson":"false"},{"path":"/absent","equalsJson":"null"}]}])).unwrap();
        let r=probe(&p,&typed,BTreeMap::new(),json!({}),json!({})).unwrap();
        assert_eq!(r["cases"][0]["input"]["items"],json!([1,true,null,[2,3]]));
        assert_eq!(r["cases"][0]["assertions"][0]["passed"],true);assert_eq!(r["cases"][0]["assertions"][1]["passed"],true);
        assert_eq!(r["cases"][0]["assertions"][2]["actualType"],"missing");assert_eq!(r["passed"],false);
    }
    #[test]fn cold_catalog_and_simulated_save_survive_fresh_invocations(){
        let p=project("export default ctx=>ctx.ui.register({id:'settings',title:'Settings',metadata:{surface:'settings'}},async input=>{if(input.type==='save')return ctx.configuration.compareAndSet('order',null,input.order);if(input.type==='catalog')return ctx.ui.toolbars.describe('sample');return ctx.configuration.get('order');});");
        let catalog=json!({"surfaces":[{"id":"sample","items":[{"id":"line","title":"Divider","kind":"separator"},{"id":"new-button","title":"Actual label","kind":"button"}]}]});
        let report=probe(&p,&cases(json!([
            {"serviceId":"settings","input":{"type":"catalog"},"assertions":[{"path":"/items/0/kind","equals":"separator"},{"path":"/items/1/title","equals":"Actual label"}]},
            {"serviceId":"settings","input":{"type":"save","order":["new-button","line"]},"assertions":[{"path":"","equals":true}]},
            {"serviceId":"settings","input":{"type":"read"},"assertions":[{"path":"","equals":["new-button","line"]}]}
        ])),BTreeMap::new(),json!({}),catalog).unwrap();
        assert_eq!(report["passed"],true,"{report}");assert_eq!(report["installed"],false);
    }
    #[test]fn module_globals_do_not_mask_cross_invocation_bugs(){
        let p=project("let observed=[];export default ctx=>ctx.services.register({id:'state',title:'State'},input=>{if(input.items)observed=input.items;return observed;});");
        let report=probe(&p,&cases(json!([
            {"serviceId":"state","input":{"items":["actual"]},"assertions":[{"path":"","equals":["actual"]}]},
            {"serviceId":"state","input":{},"assertions":[{"path":"","equals":["actual"]}]}
        ])),BTreeMap::new(),json!({}),json!({})).unwrap();
        assert_eq!(report["passed"],false);assert_eq!(report["cases"][1]["result"],json!([]));
    }
    #[test]fn unsupported_side_effects_fail_even_if_the_plugin_catches_the_error(){
        let p=project("export default ctx=>ctx.services.register({id:'unsafe',title:'Unsafe'},async()=>{try{await ctx.host.call('network.fetch',{url:'https://example.invalid'});}catch{}return 'pretend success';});");
        let report=probe(&p,&cases(json!([{"serviceId":"unsafe","input":{},"assertions":[{"path":"","equals":"pretend success"}]}])),BTreeMap::new(),json!({}),json!({})).unwrap();
        assert_eq!(report["passed"],false);assert_eq!(report["cases"][0]["unavailableHostCalls"][0]["method"],"network.fetch");
    }
    #[test]fn failed_checks_are_bound_to_exact_project_bytes(){
        let root=tempfile::tempdir().unwrap();let ws=crate::Workspace::open(root.path()).unwrap();
        let saved=ws.save("probe",project("export default ctx=>ctx.services.register({id:'state',title:'State'},()=>false);"),None).unwrap();
        ws.probe_project("probe",&saved.sha256,&cases(json!([{"serviceId":"state","input":{},"assertions":[{"path":"","equals":true}]}])),BTreeMap::new(),json!({}),json!({})).unwrap();
        assert_eq!(ws.service_checks("probe",&saved.sha256).unwrap().unwrap()["passed"],false);
        assert!(ws.package("probe",&saved.sha256).is_err());
        let updated=ws.save("probe",project("export default ctx=>ctx.services.register({id:'state',title:'State'},()=>true);"),Some(&saved.sha256)).unwrap();
        assert!(ws.service_checks("probe",&updated.sha256).unwrap().is_none());
        assert!(!root.path().join("extensions.db").exists());
    }
    #[test]fn configuration_null_matches_the_production_insert_only_contract(){
        let p=project("export default ctx=>ctx.ui.register({id:'settings',title:'Settings',metadata:{surface:'settings'}},input=>ctx.configuration.compareAndSet(input.key,null,input.value));");
        let report=probe(&p,&cases(json!([
            {"serviceId":"settings","input":{"key":"order","value":null},"assertions":[{"path":"","equals":true}]},
            {"serviceId":"settings","input":{"key":"order","value":["copy"]},"assertions":[{"path":"","equals":false}]}
        ])),BTreeMap::new(),json!({}),json!({})).unwrap();
        assert_eq!(report["passed"],true);assert_eq!(report["configuration"]["order"],Value::Null);
    }
    #[test]fn unresolved_service_is_bounded_and_cannot_report_success(){
        let p=project("export default ctx=>ctx.services.register({id:'hang',title:'Hang'},()=>new Promise(()=>{}));");
        let started=std::time::Instant::now();
        let report=probe(&p,&cases(json!([{"serviceId":"hang","input":{},"assertions":[{"path":"","equals":true}]}])),BTreeMap::new(),json!({}),json!({})).unwrap();
        assert_eq!(report["passed"],false);assert!(report["cases"][0]["error"].is_string());
        assert!(started.elapsed()<Duration::from_secs(10));
    }
}
