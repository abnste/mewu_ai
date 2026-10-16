// SPDX-License-Identifier: MPL-2.0
use mewu_extension::*;
use serde_json::json;
use std::collections::BTreeMap;

fn manifest() -> ExtensionManifest {
    serde_json::from_value(json!({"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"independent.portable","version":"1.0.0","name":"Portable","description":"","license":"MPL-2.0","entry":"index.mjs"})).unwrap()
}
#[test]
fn one_unchanged_javascript_package_defaults_to_all_three_platforms() {
    let manifest = manifest();
    let modules = BTreeMap::from([("index.mjs".into(), "export default ctx=>ctx.memory.register({id:'recall',title:'Recall'},()=>[]);".into())]);
    validate_package(&manifest, &modules).unwrap();
    assert!(manifest.platforms.is_none());
    assert!(serde_json::to_value(&manifest).unwrap().get("platforms").is_none());
    for platform in PLUGIN_PLATFORMS {
        assert!(plugin_platform_supported(manifest.platforms.as_deref(), platform));
    }
    assert!(!plugin_platform_supported(None, "unknown"));
}
#[test]
fn explicit_platforms_are_a_bounded_allowlist_without_an_any_override() {
    let mut manifest = manifest();
    let modules = BTreeMap::from([("index.mjs".into(), String::new())]);
    manifest.platforms = Some(vec!["darwin-aarch64".into(), "linux-x86_64".into()]);
    validate_package(&manifest, &modules).unwrap();
    assert!(!plugin_platform_supported(manifest.platforms.as_deref(), "windows-x86_64"));
    assert!(plugin_platform_supported(manifest.platforms.as_deref(), "darwin-aarch64"));
    for invalid in [vec![], vec!["any", "windows-x86_64"], vec!["linux-x86_64", "linux-x86_64"], vec!["macos"], vec!["ANY"]] {
        manifest.platforms = Some(invalid.into_iter().map(String::from).collect());
        assert!(validate_package(&manifest, &modules).is_err());
        assert!(!plugin_platform_supported(manifest.platforms.as_deref(), "windows-x86_64"));
    }
}
