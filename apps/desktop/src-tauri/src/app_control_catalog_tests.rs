// SPDX-License-Identifier: MPL-2.0
// Included as a child of app_control_host. These are real JSON Schema validators,
// not a frontend mock validator or a string search of source code.
use serde_json::{json, Value};
use std::collections::HashSet;

fn catalog() -> Value {
    serde_json::from_slice(include_bytes!("../../control/catalog.json")).expect("valid UTF-8 control catalog")
}

fn schema(catalog: &Value, part: &Value) -> Value {
    let mut schema = part.clone();
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["$defs"] = catalog["$defs"].clone();
    schema
}

#[test]
fn catalog_covers_exact_host_operations_and_queries_with_valid_example_inputs() {
    let catalog = catalog();
    assert_eq!(catalog["protocolVersion"], mewu_control::PROTOCOL_VERSION);
    assert_eq!(catalog["pluginId"], "mewu.cli");
    assert_eq!(catalog["automaticRetries"], false);
    let commands = catalog["commands"].as_array().unwrap();
    let operations: HashSet<_> = super::METHODS.iter().copied().collect();
    let queries: HashSet<_> = ["app.status", "app.schema", "app.capabilities", "scene.list", "scene.get", "run.get", "agent.list", "model.list", "plugin.list", "recording.status", "operation.get"].into_iter().collect();
    let mut seen = HashSet::new();
    for command in commands {
        let method = command["method"].as_str().unwrap();
        assert!(seen.insert(method), "duplicate method {method}");
        let operation = operations.contains(method);
        assert!(operation || queries.contains(method), "unimplemented method {method}");
        assert_eq!(command["execution"], if operation { "operation" } else { "query" });
        assert_eq!(command["asyncOperation"], operation);
        assert!(!command["description"].as_str().unwrap().is_empty());
        let input = schema(&catalog, &command["inputSchema"]);
        let validator = jsonschema::validator_for(&input).unwrap_or_else(|error| panic!("{method}: invalid schema: {error}"));
        assert_eq!(input["additionalProperties"], false, "unknown method arguments must be rejected for {method}");
        jsonschema::validator_for(&schema(&catalog, &command["resultSchema"]))
            .unwrap_or_else(|error| panic!("{method}: invalid result schema: {error}"));
        for example in command["examples"].as_array().filter(|values| !values.is_empty()).expect("useful examples") {
            assert_eq!(example["request"]["method"], method);
            let request: mewu_control::Request = serde_json::from_value(example["request"].clone()).expect("real request envelope");
            request.validate().expect("real protocol ID/method validation");
            let errors: Vec<_> = validator.iter_errors(&request.params).map(|error| error.to_string()).collect();
            assert!(errors.is_empty(), "{method}: example does not match schema: {errors:?}");
            let mut unknown = request.params.clone();
            unknown["arbitraryTauriCommand"] = json!("not permitted");
            assert!(!validator.is_valid(&unknown), "{method}: unknown field admitted");
        }
    }
    let expected: HashSet<_> = operations.union(&queries).copied().collect();
    assert_eq!(seen, expected);
    assert!(serde_json::to_vec(&catalog).unwrap().len() <= super::MAX_RESULT);
}

#[test]
fn catalog_real_drawing_schema_rejects_fake_rich_layouts_provenance_and_stale_wire_shapes() {
    let catalog = catalog();
    let validator = jsonschema::validator_for(&schema(&catalog, &json!({"$ref":"#/$defs/Drawing"}))).unwrap();
    let valid = json!({
        "id":"00000000-0000-4000-8000-000000000007", "kind":"text", "color":"#FF3B30", "strokeWidth":3,
        "points":[{"x":20,"y":30},{"x":320,"y":80}], "text":"第一行\n第二行", "fontSize":24,
        "textStyle":{"bold":true,"underline":true}
    });
    assert!(validator.is_valid(&valid));
    serde_json::from_value::<mewu_core::Drawing>(valid.clone()).unwrap();
    for changed in [
        json!({"kind":"rich","rich":{"layoutId":"forged"}}),
        json!({"origin":{"runId":"forged"}}),
        json!({"id":"not-a-uuid"}),
        json!({"color":"white"}),
        json!({"strokeWidth":0}),
        json!({"textStyle":{"bold":true,"extra":true}}),
        json!({"points":[{"x":-1,"y":0}]}),
        json!({"points":[{"x":0,"y":0},{"x":1,"y":1},{"x":2,"y":2}]}),
    ] {
        let mut bad = valid.clone();
        for (key, value) in changed.as_object().unwrap() { bad[key] = value.clone(); }
        assert!(!validator.is_valid(&bad), "accepted invalid drawing input {changed}");
    }
    let command = jsonschema::validator_for(&schema(&catalog, &json!({"$ref":"#/$defs/DrawingCommand"}))).unwrap();
    let mut edit = json!({"type":"add_drawing","sceneId":"00000000-0000-4000-8000-000000000001","regionId":"00000000-0000-4000-8000-000000000003","backgroundId":"00000000-0000-4000-8000-000000000002","expectedRevision":0,"drawing":valid});
    assert!(command.is_valid(&edit));
    serde_json::from_value::<mewu_core::SceneCommand>(edit.clone()).unwrap();
    edit.as_object_mut().unwrap().remove("expectedRevision");
    assert!(!command.is_valid(&edit), "drawing commands must require an exact CAS revision");
}

#[test]
fn catalog_context_references_and_query_bounds_are_closed_and_do_not_grant_authority() {
    let catalog = catalog();
    for (name, valid, invalid) in [
        ("Context", json!({"sceneId":"00000000-0000-4000-8000-000000000001","revision":8}), json!({"sceneId":"00000000-0000-4000-8000-000000000001","permission":"all"})),
        ("Refs", json!([{"kind":"region","id":"00000000-0000-4000-8000-000000000003"}]), json!([{"kind":"region","id":"00000000-0000-4000-8000-000000000003","path":"/private"}])),
    ] {
        let validator = jsonschema::validator_for(&schema(&catalog, &json!({"$ref":format!("#/$defs/{name}")}))).unwrap();
        assert!(validator.is_valid(&valid)); assert!(!validator.is_valid(&invalid));
    }
    for name in ["scene.list", "scene.get", "run.get"] {
        let command = catalog["commands"].as_array().unwrap().iter().find(|command| command["method"] == name).unwrap();
        let validator = jsonschema::validator_for(&schema(&catalog, &command["inputSchema"])).unwrap();
        let mut params = command["examples"][0]["request"]["params"].clone();
        params["limit"] = json!(101); assert!(!validator.is_valid(&params));
        params["limit"] = json!(0); assert!(!validator.is_valid(&params));
    }
}
