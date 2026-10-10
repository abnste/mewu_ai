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
        "video_vector_layouts",
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
    setup_size(store, duration, 320, 180)
}
fn setup_size(store: &mut Store, duration: u64, width: u32, height: u32) -> Source {
    let scene_id = store.snapshot().active_scene_id;
    let asset = Asset {
        id: uid(),
        name: "synthetic.mp4".into(),
        path: "owned-synthetic.mp4".into(),
        kind: AssetKind::Video,
        width: Some(width),
        height: Some(height),
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
                width,
                height,
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

fn rgba_png(w: u32, h: u32) -> Vec<u8> {
    match (w, h) {
        (10, 10) => vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 10, 0, 0, 0, 10,
            8, 6, 0, 0, 0, 141, 50, 207, 189, 0, 0, 0, 14, 73, 68, 65, 84, 120, 156, 99, 96, 24, 5,
            131, 19, 0, 0, 1, 154, 0, 1, 29, 130, 86, 168, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
            130,
        ],
        (1000, 1000) => vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 3, 232, 0, 0, 3,
            232, 8, 6, 0, 0, 0, 77, 163, 212, 228, 0, 0, 15, 60, 73, 68, 65, 84, 120, 156, 237,
            193, 1, 13, 0, 0, 0, 194, 160, 247, 79, 109, 14, 55, 160, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 119, 3, 16, 123, 0, 1, 133, 222, 249, 140, 0, 0, 0, 0,
            73, 69, 78, 68, 174, 66, 96, 130,
        ],
        _ => panic!("unknown valid PNG fixture"),
    }
}
fn renderer() -> RichRendererIdentity {
    RichRendererIdentity {
        id: "native-vector-fixture".into(),
        version: "1".into(),
        font_sha256: hash("pinned fixture font"),
        style_version: 1,
    }
}
fn content(points: Vec<VideoPixelPoint>) -> VideoVectorContent {
    VideoVectorContent::Pen {
        version: 1,
        local_points: points,
        color: "#FF4455".into(),
        stroke_width: 8.,
    }
}
fn vector_layout() -> VerifiedVideoVectorLayout {
    VerifiedVideoVectorLayout::new(
        uid(),
        content(vec![VideoPixelPoint { x: 4., y: 4. }]),
        renderer(),
        10,
        10,
        rgba_png(10, 10),
    )
    .unwrap()
}
fn vector(l: &VerifiedVideoVectorLayout, x: f64, y: f64) -> VideoAnnotationPrimitive {
    VideoAnnotationPrimitive::Vector {
        top_left: VideoPixelPoint { x, y },
        layout: l.reference().clone(),
    }
}
fn mixed(
    text: Vec<VerifiedVideoTextLayout>,
    vector: Vec<VerifiedVideoVectorLayout>,
) -> VideoAnnotationLayouts {
    VideoAnnotationLayouts { text, vector }
}
fn add_vector(store: &mut Store, f: &Source, l: VerifiedVideoVectorLayout, x: f64, y: f64) {
    store
        .append_manual_video_drawings(
            &current(store, f),
            &f.proof,
            vec![vector(&l, x, y)],
            mixed(vec![], vec![l]),
        )
        .unwrap();
}
fn update(
    store: &mut Store,
    f: &Source,
    primitive: VideoAnnotationPrimitive,
    new: VideoAnnotationLayouts,
) -> Result<Snapshot, CoreError> {
    let doc = view(store, f).annotations.unwrap();
    let old = &doc.objects[0];
    store.edit_video_annotation_with_registered_layouts(
        &current(store, f),
        VideoAnnotationMutation::Update {
            id: old.id.clone(),
            replacement: VideoAnnotationDraft {
                interval: old.interval,
                primitive,
            },
        },
        new,
    )
}
fn degrade_eight(db: &Connection) {
    db.execute_batch("DROP TRIGGER video_vector_layouts_no_update;DROP TRIGGER video_vector_layouts_no_delete;DROP TABLE video_vector_layouts;PRAGMA user_version=8;").unwrap();
}
#[test]
fn vector_content_is_typed_and_geometry_is_derived_not_supplied() {
    let p = VideoPixelPoint { x: 2.5, y: 3.5 };
    let v = VerifiedVideoVectorLayout::new(
        uid(),
        VideoVectorContent::Rect {
            version: 1,
            local_points: vec![p, VideoPixelPoint { x: 8., y: 9. }],
            color: "#112233".into(),
            stroke_width: 2.,
        },
        renderer(),
        10,
        10,
        rgba_png(10, 10),
    )
    .unwrap();
    assert_eq!(
        v.reference().geometry_bounds,
        VideoPixelRect {
            x: 2.5,
            y: 3.5,
            width: 5.5,
            height: 5.5
        }
    );
    let mut raw = serde_json::to_value(v.content()).unwrap();
    raw["sourcePoints"] = raw["localPoints"].clone();
    assert!(serde_json::from_value::<VideoVectorContent>(raw).is_err());
    for invalid in [
        VideoVectorContent::Line {
            version: 1,
            local_points: vec![p, p],
            color: "#112233".into(),
            stroke_width: 2.,
        },
        VideoVectorContent::Rect {
            version: 1,
            local_points: vec![p, VideoPixelPoint { x: 2.5, y: 9. }],
            color: "#112233".into(),
            stroke_width: 2.,
        },
        VideoVectorContent::Number {
            version: 1,
            local_points: vec![p],
            color: "#112233".into(),
            number: 1,
            diameter: 22.,
        },
        VideoVectorContent::Pen {
            version: 1,
            local_points: vec![VideoPixelPoint { x: f64::NAN, y: 0. }],
            color: "#112233".into(),
            stroke_width: 2.,
        },
        VideoVectorContent::Pen {
            version: 1,
            local_points: vec![p; 4097],
            color: "#112233".into(),
            stroke_width: 2.,
        },
    ] {
        assert!(VerifiedVideoVectorLayout::new(
            uid(),
            invalid,
            renderer(),
            10,
            10,
            rgba_png(10, 10)
        )
        .is_err());
    }
    let mapped = v.content().map_points(|p| VideoPixelPoint {
        x: p.x + 1.,
        y: p.y,
    });
    assert_eq!(mapped.kind(), VideoVectorKind::Rect);
    assert_eq!(mapped.color(), v.content().color());
    assert_eq!(mapped.stroke_width(), v.content().stroke_width());
    assert_eq!(mapped.points()[0], VideoPixelPoint { x: 3.5, y: 3.5 });
}
#[test]
fn manual_append_uses_original_full_time_and_core_identity_despite_trim() {
    let mut store = Store::open_in_memory().unwrap();
    let f = setup(&mut store, 12 * SECOND);
    assert!(store
        .video_annotation_creation_source(&f.target)
        .unwrap()
        .annotations
        .is_none());
    assert!(store.video_annotation_source(&f.target, false).is_err());
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
            Some(range(3 * SECOND, 8 * SECOND)),
        )
        .unwrap();
    let l = vector_layout();
    let full = l.reference().clone();
    add_vector(&mut store, &f, l, -4., -4.);
    let doc = view(&store, &f).annotations.unwrap();
    let o = &doc.objects[0];
    assert_eq!(o.interval, range(0, 12 * SECOND));
    assert!(o.origin.is_none());
    assert!(uuid::Uuid::parse_str(&o.id).is_ok());
    assert_eq!(doc.undo[0].after, doc.objects);
    assert_eq!(
        doc.objects[0].primitive,
        VideoAnnotationPrimitive::Vector {
            top_left: VideoPixelPoint { x: -4., y: -4. },
            layout: full
        }
    );
    assert_eq!(
        view(&store, &f).edit.unwrap().range,
        Some(range(3 * SECOND, 8 * SECOND))
    );
}
#[test]
fn full_ink_ref_survives_edge_move_reopen_and_undo_redo() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let l = vector_layout();
    let reference = l.reference().clone();
    let original_png = l.png().to_vec();
    add_vector(&mut store, &f, l, -4., -4.);
    let first = view(&store, &f).annotations.unwrap();
    update(
        &mut store,
        &f,
        VideoAnnotationPrimitive::Vector {
            top_left: VideoPixelPoint { x: 10., y: 10. },
            layout: reference.clone(),
        },
        mixed(vec![], vec![]),
    )
    .unwrap();
    let moved = view(&store, &f).annotations.unwrap();
    assert_eq!(moved.objects[0].id, first.objects[0].id);
    assert_eq!(moved.objects[0].interval, first.objects[0].interval);
    assert_eq!(moved.objects[0].origin, first.objects[0].origin);
    assert_eq!(
        store.video_vector_layout(&reference).unwrap().png(),
        original_png
    );
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    assert_eq!(view(&store, &f).annotations.unwrap(), moved);
    let literal = Store::read_video_vector_layout(&file.0, &reference).unwrap();
    assert_eq!(literal.png(), original_png);
    assert_eq!(
        literal.content().points(),
        &[VideoPixelPoint { x: 4., y: 4. }]
    );
    replay(&mut store, &f, false);
    assert_eq!(view(&store, &f).annotations.unwrap().objects, first.objects);
    replay(&mut store, &f, true);
    assert_eq!(view(&store, &f).annotations.unwrap().objects, moved.objects);
    assert_eq!(rows(&file.conn(), "video_vector_layouts").len(), 1);
}
#[test]
fn fractional_move_control_overflow_and_ref_tamper_are_atomic_rejections() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let l = vector_layout();
    let r = l.reference().clone();
    add_vector(&mut store, &f, l, -4., -4.);
    for (x, y) in [
        (-4.5, -4.),
        (-5., -4.),
        (317., 0.),
        (0., 177.),
        (f64::INFINITY, 0.),
    ] {
        let before = proof(&file.conn());
        let snapshot = store.snapshot();
        assert!(update(
            &mut store,
            &f,
            VideoAnnotationPrimitive::Vector {
                top_left: VideoPixelPoint { x, y },
                layout: r.clone()
            },
            mixed(vec![], vec![])
        )
        .is_err());
        assert_eq!(proof(&file.conn()), before);
        assert_eq!(store.snapshot(), snapshot);
    }
    let mut forged = r.clone();
    forged.geometry_bounds.x += 1.;
    assert!(update(
        &mut store,
        &f,
        VideoAnnotationPrimitive::Vector {
            top_left: VideoPixelPoint { x: 0., y: 0. },
            layout: forged
        },
        mixed(vec![], vec![])
    )
    .is_err());
    let mut forged = r;
    forged.raster_sha256 = hash("wrong bytes");
    assert!(Store::read_video_vector_layout(&file.0, &forged).is_err());
}
#[test]
fn stale_scene_source_range_document_and_running_creation_guards_do_not_write() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let l = vector_layout();
    let original = current(&store, &f);
    add_vector(&mut store, &f, l, 10., 10.);
    for field in 0..5 {
        let mut target = current(&store, &f);
        match field {
            0 => target.scene_id = uid(),
            1 => target.item_id = uid(),
            2 => target.source_id = uid(),
            3 => target.expected_range_revision += 1,
            _ => target.expected_annotation_revision += 1,
        }
        let before = proof(&file.conn());
        let l = vector_layout();
        assert!(store
            .append_manual_video_drawings(
                &target,
                &f.proof,
                vec![vector(&l, 10., 10.)],
                mixed(vec![], vec![l])
            )
            .is_err());
        assert!(store.video_annotation_creation_source(&target).is_err());
        assert_eq!(proof(&file.conn()), before);
    }
    assert!(store.video_annotation_creation_source(&original).is_err());
    let run = begin(&mut store, &f);
    assert!(store
        .video_annotation_creation_source(&current(&store, &f))
        .is_err());
    assert!(store
        .video_annotation_source(&current(&store, &f), false)
        .is_ok());
    store
        .fail_run(&run.run.scene_id, &run.run.run_id, "synthetic stopped")
        .unwrap();
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: f.target.scene_id.clone(),
        })
        .unwrap();
    assert!(store
        .video_annotation_creation_source(&current(&store, &f))
        .is_err());
}
#[test]
fn new_layout_and_update_are_one_sql_transaction_with_no_orphan_on_failure() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    add_vector(&mut store, &f, vector_layout(), 10., 10.);
    let before = proof(&file.conn());
    let state = store.snapshot();
    file.conn().execute_batch("CREATE TRIGGER reject_video_state BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    let replacement = vector_layout();
    let r = replacement.reference().clone();
    assert!(update(
        &mut store,
        &f,
        vector(&replacement, 10., 10.),
        mixed(vec![], vec![replacement])
    )
    .is_err());
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(store.snapshot(), state);
    assert!(store.video_vector_layout(&r).is_err());
    file.conn()
        .execute_batch("DROP TRIGGER reject_video_state")
        .unwrap();
    let l = vector_layout();
    let unused = vector_layout();
    let before = proof(&file.conn());
    assert!(store
        .append_manual_video_drawings(
            &current(&store, &f),
            &f.proof,
            vec![vector(&l, 10., 10.)],
            mixed(vec![], vec![l, unused])
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
}
#[test]
fn equal_update_performs_no_sql_write_preserves_redo_and_literal_ref() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let l = vector_layout();
    let r = l.reference().clone();
    add_vector(&mut store, &f, l, 10., 10.);
    update(
        &mut store,
        &f,
        VideoAnnotationPrimitive::Vector {
            top_left: VideoPixelPoint { x: 20., y: 20. },
            layout: r,
        },
        mixed(vec![], vec![]),
    )
    .unwrap();
    replay(&mut store, &f, false);
    let doc = view(&store, &f).annotations.unwrap();
    assert!(!doc.redo.is_empty());
    let before = proof(&file.conn());
    let state = store.snapshot();
    file.conn().execute_batch("CREATE TRIGGER no_state BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'noop wrote');END;CREATE TRIGGER no_vector BEFORE INSERT ON video_vector_layouts BEGIN SELECT RAISE(ABORT,'noop registered');END;").unwrap();
    let accepted = update(
        &mut store,
        &f,
        doc.objects[0].primitive.clone(),
        mixed(vec![], vec![]),
    )
    .unwrap();
    assert_eq!(accepted, state);
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(view(&store, &f).annotations.unwrap(), doc);
}
#[test]
fn four_thousand_points_stay_out_of_compact_history_and_style_edit_is_reversible() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let old = VerifiedVideoVectorLayout::new(
        uid(),
        content(vec![VideoPixelPoint { x: 4., y: 4. }; 4096]),
        renderer(),
        10,
        10,
        rgba_png(10, 10),
    )
    .unwrap();
    let old_ref = old.reference().clone();
    add_vector(&mut store, &f, old, 10., 10.);
    let doc = view(&store, &f).annotations.unwrap();
    let id = doc.objects[0].id.clone();
    assert!(serde_json::to_vec(&doc).unwrap().len() < 10_000);
    assert_eq!(
        store
            .video_vector_layout(&old_ref)
            .unwrap()
            .content()
            .points()
            .len(),
        4096
    );
    let changed = VerifiedVideoVectorLayout::new(
        uid(),
        VideoVectorContent::Pen {
            version: 1,
            local_points: vec![VideoPixelPoint { x: 4., y: 4. }; 4096],
            color: "#556677".into(),
            stroke_width: 8.,
        },
        renderer(),
        10,
        10,
        rgba_png(10, 10),
    )
    .unwrap();
    let new_ref = changed.reference().clone();
    assert_ne!(old_ref.layout_sha256, new_ref.layout_sha256);
    assert_eq!(old_ref.raster_sha256, new_ref.raster_sha256);
    update(
        &mut store,
        &f,
        vector(&changed, 10., 10.),
        mixed(vec![], vec![changed]),
    )
    .unwrap();
    assert_eq!(view(&store, &f).annotations.unwrap().objects[0].id, id);
    drop(store);
    let mut store = Store::open(&file.0).unwrap();
    replay(&mut store, &f, false);
    assert_eq!(view(&store, &f).annotations.unwrap().objects, doc.objects);
    replay(&mut store, &f, true);
    assert!(
        matches!(&view(&store,&f).annotations.unwrap().objects[0].primitive,VideoAnnotationPrimitive::Vector{layout,..} if layout==&new_ref)
    );
    assert_eq!(
        store
            .video_vector_layout(&old_ref)
            .unwrap()
            .content()
            .color(),
        "#FF4455"
    );
}
#[test]
fn old_logical_append_and_ai_received_batch_cannot_create_vector_objects() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let l = vector_layout();
    let draft = VideoAnnotationDraft {
        interval: range(0, 12 * SECOND),
        primitive: vector(&l, 10., 10.),
    };
    let before = proof(&file.conn());
    assert!(store
        .append_video_annotations(&current(&store, &f), &f.proof, vec![draft.clone()], vec![])
        .is_err());
    assert_eq!(proof(&file.conn()), before);
    let run = begin(&mut store, &f);
    let call = leases(&mut store, &run, 1).remove(0);
    let t = &run.input.manifest().targets[0];
    let mut draft = draft;
    draft.interval = range(t.frames[0].source_playback_ticks, t.sent_window.end_ticks);
    let before = proof(&file.conn());
    let state = store.snapshot();
    assert!(store
        .apply_video_annotations_with_receipt(
            &call,
            &run.authority,
            batch(&run, vec![draft]),
            vec![]
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(store.snapshot(), state);
}
#[test]
fn vector_cannot_borrow_an_old_layout_from_another_document_or_family() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let l = vector_layout();
    let r = l.reference().clone();
    add_vector(&mut store, &f, l, 10., 10.);
    let other = setup(&mut store, 12 * SECOND);
    let before = proof(&file.conn());
    assert!(store
        .append_manual_video_drawings(
            &current(&store, &other),
            &other.proof,
            vec![VideoAnnotationPrimitive::Vector {
                top_left: VideoPixelPoint { x: 10., y: 10. },
                layout: r.clone()
            }],
            mixed(vec![], vec![])
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
    let duplicate = VerifiedVideoTextLayout::new(
        r.layout_id,
        VideoTextContent {
            version: 1,
            text: "literal".into(),
            color: "#112233".into(),
            font_size: 16.,
        },
        renderer(),
        1,
        1,
        png(),
    )
    .unwrap();
    let primitive = VideoAnnotationPrimitive::Text {
        top_left: VideoPixelPoint { x: 10., y: 10. },
        layout: duplicate.reference().clone(),
    };
    assert!(store
        .append_manual_video_drawings(
            &current(&store, &other),
            &other.proof,
            vec![primitive],
            mixed(vec![duplicate], vec![])
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
}
#[test]
fn all_active_instances_share_the_raster_area_budget_even_with_one_layout() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup_size(&mut store, 12 * SECOND, 1000, 1000);
    let l = VerifiedVideoVectorLayout::new(
        uid(),
        content(vec![
            VideoPixelPoint { x: 4., y: 4. },
            VideoPixelPoint { x: 996., y: 996. },
        ]),
        renderer(),
        1000,
        1000,
        rgba_png(1000, 1000),
    )
    .unwrap();
    let primitive = vector(&l, 0., 0.);
    let before = proof(&file.conn());
    let state = store.snapshot();
    assert!(store
        .append_manual_video_drawings(
            &current(&store, &f),
            &f.proof,
            vec![primitive.clone(); 34],
            mixed(vec![], vec![l.clone()])
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(store.snapshot(), state);
    store
        .append_manual_video_drawings(
            &current(&store, &f),
            &f.proof,
            vec![primitive; 32],
            mixed(vec![], vec![l]),
        )
        .unwrap();
    assert_eq!(rows(&file.conn(), "video_vector_layouts").len(), 1);
    assert_eq!(view(&store, &f).annotations.unwrap().objects.len(), 32);
}
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, v) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb88320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *v = c;
    }
    let mut crc = 0xffff_ffffu32;
    for b in data {
        crc = table[((crc ^ u32::from(*b)) & 255) as usize] ^ (crc >> 8);
    }
    !crc
}
fn padded_png() -> Vec<u8> {
    let mut p = rgba_png(10, 10);
    let end = p.split_off(p.len() - 12);
    let n = MAX_VIDEO_VECTOR_PNG_BYTES - p.len() - end.len() - 12;
    p.extend_from_slice(&(u32::try_from(n).unwrap()).to_be_bytes());
    let chunk = p.len();
    p.extend_from_slice(b"ruSt");
    p.resize(p.len() + n, 0);
    let crc = crc32(&p[chunk..]);
    p.extend_from_slice(&crc.to_be_bytes());
    p.extend(end);
    assert_eq!(p.len(), MAX_VIDEO_VECTOR_PNG_BYTES);
    p
}
#[test]
fn text_and_vector_current_and_history_use_one_png_byte_budget() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    for n in 0..4 {
        if n % 2 == 0 {
            let l = VerifiedVideoVectorLayout::new(
                uid(),
                content(vec![VideoPixelPoint { x: 4., y: 4. }]),
                renderer(),
                10,
                10,
                padded_png(),
            )
            .unwrap();
            add_vector(&mut store, &f, l, 10., 10.);
        } else {
            let l = VerifiedVideoTextLayout::new(
                uid(),
                VideoTextContent {
                    version: 1,
                    text: format!("literal{n}"),
                    color: "#112233".into(),
                    font_size: 16.,
                },
                renderer(),
                10,
                10,
                padded_png(),
            )
            .unwrap();
            let primitive = VideoAnnotationPrimitive::Text {
                top_left: VideoPixelPoint { x: 10., y: 10. },
                layout: l.reference().clone(),
            };
            store
                .append_manual_video_drawings(
                    &current(&store, &f),
                    &f.proof,
                    vec![primitive],
                    mixed(vec![l], vec![]),
                )
                .unwrap();
        }
    }
    let doc = view(&store, &f).annotations.unwrap();
    store
        .edit_video_annotation_with_registered_layouts(
            &current(&store, &f),
            VideoAnnotationMutation::Remove {
                id: doc.objects[0].id.clone(),
            },
            mixed(vec![], vec![]),
        )
        .unwrap();
    let before = proof(&file.conn());
    let snapshot = store.snapshot();
    let l = vector_layout();
    let r = l.reference().clone();
    assert!(store
        .append_manual_video_drawings(
            &current(&store, &f),
            &f.proof,
            vec![vector(&l, 10., 10.)],
            mixed(vec![], vec![l])
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(store.snapshot(), snapshot);
    assert!(store.video_vector_layout(&r).is_err());
}
#[test]
fn unique_descriptors_are_bounded_without_inlining_or_silent_partial_batches() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let points: Vec<_> = (0..4096)
        .map(|n| VideoPixelPoint {
            x: 4.123456789012345 + (f64::from(n) * 0.000000010101010101),
            y: 4.876543210987654 - (f64::from(n) * 0.000000010101010101),
        })
        .collect();
    let mut accepted = 0;
    for _ in 0..48 {
        let l = VerifiedVideoVectorLayout::new(
            uid(),
            content(points.clone()),
            renderer(),
            10,
            10,
            rgba_png(10, 10),
        )
        .unwrap();
        let reference = l.reference().clone();
        let before = proof(&file.conn());
        let state = store.snapshot();
        let result = store.append_manual_video_drawings(
            &current(&store, &f),
            &f.proof,
            vec![vector(&l, 10., 10.)],
            mixed(vec![], vec![l]),
        );
        match result {
            Ok(_) => accepted += 1,
            Err(error) => {
                assert!(
                    error.to_string().contains("合计预算"),
                    "unexpected: {error}"
                );
                assert_eq!(proof(&file.conn()), before);
                assert_eq!(store.snapshot(), state);
                assert!(store.video_vector_layout(&reference).is_err());
                break;
            }
        }
    }
    assert!((1..48).contains(&accepted));
    assert_eq!(rows(&file.conn(), "video_vector_layouts").len(), accepted);
}
#[test]
fn db8_to9_preserves_raw_payload_every_old_schema_and_raw_sql_row() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let l = text_layout();
    append(
        &mut store,
        &f,
        vec![text(range(SECOND, 8 * SECOND), &l)],
        vec![l],
    );
    let expected = store.snapshot();
    drop(store);
    let db = file.conn();
    degrade_eight(&db);
    let raw = format!("\n {} \n", serde_json::to_string_pretty(&expected).unwrap());
    db.execute("UPDATE app_state SET payload=?1 WHERE id=1", [&raw])
        .unwrap();
    let revision = file.saved().1;
    let mut expected_app_rows = rows(&db, "app_state");
    assert_eq!(expected_app_rows.len(), 1);
    assert_eq!(expected_app_rows[0][3], Value::Integer(revision));
    expected_app_rows[0][3] = Value::Integer(revision + 1);
    let schema: Vec<(String, String, Option<String>)> = db
        .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let tables: Vec<String> = db
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name!='app_state' ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let old_rows: Vec<_> = tables.iter().map(|t| rows(&db, t)).collect();
    let migrated = Store::open(&file.0).unwrap();
    assert_eq!(migrated.snapshot(), expected);
    assert_eq!(file.saved(), (raw, revision + 1));
    assert_eq!(rows(&db, "app_state"), expected_app_rows);
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        9
    );
    for (kind, name, sql) in &schema {
        let actual: (String, Option<String>) = db
            .query_row(
                "SELECT type,sql FROM sqlite_schema WHERE name=?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((&actual.0, &actual.1), (kind, sql));
    }
    assert_eq!(
        tables.iter().map(|t| rows(&db, t)).collect::<Vec<_>>(),
        old_rows
    );
    let new_schema:Vec<(String,String,String,Option<String>)>=db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT IN (SELECT value FROM json_each(?1)) ORDER BY name").unwrap().query_map([serde_json::to_string(&schema.iter().map(|(_,n,_)|n).collect::<Vec<_>>()).unwrap()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(new_schema.len(), 4);
    let expected_table = "CREATE TABLE video_vector_layouts(layout_id TEXT PRIMARY KEY NOT NULL,layout_sha256 TEXT NOT NULL CHECK(length(layout_sha256)=64),descriptor TEXT NOT NULL CHECK(length(CAST(descriptor AS BLOB))<=262144),raster_png BLOB NOT NULL CHECK(typeof(raster_png)='blob' AND length(raster_png) BETWEEN 45 AND 8388608))";
    let expected_delete = "CREATE TRIGGER video_vector_layouts_no_delete BEFORE DELETE ON video_vector_layouts BEGIN SELECT RAISE(ABORT,'immutable video vector layout'); END";
    let expected_update = "CREATE TRIGGER video_vector_layouts_no_update BEFORE UPDATE ON video_vector_layouts BEGIN SELECT RAISE(ABORT,'immutable video vector layout'); END";
    assert_eq!(
        new_schema[1],
        (
            "table".into(),
            "video_vector_layouts".into(),
            "video_vector_layouts".into(),
            Some(expected_table.into())
        )
    );
    assert_eq!(
        new_schema[2],
        (
            "trigger".into(),
            "video_vector_layouts_no_delete".into(),
            "video_vector_layouts".into(),
            Some(expected_delete.into())
        )
    );
    assert_eq!(
        new_schema[3],
        (
            "trigger".into(),
            "video_vector_layouts_no_update".into(),
            "video_vector_layouts".into(),
            Some(expected_update.into())
        )
    );
    assert_eq!(
        new_schema.iter().map(|v| v.1.as_str()).collect::<Vec<_>>(),
        vec![
            "sqlite_autoindex_video_vector_layouts_1",
            "video_vector_layouts",
            "video_vector_layouts_no_delete",
            "video_vector_layouts_no_update"
        ]
    );
    assert_eq!(
        new_schema[0],
        (
            "index".into(),
            "sqlite_autoindex_video_vector_layouts_1".into(),
            "video_vector_layouts".into(),
            None
        )
    );
    assert!(rows(&db, "video_vector_layouts").is_empty());
    drop(migrated);
    let reopened = Store::open(&file.0).unwrap();
    assert_eq!(reopened.snapshot(), expected);
    assert_eq!(file.saved().1, revision + 1);
}
#[test]
fn db8_migration_rollback_keeps_exact_old_bytes_and_no_partial_new_schema() {
    let file = Db::new();
    let store = Store::open(&file.0).unwrap();
    drop(store);
    let db = file.conn();
    degrade_eight(&db);
    let before = file.saved();
    db.execute_batch("CREATE TRIGGER reject_upgrade BEFORE UPDATE ON app_state BEGIN SELECT RAISE(ABORT,'fixture');END").unwrap();
    assert!(Store::open(&file.0).is_err());
    assert_eq!(file.saved(), before);
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        8
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name LIKE '%video_vector_layouts%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn missing_layout_or_immutable_constraint_fails_closed_without_clearing_documents() {
    for corrupt in 0..2 {
        let file = Db::new();
        let mut store = Store::open(&file.0).unwrap();
        let f = setup(&mut store, 12 * SECOND);
        add_vector(&mut store, &f, vector_layout(), 10., 10.);
        let before = file.saved();
        drop(store);
        let db = file.conn();
        db.execute_batch("DROP TRIGGER video_vector_layouts_no_delete")
            .unwrap();
        if corrupt == 1 {
            db.execute("DELETE FROM video_vector_layouts", []).unwrap();
        }
        assert!(Store::open(&file.0).is_err());
        assert_eq!(file.saved(), before);
        assert!(ExportView::open(&file.0).is_err());
        assert_eq!(file.saved(), before);
    }
}

#[test]
fn user_update_cannot_turn_an_ai_created_object_into_a_manual_vector() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let run = begin(&mut store, &f);
    let call = leases(&mut store, &run, 1).remove(0);
    let sent = &run.input.manifest().targets[0];
    let interval = range(
        sent.frames[0].source_playback_ticks,
        sent.sent_window.end_ticks,
    );
    store
        .apply_video_annotations_with_receipt(
            &call,
            &run.authority,
            batch(&run, vec![rect(interval)]),
            vec![],
        )
        .unwrap();
    store
        .finish_run(&run.run.scene_id, &run.run.run_id, "synthetic complete")
        .unwrap();
    let before = proof(&file.conn());
    let state = store.snapshot();
    let l = vector_layout();
    assert!(update(&mut store, &f, vector(&l, 10., 10.), mixed(vec![], vec![l])).is_err());
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(store.snapshot(), state);
    let doc = view(&store, &f).annotations.unwrap();
    assert!(doc.objects[0].origin.is_some());
    assert!(matches!(
        &doc.objects[0].primitive,
        VideoAnnotationPrimitive::Rect { .. }
    ));
}

#[test]
fn source_proof_mismatch_and_sub100ms_video_never_create_layout_or_document() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let mut altered = f.proof.source().clone();
    altered.asset.path = "different-source.mp4".into();
    let wrong = VerifiedVideoAnnotationSource::new(altered, hash("other source")).unwrap();
    let l = vector_layout();
    let before = proof(&file.conn());
    assert!(store
        .append_manual_video_drawings(
            &current(&store, &f),
            &wrong,
            vec![vector(&l, 10., 10.)],
            mixed(vec![], vec![l])
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
    let short = setup(&mut store, MIN_VIDEO_INTERVAL_TICKS - 1);
    let before = proof(&file.conn());
    let l = vector_layout();
    assert!(store
        .append_manual_video_drawings(
            &current(&store, &short),
            &short.proof,
            vec![vector(&l, 10., 10.)],
            mixed(vec![], vec![l])
        )
        .is_err());
    assert_eq!(proof(&file.conn()), before);
    assert!(view(&store, &short).annotations.is_none());
}

#[test]
fn db8_running_recovery_is_explicit_and_never_claims_raw_preservation() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let run = begin(&mut store, &f);
    drop(store);
    let db = file.conn();
    degrade_eight(&db);
    let before = file.saved();
    let recovered = Store::open(&file.0).unwrap();
    let scene = recovered
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == run.run.scene_id)
        .unwrap();
    let stopped = scene.run.unwrap();
    assert_eq!(stopped.id, run.run.run_id);
    assert_eq!(stopped.status, RunStatus::Failed);
    assert_eq!(stopped.error.as_deref(), Some("上次运行已中断"));
    assert_ne!(file.saved().0, before.0);
    assert_eq!(file.saved().1, before.1 + 1);
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        9
    );
    assert!(rows(&db, "video_vector_layouts").is_empty());
    assert_eq!(rows(&db, "video_run_inputs").len(), 1);
}

#[test]
fn a_single_history_step_over_budget_rejects_the_whole_manual_add() {
    let file = Db::new();
    let mut store = Store::open(&file.0).unwrap();
    let f = setup(&mut store, 12 * SECOND);
    let run = begin(&mut store, &f);
    let call = leases(&mut store, &run, 1).remove(0);
    let sent = &run.input.manifest().targets[0];
    let interval = range(
        sent.frames[0].source_playback_ticks,
        sent.sent_window.end_ticks,
    );
    let mut precise = rect(interval);
    if let VideoAnnotationPrimitive::Rect { bounds, style } = &mut precise.primitive {
        *bounds = VideoPixelRect {
            x: 30.123456789012345,
            y: 20.123456789012345,
            width: 80.123456789012345,
            height: 50.123456789012345,
        };
        style.stroke_width = 3.123456789012345;
        style.opacity = 0.8123456789012345;
    }
    store
        .apply_video_annotations_with_receipt(
            &call,
            &run.authority,
            batch(&run, vec![precise; 47]),
            vec![],
        )
        .unwrap();
    store
        .finish_run(&run.run.scene_id, &run.run.run_id, "synthetic complete")
        .unwrap();
    let before = proof(&file.conn());
    let state = store.snapshot();
    let l = vector_layout();
    let reference = l.reference().clone();
    // Measure the actual persisted identities and full snapshots; object count
    // alone does not prove this single step crosses the serialized byte cap.
    let document = view(&store, &f).annotations.unwrap();
    let mut after = document.objects.clone();
    after.push(TimedVideoAnnotation {
        id: uid(),
        interval: range(0, f.proof.source().duration_ticks),
        primitive: vector(&l, 10., 10.),
        origin: None,
    });
    let step = VideoAnnotationStep {
        id: uid(),
        before: document.objects.clone(),
        after,
    };
    #[derive(serde::Serialize)]
    struct HistoryBudget<'a> {
        revision: u64,
        undo: [&'a VideoAnnotationStep; 1],
        redo: [VideoAnnotationStep; 0],
    }
    let before_bytes = serde_json::to_vec(&document.objects).unwrap().len();
    let single_step_bytes = serde_json::to_vec(&HistoryBudget {
        revision: document.revision.checked_add(1).unwrap(),
        undo: [&step],
        redo: [],
    })
    .unwrap()
    .len();
    eprintln!("history-budget fixture: before_objects_bytes={before_bytes} single_step_bytes={single_step_bytes} limit={MAX_VIDEO_HISTORY_BYTES}");
    assert!(single_step_bytes > MAX_VIDEO_HISTORY_BYTES);
    let db = file.conn();
    let schema_before: Vec<(String, String, String, Option<String>)> = db
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let tables: Vec<_> = schema_before
        .iter()
        .filter(|(kind, _, _, _)| kind == "table")
        .map(|(_, name, _, _)| name)
        .collect();
    let all_rows_before: Vec<_> = tables.iter().map(|t| rows(&db, t)).collect();
    let error = store
        .append_manual_video_drawings(
            &current(&store, &f),
            &f.proof,
            vec![vector(&l, 10., 10.)],
            mixed(vec![], vec![l]),
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("单次历史"),
        "unexpected: {error}"
    );
    assert_eq!(proof(&file.conn()), before);
    assert_eq!(store.snapshot(), state);
    assert!(store.video_vector_layout(&reference).is_err());
    let schema_after: Vec<(String, String, String, Option<String>)> = db
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(schema_after, schema_before);
    assert_eq!(
        tables.iter().map(|t| rows(&db, t)).collect::<Vec<_>>(),
        all_rows_before
    );
}
