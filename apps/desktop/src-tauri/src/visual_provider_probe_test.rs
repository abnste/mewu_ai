// SPDX-License-Identifier: MPL-2.0
//! Explicit isolated transport/core/compositor checks, never startup behavior.
//! This child module reuses the product parser and advertised definition.
//! Neither test constructs AppHandle or reads pixels
//! from the desktop. The live test is ignored AND needs the explicit environment
//! gate below. It reserves a persistent once-only marker before its sole send.
//! This probes transport/parser/core/compositor, not the UI sender/revocation gate.

use super::{parse_batch, RunScope};
use crate::{
    agent_tools, ai, assets, connection_host, credentials, drawing_render, mcp_host,
    plugins::PluginStore, provider_transport, visual_annotation_input::VisualCoordinateProfile,
};
use image::{DynamicImage, Rgba, RgbaImage};
use mewu_core::*;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

type Result<T> = std::result::Result<T, String>;
const SELECTED_ID: &str = "d00f61fd-6cb6-491c-b32e-340ff833152b";
const SELECTED_MODEL: &str = "doubao-seed-2-1-pro-260628";
const OUTPUT_BASE: &str = "D:/tmp/mewu-remake-20261008/visual-provider-probe";
const ALLOW_VALUE: &str = "one-request-doubao-260628";
const ONCE_MARKER: &str = "doubao-260628-real-request-reserved.once";
const PROMPT: &str = "请在图片中完成两件事：计算题目，把答案写在蓝色空框内部；用红色矩形沿橙色框的外缘圈出它。通过 visual_annotate 一次提交全部图元。保留图片原有内容。不在文字回复中代替图上作答。";

fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(error)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(error)?;
    file.write_all(&bytes).map_err(error)?;
    file.sync_all().map_err(error)
}
fn save_png(path: &Path, image: &DynamicImage) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(error)?;
    image
        .write_to(&mut file, image::ImageFormat::Png)
        .map_err(error)?;
    file.sync_all().map_err(error)
}
fn new_output(kind: &str) -> Result<PathBuf> {
    let base = PathBuf::from(OUTPUT_BASE);
    fs::create_dir_all(&base).map_err(error)?;
    let base = fs::canonicalize(base).map_err(error)?;
    let root = base.join(format!("{kind}-{}", uid()));
    fs::create_dir(&root).map_err(error)?;
    Ok(root)
}
fn shape(kind: DrawingKind, color: &str, points: &[(f64, f64)]) -> Drawing {
    Drawing {
        id: uid(),
        kind,
        color: color.into(),
        stroke_width: 5.,
        points: points.iter().map(|&(x, y)| DrawingPoint { x, y }).collect(),
        text: None,
        font_size: None,
        origin: None,
        rich: None,
    }
}
fn synthetic_profile() -> ConnectionProfile {
    ConnectionProfile {
        id: uid(),
        name: "离线合成验证".into(),
        provider_id: "compatible".into(),
        base_url: "http://127.0.0.1:9/v1".into(),
        model: SELECTED_MODEL.into(),
        has_key: false,
        credential_id: None,
        revision: 1,
        advanced: ConnectionAdvanced::default(),
    }
}

struct Fixture {
    root: PathBuf,
    assets: PathBuf,
    db_path: PathBuf,
    store: Store,
    plugins: PluginStore,
    context: RunContext,
    scope: Arc<RunScope>,
    region_id: String,
    background: Asset,
    manual: Drawing,
    before: RgbaImage,
    source_sha256: String,
    previous_undo: usize,
    body: Value,
    request_sha256: String,
}
impl Fixture {
    fn region(&self) -> Result<Region> {
        region_from(&self.store, &self.context.scene_id, &self.region_id)
    }
    fn composite(&self) -> Result<DynamicImage> {
        assets::crop_from_parts(&self.background, &self.region()?, &self.assets)
    }
}
fn region_from(store: &Store, scene: &str, region: &str) -> Result<Region> {
    store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == scene)
        .and_then(|s| s.regions.into_iter().find(|r| r.id == region))
        .ok_or_else(|| "合成选区不存在".into())
}

fn fixture(root: PathBuf, selected: &ConnectionProfile) -> Result<Fixture> {
    // Keep the existing once-only live probe on its original encoded contract.
    fixture_with_profile(
        root,
        selected,
        VisualCoordinateProfile::EncodedPixelsBestEffortV1,
    )
}
fn fixture_with_profile(
    root: PathBuf,
    selected: &ConnectionProfile,
    coordinate_profile: VisualCoordinateProfile,
) -> Result<Fixture> {
    fixture_with_thinking(root, selected, coordinate_profile, false)
}
// Explicit test-only request mode. This never changes a saved user profile or
// silently applies provider-specific options to normal product requests.
fn fixture_with_thinking(
    root: PathBuf,
    selected: &ConnectionProfile,
    coordinate_profile: VisualCoordinateProfile,
    disable_thinking: bool,
) -> Result<Fixture> {
    let assets_root = root.join("assets");
    fs::create_dir(&assets_root).map_err(error)?;
    let db_path = root.join("probe.sqlite");
    let mut store = Store::open(&db_path).map_err(error)?;
    let plugins = PluginStore::open(&root)?;
    let grant = VisualAnnotationGrant {
        plugin_id: "mewu.core.annotations".into(),
        plugin_revision: 1,
        contribution_id: "annotate".into(),
    };
    plugins.visual_annotations(&grant)?;
    let mut profile = selected.clone();
    profile.name = "隔离原位作答验证".into();
    profile.credential_id = None;
    profile.has_key = false;
    profile.revision = 1;
    store
        .save_connection_profile(profile.clone(), None)
        .map_err(error)?;
    let snapshot = store.snapshot();
    let scene_id = snapshot.active_scene_id.clone();
    let mut agent = snapshot
        .agents
        .into_iter()
        .next()
        .ok_or("合成 Agent 不存在")?;
    agent.instructions = "按用户要求完成图上作答。".into();
    agent.memory.clear();
    agent.memory_enabled = false;
    agent.memory_provider = MemoryProviderSelection::default();
    store
        .apply(SceneCommand::SaveAgent { agent })
        .map_err(error)?;
    store
        .apply(SceneCommand::SetSceneConnection {
            scene_id: scene_id.clone(),
            connection_id: Some(profile.id.clone()),
        })
        .map_err(error)?;

    let mut label = shape(DrawingKind::Text, "#16191F", &[(260., 240.)]);
    label.text = Some("2 + 2 =".into());
    label.font_size = Some(64.);
    let mut orange = shape(DrawingKind::Rect, "#ED7428", &[(945., 525.), (1135., 665.)]);
    orange.stroke_width = 6.;
    let painted = Region {
        id: uid(),
        x: 0.,
        y: 0.,
        width: 1500.,
        height: 1050.,
        drawings: vec![
            label,
            shape(DrawingKind::Rect, "#2463EB", &[(610., 220.), (840., 335.)]),
            orange,
        ],
        ..Default::default()
    };
    let source = drawing_render::crop_region(
        &DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            1500,
            1050,
            Rgba([255, 255, 255, 255]),
        )),
        &painted,
    )?;
    let source_path = assets_root.join(format!("{}.png", uid()));
    save_png(&source_path, &source)?;
    save_png(&root.join("source.png"), &source)?;
    let source_sha256 = sha(&fs::read(&source_path).map_err(error)?);
    let background = Asset {
        id: uid(),
        kind: AssetKind::Image,
        name: "synthetic-worksheet.png".into(),
        path: source_path.to_str().ok_or("合成路径无效")?.into(),
        width: Some(1500),
        height: Some(1050),
        origin_x: Some(-1500),
        origin_y: Some(0),
        scale_factor: Some(1.75),
    };
    store
        .set_background(&scene_id, background.clone())
        .map_err(error)?;
    let region_id = uid();
    store
        .apply(SceneCommand::AddRegion {
            scene_id: scene_id.clone(),
            region: Region {
                id: region_id.clone(),
                x: 113.,
                y: 79.,
                width: 1201.,
                height: 801.,
                ..Default::default()
            },
        })
        .map_err(error)?;
    let mut manual = shape(DrawingKind::Line, "#267441", &[(170., 760.), (410., 760.)]);
    manual.stroke_width = 4.;
    store
        .apply(SceneCommand::AddDrawing {
            scene_id: scene_id.clone(),
            region_id: region_id.clone(),
            background_id: background.id.clone(),
            expected_revision: 0,
            drawing: manual.clone(),
        })
        .map_err(error)?;
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene_id.clone(),
            draft: PROMPT.into(),
        })
        .map_err(error)?;
    store
        .apply(SceneCommand::SetRefs {
            scene_id: scene_id.clone(),
            refs: vec![Reference {
                kind: ReferenceKind::Region,
                id: region_id.clone(),
            }],
        })
        .map_err(error)?;
    let scene = store
        .snapshot()
        .scenes
        .into_iter()
        .find(|s| s.id == scene_id)
        .ok_or("合成会话不存在")?;
    require(
        scene.refs.len() == 1
            && scene.refs[0].kind == ReferenceKind::Region
            && scene.refs[0].id == region_id,
        "合成引用不唯一",
    )?;
    require(
        scene.messages.is_empty() && scene.items.is_empty(),
        "合成会话包含无关资料",
    )?;
    let before =
        assets::crop_from_parts(&background, &scene.regions[0], &assets_root)?.into_rgba8();
    save_png(
        &root.join("before.png"),
        &DynamicImage::ImageRgba8(before.clone()),
    )?;
    let previous_undo = scene.regions[0].drawing_history.undo.len();
    let prepared = ai::prepare_scene_visual_attachments(&scene, &assets_root, coordinate_profile)?;
    let input = prepared.visual_input().cloned().ok_or("缺少实际附件映射")?;
    require(input.manifest().targets.len() == 1, "合成目标不唯一")?;
    let target = &input.manifest().targets[0];
    require(
        target.crop.x == 113
            && target.crop.y == 79
            && target.crop.width == 1201
            && target.crop.height == 801,
        "实际裁切不匹配",
    )?;
    require(
        target.encoded.width == 1024 && target.encoded.height > 0 && target.encoded.height <= 1024,
        "未覆盖下采样",
    )?;
    let mut context = store
        .begin_run_with_memory(&scene_id, None)
        .map_err(error)?;
    require(
        !context.agent.memory_enabled
            && context.memory_authority.is_none()
            && context.mcp_servers.is_empty(),
        "合成运行含无关授权",
    )?;
    let attachments = ai::persist_attachments(prepared, &assets_root)?;
    require(
        attachments.len() == 1 && attachments[0].id == target.attachment_id,
        "附件 ID 与预分配不一致",
    )?;
    let jpeg = fs::read(&attachments[0].path).map_err(error)?;
    require(sha(&jpeg) == target.attachment_sha256, "JPEG 哈希不匹配")?;
    let decoded = image::load_from_memory(&jpeg).map_err(error)?;
    require(
        decoded.width() == target.encoded.width && decoded.height() == target.encoded.height,
        "JPEG 尺寸不匹配",
    )?;
    let attached = store
        .attach_run_visual_assets(
            &scene_id,
            &context.run_id,
            attachments,
            grant.clone(),
            input.clone(),
        )
        .map_err(error)?;
    context.messages = attached
        .scenes
        .into_iter()
        .find(|s| s.id == scene_id)
        .ok_or("绑定后会话不存在")?
        .messages;
    let scope = Arc::new(RunScope::new(scene_id, grant, input)?);
    let tools = mcp_host::RunTools::with_visual(&context, Some(scope.clone()))?;
    let definitions = tools.definitions();
    require(
        definitions.len() == 1 && definitions[0] == scope.definition(),
        "请求并非仅包含产品视觉工具",
    )?;
    require(
        tools.journal_binding(VISUAL_ANNOTATION_TOOL) == Some(scope.binding()),
        "工具来源不一致",
    )?;
    let mut body = ai::request_body(&context, &assets_root)?;
    ai::bind_visual_request_targets(&mut body, &scope.input)?;
    connection_host::apply_parameters(&mut body, &context.connection_profile)?;
    agent_tools::augment_request(&mut body, &context)?;
    tools.augment_request(&mut body)?;
    body["stream"] = json!(true);
    body["tools"] = json!(definitions);
    body = provider_transport::request_body(context.connection_profile.advanced.protocol, body)?;
    if disable_thinking {
        body["thinking"] = json!({"type":"disabled"});
    }
    // Official Doubao Chat image-understanding example documents max_tokens.
    require(
        context.connection_profile.advanced.protocol == ConnectionProtocol::ChatCompletions,
        "此探针只核验已确认的 Chat 路由",
    )?;
    body["max_tokens"] = json!(2048);
    let request_sha256 = sha(&serde_json::to_vec(&body).map_err(error)?);
    write_json(&root.join("input-manifest.json"), scope.input.manifest())?;
    write_json(&root.join("tool-definition.json"), &scope.definition())?;
    write_json(
        &root.join("preflight.json"),
        &json!({
            "model":SELECTED_MODEL,"protocol":"chat_completions","coordinateProfile":coordinate_profile.id(),
            "testOnlyThinkingOverride":if disable_thinking {json!("disabled")} else {Value::Null},
            "requestSha256":request_sha256,"outputLimit":2048,"requestLimit":1,
            "manifestSha256":scope.input.sha256(),"attachmentSha256":target_hash(&scope),
            "inputPrivateDesktop":false,"credentialStoredInProbe":false,
            "claim":"Product prepare/catalog/parser/core/compositor; does not replace AppHandle grant lifecycle tests"
        }),
    )?;
    Ok(Fixture {
        root,
        assets: assets_root,
        db_path,
        store,
        plugins,
        context,
        scope,
        region_id,
        background,
        manual,
        before,
        source_sha256,
        previous_undo,
        body,
        request_sha256,
    })
}
fn target_hash(scope: &RunScope) -> &str {
    &scope.input.manifest().targets[0].attachment_sha256
}

fn model_dispatch(f: &mut Fixture) -> Result<DispatchLease> {
    f.store
        .begin_model_request(
            &f.context.scene_id,
            &f.context.run_id,
            ModelDispatchInput {
                round: 0,
                projected_request_sha256: f.request_sha256.clone(),
            },
        )
        .map_err(error)
}

// Never accepts a synthesized replacement for the real test's returned calls.
fn commit_turn(f: &mut Fixture, model: &DispatchLease, turn: ai::ModelTurn) -> Result<Value> {
    let calls: Vec<_> = turn
        .tool_calls
        .iter()
        .map(|c| ProposedTool {
            call_id: c.id.clone(),
            binding: if c.name == VISUAL_ANNOTATION_TOOL {
                f.scope.binding()
            } else {
                ToolBinding::Rejected {
                    advertised_name: c.name.clone(),
                }
            },
            arguments_wire_sha256: sha(c.arguments.as_bytes()),
        })
        .collect();
    let committed = f
        .store
        .record_model_turn(
            model,
            CompletePublicTurn {
                text: turn.content.clone(),
                calls,
            },
        )
        .map_err(error)?;
    if turn.tool_calls.len() != 1 || turn.tool_calls[0].name != VISUAL_ANNOTATION_TOOL {
        for prepared in &committed.tools {
            f.store
                .record_tool_not_sent(&prepared.lease, NotSentReason::BudgetExhausted)
                .map_err(error)?;
        }
        return Err("单次回复未返回恰好一个已授权 visual_annotate 调用；未自动补发".into());
    }
    require(
        committed.tools.len() == 1 && committed.tools[0].call_id == turn.tool_calls[0].id,
        "工具回执不匹配",
    )?;
    let call = &turn.tool_calls[0];
    // Only explicit public output and synthetic arguments; never ModelTurn itself.
    write_json(
        &f.root.join("public-response.json"),
        &json!({"publicReply":turn.content,"toolName":call.name,"arguments":call.arguments}),
    )?;
    let batch = match parse_batch(&call.arguments, &f.scope.input) {
        Ok(value) => value,
        Err(e) => {
            f.store
                .record_tool_not_sent(&committed.tools[0].lease, NotSentReason::InvalidArguments)
                .map_err(error)?;
            return Err(e);
        }
    };
    f.plugins.visual_annotations(&f.scope.origin.grant)?;
    require(
        f.store
            .visual_input_for_run(&f.context.scene_id, &f.context.run_id, &f.scope.authority)
            .map_err(error)?
            == f.scope.input,
        "持久输入已变更",
    )?;
    let effect = f
        .store
        .apply_visual_annotations_with_receipt(&committed.tools[0].lease, &f.scope.authority, batch)
        .map_err(error)?;
    let region = f.region()?;
    require(
        region.drawings.iter().any(|d| d == &f.manual),
        "原手绘对象被改变",
    )?;
    require(
        region.drawing_history.undo.len() == f.previous_undo + 1,
        "批注没有形成一个完整撤销步",
    )?;
    let created: Vec<_> = region
        .drawings
        .iter()
        .filter(|d| d.origin.is_some())
        .cloned()
        .collect();
    require(
        !created.is_empty()
            && region.drawing_history.undo.last().is_some_and(
                |step| matches!(step,DrawingStep::Batch(batch) if batch.batch.len()==created.len()),
            ),
        "批次历史与实际对象不一致",
    )?;
    for drawing in &created {
        let origin = drawing.origin.as_ref().unwrap();
        require(
            origin.run_id == f.context.run_id && origin.manifest_sha256 == f.scope.input.sha256(),
            "批注来源不匹配",
        )?;
    }
    write_json(&f.root.join("commit-receipt.json"), &effect.receipt)?;
    write_json(&f.root.join("drawings.json"), &created)?;
    Ok(json!({"receipt":effect.receipt,"drawings":created,"localAtomicBatch":true}))
}

fn changed_pixels(a: &RgbaImage, b: &RgbaImage, source_box: [u32; 4]) -> usize {
    // Fixture crop is known but use its exact integer source offset.
    let [x0, y0, x1, y1] = source_box;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x - 113, y - 79)))
        .filter(|&(x, y)| a.get_pixel(x, y) != b.get_pixel(x, y))
        .count()
}
fn glyph_bounds(region: &Region, text: &Drawing) -> Result<Option<[u32; 4]>> {
    let mut layer = region.clone();
    layer.drawings = vec![text.clone()];
    layer.translation = None;
    let pixels = drawing_render::crop_region(
        &DynamicImage::ImageRgba8(RgbaImage::new(1500, 1050)),
        &layer,
    )?
    .into_rgba8();
    let mut bounds: Option<[u32; 4]> = None;
    for (x, y, pixel) in pixels.enumerate_pixels() {
        if pixel.0[3] == 0 {
            continue;
        }
        let (x, y) = (x + 113, y + 79);
        match &mut bounds {
            None => bounds = Some([x, y, x + 1, y + 1]),
            Some(b) => {
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x + 1);
                b[3] = b[3].max(y + 1);
            }
        }
    }
    Ok(bounds)
}
fn is_red(color: &str) -> bool {
    if color.len() != 7 {
        return false;
    }
    let component =
        |range: std::ops::Range<usize>| u8::from_str_radix(&color[range], 16).unwrap_or(0);
    component(1..3) >= 180 && component(3..5) <= 100 && component(5..7) <= 100
}
fn quality(f: &Fixture, after: &RgbaImage) -> Result<Value> {
    let region = f.region()?;
    let ai: Vec<_> = region
        .drawings
        .iter()
        .filter(|d| d.origin.is_some())
        .collect();
    let texts: Vec<_> = ai
        .iter()
        .copied()
        .filter(|d| {
            d.kind == DrawingKind::Text && d.text.as_deref().is_some_and(|s| s.trim() == "4")
        })
        .collect();
    let rectangles: Vec<_> = ai
        .iter()
        .copied()
        .filter(|d| d.kind == DrawingKind::Rect && is_red(&d.color))
        .collect();
    let glyph = if texts.len() == 1 {
        glyph_bounds(&region, texts[0])?
    } else {
        None
    };
    let text_fits = glyph.is_some_and(|b| b[0] >= 604 && b[1] >= 214 && b[2] <= 846 && b[3] <= 341);
    let edge_errors = if rectangles.len() == 1 {
        let p = &rectangles[0].points;
        Some([
            (p[0].x.min(p[1].x) - 945.).abs(),
            (p[0].y.min(p[1].y) - 525.).abs(),
            (p[0].x.max(p[1].x) - 1135.).abs(),
            (p[0].y.max(p[1].y) - 665.).abs(),
        ])
    } else {
        None
    };
    let rect_fits = edge_errors.is_some_and(|e| e.into_iter().all(|n| n <= 16.));
    let answer_pixels = changed_pixels(&f.before, after, [610, 220, 840, 335]);
    let box_pixels = changed_pixels(&f.before, after, [925, 505, 1155, 685]);
    let green_unchanged = changed_pixels(&f.before, after, [160, 750, 420, 770]) == 0;
    let passed = ai.len() == 2
        && text_fits
        && rect_fits
        && answer_pixels > 0
        && box_pixels > 0
        && green_unchanged;
    Ok(
        json!({"passed":passed,"glyphBounds":glyph,"rectangleEdgeErrors":edge_errors,
        "answerChangedPixels":answer_pixels,"boxChangedPixels":box_pixels,"manualLinePixelsUnchanged":green_unchanged,
        "aiObjectCount":ai.len(),"expectedObjectCount":2,"rectangleToleranceSourcePixels":16,"glyphToleranceSourcePixels":6}),
    )
}

fn finish_local_checks(mut f: Fixture, mut report: Value) -> Result<(PathBuf, Value)> {
    let after = f.composite()?.into_rgba8();
    save_png(
        &f.root.join("after.png"),
        &DynamicImage::ImageRgba8(after.clone()),
    )?;
    let quality = quality(&f, &after)?;
    require(
        sha(&fs::read(&f.background.path).map_err(error)?) == f.source_sha256,
        "原始 PNG 被改写",
    )?;
    f.store.cancel_run(&f.context.scene_id).map_err(error)?;
    let r = f.region()?;
    f.store
        .apply(SceneCommand::UndoDrawing {
            scene_id: f.context.scene_id.clone(),
            region_id: f.region_id.clone(),
            background_id: f.background.id.clone(),
            expected_revision: r.drawing_revision,
        })
        .map_err(error)?;
    let undone = f.composite()?.into_rgba8();
    save_png(
        &f.root.join("after-undo.png"),
        &DynamicImage::ImageRgba8(undone.clone()),
    )?;
    require(undone == f.before, "整批撤销未恢复原像素")?;
    require(
        f.region()?.drawings == vec![f.manual.clone()],
        "整批撤销没有保留原手绘",
    )?;
    let r = f.region()?;
    f.store
        .apply(SceneCommand::RedoDrawing {
            scene_id: f.context.scene_id.clone(),
            region_id: f.region_id.clone(),
            background_id: f.background.id.clone(),
            expected_revision: r.drawing_revision,
        })
        .map_err(error)?;
    let redone = f.composite()?.into_rgba8();
    save_png(
        &f.root.join("after-redo.png"),
        &DynamicImage::ImageRgba8(redone.clone()),
    )?;
    require(redone == after, "整批重做未恢复提交像素")?;
    let root = f.root.clone();
    let expected_region = f.region()?;
    drop(f.store);
    let reopened = Store::open(&f.db_path).map_err(error)?;
    let restored = region_from(&reopened, &f.context.scene_id, &f.region_id)?;
    require(restored == expected_region, "重开数据库对象/历史/来源改变")?;
    let reopened_pixels =
        assets::crop_from_parts(&f.background, &restored, &f.assets)?.into_rgba8();
    require(reopened_pixels == after, "重开后合成像素改变")?;
    report["quality"] = quality;
    report["undoRgbaEqual"] = json!(true);
    report["redoRgbaEqual"] = json!(true);
    report["reopenValid"] = json!(true);
    report["sourceUnchanged"] = json!(true);
    report["syntheticRunStop"] =
        json!("canceled after single observed tool turn; no second model request");
    report["passed"] = report["quality"]["passed"].clone();
    Ok((root, report))
}

#[test]
fn synthetic_visual_provider_fixture_uses_real_prepare_batch_and_compositor() -> Result<()> {
    let root = new_output("offline")?;
    let mut f = fixture(root.clone(), &synthetic_profile())?;
    let target = &f.scope.input.manifest().targets[0];
    // This is ONLY the separate offline fixture test; never used in the live test.
    let encoded = |x: f64, y: f64| {
        json!({
            "x":((x-f64::from(target.crop.x))*f64::from(target.resized.width)/f64::from(target.crop.width)-f64::from(target.tile.x))*f64::from(target.encoded.width)/f64::from(target.tile.width),
            "y":((y-f64::from(target.crop.y))*f64::from(target.resized.height)/f64::from(target.crop.height)-f64::from(target.tile.y))*f64::from(target.encoded.height)/f64::from(target.tile.height)
        })
    };
    let arguments=json!({"groups":[{"target":target.handle,"objects":[
        {"kind":"text","color":"#D51B21","strokeWidth":1,"points":[encoded(680.,245.)],"text":"4","fontSize":40},
        {"kind":"rect","color":"#D51B21","strokeWidth":3,"points":[encoded(945.,525.),encoded(1135.,665.)]}
    ]}]}).to_string();
    let model = model_dispatch(&mut f)?;
    let report = commit_turn(
        &mut f,
        &model,
        ai::ModelTurn {
            content: String::new(),
            tool_calls: vec![ai::ModelToolCall {
                id: uid(),
                name: VISUAL_ANNOTATION_TOOL.into(),
                arguments,
            }],
            ..Default::default()
        },
    )?;
    let (root, mut report) = finish_local_checks(f, report)?;
    report["kind"] = json!("offline synthetic fixture, not provider evidence");
    report["requestCount"] = json!(0);
    write_json(&root.join("probe-report.json"), &report)?;
    require(report["passed"] == true, "离线合成验证失败")
}

fn normalized_fixture_arguments(f: &Fixture) -> Value {
    let target = &f.scope.input.manifest().targets[0];
    // Independently known synthetic source positions, not a repaired live reply.
    let normalized = |x: f64, y: f64| {
        json!({
            "x":((x-f64::from(target.crop.x))*f64::from(target.resized.width)/f64::from(target.crop.width)-f64::from(target.tile.x))*1000./f64::from(target.tile.width),
            "y":((y-f64::from(target.crop.y))*f64::from(target.resized.height)/f64::from(target.crop.height)-f64::from(target.tile.y))*1000./f64::from(target.tile.height)
        })
    };
    json!({"groups":[{"target":target.handle,"coordinateSpace":"normalized1000","styleUnit":"encodedPixels","objects":[
        {"kind":"text","color":"#D51B21","strokeWidth":1,"points":[normalized(680.,245.)],"text":"4","fontSize":40,"anchor":"topLeft"},
        {"kind":"rect","color":"#D51B21","strokeWidth":3,"points":[normalized(945.,525.),normalized(1135.,665.)]}
    ]}]})
}
fn offline_turn(arguments: Value) -> ai::ModelTurn {
    ai::ModelTurn {
        content: String::new(),
        tool_calls: vec![ai::ModelToolCall {
            id: uid(),
            name: VISUAL_ANNOTATION_TOOL.into(),
            arguments: arguments.to_string(),
        }],
        ..Default::default()
    }
}

#[test]
fn normalized_visual_fixture_atomic_commit_undo_redo_and_reopen_uses_real_pixels() -> Result<()> {
    let root = new_output("offline-v2")?;
    let mut f = fixture_with_profile(
        root.clone(),
        &synthetic_profile(),
        VisualCoordinateProfile::ExplicitNormalized1000V2,
    )?;
    let arguments = normalized_fixture_arguments(&f);
    let definition = f.scope.definition();
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(
            definition
                .pointer("/function/parameters")
                .ok_or("缺少实际工具schema")?,
        )
        .map_err(error)?;
    require(validator.is_valid(&arguments), "实际schema拒绝v2合成参数")?;
    let body_text = f.body.to_string();
    require(
        body_text.contains("normalized1000")
            && body_text.contains("encodedPixels")
            && body_text.contains("topLeft"),
        "实际请求没有完整坐标合同",
    )?;
    let model = model_dispatch(&mut f)?;
    let report = commit_turn(&mut f, &model, offline_turn(arguments))?;
    let (root, mut report) = finish_local_checks(f, report)?;
    report["kind"] = json!("normalized v2 offline fixture, not provider evidence");
    report["requestCount"] = json!(0);
    write_json(&root.join("probe-report.json"), &report)?;
    require(report["passed"] == true, "v2实际合成像素验证失败")
}

#[test]
fn normalized_invalid_later_object_leaves_whole_batch_unapplied_and_records_not_sent() -> Result<()>
{
    let root = new_output("offline-v2-invalid")?;
    let mut f = fixture_with_profile(
        root,
        &synthetic_profile(),
        VisualCoordinateProfile::ExplicitNormalized1000V2,
    )?;
    let before = f.region()?;
    let mut arguments = normalized_fixture_arguments(&f);
    arguments["groups"][0]["objects"][1]["points"][1]["y"] = json!(1000.1);
    let model = model_dispatch(&mut f)?;
    require(
        commit_turn(&mut f, &model, offline_turn(arguments)).is_err(),
        "无效后项被提交",
    )?;
    require(f.region()? == before, "前项文字发生部分提交")?;
    require(f.composite()?.into_rgba8() == f.before, "无效批次改变像素")?;
    let db =
        Connection::open_with_flags(&f.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(error)?;
    let raw: String = db
        .query_row(
            "SELECT metadata FROM agent_run_events WHERE run_id=?1 AND sequence=2",
            [&f.context.run_id],
            |r| r.get(0),
        )
        .map_err(error)?;
    let metadata: Value = serde_json::from_str(&raw).map_err(error)?;
    require(
        metadata["summary"]["phase"] == "not_sent" && metadata["notSent"] == "invalid_arguments",
        "无效批次回执错误",
    )
}

#[test]
fn normalized_effect_database_failure_preserves_prepared_receipt_and_all_geometry() -> Result<()> {
    let root = new_output("offline-v2-db-failure")?;
    let mut f = fixture_with_profile(
        root,
        &synthetic_profile(),
        VisualCoordinateProfile::ExplicitNormalized1000V2,
    )?;
    let arguments = normalized_fixture_arguments(&f);
    let model = model_dispatch(&mut f)?;
    let commit = f
        .store
        .record_model_turn(
            &model,
            CompletePublicTurn {
                text: String::new(),
                calls: vec![ProposedTool {
                    call_id: uid(),
                    binding: f.scope.binding(),
                    arguments_wire_sha256: sha(arguments.to_string().as_bytes()),
                }],
            },
        )
        .map_err(error)?;
    let batch = parse_batch(&arguments.to_string(), &f.scope.input)?;
    let before = f.region()?;
    let db = Connection::open(&f.db_path).map_err(error)?;
    let metadata = || -> Result<String> {
        db.query_row(
            "SELECT metadata FROM agent_run_events WHERE run_id=?1 AND sequence=2",
            [&f.context.run_id],
            |r| r.get(0),
        )
        .map_err(error)
    };
    let original_receipt = metadata()?;
    db.execute_batch("CREATE TRIGGER fail_visual_effect BEFORE UPDATE ON visual_run_inputs BEGIN SELECT RAISE(ABORT,'synthetic visual effect failure'); END;").map_err(error)?;
    require(
        f.store
            .apply_visual_annotations_with_receipt(
                &commit.tools[0].lease,
                &f.scope.authority,
                batch.clone(),
            )
            .is_err(),
        "故障注入没有阻止提交",
    )?;
    require(f.region()? == before, "失败事务改变几何或撤销历史")?;
    require(metadata()? == original_receipt, "失败事务改变工具回执")?;
    require(f.composite()?.into_rgba8() == f.before, "失败事务改变像素")?;
    db.execute_batch("DROP TRIGGER fail_visual_effect;")
        .map_err(error)?;
    f.store
        .apply_visual_annotations_with_receipt(&commit.tools[0].lease, &f.scope.authority, batch)
        .map_err(error)?;
    require(
        f.region()?.drawings.len() == before.drawings.len() + 2,
        "原prepared lease未保留可提交状态",
    )
}

fn selected_profile_read_only(root: &Path) -> Result<ConnectionProfile> {
    require(root.is_absolute(), "MEWU_PROBE_DATA_ROOT 必须为绝对路径")?;
    // No Store::open/recovery/migration touches the real database.
    let db = Connection::open_with_flags(
        root.join("spaces.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "无法只读打开用户连接配置".to_string())?;
    db.busy_timeout(Duration::from_secs(3))
        .map_err(|_| "无法设置只读配置超时".to_string())?;
    let version: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| "无法读取用户数据库版本".to_string())?;
    require(version == 6, "仅允许升级完成后的 DB6 配置")?;
    let payload:Option<String>=db.query_row("SELECT CASE WHEN length(CAST(payload AS BLOB))<=67108864 THEN payload ELSE NULL END FROM app_state WHERE id=1",[],|row|row.get(0))
        .map_err(|_|"无法只读获取连接配置".to_string())?;
    let payload = payload.ok_or("配置超过探针读取预算")?;
    let snapshot: Snapshot =
        serde_json::from_str(&payload).map_err(|_| "连接配置格式无效".to_string())?;
    drop(payload);
    drop(db);
    let mut selected = snapshot
        .connections
        .into_iter()
        .filter(|p| p.id == SELECTED_ID);
    let profile = selected.next().ok_or("未找到精确指定连接")?;
    require(selected.next().is_none(), "指定连接身份重复")?;
    require(profile.model == SELECTED_MODEL, "指定连接模型已变更")?;
    require(
        profile.has_key && profile.credential_id.is_some(),
        "指定连接尚未完成密钥导入",
    )?;
    require(
        profile.advanced.protocol == ConnectionProtocol::ChatCompletions,
        "指定连接不是本次核验的 Chat 路由",
    )?;
    validate_connection_profile(&profile).map_err(|_| "指定连接无效".to_string())?;
    Ok(profile)
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Real paid provider request; root only; explicit env gate and once-only marker required"]
async fn real_doubao_one_request_prepared_annotation_and_compositor() -> Result<()> {
    require(
        std::env::var("MEWU_PROBE_ALLOW_HTTP").ok().as_deref() == Some(ALLOW_VALUE),
        "未显式开启一次真实请求",
    )?;
    let base = PathBuf::from(OUTPUT_BASE);
    fs::create_dir_all(&base).map_err(error)?;
    let marker = base.join(ONCE_MARKER);
    require(
        !marker.try_exists().map_err(error)?,
        "真实请求已预留或执行；禁止自动重试",
    )?;
    let data = std::env::var_os("MEWU_PROBE_DATA_ROOT").ok_or("缺少 MEWU_PROBE_DATA_ROOT")?;
    let data_root =
        fs::canonicalize(PathBuf::from(data)).map_err(|_| "用户数据根目录不可用".to_string())?;
    let output_base = fs::canonicalize(&base).map_err(error)?;
    require(
        !data_root.starts_with(&output_base) && !output_base.starts_with(&data_root),
        "探针输出与用户数据根目录必须独立",
    )?;
    let profile = selected_profile_read_only(&data_root)?;
    let transport = connection_host::transport_for(&profile)?;
    let root = new_output("live")?;
    let mut f = fixture(root.clone(), &profile)?;
    // Preflight completed without secrets. Read this one profile's credential
    // once, retain Zeroizing<String>, never print/serialize it or env variables.
    let key = credentials::read_profile(&data_root, &profile)
        .map_err(|_| "无法读取指定连接的密钥".to_string())?;
    require(!key.is_empty(), "指定连接密钥为空")?;
    let sends = AtomicUsize::new(0);
    let mut lease = None;
    let body = f.body.clone();
    let received=tokio::time::timeout(Duration::from_secs(60),provider_transport::stream_turn_before_send(
        &transport,&key,body,|_|{},||{
            require(sends.compare_exchange(0,1,Ordering::SeqCst,Ordering::SeqCst).is_ok(),"禁止第二次发送")?;
            let mut reserved=OpenOptions::new().write(true).create_new(true).open(&marker)
                .map_err(|_|"一次请求标记已存在；禁止重试".to_string())?;
            reserved.write_all(b"One real provider attempt reserved. Do not auto-retry or delete this marker.\n").map_err(error)?;
            reserved.sync_all().map_err(error)?;
            f.plugins.visual_annotations(&f.scope.origin.grant)?;
            lease=Some(model_dispatch(&mut f)?);
            Ok(())
        }
    )).await;
    drop(key);
    let turn = match received {
        Ok(Ok(turn)) => turn,
        other => {
            if let Some(lease) = &lease {
                f.store
                    .record_request_unknown(lease, NoResponseReason::IncompleteResponse)
                    .map_err(error)?;
            }
            let reason = if other.is_err() {
                "timeout; no retry"
            } else {
                "transport or response error; no retry"
            };
            if f.store
                .run_is_active(&f.context.scene_id, &f.context.run_id)
            {
                f.store.cancel_run(&f.context.scene_id).map_err(error)?;
            }
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"provider","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err(reason.into());
        }
    };
    let lease = lease.ok_or("回复缺少实际派发记录")?;
    let observed = commit_turn(&mut f, &lease, turn);
    let report = match observed {
        Ok(report) => report,
        Err(reason) => {
            if f.store
                .run_is_active(&f.context.scene_id, &f.context.run_id)
            {
                f.store.cancel_run(&f.context.scene_id).map_err(error)?;
            }
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"local-tool","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err("真实回复未通过本地批注提交；已保存隔离证据，未重试".into());
        }
    };
    let (root, mut report) = match finish_local_checks(f, report) {
        Ok(value) => value,
        Err(reason) => {
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"compositor-lifecycle","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err("批注已提交，但本地合成或撤销检查失败；未重新请求模型".into());
        }
    };
    report["kind"] = json!("real provider single request");
    report["model"] = json!(SELECTED_MODEL);
    report["protocol"] = json!("chat_completions");
    report["sendGateCount"] = json!(sends.load(Ordering::SeqCst));
    report["requestLimit"] = json!(1);
    report["outputTokenLimit"] = json!(2048);
    report["deadlineSeconds"] = json!(60);
    report["retryCount"] = json!(0);
    write_json(&root.join("probe-report.json"), &report)?;
    require(
        report["passed"] == true,
        "真实工具链完成，但图上定位质量未达标；参阅隔离 PNG 与坐标误差，不自动重试",
    )
}

// Separate root-authorized post-fix check: its own immutable gate/marker.
// The original failed attempt and its protocol are never replayed or changed.
#[tokio::test(flavor = "current_thread")]
#[ignore = "Real paid provider request; root only; explicit env gate and once-only marker required"]
async fn real_doubao_normalized_v2_one_request() -> Result<()> {
    require(
        std::env::var("MEWU_PROBE_ALLOW_HTTP").ok().as_deref()
            == Some("one-request-doubao-260628-normalized-v2"),
        "未显式开启一次真实请求",
    )?;
    let base = PathBuf::from(OUTPUT_BASE);
    fs::create_dir_all(&base).map_err(error)?;
    let marker = base.join("doubao-260628-normalized1000-v2-real-request-reserved.once");
    require(
        !marker.try_exists().map_err(error)?,
        "真实请求已预留或执行；禁止自动重试",
    )?;
    let data = std::env::var_os("MEWU_PROBE_DATA_ROOT").ok_or("缺少 MEWU_PROBE_DATA_ROOT")?;
    let data_root =
        fs::canonicalize(PathBuf::from(data)).map_err(|_| "用户数据根目录不可用".to_string())?;
    let output_base = fs::canonicalize(&base).map_err(error)?;
    require(
        !data_root.starts_with(&output_base) && !output_base.starts_with(&data_root),
        "探针输出与用户数据根目录必须独立",
    )?;
    let profile = selected_profile_read_only(&data_root)?;
    let transport = connection_host::transport_for(&profile)?;
    let root = new_output("live-v2")?;
    let mut f = fixture_with_profile(
        root.clone(),
        &profile,
        VisualCoordinateProfile::ExplicitNormalized1000V2,
    )?;
    // Preflight completed without secrets. Read this one profile's credential
    // once, retain Zeroizing<String>, never print/serialize it or env variables.
    let key = credentials::read_profile(&data_root, &profile)
        .map_err(|_| "无法读取指定连接的密钥".to_string())?;
    require(!key.is_empty(), "指定连接密钥为空")?;
    let sends = AtomicUsize::new(0);
    let mut lease = None;
    let body = f.body.clone();
    let received=tokio::time::timeout(Duration::from_secs(60),provider_transport::stream_turn_before_send(
        &transport,&key,body,|_|{},||{
            require(sends.compare_exchange(0,1,Ordering::SeqCst,Ordering::SeqCst).is_ok(),"禁止第二次发送")?;
            let mut reserved=OpenOptions::new().write(true).create_new(true).open(&marker)
                .map_err(|_|"一次请求标记已存在；禁止重试".to_string())?;
            reserved.write_all(b"One real provider attempt reserved. Do not auto-retry or delete this marker.\n").map_err(error)?;
            reserved.sync_all().map_err(error)?;
            f.plugins.visual_annotations(&f.scope.origin.grant)?;
            lease=Some(model_dispatch(&mut f)?);
            Ok(())
        }
    )).await;
    drop(key);
    let turn = match received {
        Ok(Ok(turn)) => turn,
        other => {
            if let Some(lease) = &lease {
                f.store
                    .record_request_unknown(lease, NoResponseReason::IncompleteResponse)
                    .map_err(error)?;
            }
            let reason = if other.is_err() {
                "timeout; no retry"
            } else {
                "transport or response error; no retry"
            };
            if f.store
                .run_is_active(&f.context.scene_id, &f.context.run_id)
            {
                f.store.cancel_run(&f.context.scene_id).map_err(error)?;
            }
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"provider","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err(reason.into());
        }
    };
    let lease = lease.ok_or("回复缺少实际派发记录")?;
    let observed = commit_turn(&mut f, &lease, turn);
    let report = match observed {
        Ok(report) => report,
        Err(reason) => {
            if f.store
                .run_is_active(&f.context.scene_id, &f.context.run_id)
            {
                f.store.cancel_run(&f.context.scene_id).map_err(error)?;
            }
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"local-tool","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err("真实回复未通过本地批注提交；已保存隔离证据，未重试".into());
        }
    };
    let (root, mut report) = match finish_local_checks(f, report) {
        Ok(value) => value,
        Err(reason) => {
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"compositor-lifecycle","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err("批注已提交，但本地合成或撤销检查失败；未重新请求模型".into());
        }
    };
    report["kind"] = json!("real provider single normalized1000-v2 request");
    report["profileId"] = json!(VisualCoordinateProfile::ExplicitNormalized1000V2.id());
    report["model"] = json!(SELECTED_MODEL);
    report["protocol"] = json!("chat_completions");
    report["sendGateCount"] = json!(sends.load(Ordering::SeqCst));
    report["requestLimit"] = json!(1);
    report["outputTokenLimit"] = json!(2048);
    report["deadlineSeconds"] = json!(60);
    report["retryCount"] = json!(0);
    write_json(&root.join("probe-report.json"), &report)?;
    require(
        report["passed"] == true,
        "真实工具链完成，但图上定位质量未达标；参阅隔离 PNG 与坐标误差，不自动重试",
    )
}

// Separate compatibility probe for the documented non-thinking request mode.
// Default-mode timeout evidence and both earlier markers remain immutable.
#[tokio::test(flavor = "current_thread")]
#[ignore = "Real paid provider request; root only; explicit env gate and once-only marker required"]
async fn real_doubao_normalized_v2_nonthinking_one_request() -> Result<()> {
    require(
        std::env::var("MEWU_PROBE_ALLOW_HTTP").ok().as_deref()
            == Some("one-request-doubao-260628-normalized-v2-nonthinking"),
        "未显式开启一次真实请求",
    )?;
    let base = PathBuf::from(OUTPUT_BASE);
    fs::create_dir_all(&base).map_err(error)?;
    let marker =
        base.join("doubao-260628-normalized1000-v2-nonthinking-real-request-reserved.once");
    require(
        !marker.try_exists().map_err(error)?,
        "真实请求已预留或执行；禁止自动重试",
    )?;
    let data = std::env::var_os("MEWU_PROBE_DATA_ROOT").ok_or("缺少 MEWU_PROBE_DATA_ROOT")?;
    let data_root =
        fs::canonicalize(PathBuf::from(data)).map_err(|_| "用户数据根目录不可用".to_string())?;
    let output_base = fs::canonicalize(&base).map_err(error)?;
    require(
        !data_root.starts_with(&output_base) && !output_base.starts_with(&data_root),
        "探针输出与用户数据根目录必须独立",
    )?;
    let profile = selected_profile_read_only(&data_root)?;
    let transport = connection_host::transport_for(&profile)?;
    let root = new_output("live-v2-nonthinking")?;
    let mut f = fixture_with_thinking(
        root.clone(),
        &profile,
        VisualCoordinateProfile::ExplicitNormalized1000V2,
        true,
    )?;
    // Preflight completed without secrets. Read this one profile's credential
    // once, retain Zeroizing<String>, never print/serialize it or env variables.
    let key = credentials::read_profile(&data_root, &profile)
        .map_err(|_| "无法读取指定连接的密钥".to_string())?;
    require(!key.is_empty(), "指定连接密钥为空")?;
    let sends = AtomicUsize::new(0);
    let mut lease = None;
    let body = f.body.clone();
    let received=tokio::time::timeout(Duration::from_secs(60),provider_transport::stream_turn_before_send(
        &transport,&key,body,|_|{},||{
            require(sends.compare_exchange(0,1,Ordering::SeqCst,Ordering::SeqCst).is_ok(),"禁止第二次发送")?;
            let mut reserved=OpenOptions::new().write(true).create_new(true).open(&marker)
                .map_err(|_|"一次请求标记已存在；禁止重试".to_string())?;
            reserved.write_all(b"One real provider attempt reserved. Do not auto-retry or delete this marker.\n").map_err(error)?;
            reserved.sync_all().map_err(error)?;
            f.plugins.visual_annotations(&f.scope.origin.grant)?;
            lease=Some(model_dispatch(&mut f)?);
            Ok(())
        }
    )).await;
    drop(key);
    let turn = match received {
        Ok(Ok(turn)) => turn,
        other => {
            if let Some(lease) = &lease {
                f.store
                    .record_request_unknown(lease, NoResponseReason::IncompleteResponse)
                    .map_err(error)?;
            }
            let reason = if other.is_err() {
                "timeout; no retry"
            } else {
                "transport or response error; no retry"
            };
            if f.store
                .run_is_active(&f.context.scene_id, &f.context.run_id)
            {
                f.store.cancel_run(&f.context.scene_id).map_err(error)?;
            }
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"provider","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err(reason.into());
        }
    };
    let lease = lease.ok_or("回复缺少实际派发记录")?;
    let observed = commit_turn(&mut f, &lease, turn);
    let report = match observed {
        Ok(report) => report,
        Err(reason) => {
            if f.store
                .run_is_active(&f.context.scene_id, &f.context.run_id)
            {
                f.store.cancel_run(&f.context.scene_id).map_err(error)?;
            }
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"local-tool","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err("真实回复未通过本地批注提交；已保存隔离证据，未重试".into());
        }
    };
    let (root, mut report) = match finish_local_checks(f, report) {
        Ok(value) => value,
        Err(reason) => {
            write_json(
                &root.join("probe-report.json"),
                &json!({"passed":false,"stage":"compositor-lifecycle","reason":reason,"sendGateCount":sends.load(Ordering::SeqCst),"requestLimit":1}),
            )?;
            return Err("批注已提交，但本地合成或撤销检查失败；未重新请求模型".into());
        }
    };
    report["kind"] = json!("real provider single normalized1000-v2 nonthinking request");
    report["profileId"] = json!(VisualCoordinateProfile::ExplicitNormalized1000V2.id());
    report["testOnlyThinkingOverride"] = json!("disabled");
    report["model"] = json!(SELECTED_MODEL);
    report["protocol"] = json!("chat_completions");
    report["sendGateCount"] = json!(sends.load(Ordering::SeqCst));
    report["requestLimit"] = json!(1);
    report["outputTokenLimit"] = json!(2048);
    report["deadlineSeconds"] = json!(60);
    report["retryCount"] = json!(0);
    write_json(&root.join("probe-report.json"), &report)?;
    require(
        report["passed"] == true,
        "真实工具链完成，但图上定位质量未达标；参阅隔离 PNG 与坐标误差，不自动重试",
    )
}

#[test]
fn visual_nonthinking_probe_mode_is_explicit_hashed_and_never_saved_in_profile() -> Result<()> {
    let profile = synthetic_profile();
    let root = new_output("offline-v2-nonthinking")?;
    let f = fixture_with_thinking(
        root,
        &profile,
        VisualCoordinateProfile::ExplicitNormalized1000V2,
        true,
    )?;
    require(
        f.body["thinking"] == json!({"type":"disabled"}),
        "思考模式未明确关闭",
    )?;
    require(
        f.request_sha256 == sha(&serde_json::to_vec(&f.body).map_err(error)?),
        "派发摘要没有绑定实际选项",
    )?;
    require(
        f.context.connection_profile.advanced == profile.advanced,
        "隔离模式改变了保存的连接参数",
    )?;
    let default = fixture_with_profile(
        new_output("offline-v2-default-mode")?,
        &profile,
        VisualCoordinateProfile::ExplicitNormalized1000V2,
    )?;
    require(default.body.get("thinking").is_none(), "默认模式被隐式改变")
}
