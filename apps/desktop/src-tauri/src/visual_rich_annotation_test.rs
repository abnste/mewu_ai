// SPDX-License-Identifier: MPL-2.0
//! Synthetic images and independent SQLite only; no model, UI or clipboard.
use super::*;
use mewu_core::*;
use rusqlite::{types::Value as SqlValue, Connection};
use std::sync::atomic::{AtomicUsize, Ordering};

fn table_object() -> Value {
    json!({"kind":"table","points":[{"x":0,"y":0},{"x":1000,"y":1000}],"table":{"version":1,"header":["项目","值","备注"],"rows":[["中文\n第二行","0001","=1+2"],["空","","<svg/> $x$"]],"align":["left","center","right"]}})
}
fn table_args(input: &VerifiedVisualInput) -> Value {
    let mut value = super::tests::normalized_args(input);
    value["groups"][0]["objects"] = json!([table_object()]);
    value
}
fn formula_object() -> Value {
    json!({"kind":"formula","points":[{"x":0,"y":0},{"x":1000,"y":1000}],"tex":"\\frac{x}{2}","color":"#123ABC","fontSize":16})
}

#[test]
fn rich_schema_and_parser_keep_units_and_forbid_model_assets_or_flattened_cells() {
    let input = super::tests::normalized_input();
    let scope = super::tests::scope(input.clone());
    let definition = scope.definition();
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(definition.pointer("/function/parameters").unwrap())
        .unwrap();
    let original = table_args(&input);
    assert!(validator.is_valid(&original));
    assert!(parse_plan(&original.to_string(), &input).is_ok());
    for field in [
        "rich",
        "layoutId",
        "path",
        "png",
        "svg",
        "origin",
        "fontSize",
        "color",
        "strokeWidth",
        "anchor",
        "text",
    ] {
        let mut bad = original.clone();
        bad["groups"][0]["objects"][0][field] = json!("spoof");
        assert!(!validator.is_valid(&bad));
        assert!(parse_plan(&bad.to_string(), &input).is_err());
    }
    for field in ["coordinateSpace", "styleUnit"] {
        let mut bad = original.clone();
        bad["groups"][0].as_object_mut().unwrap().remove(field);
        assert!(!validator.is_valid(&bad));
        assert!(parse_plan(&bad.to_string(), &input).is_err());
        bad["groups"][0][field] = Value::Null;
        assert!(parse_plan(&bad.to_string(), &input).is_err());
    }
    let mut bad = original.clone();
    bad["groups"][0]["objects"][0]["table"]["rows"] = json!([["ragged"]]);
    assert!(parse_plan(&bad.to_string(), &input).is_err());
    let mut bad = original.clone();
    bad["groups"][0]["objects"][0]["table"]["align"][0] = json!("justify");
    assert!(!validator.is_valid(&bad));
    assert!(parse_plan(&bad.to_string(), &input).is_err());
    let mut bad = original.clone();
    bad["groups"][0]["objects"][0]["kind"] = json!("rich");
    assert!(parse_plan(&bad.to_string(), &input).is_err());
    let mut formula = original;
    formula["groups"][0]["objects"] = json!([formula_object()]);
    assert!(validator.is_valid(&formula));
    let plan = parse_plan(&formula.to_string(), &input).unwrap();
    let ParsedObject::Rich {
        content: RichContent::Formula { font_size, tex, .. },
        bounds,
        ..
    } = &plan.groups[0].objects[0]
    else {
        panic!("real formula plan")
    };
    assert_eq!(*font_size, 32.);
    assert_eq!(tex, "\\frac{x}{2}");
    assert_eq!(
        *bounds,
        [
            DrawingPoint { x: 300., y: 300. },
            DrawingPoint { x: 900., y: 700. }
        ]
    );
    formula["groups"][0]["objects"][0]["fontSize"] = json!(72);
    assert!(parse_plan(&formula.to_string(), &input).is_err());
    formula["groups"][0]["objects"][0]["fontSize"] = json!(16);
    for tex in ["   ", "x\r\ny"] {
        formula["groups"][0]["objects"][0]["tex"] = json!(tex);
        assert!(parse_plan(&formula.to_string(), &input).is_err());
    }
}

#[tokio::test]
async fn native_table_png_preserves_literal_cells_complete_pixels_and_proportional_source_mapping()
{
    let input = super::tests::normalized_input();
    let plan = parse_plan(&table_args(&input).to_string(), &input).unwrap();
    let (batch, layouts) = render_plan(plan, std::env::temp_dir(), || Ok(()))
        .await
        .unwrap();
    assert_eq!(layouts.len(), 1);
    let layout = &layouts[0];
    let pixels = crate::rich_annotation_host::decode(layout).unwrap();
    assert_eq!(pixels.dimensions(), (layout.width(), layout.height()));
    assert!(pixels.width() > 100 && pixels.height() > 50);
    let ink = pixels
        .pixels()
        .filter(|p| p[0] < 120 && p[1] < 120 && p[2] < 120 && p[3] > 200)
        .count();
    assert!(ink > 100, "real text/grid raster ink");
    let RichContent::Table {
        header,
        rows,
        align,
        ..
    } = layout.content()
    else {
        panic!("typed table")
    };
    assert_eq!(header[0], "项目");
    assert_eq!(rows[0][0], "中文\n第二行");
    assert_eq!(rows[1][2], "<svg/> $x$");
    assert_eq!(align[1], Some(RichAlignment::Center));
    let drawing = &batch.groups[0].drawings[0];
    assert_eq!(drawing.kind, DrawingKind::Rich);
    assert_eq!(drawing.rich.as_ref(), Some(layout.reference()));
    assert!(drawing.origin.is_none());
    assert_eq!(drawing.points[0], DrawingPoint { x: 300., y: 300. });
    let width = drawing.points[1].x - drawing.points[0].x;
    let height = drawing.points[1].y - drawing.points[0].y;
    assert!((width / f64::from(layout.width()) - height / f64::from(layout.height())).abs() < 1e-9);
    assert!(drawing.points[1].x <= 900. && drawing.points[1].y <= 700.);
}

struct Fixture {
    root: PathBuf,
    store: Store,
    input: VerifiedVisualInput,
    authority: VerifiedVisualAuthority,
    scene: String,
    region: String,
    background: String,
    run: String,
    lease: DispatchLease,
}
impl Fixture {
    fn new() -> Self {
        let id = || uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("mewu-native-rich-{}", id()));
        std::fs::create_dir(&root).unwrap();
        let assets = root.join("assets");
        std::fs::create_dir(&assets).unwrap();
        let source = assets.join("source.png");
        image::RgbaImage::from_pixel(800, 600, image::Rgba([20, 100, 80, 255]))
            .save(&source)
            .unwrap();
        let mut store = Store::open(root.join("spaces.db")).unwrap();
        let scene = store.snapshot().active_scene_id;
        let background = id();
        store
            .set_background(
                &scene,
                Asset {
                    id: background.clone(),
                    kind: AssetKind::Image,
                    name: "source.png".into(),
                    path: source.to_string_lossy().into_owned(),
                    width: Some(800),
                    height: Some(600),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                },
            )
            .unwrap();
        let region = id();
        store
            .apply(SceneCommand::AddRegion {
                scene_id: scene.clone(),
                region: Region {
                    id: region.clone(),
                    x: 0.,
                    y: 0.,
                    width: 800.,
                    height: 600.,
                    ..Default::default()
                },
            })
            .unwrap();
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "原位表格".into(),
            })
            .unwrap();
        let context = store.begin_run(&scene).unwrap();
        let attachment = Asset {
            id: id(),
            kind: AssetKind::Image,
            name: "sent.jpg".into(),
            path: "sent.jpg".into(),
            width: Some(800),
            height: Some(600),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let input = VerifiedVisualInput::new(VisualInputManifest {
            version: 1,
            targets: vec![VisualInputTarget {
                handle: id(),
                attachment_id: attachment.id.clone(),
                attachment_ordinal: 0,
                attachment_sha256: "a".repeat(64),
                region_id: region.clone(),
                source: visual_source_fence(
                    context.background.as_ref().unwrap(),
                    &context.regions[0],
                )
                .unwrap(),
                crop: VisualPixelRect {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                resized: VisualPixelSize {
                    width: 800,
                    height: 600,
                },
                tile: VisualPixelRect {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                encoded: VisualPixelSize {
                    width: 800,
                    height: 600,
                },
                profile_id: VisualCoordinateProfile::ExplicitNormalized1000V2
                    .id()
                    .into(),
            }],
        })
        .unwrap();
        let grant = VisualAnnotationGrant {
            plugin_id: "mewu.annotations".into(),
            plugin_revision: 1,
            contribution_id: "answer".into(),
        };
        let authority = VerifiedVisualAuthority::new(grant.clone(), input.sha256().into()).unwrap();
        store
            .attach_run_visual_assets(
                &scene,
                &context.run_id,
                vec![attachment],
                grant.clone(),
                input.clone(),
            )
            .unwrap();
        let model = store
            .begin_model_request(
                &scene,
                &context.run_id,
                ModelDispatchInput {
                    round: 0,
                    projected_request_sha256: "b".repeat(64),
                },
            )
            .unwrap();
        let mut prepared = store
            .record_model_turn(
                &model,
                CompletePublicTurn {
                    text: String::new(),
                    calls: vec![ProposedTool {
                        call_id: id(),
                        binding: grant.binding(input.sha256()),
                        arguments_wire_sha256: "c".repeat(64),
                    }],
                },
            )
            .unwrap()
            .tools;
        let lease = prepared.remove(0).lease;
        Self {
            root,
            store,
            input,
            authority,
            scene,
            region,
            background,
            run: context.run_id,
            lease,
        }
    }
    fn proof(&self) -> Vec<Vec<Vec<SqlValue>>> {
        let db = Connection::open(self.root.join("spaces.db")).unwrap();
        [
            "app_state",
            "drawing_layouts",
            "visual_run_inputs",
            "agent_run_events",
        ]
        .into_iter()
        .map(|name| {
            let mut statement = db
                .prepare(&format!("SELECT * FROM {name} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            let result = statement
                .query_map([], |row| {
                    (0..columns).map(|i| row.get::<_, SqlValue>(i)).collect()
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            result
        })
        .collect()
    }
    fn undo(&mut self, redo: bool) {
        let revision = self.store.snapshot().scenes[0].regions[0].drawing_revision;
        let command = if redo {
            SceneCommand::RedoDrawing {
                scene_id: self.scene.clone(),
                region_id: self.region.clone(),
                background_id: self.background.clone(),
                expected_revision: revision,
            }
        } else {
            SceneCommand::UndoDrawing {
                scene_id: self.scene.clone(),
                region_id: self.region.clone(),
                background_id: self.background.clone(),
                expected_revision: revision,
            }
        };
        self.store.apply(command).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Close the writer before removing only exact self-owned fixture files.
        let replacement = Store::open_in_memory().unwrap();
        drop(std::mem::replace(&mut self.store, replacement));
        for file in [
            "spaces.db",
            "spaces.db-wal",
            "spaces.db-shm",
            "assets/source.png",
        ] {
            let _ = std::fs::remove_file(self.root.join(file));
        }
        let _ = std::fs::remove_dir(self.root.join("assets"));
        let _ = std::fs::remove_dir(&self.root);
    }
}

#[tokio::test]
async fn native_png_and_vectors_commit_one_real_sqlite_batch_with_ordered_pixels_and_reopen() {
    let mut fixture = Fixture::new();
    let mut value = table_args(&fixture.input);
    let mut table = table_object();
    table["points"] = json!([{"x":100,"y":100},{"x":900,"y":900}]);
    let line = |color: &str| json!({"kind":"line","color":color,"strokeWidth":4,"points":[{"x":100,"y":160},{"x":600,"y":160}]});
    value["groups"][0]["objects"] = json!([line("#ff0000"), table, line("#0000ff")]);
    let (batch, layouts) = render_plan(
        parse_plan(&value.to_string(), &fixture.input).unwrap(),
        fixture.root.clone(),
        || Ok(()),
    )
    .await
    .unwrap();
    let reference = layouts[0].reference().clone();
    fixture
        .store
        .apply_visual_annotations_with_rich_receipt(
            &fixture.lease,
            &fixture.authority,
            batch,
            layouts,
        )
        .unwrap();
    let saved = fixture.store.snapshot();
    let scene = &saved.scenes[0];
    assert_eq!(scene.regions[0].drawings.len(), 3);
    assert_eq!(scene.regions[0].drawing_history.undo.len(), 1);
    let pixels = crate::assets::crop(scene, &fixture.region, &fixture.root.join("assets"))
        .unwrap()
        .into_rgba8();
    let pixel = pixels.get_pixel(90, 96);
    assert!(
        pixel[2] > 200 && pixel[0] < 30,
        "later blue vector must composite above actual table PNG"
    );
    fixture.undo(false);
    assert!(fixture.store.snapshot().scenes[0].regions[0]
        .drawings
        .is_empty());
    assert!(Store::read_rich_layout(&fixture.root.join("spaces.db"), &reference).is_ok());
    fixture.undo(true);
    assert_eq!(
        fixture.store.snapshot().scenes[0].regions[0].drawings,
        scene.regions[0].drawings
    );
    fixture
        .store
        .finish_run(&fixture.scene, &fixture.run, "表格完成")
        .unwrap();
    let saved = fixture.store.snapshot();
    let original = std::mem::replace(&mut fixture.store, Store::open_in_memory().unwrap());
    drop(original);
    fixture.store = Store::open(fixture.root.join("spaces.db")).unwrap();
    assert_eq!(fixture.store.snapshot(), saved);
    let reopened = crate::assets::crop(
        &fixture.store.snapshot().scenes[0],
        &fixture.region,
        &fixture.root.join("assets"),
    )
    .unwrap()
    .into_rgba8();
    assert_eq!(reopened, pixels);
}

#[tokio::test]
async fn complete_first_table_then_unreadable_second_table_never_writes_half_a_batch() {
    let mut fixture = Fixture::new();
    let before = fixture.proof();
    let mut value = table_args(&fixture.input);
    let mut tiny = table_object();
    tiny["points"] = json!([{"x":1,"y":1},{"x":2,"y":2}]);
    value["groups"][0]["objects"] = json!([table_object(), tiny]);
    assert!(render_plan(
        parse_plan(&value.to_string(), &fixture.input).unwrap(),
        fixture.root.clone(),
        || Ok(())
    )
    .await
    .is_err());
    assert_eq!(fixture.proof(), before);
    fixture
        .store
        .record_tool_not_sent(&fixture.lease, NotSentReason::PreparationFailed)
        .unwrap();
    assert!(fixture.store.snapshot().scenes[0].regions[0]
        .drawings
        .is_empty());
    let db = Connection::open(fixture.root.join("spaces.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM drawing_layouts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn cancellation_after_real_table_render_discards_uncommitted_pixels() {
    let fixture = Fixture::new();
    let before = fixture.proof();
    let checks = AtomicUsize::new(0);
    let result = render_plan(
        parse_plan(&table_args(&fixture.input).to_string(), &fixture.input).unwrap(),
        fixture.root.clone(),
        || {
            if checks.fetch_add(1, Ordering::SeqCst) >= 2 {
                Err("source changed after rendering".into())
            } else {
                Ok(())
            }
        },
    )
    .await;
    assert!(result.is_err());
    assert!(checks.load(Ordering::SeqCst) >= 3);
    assert_eq!(fixture.proof(), before);
}

/// This deliberately launches the real, same-build product worker executable.
/// Normal tests never read these environment variables or create these outputs.
#[tokio::test]
#[ignore = "explicit same-build product formula worker; synthetic private output only"]
async fn product_formula_worker_renders_four_math_cases_in_mixed_sqlite_documents() {
    use image::GenericImageView;
    use std::path::Path;
    let executable = PathBuf::from(
        std::env::var_os("MEWU_FORMULA_TEST_EXE")
            .expect("MEWU_FORMULA_TEST_EXE must select the same-build product executable"),
    );
    assert!(executable.is_absolute() && executable.is_file());
    let output = PathBuf::from(
        std::env::var_os("MEWU_FORMULA_TEST_OUTPUT")
            .expect("MEWU_FORMULA_TEST_OUTPUT must select an existing private output directory"),
    );
    assert!(output.is_absolute());
    let output = output.canonicalize().unwrap();
    let private = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.private")
        .canonicalize()
        .unwrap();
    assert!(output.starts_with(&private) && output != private);
    let worker_root = output.join("synthetic-worker-data");
    std::fs::create_dir(&worker_root).unwrap();
    let cases = [
        ("fraction", r"\frac{a+b}{c+d}"),
        ("chinese-label", r"\text{面积}=r^2"),
        ("matrix", r"\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}"),
        ("sum", r"\sum_{k=1}^{n} k=\frac{n(n+1)}{2}"),
    ];
    let mut evidence = Vec::new();
    for (case, tex) in cases {
        let mut fixture = Fixture::new();
        let before = fixture.proof();
        let mut value = table_args(&fixture.input);
        let mut table = table_object();
        table["points"] = json!([{"x":50,"y":50},{"x":450,"y":950}]);
        let formula = json!({"kind":"formula","points":[{"x":500,"y":50},{"x":950,"y":950}],"tex":tex,"color":"#173F8A","fontSize":24});
        let line = |color: &str| json!({"kind":"line","color":color,"strokeWidth":4,"points":[{"x":30,"y":150},{"x":950,"y":150}]});
        value["groups"][0]["objects"] = json!([line("#ff0000"), table, formula, line("#0000ff")]);
        let (batch, layouts) = render_plan(
            parse_plan(&value.to_string(), &fixture.input).unwrap(),
            worker_root.clone(),
            || Ok(()),
        )
        .await
        .unwrap_or_else(|error| panic!("{case}: actual product formula worker failed: {error}"));
        assert_eq!(fixture.proof(), before, "rendering cannot write SQLite");
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].reference().kind, RichKind::Table);
        assert_eq!(layouts[1].reference().kind, RichKind::Formula);
        let formula_layout = &layouts[1];
        let RichContent::Formula {
            tex: retained,
            font_size,
            color,
            ..
        } = formula_layout.content()
        else {
            panic!("true formula descriptor required")
        };
        assert_eq!(retained, tex);
        assert_eq!(*font_size, 24.);
        assert_eq!(color, "#173F8A");
        let formula_pixels = crate::rich_annotation_host::decode(formula_layout).unwrap();
        let ink = formula_pixels.pixels().filter(|p| p[3] >= 32).count();
        assert!(ink > 20 && formula_pixels.width() > 8 && formula_pixels.height() > 8);
        assert!(
            formula_pixels.pixels().any(|p| p[3] == 0),
            "formula is a transparent native raster"
        );
        let formula_reference = formula_layout.reference().clone();
        let table_reference = layouts[0].reference().clone();
        let formula_raster_hash = formula_reference.raster_sha256.clone();
        fixture
            .store
            .apply_visual_annotations_with_rich_receipt(
                &fixture.lease,
                &fixture.authority,
                batch,
                layouts,
            )
            .unwrap();
        let committed = fixture.store.snapshot();
        let scene = &committed.scenes[0];
        assert_eq!(scene.regions[0].drawings.len(), 4);
        assert_eq!(scene.regions[0].drawing_history.undo.len(), 1);
        assert_eq!(scene.regions[0].drawings[0].kind, DrawingKind::Line);
        assert_eq!(
            scene.regions[0].drawings[1].rich.as_ref(),
            Some(&table_reference)
        );
        assert_eq!(
            scene.regions[0].drawings[2].rich.as_ref(),
            Some(&formula_reference)
        );
        assert_eq!(scene.regions[0].drawings[3].kind, DrawingKind::Line);
        let crop = crate::assets::crop(scene, &fixture.region, &fixture.root.join("assets"))
            .unwrap()
            .into_rgba8();
        assert_eq!(crop.dimensions(), (800, 600));
        let pixel = crop.get_pixel(90, 90);
        assert!(
            pixel[2] > 200 && pixel[0] < 30,
            "last blue line covers native table PNG"
        );
        let png_path = output.join(format!("{case}.png"));
        assert!(!png_path.exists());
        crop.save(&png_path).unwrap();
        assert_eq!(
            crate::assets::image(
                &Asset {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: AssetKind::Image,
                    name: format!("{case}.png"),
                    path: png_path.to_string_lossy().into_owned(),
                    width: Some(800),
                    height: Some(600),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                },
                &output
            )
            .unwrap()
            .into_rgba8(),
            crop
        );
        let composed_sha256 = format!("{:x}", Sha256::digest(std::fs::read(&png_path).unwrap()));
        fixture.undo(false);
        assert!(fixture.store.snapshot().scenes[0].regions[0]
            .drawings
            .is_empty());
        assert!(
            Store::read_rich_layout(&fixture.root.join("spaces.db"), &formula_reference).is_ok()
        );
        fixture.undo(true);
        assert_eq!(
            fixture.store.snapshot().scenes[0].regions[0].drawings,
            scene.regions[0].drawings
        );
        let redone = crate::assets::crop(
            &fixture.store.snapshot().scenes[0],
            &fixture.region,
            &fixture.root.join("assets"),
        )
        .unwrap()
        .into_rgba8();
        assert_eq!(redone, crop);
        fixture
            .store
            .finish_run(&fixture.scene, &fixture.run, "synthetic formula complete")
            .unwrap();
        let saved = fixture.store.snapshot();
        let writer = std::mem::replace(&mut fixture.store, Store::open_in_memory().unwrap());
        drop(writer);
        fixture.store = Store::open(fixture.root.join("spaces.db")).unwrap();
        assert_eq!(fixture.store.snapshot(), saved);
        let reopened = crate::assets::crop(
            &fixture.store.snapshot().scenes[0],
            &fixture.region,
            &fixture.root.join("assets"),
        )
        .unwrap()
        .into_rgba8();
        assert_eq!(reopened, crop);
        let reopened_path = output.join(format!("{case}-reopened.png"));
        assert!(!reopened_path.exists());
        reopened.save(&reopened_path).unwrap();
        assert_eq!(
            composed_sha256,
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(&reopened_path).unwrap())
            )
        );
        evidence.push(json!({"case":case,"png":format!("{case}.png"),"reopenedPng":format!("{case}-reopened.png"),"formulaWidth":formula_pixels.width(),"formulaHeight":formula_pixels.height(),"formulaInkPixels":ink,"formulaRasterSha256":formula_raster_hash,"composedSha256":composed_sha256,"drawings":4,"singleBatch":true,"undoRedo":true,"reopened":true}));
    }
    assert!(crate::formula_layout::wait_until_idle(
        std::time::Duration::from_secs(2)
    ));
    assert!(table_workers_are_idle());
    let proof = output.join("product-formula-proof.json");
    assert!(!proof.exists());
    std::fs::write(
        proof,
        serde_json::to_vec_pretty(&json!({"syntheticOnly":true,"caseCount":4,"cases":evidence}))
            .unwrap(),
    )
    .unwrap();
}
