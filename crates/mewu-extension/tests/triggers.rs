// SPDX-License-Identifier: MPL-2.0
use mewu_extension::*;
use serde_json::json;

fn registration(metadata: serde_json::Value) -> ServiceRegistration {
    serde_json::from_value(json!({"kind":"trigger","id":"example.hold","title":"Hold","metadata":metadata})).unwrap()
}
#[test]
fn modifier_codes_and_combinations_pass_contract_validation() {
    for metadata in [
        json!({"trigger":{"code":"Alt"}}),
        json!({"trigger":{"code":"Control"}}),
        json!({"trigger":{"code":"Shift"}}),
        json!({"trigger":{"code":"Alt","with":[]}}),
        json!({"trigger":{"code":"Control","with":["Alt"]}}),
        json!({"trigger":{"code":"Shift","with":["Ctrl","Alt"]}}),
        json!({"platforms":["any"],"trigger":{"code":"Alt"}}),
    ] {
        validate_trigger(&registration(metadata.clone())).unwrap();
    }
    // The check only constrains trigger registrations.
    let other: ServiceRegistration =
        serde_json::from_value(json!({"kind":"command","id":"c","title":"C"})).unwrap();
    validate_trigger(&other).unwrap();
}
#[test]
fn letters_digits_and_repeated_modifiers_fail_contract_validation() {
    for metadata in [
        json!({}),
        json!({"trigger":"Alt"}),
        json!({"trigger":{}}),
        json!({"trigger":{"code":"KeyA"}}),
        json!({"trigger":{"code":"a"}}),
        json!({"trigger":{"code":"Enter"}}),
        json!({"trigger":{"code":true}}),
        json!({"trigger":{"code":"Alt","with":"Ctrl"}}),
        json!({"trigger":{"code":"Alt","with":[42]}}),
        json!({"trigger":{"code":"Alt","with":["Ctrl","Ctrl"]}}),
        json!({"trigger":{"code":"Alt","with":["Alt"]}}),
        json!({"trigger":{"code":"Control","with":["Ctrl","Shift"]}}),
    ] {
        assert!(
            validate_trigger(&registration(metadata.clone())).is_err(),
            "metadata must be rejected: {metadata}"
        );
    }
    let missing: ServiceRegistration =
        serde_json::from_value(json!({"kind":"trigger","id":"example.hold","title":"Hold"})).unwrap();
    let error = validate_trigger(&missing).unwrap_err();
    assert!(
        error.to_string().contains("触发器缺少按键描述"),
        "unexpected message: {error}"
    );
}
