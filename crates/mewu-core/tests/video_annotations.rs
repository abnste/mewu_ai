// SPDX-License-Identifier: MPL-2.0
//! Real isolated SQLite domain tests; fixed PNG is a byte-identity fixture,
//! not a claim that native frame decoding, text shaping or export has run.
use mewu_core::*;
use rusqlite::{types::Value, Connection};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const SECOND: u64 = 10_000_000;
fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
struct Db(PathBuf);
impl Db {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("mewu-video-annotations-{}.sqlite", uid()));
        assert!(!path
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("c:\\hermes"));
        Self(path)
    }
    fn conn(&self) -> Connection {
        Connection::open(&self.0).unwrap()
    }
    fn saved(&self) -> (String, i64) {
        self.conn()
            .query_row(
                "SELECT payload,revision FROM app_state WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
fn rows(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    let without_rowid: i64 = db
        .query_row(
            "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
            [table],
            |r| r.get(0),
        )
        .unwrap();
    assert!((0..=1).contains(&without_rowid));
    let order = if without_rowid == 1 {
        let mut columns = db
            .prepare("SELECT name FROM pragma_table_xinfo(?1) WHERE pk>0 ORDER BY pk")
            .unwrap();
        let primary = columns
            .query_map([table], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !primary.is_empty(),
            "WITHOUT ROWID needs explicit PK: {table}"
        );
        primary
            .iter()
            .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",")
    } else {
        "rowid".to_string()
    };
    let projection = if without_rowid == 1 { "*" } else { "rowid,*" };
    let mut s = db
        .prepare(&format!(
            "SELECT {projection} FROM \"{}\" ORDER BY {order}",
            table.replace('"', "\"\"")
        ))
        .unwrap();
    let n = s.column_count();
    s.query_map([], |r| (0..n).map(|i| r.get(i)).collect())
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
fn proof(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    [
        "app_state",
        "video_run_inputs",
        "video_text_layouts",
        "agent_run_records",
        "agent_run_events",
    ]
    .iter()
    .map(|t| rows(db, t))
    .collect()
}
fn range(start: u64, end: u64) -> VideoRange {
    VideoRange {
        start_ticks: start,
        end_ticks: end,
    }
}
struct Source {
    target: VideoAnnotationTarget,
    proof: VerifiedVideoAnnotationSource,
}
fn setup(store: &mut Store, duration: u64) -> Source {
    let scene_id = store.snapshot().active_scene_id;
    let asset = Asset {
        id: uid(),
        name: "synthetic.mp4".into(),
        path: "owned-synthetic.mp4".into(),
        kind: AssetKind::Video,
        width: Some(320),
        height: Some(180),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    };
    let s = store.add_asset(&scene_id, asset.clone()).unwrap();
    let item = s
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .unwrap()
        .items
        .last()
        .unwrap();
    Source {
        target: VideoAnnotationTarget {
            scene_id,
            item_id: item.id.clone(),
            source_id: asset.id.clone(),
            expected_range_revision: 0,
            expected_annotation_revision: 0,
        },
        proof: VerifiedVideoAnnotationSource::new(
            VerifiedVideoSource {
                asset,
                duration_ticks: duration,
                width: 320,
                height: 180,
            },
            hash("actual source bytes fixture"),
        )
        .unwrap(),
    }
}
fn view(store: &Store, f: &Source) -> VideoSourceView {
    store
        .video_source(&f.target.scene_id, &f.target.item_id)
        .unwrap()
}
fn current(store: &Store, f: &Source) -> VideoAnnotationTarget {
    let v = view(store, f);
    VideoAnnotationTarget {
        expected_range_revision: v.edit.map_or(0, |e| e.revision),
        expected_annotation_revision: v.annotations.map_or(0, |d| d.revision),
        ..f.target.clone()
    }
}
fn rect(interval: VideoRange) -> VideoAnnotationDraft {
    VideoAnnotationDraft {
        interval,
        primitive: VideoAnnotationPrimitive::Rect {
            bounds: VideoPixelRect {
                x: 30.,
                y: 20.,
                width: 80.,
                height: 50.,
            },
            style: VideoRectStyle {
                color: "#5B85E8".into(),
                stroke_width: 3.,
                opacity: 0.8,
            },
        },
    }
}
fn png() -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}
fn text_layout() -> VerifiedVideoTextLayout {
    VerifiedVideoTextLayout::new(
        uid(),
        VideoTextContent {
            version: 1,
            text: "答案：42".into(),
            color: "#F0F2F6".into(),
            font_size: 24.,
        },
        RichRendererIdentity {
            id: "native-video-text".into(),
            version: "fixture1".into(),
            font_sha256: hash("fixed font"),
            style_version: 1,
        },
        1,
        1,
        png(),
    )
    .unwrap()
}
fn text(interval: VideoRange, l: &VerifiedVideoTextLayout) -> VideoAnnotationDraft {
    VideoAnnotationDraft {
        interval,
        primitive: VideoAnnotationPrimitive::Text {
            top_left: VideoPixelPoint { x: 120., y: 80. },
            layout: l.reference().clone(),
        },
    }
}
fn append(
    store: &mut Store,
    f: &Source,
    drafts: Vec<VideoAnnotationDraft>,
    l: Vec<VerifiedVideoTextLayout>,
) {
    store
        .append_video_annotations(&current(store, f), &f.proof, drafts, l)
        .unwrap();
}
fn replay(store: &mut Store, f: &Source, redo: bool) {
    let d = view(store, f).annotations.unwrap();
    let id = if redo {
        d.redo.last().unwrap().id.clone()
    } else {
        d.undo.last().unwrap().id.clone()
    };
    store
        .edit_video_annotation_with_layouts(
            &current(store, f),
            if redo {
                VideoAnnotationMutation::Redo {
                    expected_operation_id: id,
                }
            } else {
                VideoAnnotationMutation::Undo {
                    expected_operation_id: id,
                }
            },
            vec![],
        )
        .unwrap();
}
fn make_input(
    store: &Store,
    f: &Source,
) -> (VerifiedVideoInput, Vec<Asset>, VisualAnnotationGrant) {
    let source = video_source_fence(&view(store, f), &f.proof).unwrap();
    let retained = source
        .range
        .unwrap_or(range(0, source.source_duration_ticks));
    let ticks = [
        retained.start_ticks,
        retained.start_ticks + (retained.end_ticks - retained.start_ticks) / 2,
    ];
    let mut assets = Vec::new();
    let mut frames = Vec::new();
    for (ordinal, tick) in ticks.into_iter().enumerate() {
        let asset = Asset {
            id: uid(),
            name: format!("frame-{ordinal}.jpg"),
            path: format!("synthetic-frame-{ordinal}.jpg"),
            kind: AssetKind::Image,
            width: Some(160),
            height: Some(90),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        frames.push(VideoInputFrame {
            handle: uid(),
            attachment_id: asset.id.clone(),
            attachment_ordinal: ordinal as u32,
            jpeg_sha256: hash(&asset.name),
            encoded_width: 160,
            encoded_height: 90,
            source_pts_ticks: tick.saturating_sub(SECOND / 2),
            sample_duration_ticks: SECOND,
            source_playback_ticks: tick,
        });
        assets.push(asset);
    }
    let input = VerifiedVideoInput::new(VideoInputManifest {
        version: 1,
        profile_id: VIDEO_PROFILE_ID.into(),
        targets: vec![VideoInputTarget {
            handle: uid(),
            item_id: f.target.item_id.clone(),
            source,
            sent_window: retained,
            range_end_handle: uid(),
            frames,
        }],
    })
    .unwrap();
    (
        input,
        assets,
        VisualAnnotationGrant {
            plugin_id: "mewu.annotations".into(),
            plugin_revision: 1,
            contribution_id: "answer".into(),
        },
    )
}
struct RunFixture {
    run: RunContext,
    input: VerifiedVideoInput,
    grant: VisualAnnotationGrant,
    authority: VerifiedVideoAnnotationAuthority,
}
fn begin(store: &mut Store, f: &Source) -> RunFixture {
    store
        .apply(SceneCommand::SetRefs {
            scene_id: f.target.scene_id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Item,
                id: f.target.item_id.clone(),
            }],
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: f.target.scene_id.clone(),
            draft: "把视频答案标出来".into(),
        })
        .unwrap();
    let (input, assets, grant) = make_input(store, f);
    let authority =
        VerifiedVideoAnnotationAuthority::new(grant.clone(), input.sha256().into()).unwrap();
    let run = store
        .begin_run_with_video_input(
            &f.target.scene_id,
            None,
            VerifiedVideoRunInput::new(grant.clone(), input.clone(), assets).unwrap(),
        )
        .unwrap();
    RunFixture {
        run,
        input,
        grant,
        authority,
    }
}
fn leases(store: &mut Store, f: &RunFixture, n: usize) -> Vec<DispatchLease> {
    let model = store
        .begin_model_request(
            &f.run.scene_id,
            &f.run.run_id,
            ModelDispatchInput {
                round: 0,
                projected_request_sha256: hash("request"),
            },
        )
        .unwrap();
    store
        .record_model_turn(
            &model,
            CompletePublicTurn {
                text: String::new(),
                calls: (0..n)
                    .map(|_| ProposedTool {
                        call_id: uid(),
                        binding: f.grant.video_binding(f.input.sha256()),
                        arguments_wire_sha256: hash("args"),
                    })
                    .collect(),
            },
        )
        .unwrap()
        .tools
        .into_iter()
        .map(|t| t.lease)
        .collect()
}
fn batch(f: &RunFixture, objects: Vec<VideoAnnotationDraft>) -> ValidatedVideoAppendBatch {
    ValidatedVideoAppendBatch {
        groups: vec![VideoAppendGroup {
            target_handle: f.input.manifest().targets[0].handle.clone(),
            objects,
        }],
    }
}

#[test]
fn trim_history_and_annotation_history_are_independent_durable_source_time() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let l = text_layout();
    let reference = l.reference().clone();
    append(
        &mut store,
        &f,
        vec![
            rect(range(SECOND, 8 * SECOND)),
            text(range(2 * SECOND, 9 * SECOND), &l),
        ],
        vec![l],
    );
    let before = view(&store, &f).annotations.unwrap();
    store
        .set_video_range(
            &VideoTarget {
                scene_id: f.target.scene_id.clone(),
                item_id: f.target.item_id.clone(),
                source_id: f.target.source_id.clone(),
                expected_revision: 0,
            },
            f.proof.source(),
            None,
            Some(range(3 * SECOND, 7 * SECOND)),
        )
        .unwrap();
    assert_eq!(view(&store, &f).annotations, Some(before.clone()));
    assert_eq!(
        project_video_annotation_interval(
            before.objects[0].interval,
            range(3 * SECOND, 7 * SECOND)
        ),
        Some(range(0, 4 * SECOND))
    );
    assert_eq!(
        project_video_annotation_interval(range(0, SECOND), range(2 * SECOND, 3 * SECOND)),
        None
    );
    assert_eq!(
        video_annotations_at(&before, SourcePlaybackTicks(SECOND)).count(),
        1
    );
    assert_eq!(
        video_annotations_at(&before, SourcePlaybackTicks(9 * SECOND)).count(),
        0
    );
    replay(&mut store, &f, false);
    assert!(view(&store, &f).annotations.unwrap().objects.is_empty());
    let trim = view(&store, &f).edit;
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    assert_eq!(view(&store, &f).edit, trim);
    replay(&mut store, &f, true);
    assert_eq!(
        view(&store, &f).annotations.unwrap().objects,
        before.objects
    );
    assert_eq!(
        Store::read_video_text_layout(&file.0, &reference)
            .unwrap()
            .png(),
        png()
    );
}
#[test]
fn stale_update_item_cannot_replace_video_range_document_or_history() {
    let mut store = Store::open_in_memory().unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let mut stale = store.snapshot().scenes[0].items[0].clone();
    append(&mut store, &f, vec![rect(range(0, SECOND))], vec![]);
    let before = view(&store, &f);
    stale.x += 0.1;
    stale.video_annotations = None;
    stale.video_edit = None;
    store
        .apply(SceneCommand::UpdateItem {
            scene_id: f.target.scene_id.clone(),
            item: stale,
        })
        .unwrap();
    assert_eq!(view(&store, &f), before);
}
#[test]
fn manual_cas_noop_and_history_identity_never_write_stale_or_unused_layouts() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    append(&mut store, &f, vec![rect(range(0, SECOND))], vec![]);
    let d = view(&store, &f).annotations.unwrap();
    let o = &d.objects[0];
    let before = file.saved();
    let mutation = VideoAnnotationMutation::Update {
        id: o.id.clone(),
        replacement: VideoAnnotationDraft {
            interval: o.interval,
            primitive: o.primitive.clone(),
        },
    };
    store
        .edit_video_annotation_with_layouts(&current(&store, &f), mutation.clone(), vec![])
        .unwrap();
    assert_eq!(file.saved(), before);
    assert!(store
        .edit_video_annotation_with_layouts(&current(&store, &f), mutation, vec![text_layout()])
        .is_err());
    assert_eq!(file.saved(), before);
    for key in 0..4 {
        let mut stale = current(&store, &f);
        match key {
            0 => stale.source_id = uid(),
            1 => stale.item_id = uid(),
            2 => stale.expected_range_revision += 1,
            _ => stale.expected_annotation_revision += 1,
        };
        assert!(store
            .edit_video_annotation_with_layouts(
                &stale,
                VideoAnnotationMutation::Remove { id: o.id.clone() },
                vec![]
            )
            .is_err());
    }
    assert!(store
        .edit_video_annotation_with_layouts(
            &current(&store, &f),
            VideoAnnotationMutation::Undo {
                expected_operation_id: uid()
            },
            vec![]
        )
        .is_err());
    assert_eq!(file.saved(), before);
}
#[test]
fn manual_batch_invalid_late_object_and_database_abort_leave_no_raster_or_history() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let db = file.conn();
    let old = proof(&db);
    let state = store.snapshot();
    let l = text_layout();
    let mut invalid = rect(range(0, SECOND));
    if let VideoAnnotationPrimitive::Rect { bounds, .. } = &mut invalid.primitive {
        bounds.x = 319.;
    }
    assert!(store
        .append_video_annotations(
            &current(&store, &f),
            &f.proof,
            vec![text(range(0, SECOND), &l), invalid],
            vec![l]
        )
        .is_err());
    assert_eq!(proof(&db), old);
    assert_eq!(store.snapshot(), state);
    db.execute_batch("CREATE TRIGGER fail_video_annotation BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    let l = text_layout();
    assert!(store
        .append_video_annotations(
            &current(&store, &f),
            &f.proof,
            vec![text(range(0, SECOND), &l)],
            vec![l]
        )
        .is_err());
    assert_eq!(proof(&db), old);
    assert_eq!(store.snapshot(), state);
}
#[test]
fn text_layout_identity_is_immutable_and_other_document_reference_is_rejected() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let l = text_layout();
    let r = l.reference().clone();
    append(&mut store, &f, vec![text(range(0, SECOND), &l)], vec![l]);
    let db = file.conn();
    assert!(db
        .execute("UPDATE video_text_layouts SET descriptor='{}'", [])
        .is_err());
    assert!(db.execute("DELETE FROM video_text_layouts", []).is_err());
    let other = setup(&mut store, 10 * SECOND);
    let before = proof(&db);
    let draft = VideoAnnotationDraft {
        interval: range(0, SECOND),
        primitive: VideoAnnotationPrimitive::Text {
            top_left: VideoPixelPoint { x: 0., y: 0. },
            layout: r.clone(),
        },
    };
    assert!(store
        .append_video_annotations(&current(&store, &other), &other.proof, vec![draft], vec![])
        .is_err());
    assert_eq!(proof(&db), before);
    let mut forged = r;
    forged.raster_sha256 = hash("wrong");
    assert!(Store::read_video_text_layout(&file.0, &forged).is_err());
}
#[test]
fn first_run_registration_is_atomic_and_workflow_keeps_the_pending_user_draft() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: f.target.scene_id.clone(),
            draft: "用户尚未发送的内容".into(),
        })
        .unwrap();
    let (input, assets, grant) = make_input(&store, &f);
    let old = file.saved();
    let db = file.conn();
    let state = store.snapshot();
    // Chat draft has no item reference: no run/message/input row can be created.
    assert!(store
        .begin_run_with_video_input(
            &f.target.scene_id,
            None,
            VerifiedVideoRunInput::new(grant.clone(), input.clone(), assets.clone()).unwrap()
        )
        .is_err());
    assert_eq!(file.saved(), old);
    assert_eq!(store.snapshot(), state);
    assert!(rows(&db, "video_run_inputs").is_empty());
    assert!(rows(&db, "agent_run_records").is_empty());
    let context = store
        .begin_video_workflow_with_input(
            &f.target.scene_id,
            "标注视频中的错误",
            &f.target.item_id,
            VerifiedVideoRunInput::new(grant, input, assets.clone()).unwrap(),
        )
        .unwrap();
    assert_eq!(store.snapshot().scenes[0].draft, "用户尚未发送的内容");
    assert_eq!(context.messages.last().unwrap().text, "标注视频中的错误");
    assert_eq!(
        context.messages.last().unwrap().attachments.as_ref(),
        Some(&assets)
    );
    assert_eq!(rows(&db, "video_run_inputs").len(), 1);
}
#[test]
fn complete_range_sparse_input_accepts_held_pts_but_rejects_reordered_or_forged_evidence() {
    let mut store = Store::open_in_memory().unwrap();
    let f = setup(&mut store, 20 * SECOND);
    store
        .set_video_range(
            &VideoTarget {
                scene_id: f.target.scene_id.clone(),
                item_id: f.target.item_id.clone(),
                source_id: f.target.source_id.clone(),
                expected_revision: 0,
            },
            f.proof.source(),
            None,
            Some(range(SECOND, 19 * SECOND)),
        )
        .unwrap();
    let (input, _, _) = make_input(&store, &f);
    assert!(input.manifest().targets[0].frames[0].source_pts_ticks < SECOND);
    assert!(
        input.manifest().targets[0].sent_window.end_ticks
            - input.manifest().targets[0].sent_window.start_ticks
            > 3 * SECOND
    );
    let mut held = input.manifest().clone();
    held.targets[0].frames[1].source_pts_ticks = held.targets[0].frames[0].source_pts_ticks;
    VerifiedVideoInput::new(held).unwrap();
    for case in 0..6 {
        let mut bad = input.manifest().clone();
        match case {
            0 => bad.profile_id = "guess-fps".into(),
            1 => bad.targets[0].frames[1].handle = bad.targets[0].frames[0].handle.clone(),
            2 => {
                bad.targets[0].frames[1].source_playback_ticks =
                    bad.targets[0].frames[0].source_playback_ticks
            }
            3 => bad.targets[0].frames[0].source_pts_ticks = 2 * SECOND,
            4 => bad.targets[0].sent_window.end_ticks -= SECOND,
            _ => bad.targets[0].frames[0].encoded_height = 40,
        };
        assert!(VerifiedVideoInput::new(bad).is_err(), "case {case}");
    }
    let mut raw = serde_json::to_value(input.manifest()).unwrap();
    raw["targets"][0]["source"]
        .as_object_mut()
        .unwrap()
        .remove("range");
    assert!(serde_json::from_value::<VideoInputManifest>(raw).is_err());
}
#[test]
fn two_ai_commits_advance_only_owned_cursor_and_persist_exact_receipt_origin() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let run = begin(&mut store, &f);
    let calls = leases(&mut store, &run, 2);
    let t = &run.input.manifest().targets[0];
    let l = text_layout();
    let reference = l.reference().clone();
    let first = store
        .apply_video_annotations_with_receipt(
            &calls[0],
            &run.authority,
            batch(
                &run,
                vec![
                    rect(range(
                        t.frames[0].source_playback_ticks,
                        t.frames[1].source_playback_ticks,
                    )),
                    text(
                        range(t.frames[1].source_playback_ticks, t.sent_window.end_ticks),
                        &l,
                    ),
                ],
            ),
            vec![l],
        )
        .unwrap();
    assert!(first
        .result
        .model_visible_json
        .unwrap()
        .contains("\"applied\":true"));
    let before = view(&store, &f).annotations.unwrap();
    assert_eq!(before.objects.len(), 2);
    assert_eq!(before.undo.len(), 1);
    assert_eq!(
        store
            .video_input_for_run(&run.run.scene_id, &run.run.run_id, &run.authority)
            .unwrap(),
        run.input
    );
    store
        .apply_video_annotations_with_receipt(
            &calls[1],
            &run.authority,
            batch(
                &run,
                vec![rect(range(
                    t.frames[0].source_playback_ticks,
                    t.sent_window.end_ticks,
                ))],
            ),
            vec![],
        )
        .unwrap();
    let doc = view(&store, &f).annotations.unwrap();
    assert_eq!(doc.revision, 2);
    assert_eq!(doc.undo.len(), 2);
    assert!(doc
        .objects
        .iter()
        .all(|o| o.origin.as_ref().unwrap().run_id == run.run.run_id));
    store
        .finish_run(&run.run.scene_id, &run.run.run_id, "视频批注已写入")
        .unwrap();
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    assert_eq!(view(&store, &f).annotations, Some(doc.clone()));
    let object = &doc.objects[0];
    let origin = object.origin.clone();
    store
        .edit_video_annotation_with_layouts(
            &current(&store, &f),
            VideoAnnotationMutation::Update {
                id: object.id.clone(),
                replacement: rect(range(SECOND, 3 * SECOND)),
            },
            vec![],
        )
        .unwrap();
    assert_eq!(
        view(&store, &f).annotations.unwrap().objects[0].origin,
        origin
    );
    assert_eq!(store.video_text_layout(&reference).unwrap().png(), png());
}
#[test]
fn late_invalid_ai_object_or_receipt_abort_rolls_back_every_local_effect() {
    for sql_failure in [false, true] {
        let file = Db::new();
        let mut store = Store::open(&file.0).unwrap();
        let f = setup(&mut store, 10 * SECOND);
        let run = begin(&mut store, &f);
        let lease = leases(&mut store, &run, 1).remove(0);
        let db = file.conn();
        let before = proof(&db);
        let state = store.snapshot();
        let l = text_layout();
        let mut objects = vec![text(range(0, 5 * SECOND), &l)];
        if sql_failure {
            db.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE ON agent_run_events BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
        } else {
            objects.push(rect(range(SECOND, 5 * SECOND)));
        } // start has no sent frame.
        assert!(store
            .apply_video_annotations_with_receipt(
                &lease,
                &run.authority,
                batch(&run, objects),
                vec![l]
            )
            .is_err());
        assert_eq!(proof(&db), before);
        assert_eq!(store.snapshot(), state);
    }
}
#[test]
fn frozen_run_commits_to_original_scene_without_activating_it_and_closed_rejects_late() {
    let mut store = Store::open_in_memory().unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let run = begin(&mut store, &f);
    let calls = leases(&mut store, &run, 2);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: f.target.scene_id.clone(),
        })
        .unwrap();
    let active = store.snapshot().active_scene_id;
    assert_ne!(active, f.target.scene_id);
    store
        .apply_video_annotations_with_receipt(
            &calls[0],
            &run.authority,
            batch(&run, vec![rect(range(0, 10 * SECOND))]),
            vec![],
        )
        .unwrap();
    assert_eq!(store.snapshot().active_scene_id, active);
    assert_eq!(view(&store, &f).annotations.unwrap().objects.len(), 1);
    store
        .apply(SceneCommand::CloseScene {
            scene_id: f.target.scene_id.clone(),
        })
        .unwrap();
    let state = store.snapshot();
    assert!(store
        .apply_video_annotations_with_receipt(
            &calls[1],
            &run.authority,
            batch(&run, vec![rect(range(0, 10 * SECOND))]),
            vec![]
        )
        .is_err());
    assert_eq!(store.snapshot(), state);
}
#[test]
fn manual_edit_is_blocked_during_run_and_durable_revocation_rejects_prepared_tool() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    append(&mut store, &f, vec![rect(range(0, SECOND))], vec![]);
    let object = view(&store, &f).annotations.unwrap().objects[0].clone();
    let run = begin(&mut store, &f);
    let lease = leases(&mut store, &run, 1).remove(0);
    assert!(matches!(
        store.edit_video_annotation_with_layouts(
            &current(&store, &f),
            VideoAnnotationMutation::Remove { id: object.id },
            vec![]
        ),
        Err(CoreError::RunInProgress)
    ));
    store
        .revoke_video_authority(&run.run.scene_id, &run.run.run_id)
        .unwrap();
    assert!(store
        .video_input_for_run(&run.run.scene_id, &run.run.run_id, &run.authority)
        .is_err());
    let db = file.conn();
    let before = proof(&db);
    assert!(store
        .apply_video_annotations_with_receipt(
            &lease,
            &run.authority,
            batch(&run, vec![rect(range(0, 10 * SECOND))]),
            vec![]
        )
        .is_err());
    assert_eq!(proof(&db), before);
}
fn downgrade_to_seven(db: &Connection) {
    db.execute_batch("DROP TRIGGER video_text_layouts_no_update;DROP TRIGGER video_text_layouts_no_delete;DROP TABLE video_vector_layouts;DROP TABLE video_text_layouts;DROP TABLE video_run_inputs;PRAGMA user_version=7;").unwrap();
}
#[test]
fn db7_normal_upgrade_preserves_raw_payload_and_all_existing_sql_rows_and_schema() {
    let file = Db::new();
    let store = Store::open(&file.0).unwrap();
    let state = store.snapshot();
    drop(store);
    let db = file.conn();
    downgrade_to_seven(&db);
    let raw = format!("\n {} \n", serde_json::to_string_pretty(&state).unwrap());
    db.execute("UPDATE app_state SET payload=?1 WHERE id=1", [&raw])
        .unwrap();
    let revision: i64 = db
        .query_row("SELECT revision FROM app_state", [], |r| r.get(0))
        .unwrap();
    let schema: Vec<(String, String, Option<String>)> = db
        .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let tables: Vec<String> = db
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name!='app_state' ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let all_rows: Vec<_> = tables.iter().map(|t| rows(&db, t)).collect();
    let migrated = Store::open(&file.0).unwrap();
    assert_eq!(file.saved(), (raw, revision + 1));
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        9
    );
    assert_eq!(migrated.snapshot(), state);
    for (kind, name, sql) in &schema {
        let actual: (String, Option<String>) = db
            .query_row(
                "SELECT type,sql FROM sqlite_schema WHERE name=?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(&actual.0, kind);
        assert_eq!(&actual.1, sql);
    }
    assert_eq!(
        tables.iter().map(|t| rows(&db, t)).collect::<Vec<_>>(),
        all_rows
    );
    let new_count:i64=db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('video_run_inputs','sqlite_autoindex_video_run_inputs_1','video_text_layouts','sqlite_autoindex_video_text_layouts_1','video_text_layouts_no_update','video_text_layouts_no_delete')",[],|r|r.get(0)).unwrap();
    assert_eq!(new_count, 6);
    assert!(rows(&db, "video_run_inputs").is_empty());
    assert!(rows(&db, "video_text_layouts").is_empty());
}
#[test]
fn db7_running_recovery_explicitly_settles_old_run_instead_of_claiming_raw_preservation() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let scene = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "旧运行待恢复".into(),
        })
        .unwrap();
    let run = store.begin_run(&scene).unwrap();
    drop(store);
    let db = file.conn();
    downgrade_to_seven(&db);
    let old = file.saved();
    let store = Store::open(&file.0).unwrap();
    let after = file.saved();
    assert_ne!(after.0, old.0);
    assert_eq!(after.1, old.1 + 1);
    assert!(!store.run_is_active(&scene, &run.run_id));
    let recovered = store.snapshot();
    let old_run = recovered
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .unwrap()
        .run
        .as_ref()
        .unwrap();
    assert_eq!(old_run.status, RunStatus::Failed);
    assert_eq!(old_run.error.as_deref(), Some("上次运行已中断"));
    assert!(rows(&db, "video_run_inputs").is_empty());
    assert!(rows(&db, "video_text_layouts").is_empty());
}
#[test]
fn corrupted_history_only_text_or_run_counter_is_rejected_without_overwriting_data() {
    for run_counter in [false, true] {
        let file = Db::new();
        let mut store = Store::open(&file.0).unwrap();
        let f = setup(&mut store, 10 * SECOND);
        if run_counter {
            let run = begin(&mut store, &f);
            let lease = leases(&mut store, &run, 1).remove(0);
            store
                .apply_video_annotations_with_receipt(
                    &lease,
                    &run.authority,
                    batch(&run, vec![rect(range(0, 10 * SECOND))]),
                    vec![],
                )
                .unwrap();
            store
                .finish_run(&run.run.scene_id, &run.run.run_id, "完成")
                .unwrap();
            drop(store);
            let db = file.conn();
            db.execute(
                "UPDATE video_run_inputs SET data=json_set(data,'$.createdCount',0)",
                [],
            )
            .unwrap();
            let old = proof(&db);
            assert!(Store::open(&file.0).is_err());
            assert_eq!(proof(&db), old);
        } else {
            let l = text_layout();
            append(&mut store, &f, vec![text(range(0, SECOND), &l)], vec![l]);
            replay(&mut store, &f, false);
            drop(store);
            let db = file.conn();
            db.execute_batch(
                "DROP TRIGGER video_text_layouts_no_delete;DELETE FROM video_text_layouts;",
            )
            .unwrap();
            let old = proof(&db);
            assert!(Store::open(&file.0).is_err());
            assert_eq!(proof(&db), old);
        }
    }
}
#[test]
fn snapshot_strict_video_types_cannot_be_injected_as_image_rich_or_forged_origin() {
    let mut store = Store::open_in_memory().unwrap();
    let f = setup(&mut store, 10 * SECOND);
    append(&mut store, &f, vec![rect(range(0, SECOND))], vec![]);
    let d = view(&store, &f).annotations.unwrap();
    let mut v = serde_json::to_value(&d).unwrap();
    v["objects"][0]["primitive"]["kind"] = json!("rich");
    assert!(serde_json::from_value::<VideoAnnotationDocument>(v).is_err());
    let mut v = serde_json::to_value(&d).unwrap();
    v["objects"][0]["primitive"]["tracking"] = json!(true);
    assert!(serde_json::from_value::<VideoAnnotationDocument>(v).is_err());
    let mut raw = serde_json::to_value(VideoTextContent {
        version: 1,
        text: "x".into(),
        color: "#112233".into(),
        font_size: 24.,
    })
    .unwrap();
    raw["html"] = json!("<script/>");
    assert!(serde_json::from_value::<VideoTextContent>(raw).is_err());
    let l = text_layout();
    assert!(VerifiedVideoTextLayout::new(
        uid(),
        l.content().clone(),
        l.renderer().clone(),
        1,
        1,
        {
            let mut p = png();
            p[25] = 4;
            p
        }
    )
    .is_err());
}

#[test]
fn exact_frame_handles_allow_trim_end_but_never_pts_seconds_or_unknown_boundary() {
    let mut store = Store::open_in_memory().unwrap();
    let f = setup(&mut store, 10 * SECOND);
    store
        .set_video_range(
            &VideoTarget {
                scene_id: f.target.scene_id.clone(),
                item_id: f.target.item_id.clone(),
                source_id: f.target.source_id.clone(),
                expected_revision: 0,
            },
            f.proof.source(),
            None,
            Some(range(SECOND, 9 * SECOND)),
        )
        .unwrap();
    let (input, _, _) = make_input(&store, &f);
    let t = &input.manifest().targets[0];
    let interval = ModelVideoInterval {
        start_frame_handle: t.frames[0].handle.clone(),
        end_handle: t.range_end_handle.clone(),
    };
    assert_eq!(
        resolve_video_model_interval(t, &interval).unwrap(),
        range(SECOND, 9 * SECOND)
    );
    assert_ne!(
        resolve_video_model_interval(t, &interval)
            .unwrap()
            .start_ticks,
        t.frames[0].source_pts_ticks
    );
    for changed in 0..3 {
        let mut bad = interval.clone();
        match changed {
            0 => bad.start_frame_handle = uid(),
            1 => bad.end_handle = uid(),
            _ => bad.end_handle = t.frames[0].handle.clone(),
        };
        assert!(resolve_video_model_interval(t, &bad).is_err());
    }
    let source = video_annotation_source_rect(
        VideoPixelRect {
            x: 100.,
            y: 200.,
            width: 500.,
            height: 300.,
        },
        320,
        180,
    )
    .unwrap();
    assert_eq!(
        source,
        VideoPixelRect {
            x: 32.,
            y: 36.,
            width: 160.,
            height: 54.
        }
    );
    assert!(video_annotation_source_rect(
        VideoPixelRect {
            x: 990.,
            y: 0.,
            width: 20.,
            height: 100.
        },
        320,
        180
    )
    .is_err());
}
#[test]
fn active_total_raster_area_budget_rejects_whole_batch_without_clipping() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let big = Asset {
        id: uid(),
        width: Some(2000),
        height: Some(1800),
        ..f.proof.source().asset.clone()
    };
    // A separate real domain source is below the 4M single-source admission cap.
    let s = store.add_asset(&f.target.scene_id, big.clone()).unwrap();
    let item = s.scenes[0].items.last().unwrap();
    let target = VideoAnnotationTarget {
        item_id: item.id.clone(),
        source_id: big.id.clone(),
        expected_range_revision: 0,
        expected_annotation_revision: 0,
        ..f.target.clone()
    };
    let proof_source = VerifiedVideoAnnotationSource::new(
        VerifiedVideoSource {
            asset: big,
            width: 2000,
            height: 1800,
            duration_ticks: 10 * SECOND,
        },
        hash("large actual bytes fixture"),
    )
    .unwrap();
    let draft = VideoAnnotationDraft {
        interval: range(0, SECOND),
        primitive: VideoAnnotationPrimitive::Rect {
            bounds: VideoPixelRect {
                x: 0.,
                y: 0.,
                width: 2000.,
                height: 1800.,
            },
            style: VideoRectStyle {
                color: "#112233".into(),
                stroke_width: 2.,
                opacity: 1.,
            },
        },
    };
    let db = file.conn();
    let before = proof(&db);
    let state = store.snapshot();
    // 10*3.6M exceeds32Mi; 9*3.6M remains legal. No partial8/9-object commit.
    assert!(store
        .append_video_annotations(&target, &proof_source, vec![draft.clone(); 10], vec![])
        .is_err());
    assert_eq!(proof(&db), before);
    assert_eq!(store.snapshot(), state);
    store
        .append_video_annotations(&target, &proof_source, vec![draft; 9], vec![])
        .unwrap();
    assert_eq!(
        store
            .video_source(&target.scene_id, &target.item_id)
            .unwrap()
            .annotations
            .unwrap()
            .objects
            .len(),
        9
    );
}
#[test]
fn migration_abort_and_stale_store_write_never_leave_half_schema_or_document() {
    let file = Db::new();
    let store = Store::open(&file.0).unwrap();
    drop(store);
    let db = file.conn();
    downgrade_to_seven(&db);
    db.execute_batch("CREATE TRIGGER fail_migration BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    let old = file.saved();
    assert!(Store::open(&file.0).is_err());
    assert_eq!(file.saved(), old);
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        7
    );
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name LIKE 'video_%' OR name LIKE 'sqlite_autoindex_video_%'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    db.execute_batch("DROP TRIGGER fail_migration").unwrap();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let mut other = Store::open(&file.0).unwrap();
    other
        .apply(SceneCommand::SetDraft {
            scene_id: f.target.scene_id.clone(),
            draft: "另一写者".into(),
        })
        .unwrap();
    let before = proof(&db);
    let state = store.snapshot();
    let l = text_layout();
    assert!(matches!(
        store.append_video_annotations(
            &current(&store, &f),
            &f.proof,
            vec![text(range(0, SECOND), &l)],
            vec![l]
        ),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(proof(&db), before);
    assert_eq!(store.snapshot(), state);
}

#[test]
fn cursor_tampering_cannot_authorize_a_doc_revision_or_hash_without_final_receipt() {
    for committed in [false, true] {
        for field in ["annotationRevision", "documentSha256"] {
            let file = Db::new();
            let mut store = Store::open(&file.0).unwrap();
            let f = setup(&mut store, 10 * SECOND);
            let run = begin(&mut store, &f);
            if committed {
                let lease = leases(&mut store, &run, 1).remove(0);
                store
                    .apply_video_annotations_with_receipt(
                        &lease,
                        &run.authority,
                        batch(&run, vec![rect(range(0, 10 * SECOND))]),
                        vec![],
                    )
                    .unwrap();
            }
            store
                .finish_run(&run.run.scene_id, &run.run.run_id, "完成")
                .unwrap();
            drop(store);
            let db = file.conn();
            let raw: String = db
                .query_row(
                    "SELECT data FROM video_run_inputs WHERE run_id=?1",
                    [&run.run.run_id],
                    |r| r.get(0),
                )
                .unwrap();
            let mut parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
            parsed["cursors"][&f.target.item_id][field] = if field == "documentSha256" {
                json!(hash("arbitrary cursor"))
            } else {
                json!(9)
            };
            db.execute(
                "UPDATE video_run_inputs SET data=?1 WHERE run_id=?2",
                rusqlite::params![serde_json::to_string(&parsed).unwrap(), run.run.run_id],
            )
            .unwrap();
            let before = proof(&db);
            assert!(
                Store::open(&file.0).is_err(),
                "committed={committed} field={field}"
            );
            assert_eq!(proof(&db), before);
        }
    }
}
#[test]
fn prepared_video_input_with_stale_document_fence_keeps_real_draft_and_no_new_run() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: f.target.scene_id.clone(),
            draft: "真实待发输入".into(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: f.target.scene_id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Item,
                id: f.target.item_id.clone(),
            }],
        })
        .unwrap();
    let (input, assets, grant) = make_input(&store, &f);
    append(&mut store, &f, vec![rect(range(0, SECOND))], vec![]);
    let before = file.saved();
    let state = store.snapshot();
    assert!(matches!(
        store.begin_run_with_video_input(
            &f.target.scene_id,
            None,
            VerifiedVideoRunInput::new(grant, input, assets).unwrap()
        ),
        Err(CoreError::VideoAnnotationConflict)
    ));
    assert_eq!(file.saved(), before);
    assert_eq!(store.snapshot(), state);
    assert!(rows(&file.conn(), "video_run_inputs").is_empty());
}

fn extra_context(kind: AssetKind) -> Asset {
    let image = kind == AssetKind::Image;
    Asset {
        id: uid(),
        name: "其它真实上下文".into(),
        kind,
        path: format!("synthetic-context-{}", uid()),
        width: image.then_some(80),
        height: image.then_some(60),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
    }
}
fn with_extra_context(
    input: &VerifiedVideoInput,
    frames: &[Asset],
    extras: Vec<Asset>,
) -> (VerifiedVideoInput, Vec<Asset>) {
    let mut manifest = input.manifest().clone();
    for frame in &mut manifest.targets[0].frames {
        frame.attachment_ordinal += extras.len() as u32;
    }
    let input = VerifiedVideoInput::new(manifest).unwrap();
    let mut assets = extras;
    assets.extend_from_slice(frames);
    (input, assets)
}

#[test]
fn video_run_atomically_preserves_four_other_context_attachments_and_exact_frame_ordinals() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    store
        .apply(SceneCommand::SetDraft {
            scene_id: f.target.scene_id.clone(),
            draft: "请结合文字与图片在视频中标注".into(),
        })
        .unwrap();
    store
        .apply(SceneCommand::SetRefs {
            scene_id: f.target.scene_id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Item,
                id: f.target.item_id.clone(),
            }],
        })
        .unwrap();
    let (input, frames, grant) = make_input(&store, &f);
    let (input, assets) = with_extra_context(
        &input,
        &frames,
        [
            AssetKind::Image,
            AssetKind::Html,
            AssetKind::Svg,
            AssetKind::Text,
        ]
        .into_iter()
        .map(extra_context)
        .collect(),
    );
    let authority =
        VerifiedVideoAnnotationAuthority::new(grant.clone(), input.sha256().into()).unwrap();
    let run = store
        .begin_run_with_video_input(
            &f.target.scene_id,
            None,
            VerifiedVideoRunInput::new(grant, input.clone(), assets.clone()).unwrap(),
        )
        .unwrap();
    let user_message_id = run.messages.last().unwrap().id.clone();
    assert_eq!(
        run.messages.last().unwrap().attachments.as_deref(),
        Some(assets.as_slice())
    );
    assert_eq!(input.manifest().targets[0].frames[0].attachment_ordinal, 4);
    assert_eq!(input.manifest().targets[0].frames[1].attachment_ordinal, 5);
    assert_eq!(
        store
            .video_input_for_run(&run.scene_id, &run.run_id, &authority)
            .unwrap(),
        input
    );
    store
        .finish_run(&run.scene_id, &run.run_id, "完成")
        .unwrap();
    let saved = file.saved();
    drop(store);
    let reopened = Store::open(&file.0).unwrap();
    let scene = reopened
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == f.target.scene_id)
        .unwrap();
    assert_eq!(
        scene
            .messages
            .iter()
            .find(|m| m.id == user_message_id)
            .unwrap()
            .attachments
            .as_deref(),
        Some(assets.as_slice())
    );
    assert_eq!(file.saved(), saved);
}

#[test]
fn mixed_video_context_rejects_extra_kinds_duplicates_dimensions_and_frame_retarget_without_writes()
{
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let (original, frames, grant) = make_input(&store, &f);
    let (input, assets) = with_extra_context(
        &original,
        &frames,
        [
            AssetKind::Image,
            AssetKind::Html,
            AssetKind::Svg,
            AssetKind::Text,
        ]
        .into_iter()
        .map(extra_context)
        .collect(),
    );
    let saved = proof(&file.conn());
    for mutation in 0..12 {
        let mut bad = assets.clone();
        match mutation {
            0 => bad[0].kind = AssetKind::Video,
            1 => bad[0].kind = AssetKind::File,
            2 => bad[0].id = bad[1].id.clone(),
            3 => bad[0].path = " ".into(),
            4 => bad[0].width = None,
            5 => bad[1].width = Some(80),
            6 => bad[2].height = Some(0),
            7 => bad[0].scale_factor = Some(f64::NAN),
            8 => bad.swap(4, 5),
            9 => {
                bad.pop();
            }
            10 => bad.push(extra_context(AssetKind::Text)),
            _ => bad[5].width = Some(159),
        }
        assert!(
            VerifiedVideoRunInput::new(grant.clone(), input.clone(), bad).is_err(),
            "mutation={mutation}"
        );
        assert_eq!(proof(&file.conn()), saved);
    }
    for ordinals in [[4, 4], [5, 4], [4, 6]] {
        let mut manifest = input.manifest().clone();
        manifest.targets[0].frames[0].attachment_ordinal = ordinals[0];
        manifest.targets[0].frames[1].attachment_ordinal = ordinals[1];
        assert!(VerifiedVideoInput::new(manifest).is_err());
    }
    // Offset is not required to be zero; fewer context files cannot satisfy it.
    let (shifted, _) = with_extra_context(&original, &frames, vec![extra_context(AssetKind::Text)]);
    assert!(VerifiedVideoRunInput::new(grant, shifted, frames).is_err());
    assert_eq!(proof(&file.conn()), saved);
}

fn manual_edit_layout(body: &str) -> VerifiedVideoTextLayout {
    VerifiedVideoTextLayout::new(
        uid(),
        VideoTextContent {
            version: 1,
            text: body.into(),
            color: "#f0F2f6".into(),
            font_size: 23.5,
        },
        RichRendererIdentity {
            id: "native-video-text".into(),
            version: "fixture1".into(),
            font_sha256: hash("fixed font"),
            style_version: 1,
        },
        1,
        1,
        png(),
    )
    .unwrap()
}

#[test]
fn blackboard_video_copy_keeps_ai_origin_receipts_after_reopen_and_original_item_removal() {
    let file=Db::new();let mut store=Store::open(&file.0).unwrap();
    let f=setup(&mut store,10*SECOND);let run=begin(&mut store,&f);
    let lease=leases(&mut store,&run,1).remove(0);let layout=text_layout();
    let sent=&run.input.manifest().targets[0];
    store.apply_video_annotations_with_receipt(&lease,&run.authority,batch(&run,vec![text(range(sent.frames[0].source_playback_ticks,sent.sent_window.end_ticks),&layout)]),vec![layout]).unwrap();
    store.finish_run(&run.run.scene_id,&run.run.run_id,"synthetic complete").unwrap();
    let original=view(&store,&f).annotations.unwrap();assert!(original.objects[0].origin.is_some());
    let make_board=|preview:bool|{let id=uid();Asset{id:id.clone(),path:format!("D:/assets/{id}.{}.png",if preview{"board-preview"}else{"board"}),name:"黑板.png".into(),kind:AssetKind::Image,width:Some(1280),height:Some(720),origin_x:None,origin_y:None,scale_factor:None}};
    store.create_blackboard_document(&f.target.scene_id,make_board(false)).unwrap();
    let snapshot=store.snapshot();let board=snapshot.scenes.iter().find(|s|s.id==snapshot.active_scene_id).unwrap().clone();
    let copy=board.items.iter().find(|i|i.asset.kind==AssetKind::Video).unwrap();
    assert_ne!(copy.id,f.target.item_id);assert_eq!(copy.video_annotations.as_ref(),Some(&original));
    let link=board.blackboard_link.clone().unwrap();
    store.finish_blackboard_document(&board,make_board(true)).unwrap();
    store.apply(SceneCommand::RemoveItem{scene_id:f.target.scene_id.clone(),item_id:f.target.item_id.clone()}).unwrap();
    store.open_blackboard_document(&f.target.scene_id,&link.item_id).unwrap();
    let before=store.snapshot();drop(store);
    let reopened=Store::open(&file.0).unwrap();assert_eq!(reopened.snapshot(),before);
}

#[test]
fn manual_text_update_retains_ai_identity_interval_literal_content_and_durable_history() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let run = begin(&mut store, &f);
    let lease = leases(&mut store, &run, 1).remove(0);
    let body = "  原始\t文字\n<svg>&\"quoted\" 数学α  ";
    let old_layout = manual_edit_layout(body);
    let old_ref = old_layout.reference().clone();
    let sent = &run.input.manifest().targets[0];
    let interval = resolve_video_model_interval(
        sent,
        &ModelVideoInterval {
            start_frame_handle: sent.frames[1].handle.clone(),
            end_handle: sent.range_end_handle.clone(),
        },
    )
    .unwrap();
    assert_eq!(interval, range(5 * SECOND, 10 * SECOND));
    assert_ne!(interval.start_ticks, sent.frames[1].source_pts_ticks);
    store
        .apply_video_annotations_with_receipt(
            &lease,
            &run.authority,
            batch(&run, vec![text(interval, &old_layout)]),
            vec![old_layout],
        )
        .unwrap();
    store
        .finish_run(&run.run.scene_id, &run.run.run_id, "synthetic complete")
        .unwrap();
    let original = view(&store, &f).annotations.unwrap();
    let object = original.objects[0].clone();
    assert!(object.origin.is_some());
    assert_eq!(
        store.video_text_layout(&old_ref).unwrap().content().text,
        body
    );
    let VideoAnnotationPrimitive::Text { top_left, .. } = object.primitive else {
        panic!("text fixture")
    };
    let replacement_body = "  改后\t仍保留\n<svg>&\"exact\" β  ";
    let replacement = manual_edit_layout(replacement_body);
    let next_ref = replacement.reference().clone();
    let before_raw = file.saved();
    store
        .edit_video_annotation_with_layouts(
            &current(&store, &f),
            VideoAnnotationMutation::Update {
                id: object.id.clone(),
                replacement: VideoAnnotationDraft {
                    interval: object.interval,
                    primitive: VideoAnnotationPrimitive::Text {
                        top_left,
                        layout: next_ref.clone(),
                    },
                },
            },
            vec![replacement],
        )
        .unwrap();
    let updated = view(&store, &f).annotations.unwrap();
    assert_eq!(updated.revision, original.revision + 1);
    assert_eq!(updated.undo.len(), original.undo.len() + 1);
    assert!(updated.redo.is_empty());
    assert_eq!(file.saved().1, before_raw.1 + 1);
    assert_eq!(updated.objects[0].id, object.id);
    assert_eq!(updated.objects[0].origin, object.origin);
    assert_eq!(updated.objects[0].interval, object.interval);
    assert_eq!(
        store.video_text_layout(&next_ref).unwrap().content().text,
        replacement_body
    );
    assert_eq!(
        store.video_text_layout(&old_ref).unwrap().content().text,
        body
    );
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    assert_eq!(view(&store, &f).annotations, Some(updated.clone()));
    assert_eq!(
        Store::read_video_text_layout(&file.0, &next_ref)
            .unwrap()
            .content()
            .text,
        replacement_body
    );
    replay(&mut store, &f, false);
    assert_eq!(
        view(&store, &f).annotations.unwrap().objects,
        original.objects
    );
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    replay(&mut store, &f, true);
    assert_eq!(
        view(&store, &f).annotations.unwrap().objects,
        updated.objects
    );
    assert_eq!(
        store.video_text_layout(&old_ref).unwrap().content().text,
        body
    );
}

#[test]
fn manual_existing_text_update_sql_abort_rolls_back_new_layout_history_and_state() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let old = manual_edit_layout("durable original\nexact body");
    let old_ref = old.reference().clone();
    append(
        &mut store,
        &f,
        vec![text(range(SECOND, 8 * SECOND), &old)],
        vec![old],
    );
    let object = view(&store, &f).annotations.unwrap().objects[0].clone();
    let VideoAnnotationPrimitive::Text { top_left, .. } = object.primitive else {
        panic!("text fixture")
    };
    let db = file.conn();
    let before_sql = proof(&db);
    let before_state = store.snapshot();
    // The candidate layout is inserted BEFORE this app_state update. Therefore
    // this failure proves the encompassing transaction, not early validation.
    db.execute_batch("CREATE TRIGGER reject_manual_text_commit BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'synthetic after layout insert'); END").unwrap();
    let next = manual_edit_layout("replacement not allowed to persist");
    let next_ref = next.reference().clone();
    assert!(store
        .edit_video_annotation_with_layouts(
            &current(&store, &f),
            VideoAnnotationMutation::Update {
                id: object.id,
                replacement: VideoAnnotationDraft {
                    interval: object.interval,
                    primitive: VideoAnnotationPrimitive::Text {
                        top_left,
                        layout: next_ref.clone()
                    },
                },
            },
            vec![next],
        )
        .is_err());
    assert_eq!(proof(&db), before_sql);
    assert_eq!(store.snapshot(), before_state);
    assert!(store.video_text_layout(&next_ref).is_err());
    assert_eq!(
        store.video_text_layout(&old_ref).unwrap().content().text,
        "durable original\nexact body"
    );
    db.execute_batch("DROP TRIGGER reject_manual_text_commit")
        .unwrap();
    drop(store);
    assert_eq!(Store::open(&file.0).unwrap().snapshot(), before_state);
}

#[test]
fn manual_text_noop_preserves_redo_without_sql_write_and_stale_store_cannot_ack() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let old = manual_edit_layout("keep literal");
    append(
        &mut store,
        &f,
        vec![text(range(SECOND, 8 * SECOND), &old)],
        vec![old],
    );
    let object = view(&store, &f).annotations.unwrap().objects[0].clone();
    let VideoAnnotationPrimitive::Text { layout, .. } = &object.primitive else {
        panic!("text fixture")
    };
    let old_ref = layout.clone();
    let moved = VideoAnnotationPrimitive::Text {
        top_left: VideoPixelPoint { x: 121.25, y: 80.5 },
        layout: old_ref.clone(),
    };
    store
        .edit_video_annotation_with_layouts(
            &current(&store, &f),
            VideoAnnotationMutation::Update {
                id: object.id.clone(),
                replacement: VideoAnnotationDraft {
                    interval: object.interval,
                    primitive: moved,
                },
            },
            vec![],
        )
        .unwrap();
    replay(&mut store, &f, false);
    let doc = view(&store, &f).annotations.unwrap();
    assert_eq!(doc.redo.len(), 1);
    let noop = VideoAnnotationMutation::Update {
        id: object.id,
        replacement: VideoAnnotationDraft {
            interval: object.interval,
            primitive: object.primitive,
        },
    };
    let target = current(&store, &f);
    let before = proof(&file.conn());
    let state = store.snapshot();
    let db = file.conn();
    db.execute_batch("CREATE TRIGGER reject_noop_app_write BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'no-op must not write'); END;CREATE TRIGGER reject_noop_layout_insert BEFORE INSERT ON video_text_layouts BEGIN SELECT RAISE(ABORT,'no-op must reuse old ref'); END;").unwrap();
    store
        .edit_video_annotation_with_layouts(&target, noop.clone(), vec![])
        .unwrap();
    assert_eq!(proof(&db), before);
    assert_eq!(store.snapshot(), state);
    assert_eq!(view(&store, &f).annotations.unwrap().redo, doc.redo);
    assert!(store
        .edit_video_annotation_with_layouts(
            &target,
            noop.clone(),
            vec![manual_edit_layout("keep literal")]
        )
        .is_err());
    assert_eq!(proof(&db), before);
    db.execute_batch("DROP TRIGGER reject_noop_app_write;DROP TRIGGER reject_noop_layout_insert")
        .unwrap();
    let mut second = Store::open(&file.0).unwrap();
    second
        .apply(SceneCommand::SetDraft {
            scene_id: f.target.scene_id.clone(),
            draft: "other writer changes global revision only".into(),
        })
        .unwrap();
    let persisted = proof(&db);
    assert!(matches!(
        store.edit_video_annotation_with_layouts(&target, noop, vec![]),
        Err(CoreError::ConcurrentModification)
    ));
    assert_eq!(proof(&db), persisted);
    assert_eq!(store.snapshot(), state);
    drop(second);
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    replay(&mut store, &f, true);
    assert_eq!(
        view(&store, &f).annotations.unwrap().objects[0].primitive,
        VideoAnnotationPrimitive::Text {
            top_left: VideoPixelPoint { x: 121.25, y: 80.5 },
            layout: old_ref
        }
    );
}

#[test]
fn manual_text_move_uses_full_layout_bounds_and_exact_ref_not_only_layout_id() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let layout = manual_edit_layout("edge text");
    let reference = layout.reference().clone();
    append(
        &mut store,
        &f,
        vec![text(range(SECOND, 8 * SECOND), &layout)],
        vec![layout],
    );
    let original = view(&store, &f).annotations.unwrap().objects[0].clone();
    let target = current(&store, &f);
    let before = proof(&file.conn());
    for point in [
        VideoPixelPoint { x: 319.25, y: 179. },
        VideoPixelPoint { x: 319., y: 179.25 },
        VideoPixelPoint { x: -0.001, y: 0. },
        VideoPixelPoint { x: f64::NAN, y: 0. },
    ] {
        assert!(store
            .edit_video_annotation_with_layouts(
                &target,
                VideoAnnotationMutation::Update {
                    id: original.id.clone(),
                    replacement: VideoAnnotationDraft {
                        interval: original.interval,
                        primitive: VideoAnnotationPrimitive::Text {
                            top_left: point,
                            layout: reference.clone()
                        },
                    }
                },
                vec![],
            )
            .is_err());
        assert_eq!(proof(&file.conn()), before);
    }
    let mut forged = reference.clone();
    forged.width += 1;
    assert!(Store::read_video_text_layout(&file.0, &forged).is_err());
    store
        .edit_video_annotation_with_layouts(
            &target,
            VideoAnnotationMutation::Update {
                id: original.id.clone(),
                replacement: VideoAnnotationDraft {
                    interval: original.interval,
                    primitive: VideoAnnotationPrimitive::Text {
                        top_left: VideoPixelPoint { x: 319., y: 179. },
                        layout: reference.clone(),
                    },
                },
            },
            vec![],
        )
        .unwrap();
    let moved = view(&store, &f).annotations.unwrap();
    assert_eq!(moved.objects[0].id, original.id);
    assert_eq!(moved.objects[0].interval, original.interval);
    assert_eq!(moved.objects[0].origin, original.origin);
    assert_eq!(
        Store::read_video_text_layout(&file.0, &reference)
            .unwrap()
            .content()
            .text,
        "edge text"
    );
    replay(&mut store, &f, false);
    assert_eq!(view(&store, &f).annotations.unwrap().objects[0], original);
}

#[test]
fn manual_source_getter_is_exact_target_no_write_and_readonly_can_inspect_running() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 10 * SECOND);
    let empty_target = current(&store, &f);
    assert!(store.video_annotation_source(&empty_target, false).is_err());
    assert!(store.video_annotation_source(&empty_target, true).is_err());
    append(
        &mut store,
        &f,
        vec![rect(range(SECOND, 8 * SECOND))],
        vec![],
    );
    assert!(store.video_annotation_source(&empty_target, false).is_err());
    let target = current(&store, &f);
    let before = proof(&file.conn());
    let state = store.snapshot();
    let expected = view(&store, &f);
    assert_eq!(
        store.video_annotation_source(&target, false).unwrap(),
        expected
    );
    assert_eq!(
        store.video_annotation_source(&target, true).unwrap(),
        expected
    );
    for field in 0..5 {
        let mut stale = target.clone();
        match field {
            0 => stale.scene_id = uid(),
            1 => stale.item_id = uid(),
            2 => stale.source_id = uid(),
            3 => stale.expected_range_revision += 1,
            _ => stale.expected_annotation_revision += 1,
        }
        assert!(store.video_annotation_source(&stale, false).is_err());
        assert!(store.video_annotation_source(&stale, true).is_err());
    }
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(store.snapshot(), state);
    let run = begin(&mut store, &f);
    let before = proof(&file.conn());
    assert_eq!(
        store.video_annotation_source(&target, false).unwrap(),
        expected
    );
    assert!(matches!(
        store.video_annotation_source(&target, true),
        Err(CoreError::RunInProgress)
    ));
    assert_eq!(proof(&file.conn()), before);
    store
        .finish_run(&run.run.scene_id, &run.run.run_id, "synthetic complete")
        .unwrap();
    assert_eq!(
        store.video_annotation_source(&target, true).unwrap(),
        expected
    );
}

#[test]
fn manual_source_getter_both_modes_reject_inactive_frozen_closed_or_removed_objects() {
    let mut store = Store::open_in_memory().unwrap();
    let f = setup(&mut store, 10 * SECOND);
    append(
        &mut store,
        &f,
        vec![rect(range(SECOND, 8 * SECOND))],
        vec![],
    );
    let target = current(&store, &f);
    let expected = view(&store, &f);
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: f.target.scene_id.clone(),
        })
        .unwrap();
    for editing in [false, true] {
        assert!(store.video_annotation_source(&target, editing).is_err());
    }
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: f.target.scene_id.clone(),
        })
        .unwrap();
    assert_eq!(
        store.video_annotation_source(&target, false).unwrap(),
        expected
    );
    assert_eq!(
        store.video_annotation_source(&target, true).unwrap(),
        expected
    );
    store
        .apply(SceneCommand::CloseScene {
            scene_id: f.target.scene_id.clone(),
        })
        .unwrap();
    for editing in [false, true] {
        assert!(store.video_annotation_source(&target, editing).is_err());
    }
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: f.target.scene_id.clone(),
        })
        .unwrap();
    store
        .apply(SceneCommand::RemoveItem {
            scene_id: f.target.scene_id.clone(),
            item_id: f.target.item_id.clone(),
        })
        .unwrap();
    for editing in [false, true] {
        assert!(store.video_annotation_source(&target, editing).is_err());
    }
}
