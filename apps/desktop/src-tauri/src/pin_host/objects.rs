// SPDX-License-Identifier: MPL-2.0
//! Desktop objects keep their own pixel lease, independently of scene lifetime.
//! Referencing creates a durable attachment; it never saves a staging path.
use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinObject {
    id: String,
    image_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    media: Option<media::PinVideo>,
    width: u32,
    height: u32,
    quarter_turns: u8,
    opacity: f64,
    topmost: bool,
    attachment_asset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin_asset_id: Option<String>,
    x: f64,
    y: f64,
    display_width: f64,
    display_height: f64,
}
pub(crate) fn changed(app: &AppHandle) {
    let _ = app.emit_to("space", "pin-objects-changed", ());
}

#[tauri::command]
pub async fn get_pin_objects(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<PinObject>, String> {
    space_owner(&window)?;
    let viewport = pin_window::current_space_rect(&app).await?;
    if viewport.width == 0 || viewport.height == 0 {
        return Err("空间尺寸无效".into());
    }
    let registry = app.state::<PinRegistry>();
    let mut objects = {
        let state = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        state
            .entries
            .iter()
            .filter(|(_, e)| e.live)
            .map(|(label, e)| {
                let [origin_asset_id, attachment_asset_id] = e.presentation_asset_ids();
                (
                    label.clone(),
                    PinObject {
                        id: e.id.clone(),
                        image_url: format!(
                            "{}/{}",
                            app.state::<Host>().content_origin,
                            e.source.asset().id
                        ),
                        media: e.source.0.media.clone(),
                        width: e.source.asset().width.unwrap(),
                        height: e.source.asset().height.unwrap(),
                        quarter_turns: e.quarter_turns,
                        opacity: e.opacity,
                        topmost: e.topmost,
                        attachment_asset_id: attachment_asset_id.map(str::to_owned),
                        origin_asset_id: origin_asset_id.map(str::to_owned),
                        x: 0.,
                        y: 0.,
                        display_width: 0.,
                        display_height: 0.,
                    },
                )
            })
            .collect::<Vec<_>>()
    };
    let mut result = Vec::new();
    for (label, mut object) in objects.drain(..) {
        if let Some(pin) = app.get_webview_window(&label) {
            if let Ok(rect) = pin_window::current_rect(&pin) {
                let padding = pin_window::PADDING;
                object.x = (f64::from(rect.x) + f64::from(padding) - f64::from(viewport.x))
                    / f64::from(viewport.width);
                object.y = (f64::from(rect.y) + f64::from(padding) - f64::from(viewport.y))
                    / f64::from(viewport.height);
                object.display_width =
                    f64::from(rect.width.saturating_sub(padding * 2)) / f64::from(viewport.width);
                object.display_height =
                    f64::from(rect.height.saturating_sub(padding * 2)) / f64::from(viewport.height);
                result.push(object);
            }
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectControl {
    Close {},
    Move { x: f64, y: f64 },
    Zoom { direction: i8 },
}
#[tauri::command]
pub async fn control_pin_object(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    action: ObjectControl,
) -> Result<(), String> {
    space_owner(&window)?;
    if !valid_uuid(&id) {
        return Err("贴图对象无效".into());
    }
    app.state::<Host>().exit.ensure_running()?;
    let label = format!("pin-{id}");
    let pin = app.get_webview_window(&label).ok_or("贴图已关闭")?;
    {
        let registry = app.state::<PinRegistry>();
        let state = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        if !state.entries.get(&label).is_some_and(|entry| entry.live) {
            return Err("贴图已关闭".into());
        }
    }
    match action {
        ObjectControl::Close {} => close_entry(&app, &label, false),
        ObjectControl::Zoom { direction } => {
            pin_window::control(&app, &pin, PinAction::Zoom { direction }).await?
        }
        ObjectControl::Move { x, y } => {
            if !x.is_finite()
                || !y.is_finite()
                || !(-16. ..=16.).contains(&x)
                || !(-16. ..=16.).contains(&y)
            {
                return Err("贴图位置无效".into());
            }
            let vp = pin_window::current_space_rect(&app).await?;
            let mut rect = pin_window::current_rect(&pin)?;
            rect.x = (f64::from(vp.x) + x * f64::from(vp.width) - f64::from(pin_window::PADDING))
                .round() as i32;
            rect.y = (f64::from(vp.y) + y * f64::from(vp.height) - f64::from(pin_window::PADDING))
                .round() as i32;
            pin_window::apply_rect(&pin, rect)?;
        }
    }
    changed(&app);
    Ok(())
}

struct AttachmentFile {
    path: PathBuf,
    adopted: bool,
}
impl Drop for AttachmentFile {
    fn drop(&mut self) {
        if !self.adopted {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn prepare_attachment(
    source: &PinAssetLease,
    turns: u8,
    root: &Path,
) -> Result<(Asset, AttachmentFile), String> {
    let mut lease = crate::image_host::SourceLease::open(source.asset(), source.root())?;
    let image = rotate(lease.decode_rgba(source.asset())?, turns);
    lease.verify()?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.pin.png"));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|_| "无法保存贴图")?;
    let guard = AttachmentFile {
        path: path.clone(),
        adopted: false,
    };
    let result = encode(&image, &mut output)
        .and_then(|_| output.sync_all().map_err(|_| "无法保存贴图".into()));
    drop(output);
    result?;
    Ok((
        Asset {
            capture_coordinates: None,
            id,
            kind: AssetKind::Image,
            path: path.to_string_lossy().into(),
            name: "贴图.png".into(),
            width: Some(image.width()),
            height: Some(image.height()),
            scale_factor: None,
            origin_x: None,
            origin_y: None,
        },
        guard,
    ))
}

#[tauri::command]
pub async fn reference_pin_object(
    app: AppHandle,
    window: WebviewWindow,
    id: Option<String>,
    scene_id: Option<String>,
) -> Result<Option<crate::HostSnapshot>, String> {
    if (window.label() == "space" && scene_id.is_none())
        || (window.label() != "space" && scene_id.is_some())
    {
        return Err("会话身份无效".into());
    }
    let label = if window.label() == "space" {
        let id = id.filter(|id| valid_uuid(id)).ok_or("贴图对象无效")?;
        format!("pin-{id}")
    } else if is_pin_label(window.label()) && id.is_none() {
        window.label().into()
    } else {
        return Err("此窗口不能引用贴图".into());
    };
    app.state::<Host>().exit.ensure_running()?;
    let registry = app.state::<PinRegistry>();
    let (source, turns, cached) = {
        let state = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        let entry = state
            .entries
            .get(&label)
            .filter(|e| e.live)
            .ok_or("贴图已关闭")?;
        (
            entry.source.clone(),
            entry.quarter_turns,
            entry.object_assets.get(&entry.quarter_turns).cloned(),
        )
    };
    if source.0.media.is_some() {
        return Err("视频置顶暂不支持引用".into());
    }
    let host = app.state::<Host>();
    let (scene_id, expected) = {
        let engine = host.lock()?;
        let snap = engine.store.snapshot();
        let scene_id = scene_id.unwrap_or_else(|| snap.active_scene_id.clone());
        let scene = snap
            .scenes
            .iter()
            .find(|s| {
                s.id == scene_id
                    && snap.active_scene_id == s.id
                    && !s.closed
                    && !s.frozen
                    && s.blackboard_link.is_none()
                    && !s
                        .run
                        .as_ref()
                        .is_some_and(|r| r.status == mewu_core::RunStatus::Running)
            })
            .ok_or("会话不可引用")?;
        (scene_id, scene.clone())
    };
    let (asset, mut file) = if let Some(asset) = cached {
        (asset, None)
    } else {
        let work = registry.work.begin(&app)?;
        let decode = registry
            .decode
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "贴图处理已结束")?;
        let owned_source = source.clone();
        let root = host.assets.clone();
        let (asset, file) = tauri::async_runtime::spawn_blocking(move || {
            let (_work, _decode) = (work, decode);
            prepare_attachment(&owned_source, turns, &root)
        })
        .await
        .map_err(|_| "贴图保存中断")??;
        (asset, Some(file))
    };
    let snapshot = {
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        let snap = engine.store.snapshot();
        if snap.active_scene_id != scene_id
            || snap.scenes.iter().find(|s| s.id == scene_id) != Some(&expected)
        {
            return Err("会话已更改，请重新引用".into());
        }
        let mut state = registry.state.lock().map_err(|_| "贴图状态不可用")?;
        let entry = state
            .entries
            .get_mut(&label)
            .filter(|e| e.live && e.source.asset() == source.asset() && e.quarter_turns == turns)
            .ok_or("贴图已更改或关闭")?;
        let snapshot = engine
            .store
            .reference_shared_image(&scene_id, asset.clone())
            .map_err(|e| e.to_string())?;
        if let Some(file) = &mut file {
            file.adopted = true;
        }
        entry.object_assets.insert(turns, asset);
        snapshot
    };
    changed(&app);
    let published = crate::publish(&app, &snapshot);
    Ok(if window.label() == "space" {
        Some(published)
    } else {
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use mewu_core::{Drawing, DrawingKind, DrawingPoint, ImageAnnotationEdit, ReferenceKind, SceneCommand, Store};
    use sha2::{Digest, Sha256};

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let (Ok(path), Ok(parent)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize()) else { return; };
            if path.parent() == Some(parent.as_path()) && path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_prefix("mewu-annotated-pin-reference-")).is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()) {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
    fn test_entry(source: PinAssetLease, registry: &PinRegistry) -> Entry {
        Entry {
            id: uuid::Uuid::new_v4().to_string(), revision: 1, source,
            origin_asset_id: None, object_assets: HashMap::new(), quarter_turns: 0,
            topmost: true, opacity: 1., epoch: 0,
            original: pin_window::Rect { x: 0, y: 0, width: 664, height: 424 },
            source_viewport: pin_window::Rect { x: 0, y: 0, width: 1920, height: 1080 },
            live: true, built: true, request_id: uuid::Uuid::new_v4().to_string(),
            cancel: Cancel::default(), ready: None,
            _window_slot: Arc::new(registry.windows.clone().try_acquire_owned().unwrap()),
        }
    }
    #[test]
    fn annotated_pin_references_baked_pixels_at_zero_and_quarter_turn_after_source_close_and_reopen() {
        let root = Fixture(std::env::temp_dir().join(format!("mewu-annotated-pin-reference-{}", uuid::Uuid::new_v4())));
        let assets_root = root.0.join("assets");
        std::fs::create_dir_all(&assets_root).unwrap();
        let database = root.0.join("storage.db");
        let mut store = Store::open(&database).unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let asset_id = uuid::Uuid::new_v4().to_string();
        let original_path = assets_root.join(format!("{asset_id}.png"));
        let original = RgbaImage::from_pixel(640, 400, image::Rgba([255, 255, 255, 255]));
        original.save(&original_path).unwrap();
        let original_hash = Sha256::digest(std::fs::read(&original_path).unwrap());
        let asset = Asset {
            id: asset_id.clone(), kind: AssetKind::Image, name: "exam.png".into(),
            path: original_path.to_string_lossy().into(), width: Some(640), height: Some(400),
            capture_coordinates: None, origin_x: None, origin_y: None, scale_factor: None,
        };
        store.add_asset(&scene_id, asset.clone()).unwrap();
        let item_id = store.scene(&scene_id).unwrap().items[0].id.clone();
        store.apply(SceneCommand::EditImageAnnotations {
            scene_id: scene_id.clone(), item_id: item_id.clone(), source_asset_id: asset_id.clone(), expected_revision: 0,
            action: ImageAnnotationEdit::AddDrawing { drawing: Drawing {
                id: uuid::Uuid::new_v4().to_string(), kind: DrawingKind::Line, color: "#ff0000".into(), stroke_width: 12.,
                points: vec![DrawingPoint { x: 80., y: 200. }, DrawingPoint { x: 560., y: 200. }],
                text: None, font_size: None, text_style: None, rich: None, origin: None,
            } },
        }).unwrap();
        let imported = store.scene(&scene_id).unwrap().items[0].clone();
        let composited = assets::compose_image_item(&imported, original.clone(), &assets_root).unwrap();
        assert_eq!(*composited.get_pixel(320, 200), image::Rgba([255, 0, 0, 255]));
        assert_ne!(composited, original);
        let registry = PinRegistry::default();
        let pixels = registry.pixels.clone().try_acquire_many_owned(640 * 400).unwrap();
        let source = stage(&root.0, &composited, pixels).unwrap();
        let staged_path = PathBuf::from(&source.asset().path);
        let mut entry = test_entry(source.clone(), &registry);
        register_imported_attachment(&mut entry, &imported);
        assert_eq!(entry.origin_asset_id.as_deref(), Some(asset_id.as_str()));
        assert!(!entry.object_assets.contains_key(&0), "the uncomposited source is display identity, not a reference cache");
        assert_eq!(original_hash, Sha256::digest(std::fs::read(&original_path).unwrap()));
        store.apply(SceneCommand::CloseScene { scene_id }).unwrap();
        let mut referenced = Vec::new();
        for turns in [0, 1] {
            let (attachment, mut guard) = prepare_attachment(&entry.source, turns, &assets_root).unwrap();
            let expected = rotate(composited.clone(), turns);
            assert_ne!(attachment.id, asset_id);
            assert_eq!(assets::image(&attachment, &assets_root).unwrap().into_rgba8(), expected);
            let active = store.snapshot().active_scene_id;
            let snapshot = store.reference_shared_image(&active, attachment.clone()).unwrap();
            guard.adopted = true;
            entry.object_assets.insert(turns, attachment.clone());
            assert_eq!(store.reference_shared_image(&active, attachment.clone()).unwrap(), snapshot);
            let scene = store.scene(&active).unwrap();
            assert_eq!(scene.items.len(), 1); assert_eq!(scene.refs.len(), 1);
            assert_eq!(scene.refs[0].kind, ReferenceKind::Item);
            assert!(scene.items[0].image_annotations.is_none(), "the snapshot pixels are already baked exactly once");
            assert_eq!(assets::image_item(&scene.items[0], &assets_root).unwrap().into_rgba8(), expected);
            let prepared = crate::ai::prepare_scene_visual_attachments(scene, &assets_root, crate::visual_annotation_input::VisualCoordinateProfile::ExplicitNormalized1000V2).unwrap();
            let target = &prepared.visual_input().unwrap().manifest().targets[0];
            assert_eq!(target.image_item_id.as_deref(), Some(scene.items[0].id.as_str()));
            let sent = crate::ai::persist_attachments(prepared, &assets_root).unwrap();
            assert_eq!(sent.len(), 1);
            let encoded = assets::image(&sent[0], &assets_root).unwrap().into_rgb8();
            let pixel = encoded.get_pixel(encoded.width() / 2, encoded.height() / 2);
            assert!(pixel[0] > 220 && pixel[1] < 40 && pixel[2] < 40, "AI receives the red ink through the actual JPEG preparation");
            referenced.push((active, attachment, expected));
            store.apply(SceneCommand::NewScene).unwrap();
        }
        assert_eq!(original_hash, Sha256::digest(std::fs::read(&original_path).unwrap()));
        std::fs::remove_file(&original_path).unwrap();
        drop(entry); drop(source);
        assert!(!staged_path.exists(), "closing the independent pin releases its staging source");
        drop(store);
        let reopened = Store::open(&database).unwrap();
        for (scene_id, attachment, expected) in referenced {
            let scene = reopened.scene(&scene_id).unwrap();
            assert_eq!(scene.items[0].asset, attachment);
            assert_eq!(assets::image_item(&scene.items[0], &assets_root).unwrap().into_rgba8(), expected);
        }
    }

    #[test]
    fn unannotated_import_keeps_its_existing_source_reference_cache() {
        let root = Fixture(std::env::temp_dir().join(format!("mewu-annotated-pin-reference-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(&root.0).unwrap();
        let registry = PinRegistry::default();
        let source = stage(&root.0, &RgbaImage::new(2, 2), registry.pixels.clone().try_acquire_many_owned(4).unwrap()).unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let scene = store.snapshot().active_scene_id;
        let asset = Asset { id: uuid::Uuid::new_v4().to_string(), path: "owned/image.png".into(), name: "image.png".into(), kind: AssetKind::Image, width: Some(2), height: Some(2), capture_coordinates: None, origin_x: None, origin_y: None, scale_factor: None };
        store.add_asset(&scene, asset.clone()).unwrap();
        let mut entry = test_entry(source, &registry);
        register_imported_attachment(&mut entry, &store.scene(&scene).unwrap().items[0]);
        assert_eq!(entry.origin_asset_id.as_deref(), Some(asset.id.as_str()));
        assert_eq!(entry.object_assets.get(&0), Some(&asset));
    }

    #[test]
    fn live_pin_image_targets_are_readonly_mixed_targets_survive_and_close_restores_writing() {
        use mewu_core::{Reference, Region};
        use crate::visual_annotation_input::VisualCoordinateProfile;
        let root = Fixture(std::env::temp_dir().join(format!("mewu-annotated-pin-reference-{}", uuid::Uuid::new_v4())));
        let assets_root = root.0.join("assets");
        std::fs::create_dir_all(&assets_root).unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let mut images = Vec::new();
        for name in ["pinned.png", "visible.png"] {
            let id = uuid::Uuid::new_v4().to_string();
            let path = assets_root.join(format!("{id}.png"));
            RgbaImage::from_pixel(64, 40, image::Rgba([255, 255, 255, 255])).save(&path).unwrap();
            let asset = Asset { id, path: path.to_string_lossy().into(), name: name.into(), kind: AssetKind::Image, width: Some(64), height: Some(40), capture_coordinates: None, origin_x: None, origin_y: None, scale_factor: None };
            store.add_asset(&scene_id, asset.clone()).unwrap();
            images.push(asset);
        }
        let items = store.scene(&scene_id).unwrap().items.clone();
        store.apply(SceneCommand::SetRefs { scene_id: scene_id.clone(), refs: vec![Reference { kind: ReferenceKind::Item, id: items[0].id.clone() }] }).unwrap();
        let registry = PinRegistry::default();
        let source = stage(&root.0, &RgbaImage::new(2, 2), registry.pixels.clone().try_acquire_many_owned(4).unwrap()).unwrap();
        let mut entry = test_entry(source, &registry);
        entry.live = false;
        register_imported_attachment(&mut entry, &items[0]);
        let label = format!("pin-{}", entry.id);
        {
            let mut pins = registry.state.lock().unwrap();
            pins.entries.insert(label.clone(), entry);
            assert!(pins.hidden_image_assets(store.scene(&scene_id).unwrap()).is_empty(), "origin identity is registered before publication but pending windows do not hide content");
            pins.entries.get_mut(&label).unwrap().live = true;
        }
        let profile = VisualCoordinateProfile::ExplicitNormalized1000V2;
        let hidden = registry.hidden_image_assets(store.scene(&scene_id).unwrap()).unwrap();
        assert_eq!(hidden, HashSet::from([images[0].id.clone()]));
        let readonly = crate::ai::prepare_scene_visual_attachments_excluding_pins(store.scene(&scene_id).unwrap(), &assets_root, profile, &hidden).unwrap();
        assert!(readonly.visual_input().is_none(), "read-only inputs are None, never an invalid empty VerifiedVisualInput");
        assert_eq!(readonly.attachment_count(), 1);
        let sent = crate::ai::persist_attachments(readonly, &assets_root).unwrap();
        assert_eq!(assets::image(&sent[0], &assets_root).unwrap().dimensions(), (64, 40));

        let background_id = uuid::Uuid::new_v4().to_string();
        let background = Asset { id: background_id, name: "capture.png".into(), ..images[0].clone() };
        store.set_background(&scene_id, background).unwrap();
        let region = Region { id: uuid::Uuid::new_v4().to_string(), x: 0., y: 0., width: 64., height: 40., ..Region::default() };
        store.apply(SceneCommand::AddRegion { scene_id: scene_id.clone(), region: region.clone() }).unwrap();
        let refs = vec![Reference { kind: ReferenceKind::Item, id: items[0].id.clone() }, Reference { kind: ReferenceKind::Item, id: items[1].id.clone() }, Reference { kind: ReferenceKind::Region, id: region.id.clone() }];
        store.apply(SceneCommand::SetRefs { scene_id: scene_id.clone(), refs }).unwrap();
        let scene = store.scene(&scene_id).unwrap();
        let mixed = crate::ai::prepare_scene_visual_attachments_excluding_pins(scene, &assets_root, profile, &hidden).unwrap();
        assert_eq!(mixed.attachment_count(), 3, "the read-only image is still sent for vision");
        let mixed_input = mixed.visual_input().unwrap();
        assert_eq!(mixed_input.manifest().targets.len(), 2);
        assert!(mixed_input.manifest().targets.iter().any(|target| target.image_item_id.as_deref() == Some(items[1].id.as_str())));
        assert!(mixed_input.manifest().targets.iter().any(|target| target.region_id == region.id));
        registry.state.lock().unwrap().ensure_visual_targets_visible(scene, mixed_input).unwrap();

        registry.state.lock().unwrap().entries.get_mut(&label).unwrap().live = false;
        let hidden = registry.hidden_image_assets(scene).unwrap();
        assert!(hidden.is_empty());
        let unpinned = crate::ai::prepare_scene_visual_attachments_excluding_pins(scene, &assets_root, profile, &hidden).unwrap();
        let input = unpinned.visual_input().unwrap();
        assert_eq!(input.manifest().targets.len(), 3, "closing a pin restores writing on the next preparation without permanent item state");
        registry.state.lock().unwrap().ensure_visual_targets_visible(scene, input).unwrap();
        let before = store.snapshot();
        {
            let mut pins = registry.state.lock().unwrap();
            let entry = pins.entries.get_mut(&label).unwrap();
            entry.live = true;
            assert!(pins.ensure_visual_targets_visible(scene, input).is_err(), "a pin published after preparation rejects the old writable target before commit");
            let entry = pins.entries.get_mut(&label).unwrap();
            entry.quarter_turns = 1;
            assert!(pins.hidden_image_assets(scene).is_empty(), "origin identity only hides the original orientation");
            pins.entries.get_mut(&label).unwrap().object_assets.insert(1, images[1].clone());
            assert_eq!(pins.hidden_image_assets(scene), HashSet::from([images[1].id.clone()]));
            assert!(pins.ensure_visual_targets_visible(scene, input).is_err(), "rotation may hide a different cached item and is checked under the same commit guard");
            let mut board = scene.clone();
            let background = board.background.as_mut().unwrap();
            background.name = "黑板.png".into();
            background.path = assets_root.join(format!("{}.board.png", background.id)).to_string_lossy().into();
            assert!(pins.hidden_image_assets(&board).is_empty(), "blackboard media remain visible just as in App.canvasScene");
            pins.ensure_visual_targets_visible(&board, input).unwrap();
        }
        assert_eq!(store.snapshot(), before);
    }
    #[test]
    fn move_shape_rejects_forged_fields() {
        assert!(
            serde_json::from_str::<ObjectControl>(r#"{"type":"move","x":0.1,"y":0.2}"#).is_ok()
        );
        assert!(
            serde_json::from_str::<ObjectControl>(r#"{"type":"close","path":"C:/x"}"#).is_err()
        );
    }
    #[test]
    fn attachment_is_immutable_rotated_and_removed_if_not_committed() {
        let root =
            std::env::temp_dir().join(format!("mewu-pin-attachment-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let pixels = Arc::new(Semaphore::new(6));
        let slot = pixels.try_acquire_many_owned(6).unwrap();
        let image = RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8, y as u8, 77, 255]));
        let source = stage(&root, &image, slot).unwrap();
        let (asset, file) = prepare_attachment(&source, 1, &root).unwrap();
        assert_eq!((asset.width, asset.height), (Some(2), Some(3)));
        assert_eq!(
            assets::image(&asset, &root).unwrap().into_rgba8(),
            image::imageops::rotate90(&image)
        );
        assert_ne!(asset.path, source.asset().path);
        drop(file);
        assert!(!Path::new(&asset.path).exists());
        assert!(Path::new(&source.asset().path).exists());
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
}
