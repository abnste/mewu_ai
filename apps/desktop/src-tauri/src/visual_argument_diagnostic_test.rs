// SPDX-License-Identifier: MPL-2.0
//! Production parser/receipt paths, synthetic inputs and isolated SQLite only.
use super::*;
use mewu_core::*;
use rusqlite::Connection;

fn rejection(scope: &RunScope, value: &Value) -> Rejection {
    scope
        .parse_plan_diagnosed(&value.to_string())
        .err()
        .unwrap()
}
fn diagnostic(error: &Rejection, field: &str, reason: DiagnosticReason) {
    let actual = error
        .diagnostic
        .as_ref()
        .expect("safe fixed field diagnostic");
    assert_eq!(actual.field_path, field);
    assert_eq!(actual.reason, reason);
    let encoded = serde_json::to_string(actual).unwrap();
    assert!(!encoded.contains("private-secret"));
    assert!(!error.message.contains("private-secret"));
}
fn object(value: &mut Value) -> &mut Value {
    &mut value["groups"][0]["objects"][0]
}

#[test]
fn table_version_type_is_advertised_and_rejection_teaches_the_actual_contract() {
    let scope = tests::scope(tests::normalized_input());
    let mut args = tests::normalized_args(&scope.input);
    *object(&mut args) = json!({"kind":"table","points":[{"x":0,"y":0},{"x":1000,"y":1000}],
        "table":{"version":"1","header":["x","2x+3"],"rows":[["1","5"],["4","11"]],"align":["left","right"]}});
    let error = rejection(&scope, &args);
    diagnostic(&error, "$/groups/0/objects/0/table/version", DiagnosticReason::Type);
    assert_eq!(error.diagnostic.as_ref().unwrap().expected_type, Some(ArgumentExpectedType::Integer));
    let mut fixture = ReceiptFixture::new();
    let (_, output) = reject_arguments(&mut fixture.store, &fixture.lease, &error, &scope.input).unwrap();
    let feedback: Value = serde_json::from_str(&output.model_json).unwrap();
    assert_eq!(feedback["executed"], false);
    assert_eq!(feedback["tableSchema"]["properties"]["version"]["type"], "integer");
    assert_eq!(feedback["tableSchema"]["properties"]["version"]["const"], 1);
    let schema = scope.definition();
    let validator = jsonschema::validator_for(&schema["function"]["parameters"]).unwrap();
    assert!(!validator.is_valid(&args));
    object(&mut args)["table"]["version"] = json!(1);
    assert!(validator.is_valid(&args));
    assert!(scope.parse_plan_diagnosed(&args.to_string()).is_ok());
    // No coercion of unsupported versions, lossy numbers, or unrelated fields.
    for invalid in [json!(2), json!(1.5), json!(null), json!({"private-secret":1})] {
        object(&mut args)["table"]["version"] = invalid;
        assert!(scope.parse_plan_diagnosed(&args.to_string()).is_err());
    }
}

#[test]
fn visual_diagnostics_locate_points_triangle_style_and_formula_without_raw_values() {
    let scope = tests::scope(tests::normalized_input());
    let original = tests::normalized_args(&scope.input);
    assert!(scope.parse_plan_diagnosed(&original.to_string()).is_ok());
    let mut points = original.clone();
    object(&mut points)["points"] = json!({"private-secret":"not an array"});
    let error = rejection(&scope, &points);
    diagnostic(
        &error,
        "$/groups/0/objects/0/points",
        DiagnosticReason::Type,
    );
    assert_eq!(
        error.diagnostic.unwrap().expected_type,
        Some(ArgumentExpectedType::Array)
    );
    let mut triangle = original.clone();
    object(&mut triangle)["kind"] = json!("triangle");
    diagnostic(
        &rejection(&scope, &triangle),
        "$/groups/0/objects/0/kind",
        DiagnosticReason::Enum,
    );
    let mut style = original.clone();
    object(&mut style)["style"] = json!({"private-secret":"not an allowed field"});
    diagnostic(
        &rejection(&scope, &style),
        "$/groups/0/objects/0",
        DiagnosticReason::AdditionalProperties,
    );
    let mut formula = original.clone();
    *object(&mut formula) = json!({"kind":"formula","points":[{"x":100,"y":100},{"x":900,"y":900}],"tex":"private-secret","color":"#123456"});
    diagnostic(
        &rejection(&scope, &formula),
        "$/groups/0/objects/0/fontSize",
        DiagnosticReason::Required,
    );
    object(&mut formula)["fontSize"] = json!(16);
    object(&mut formula)["strokeWidth"] = json!(1);
    diagnostic(
        &rejection(&scope, &formula),
        "$/groups/0/objects/0",
        DiagnosticReason::AdditionalProperties,
    );
    object(&mut formula)
        .as_object_mut()
        .unwrap()
        .remove("strokeWidth");
    assert!(scope.parse_plan_diagnosed(&formula.to_string()).is_ok());
}

#[test]
fn visual_diagnostics_choose_the_actual_target_and_kind_union_branches() {
    let mut manifest = tests::normalized_input().manifest().clone();
    let mut second = manifest.targets[0].clone();
    second.handle = uuid::Uuid::new_v4().to_string();
    second.attachment_id = uuid::Uuid::new_v4().to_string();
    second.attachment_ordinal = 1;
    second.region_id = uuid::Uuid::new_v4().to_string();
    manifest.targets.push(second.clone());
    let scope = tests::scope(VerifiedVisualInput::new(manifest).unwrap());
    let mut args = tests::normalized_args(&scope.input);
    let mut group = args["groups"][0].clone();
    group["target"] = json!(second.handle);
    group["styleUnit"] = json!(17);
    args["groups"].as_array_mut().unwrap().push(group);
    diagnostic(
        &rejection(&scope, &args),
        "$/groups/1/styleUnit",
        DiagnosticReason::Type,
    );
    args["groups"][1]["styleUnit"] = json!("encodedPixels");
    args["groups"][1]["objects"][0]["fontSize"] = json!({"private-secret":12});
    let error = rejection(&scope, &args);
    diagnostic(
        &error,
        "$/groups/1/objects/0/fontSize",
        DiagnosticReason::Type,
    );
    assert_eq!(
        error.diagnostic.unwrap().expected_type,
        Some(ArgumentExpectedType::Number)
    );
}

#[test]
fn visual_diagnostics_preserve_semantic_units_geometry_and_rich_limits() {
    let scope = tests::scope(tests::normalized_input());
    let good = tests::normalized_args(&scope.input);
    let mut missing = good.clone();
    missing["groups"][0]
        .as_object_mut()
        .unwrap()
        .remove("coordinateSpace");
    diagnostic(
        &rejection(&scope, &missing),
        "$/groups/0/coordinateSpace",
        DiagnosticReason::Required,
    );
    let mut bounds = good.clone();
    object(&mut bounds)["points"][0]["x"] = json!(1001);
    diagnostic(
        &rejection(&scope, &bounds),
        "$/groups/0/objects/0/points/0/x",
        DiagnosticReason::Data,
    );
    let mut line = good.clone();
    *object(&mut line) = json!({"kind":"line","points":[{"x":10,"y":20},{"x":10,"y":20}],"color":"#123456","strokeWidth":1});
    diagnostic(
        &rejection(&scope, &line),
        "$/groups/0/objects/0/points",
        DiagnosticReason::Data,
    );
    let mut table = good.clone();
    *object(&mut table) = json!({"kind":"table","points":[{"x":0,"y":0},{"x":1000,"y":1000}],"table":{"version":1,"header":["A","B"],"rows":[["private-secret"]],"align":["left","right"]}});
    diagnostic(
        &rejection(&scope, &table),
        "$/groups/0/objects/0/table",
        DiagnosticReason::Data,
    );
    let mut pen = good;
    *object(&mut pen) = json!({"kind":"pen","points":[{"x":10,"y":10},{"x":20,"y":10},{"x":15,"y":20},{"x":10,"y":10}],"color":"#123456","strokeWidth":1});
    assert!(
        scope.parse_plan_diagnosed(&pen.to_string()).is_ok(),
        "A supported pen path can draw a closed triangle without inventing a new kind"
    );
}

#[test]
fn visual_diagnostics_respect_pixel_profiles_without_guessing_normalized_axes() {
    for (width, height, x, y) in [(400, 200, 450., 30.), (2000, 2000, 1500., 2001.)] {
        let mut manifest = tests::input().manifest().clone();
        manifest.targets[0].encoded = VisualPixelSize { width, height };
        let scope = tests::scope(VerifiedVisualInput::new(manifest).unwrap());
        let mut args = tests::args(&scope.input);
        object(&mut args)["points"][0] = json!({"x":x,"y":y});
        diagnostic(
            &rejection(&scope, &args),
            "$/groups/0/objects/0/points/0",
            DiagnosticReason::Data,
        );
    }
    let scope = tests::scope(tests::normalized_input());
    for (axis, x, y) in [("x", 1001., 10.), ("y", 10., 1001.)] {
        let mut args = tests::normalized_args(&scope.input);
        object(&mut args)["points"][0] = json!({"x":x,"y":y});
        diagnostic(
            &rejection(&scope, &args),
            &format!("$/groups/0/objects/0/points/0/{axis}"),
            DiagnosticReason::Data,
        );
    }
}

#[test]
fn visual_diagnostics_apply_after_the_bounded_minimax_wire_adapter() {
    let profile = ConnectionProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "synthetic".into(),
        provider_id: "minimax-cn".into(),
        base_url: "https://api.minimax.cn/v1".into(),
        model: "MiniMax-M3".into(),
        has_key: false,
        credential_id: None,
        revision: 1,
        advanced: Default::default(),
    };
    let scope = tests::scope(tests::normalized_input()).for_connection(&profile);
    let mut wire = tests::normalized_args(&scope.input);
    object(&mut wire)["fontSize"] = json!("16");
    object(&mut wire)["strokeWidth"] = json!("1");
    object(&mut wire)["points"] = json!({"item":{"x":"100","y":"100"}});
    assert!(scope.parse_plan_diagnosed(&wire.to_string()).is_ok());
    wire["groups"][0]["styleUnit"] = json!(17);
    let envelope_error = rejection(&scope, &wire);
    diagnostic(
        &envelope_error,
        "$/groups/0/styleUnit",
        DiagnosticReason::Type,
    );
    assert_eq!(
        envelope_error.diagnostic.unwrap().expected_type,
        Some(ArgumentExpectedType::String)
    );
    wire["groups"][0]["styleUnit"] = json!("encodedPixels");
    let mut unknown_unit = wire.clone();
    unknown_unit["groups"][0]["styleUnit"] = json!("private-secret");
    assert!(
        rejection(&scope, &unknown_unit).diagnostic.is_none(),
        "Envelope string/enum failure must not be guessed from an accepted MiniMax object shape"
    );
    object(&mut wire)["fontSize"] = json!("private-secret");
    diagnostic(
        &rejection(&scope, &wire),
        "$/groups/0/objects/0/fontSize",
        DiagnosticReason::Data,
    );
    object(&mut wire)["fontSize"] = json!("16");
    object(&mut wire)["points"]["item"]["x"] = json!("1001");
    diagnostic(
        &rejection(&scope, &wire),
        "$/groups/0/objects/0/points/0/x",
        DiagnosticReason::Data,
    );
}

#[test]
fn visual_diagnostics_do_not_invent_duplicate_fields_or_oversized_details() {
    let scope = tests::scope(tests::normalized_input());
    let good = tests::normalized_args(&scope.input).to_string();
    let duplicate = good.replacen('{', "{\"groups\":[],", 1);
    assert!(scope
        .parse_plan_diagnosed(&duplicate)
        .err()
        .unwrap()
        .diagnostic
        .is_none());
    let mut also_invalid = tests::normalized_args(&scope.input);
    also_invalid["groups"][0]["styleUnit"] = json!(17);
    let duplicate_with_other_error = also_invalid.to_string().replacen('{', "{\"groups\":[],", 1);
    assert!(scope.parse_plan_diagnosed(&duplicate_with_other_error).err().unwrap().diagnostic.is_none(), "The collapsed second group must not be reported as the cause of the earlier duplicate-field rejection");
    assert!(scope
        .parse_plan_diagnosed(&" ".repeat(32 * 1024 + 1))
        .err()
        .unwrap()
        .diagnostic
        .is_none());
    let error = scope.parse_plan_diagnosed("{\"groups\":[").err().unwrap();
    diagnostic(&error, "$", DiagnosticReason::Eof);
}

struct ReceiptFixture {
    root: PathBuf,
    store: Store,
    scene: String,
    run: String,
    lease: DispatchLease,
}
impl ReceiptFixture {
    fn new() -> Self {
        let uid = || uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("mewu-visual-diagnostic-{}", uid()));
        std::fs::create_dir(&root).unwrap();
        let mut store = Store::open(root.join("spaces.db")).unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "synthetic diagnostic".into(),
            })
            .unwrap();
        let run = store.begin_run(&scene).unwrap().run_id;
        let model = store
            .begin_model_request(
                &scene,
                &run,
                ModelDispatchInput {
                    round: 0,
                    projected_request_sha256: "a".repeat(64),
                },
            )
            .unwrap();
        let grant = VisualAnnotationGrant {
            plugin_id: "mewu.annotations".into(),
            plugin_revision: 1,
            contribution_id: "answer".into(),
        };
        let lease = store
            .record_model_turn(
                &model,
                CompletePublicTurn {
                    text: String::new(),
                    calls: vec![ProposedTool {
                        call_id: uid(),
                        binding: grant.binding(&"b".repeat(64)),
                        arguments_wire_sha256: "c".repeat(64),
                    }],
                },
            )
            .unwrap()
            .tools
            .remove(0)
            .lease;
        Self {
            root,
            store,
            scene,
            run,
            lease,
        }
    }
    fn detail(&self) -> JournalEntryDetail {
        let page = self
            .store
            .run_journal_page(&self.scene, &self.run, None, 25)
            .unwrap()
            .unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.kind == ReceiptKind::Tool)
            .unwrap();
        self.store
            .run_journal_entry(&self.scene, &self.run, &entry.id, page.summary.revision)
            .unwrap()
    }
}

#[test]
fn visual_diagnostic_feedback_matches_the_durable_not_sent_receipt_and_survives_reopen() {
    let mut fixture = ReceiptFixture::new();
    let scope = tests::scope(tests::normalized_input());
    let mut args = tests::normalized_args(&scope.input);
    object(&mut args)["fontSize"] = json!({"private-secret":"wrong type"});
    let error = rejection(&scope, &args);
    let (_, output) =
        reject_arguments(&mut fixture.store, &fixture.lease, &error, &scope.input).unwrap();
    let feedback: Value = serde_json::from_str(&output.model_json).unwrap();
    assert_eq!(feedback["executed"], false);
    assert_eq!(
        feedback["argumentDiagnostic"],
        serde_json::to_value(&error.diagnostic).unwrap()
    );
    assert!(!output.model_json.contains("private-secret"));
    assert_eq!(
        feedback["exampleArguments"],
        invocation_example(&scope.input).unwrap()
    );
    assert!(feedback["invocationGuidance"]
        .as_str()
        .unwrap()
        .contains("groups[i].target"));
    let example = &feedback["exampleArguments"];
    assert!(example.is_object());
    assert!(scope.parse_plan(&example.to_string()).is_ok());
    let schema = scope.definition();
    assert!(jsonschema::validator_for(&schema["function"]["parameters"])
        .unwrap()
        .is_valid(example));
    let detail = fixture.detail();
    assert_eq!(detail.entry.phase, ReceiptPhase::NotSent);
    assert_eq!(
        detail.not_sent_reason,
        Some(NotSentReason::InvalidArguments)
    );
    assert_eq!(detail.argument_diagnostic, error.diagnostic);
    let scene = fixture.scene.clone();
    let run = fixture.run.clone();
    let root = fixture.root.clone();
    drop(fixture.store);
    let reopened = Store::open(root.join("spaces.db")).unwrap();
    let page = reopened
        .run_journal_page(&scene, &run, None, 25)
        .unwrap()
        .unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.kind == ReceiptKind::Tool)
        .unwrap();
    assert_eq!(
        reopened
            .run_journal_entry(&scene, &run, &entry.id, page.summary.revision)
            .unwrap()
            .argument_diagnostic,
        error.diagnostic
    );
}

#[test]
fn visual_failed_diagnostic_receipt_never_returns_model_correction() {
    let mut fixture = ReceiptFixture::new();
    let sql = Connection::open(fixture.root.join("spaces.db")).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_visual_diagnostic BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'synthetic diagnostic failure'); END").unwrap();
    let error = Rejection::field(
        "批注参数无效",
        "$/groups/0/objects/0/points",
        DiagnosticReason::Type,
    );
    let scope = tests::scope(tests::normalized_input());
    assert!(reject_arguments(&mut fixture.store, &fixture.lease, &error, &scope.input).is_err());
    let detail = fixture.detail();
    assert_eq!(detail.entry.phase, ReceiptPhase::Prepared);
    assert_eq!(detail.argument_diagnostic, None);
    assert!(fixture.store.snapshot().scenes.iter().all(|scene| scene
        .regions
        .iter()
        .all(|region| region.drawings.is_empty())));
}
