// SPDX-License-Identifier: MPL-2.0
use mewu_extension::{validate_module_graph, ExtensionManifest, ExtensionPackage};
use std::collections::BTreeMap;
fn package(source: &str, others: &[(&str, &str)]) -> ExtensionPackage {
    let manifest:ExtensionManifest=serde_json::from_value(serde_json::json!({"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"qa.graph","version":"1.0.0","name":"Graph","description":"","license":"MPL-2.0","entry":"index.mjs"})).unwrap();
    let mut modules = BTreeMap::from([("index.mjs".into(), source.into())]);
    for (name, source) in others {
        modules.insert((*name).into(), (*source).into());
    }
    ExtensionPackage { manifest, modules }
}
#[test]
fn missing_static_import_and_reexport_fail_before_activation() {
    for source in [
        "import {html} from './view.mjs';export default ()=>html;",
        "export {html} from './view.mjs';",
    ] {
        assert!(validate_module_graph(&package(source, &[])).is_err());
        assert!(validate_module_graph(&package(
            source,
            &[("view.mjs", "export const html='ok';")]
        ))
        .is_ok());
    }
}
#[test]
fn parsing_and_linking_never_evaluate_module_bodies_or_jobs() {
    let p = package(
        "import './view.mjs'; while(true){};export default ()=>{};",
        &[(
            "view.mjs",
            "throw new Error('must not execute');Promise.resolve().then(()=>{while(true){}});",
        )],
    );
    assert!(validate_module_graph(&p).is_ok());
}
#[test]
fn invalid_module_syntax_is_rejected_before_any_evaluation() {
    assert!(validate_module_graph(&package("export const = ;", &[])).is_err());
}
#[test]
fn real_parser_distinguishes_strings_comments_cycles_and_sdk_imports() {
    assert!(validate_module_graph(&package("import {definePlugin} from '@mewu/sdk';// import './missing.mjs'\nconst s=\"import './missing2.mjs'\"; export default definePlugin(()=>s);",&[])).is_ok());
    assert!(validate_module_graph(&package(
        "import {b} from './b.mjs';export const a=1;export default ()=>b;",
        &[(
            "b.mjs",
            "import {a} from './index.mjs';export const b=()=>a;"
        )]
    ))
    .is_ok());
}
