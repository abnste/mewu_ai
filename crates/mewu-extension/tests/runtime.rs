// SPDX-License-Identifier: MPL-2.0
use mewu_extension::*;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

fn manifest() -> ExtensionManifest {
    serde_json::from_value(json!({"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"external.demo","version":"1.0.0","name":"Demo","description":"","license":"MPL-2.0","entry":"index.js"})).unwrap()
}
fn runtime(
    source: &str,
    modules: &[(&str, &str)],
    limits: RuntimeLimits,
) -> (
    ExtensionRuntime,
    mpsc::Receiver<WorkerResponse>,
    Arc<AtomicBool>,
) {
    let mut contents = BTreeMap::from([("index.js".into(), source.into())]);
    for (name, source) in modules {
        contents.insert(name.to_string(), source.to_string());
    }
    let (tx, rx) = mpsc::channel();
    let canceled = Arc::new(AtomicBool::new(false));
    let runtime = ExtensionRuntime::new(
        ExtensionPackage {
            manifest: manifest(),
            modules: contents,
        },
        limits,
        canceled.clone(),
        Arc::new(move |message| tx.send(message).map_err(|_| ExtensionError::Transport)),
    )
    .unwrap();
    (runtime, rx, canceled)
}
fn initialize(runtime: &mut ExtensionRuntime) -> Result<Vec<ServiceRegistration>, ExtensionError> {
    loop {
        if let Some(result) = runtime.poll_initialization()? {
            return Ok(result);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn result(runtime: &ExtensionRuntime) -> Result<Value, ExtensionError> {
    loop {
        if let Some(result) = runtime.poll_result()? {
            return Ok(result);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn all_nine_service_kinds_execute_third_party_handlers() {
    let (mut runtime, _, _) = runtime("export default ctx => { for (const key of ['commands','tools','agents','memory','models','channels','renderers','ui','services']) ctx[key].register({id:key,title:key}, input => ({kind:key,input})); }", &[], RuntimeLimits::default());
    let registrations = initialize(&mut runtime).unwrap();
    assert_eq!(registrations.len(), 9);
    for service in registrations {
        runtime
            .begin_invoke(&service.id, &json!({"hello": "世界"}), &json!({}))
            .unwrap();
        assert_eq!(
            result(&runtime).unwrap(),
            json!({"kind":service.id,"input":{"hello":"世界"}})
        );
    }
}

#[test]
fn portable_runtime_preserves_feature_platforms_and_rejects_malformed_restrictions() {
    let source = "export default ctx=>{ctx.ui.register({id:'ruler',title:'Ruler'},()=>42);ctx.tools.register({id:'native',title:'Native',metadata:{platforms:['windows-x86_64'],requiresRegion:true}},()=>43);}";
    let (mut instance, _, _) = runtime(source, &[], RuntimeLimits::default());
    let services = initialize(&mut instance).unwrap();
    assert_eq!(services.len(), 2);
    assert!(service_platforms(&services[0]).unwrap().is_none());
    assert_eq!(service_platforms(&services[1]).unwrap().unwrap(), vec!["windows-x86_64"]);
    for metadata in [json!({"platforms":[]}), json!({"platforms":["any","linux-x86_64"]}), json!({"platforms":"windows-x86_64"}), json!({"requiresRegion":"true"})] {
        let source = format!("export default ctx=>ctx.tools.register({{id:'native',title:'Native',metadata:{metadata}}},()=>42);");
        let (mut instance, _, _) = runtime(&source, &[], RuntimeLimits::default());
        assert!(initialize(&mut instance).is_err());
    }
}

#[test]
fn genuine_async_host_roundtrip_retains_multimodal_json_and_context() {
    let (mut runtime, rx, _) = runtime("export default ctx => ctx.tools.register({id:'observe',title:'Observe'},async (input,run) => { const observation=await ctx.region.observe(run.context.regionId); const answer=await ctx.model.invoke({observation,prompt:input.prompt}); return {answer,observation}; });", &[], RuntimeLimits::default());
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke(
            "observe",
            &json!({"prompt":"read"}),
            &json!({"regionId":"region-42"}),
        )
        .unwrap();
    assert!(runtime.poll_result().unwrap().is_none());
    let call = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let WorkerResponse::HostCall { id, method, params } = call else {
        panic!()
    };
    assert_eq!(method, "region.observe");
    assert_eq!(params, json!({"regionId":"region-42"}));
    let observation = json!({"content":[{"type":"text","text":"ready"},{"type":"image","artifactId":"image-7","width":640,"height":480}]});
    runtime
        .host_result(&id, Some(observation.clone()), None)
        .unwrap();
    assert!(runtime.poll_result().unwrap().is_none());
    let WorkerResponse::HostCall { id, method, params } =
        rx.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!()
    };
    assert_eq!(method, "model.invoke");
    assert_eq!(params["observation"], observation);
    runtime
        .host_result(&id, Some(json!({"action":"click","x":20,"y":30})), None)
        .unwrap();
    assert_eq!(
        result(&runtime).unwrap(),
        json!({"answer":{"action":"click","x":20,"y":30},"observation":observation})
    );
}

#[test]
fn transient_overlay_document_roundtrips_verbatim_through_the_host_bridge() {
    let (mut runtime, rx, _) = runtime("export default ctx => ctx.commands.register({id:'overlay',title:'Overlay'},async (input,run) => ({surface:await ctx.ui.transient({html:input.html,width:260,height:260})}));", &[], RuntimeLimits::default());
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke("overlay", &json!({"html":"<svg id=\"crosshair\"></svg>"}), &json!({"sceneId":"scene-1"}))
        .unwrap();
    // The handler starts in a microtask; pump once before awaiting its call.
    assert!(runtime.poll_result().unwrap().is_none());
    let WorkerResponse::HostCall { id, method, params } =
        rx.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("expected ui.transient host call")
    };
    assert_eq!(method, "ui.transient");
    assert_eq!(params, json!({"html":"<svg id=\"crosshair\"></svg>","width":260,"height":260}));
    runtime
        .host_result(&id, Some(json!({"surfaceId":"0198f3c2-7d5a-4c8e-9f3f-08d91f3a1111"})), None)
        .unwrap();
    assert_eq!(
        result(&runtime).unwrap(),
        json!({"surface":{"surfaceId":"0198f3c2-7d5a-4c8e-9f3f-08d91f3a1111"}})
    );
}

#[test]
fn package_relative_and_sdk_imports_work() {
    let (mut runtime, _, _) = runtime("import {definePlugin} from '@mewu/sdk'; import {value} from './lib/value.mjs'; export default definePlugin(ctx => ctx.commands.register({id:'read',title:'Read'},()=>value));", &[("lib/value.mjs","import {base} from '../base.js'; export const value=base+1;"),("base.js","export const base=41;")], RuntimeLimits::default());
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke("read", &Value::Null, &Value::Null)
        .unwrap();
    assert_eq!(result(&runtime).unwrap(), 42);
}

#[test]
fn imports_cannot_escape_package_or_load_os_url_native_modules() {
    for specifier in [
        "../outside.js",
        "./../outside.js",
        "/etc/passwd",
        "file:///etc/passwd",
        "https://evil.invalid/x.js",
        "std",
        "os",
        "node:fs",
        "C:/private.js",
    ] {
        let source = format!(
            "import value from {}; export default () => value;",
            serde_json::to_string(specifier).unwrap()
        );
        let package = ExtensionPackage {
            manifest: manifest(),
            modules: BTreeMap::from([("index.js".into(), source)]),
        };
        let result = ExtensionRuntime::new(
            package,
            RuntimeLimits::default(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| Ok(())),
        );
        match result {
            Err(_) => {}
            Ok(mut runtime) => assert!(initialize(&mut runtime).is_err(), "specifier {specifier}"),
        }
    }
}

#[test]
fn no_process_filesystem_network_globals_are_installed() {
    let (mut runtime, _, _) = runtime("export default ctx => ctx.commands.register({id:'read',title:'Read'},()=>['process','require','fetch','WebSocket','XMLHttpRequest','Deno','Bun','std','os'].map(name=>typeof globalThis[name]));", &[], RuntimeLimits::default());
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke("read", &Value::Null, &Value::Null)
        .unwrap();
    assert_eq!(result(&runtime).unwrap(), json!(vec!["undefined"; 9]));
}

#[test]
fn synchronous_infinite_loop_is_interrupted_by_deadline() {
    let limits = RuntimeLimits {
        request_timeout: Duration::from_millis(40),
        ..Default::default()
    };
    let (mut runtime, _, _) = runtime("export default ctx => ctx.commands.register({id:'loop',title:'Loop'},()=>{while(true){}});", &[], limits);
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke("loop", &Value::Null, &Value::Null)
        .unwrap();
    assert!(matches!(result(&runtime), Err(ExtensionError::Timeout)));
}

#[test]
fn synchronous_infinite_loop_is_interrupted_by_cancel_signal() {
    let (mut runtime, _, canceled) = runtime("export default ctx => ctx.commands.register({id:'loop',title:'Loop'},()=>{while(true){}});", &[], RuntimeLimits::default());
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke("loop", &Value::Null, &Value::Null)
        .unwrap();
    let canceler = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        canceled.store(true, Ordering::Release);
    });
    assert!(matches!(result(&runtime), Err(ExtensionError::Canceled)));
    canceler.join().unwrap();
}

#[test]
fn unresolved_promise_times_out_and_host_response_ids_are_checked() {
    let (mut runtime, _, _) = runtime("export default ctx => ctx.commands.register({id:'wait',title:'Wait'},()=>new Promise(()=>{}));", &[], RuntimeLimits { request_timeout: Duration::from_millis(30), ..Default::default() });
    initialize(&mut runtime).unwrap();
    assert!(runtime
        .host_result("unowned", Some(Value::Null), None)
        .is_err());
    runtime
        .begin_invoke("wait", &Value::Null, &Value::Null)
        .unwrap();
    assert!(matches!(result(&runtime), Err(ExtensionError::Timeout)));
}

#[test]
fn activation_can_await_host_longer_than_short_service_budget_then_loop_still_times_out() {
    let limits = RuntimeLimits {
        initialization_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_millis(30),
        ..Default::default()
    };
    let (mut runtime, rx, _) = runtime("export default async ctx => { await ctx.storage.get('activation-state'); ctx.commands.register({id:'loop',title:'Loop'},()=>{while(true){}}); };", &[], limits);
    assert!(runtime.poll_initialization().unwrap().is_none());
    let WorkerResponse::HostCall { id, method, .. } =
        rx.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("expected initialization host call")
    };
    assert_eq!(method, "storage.get");
    // The extension is genuinely pending a host response across more than two
    // service budgets. This is not an artificial sleep before activation starts.
    std::thread::sleep(Duration::from_millis(90));
    runtime.host_result(&id, Some(Value::Null), None).unwrap();
    assert_eq!(initialize(&mut runtime).unwrap().len(), 1);
    let started = Instant::now();
    runtime
        .begin_invoke("loop", &Value::Null, &Value::Null)
        .unwrap();
    assert!(matches!(result(&runtime), Err(ExtensionError::Timeout)));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "service must not inherit the five-second activation budget"
    );
}

#[test]
fn untrusted_activation_remains_bounded_by_its_own_deadline() {
    let limits = RuntimeLimits {
        initialization_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(5),
        ..Default::default()
    };
    let (mut runtime, _, _) = runtime("export default () => { while(true) {} };", &[], limits);
    let started = Instant::now();
    assert!(matches!(
        initialize(&mut runtime),
        Err(ExtensionError::Timeout)
    ));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "activation must not inherit the five-second service budget"
    );
}

#[test]
fn plugin_exceptions_do_not_expose_source_or_secrets() {
    let (mut runtime, _, _) = runtime("export default ctx => ctx.commands.register({id:'bad',title:'Bad'},()=>{throw new Error('secret-api-key-123');});", &[], RuntimeLimits::default());
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke("bad", &Value::Null, &Value::Null)
        .unwrap();
    let error = result(&runtime).unwrap_err();
    assert!(!error.to_string().contains("secret"));
}

#[test]
fn malformed_registration_and_duplicate_ids_fail_activation() {
    for source in ["export default ctx=>ctx.tools.register({id:'bad',title:'Bad',privateKey:'secret'},()=>0);", "export default ctx=>{ctx.tools.register({id:'a',title:'A'},()=>0);ctx.commands.register({id:'a',title:'A'},()=>0);}"] {
        let (mut runtime, _, _) = runtime(source, &[], RuntimeLimits::default()); assert!(initialize(&mut runtime).is_err());
    }
}

#[test]
fn memory_limit_bounds_extension_allocations() {
    let (mut runtime, _, _) = runtime("export default ctx=>ctx.commands.register({id:'oom',title:'OOM'},()=>new Array(10000000).fill('memory'));", &[], RuntimeLimits { memory_bytes: 4 * 1024 * 1024, ..Default::default() });
    initialize(&mut runtime).unwrap();
    runtime
        .begin_invoke("oom", &Value::Null, &Value::Null)
        .unwrap();
    assert!(result(&runtime).is_err());
}

#[test]
fn deactivation_revokes_service_handlers() {
    let (mut runtime, _, _) = runtime(
        "export default ctx=>ctx.commands.register({id:'a',title:'A'},()=>42);",
        &[],
        RuntimeLimits::default(),
    );
    initialize(&mut runtime).unwrap();
    runtime.deactivate().unwrap();
    assert!(runtime
        .begin_invoke("a", &Value::Null, &Value::Null)
        .is_err());
}

#[test]
fn manifest_and_module_limits_reject_malformed_packages() {
    let mut value = serde_json::to_value(manifest()).unwrap();
    value["execute"] = json!("anything");
    assert!(serde_json::from_value::<ExtensionManifest>(value).is_err());
    for entry in [
        "../index.js",
        "/index.js",
        "C:/index.js",
        "CON.js",
        "dir//index.js",
        "dir/../index.js",
        "index.exe",
        "a?.js",
    ] {
        let mut manifest = manifest();
        manifest.entry = entry.into();
        let modules = BTreeMap::from([(entry.into(), "export default ()=>{}".into())]);
        assert!(validate_package(&manifest, &modules).is_err(), "{entry}");
    }
    let modules = BTreeMap::from([
        ("index.js".into(), "export default ()=>{}".into()),
        ("INDEX.js".into(), "export default ()=>{}".into()),
    ]);
    assert!(validate_package(&manifest(), &modules).is_err());
    let modules = BTreeMap::from([("index.js".into(), "\u{feff}export default ()=>{}".into())]);
    assert!(validate_package(&manifest(), &modules).is_err());
}
