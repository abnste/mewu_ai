// SPDX-License-Identifier: MPL-2.0
//! Desktop objects keep their own pixel lease, independently of scene lifetime.
//! Referencing creates a durable attachment; it never saves a staging path.
use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinObject {
    id: String,
    image_url: String,
    width: u32,
    height: u32,
    quarter_turns: u8,
    opacity: f64,
    topmost: bool,
    attachment_asset_id: Option<String>,
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
                (
                    label.clone(),
                    PinObject {
                        id: e.id.clone(),
                        image_url: format!(
                            "{}/{}",
                            app.state::<Host>().content_origin,
                            e.source.asset().id
                        ),
                        width: e.source.asset().width.unwrap(),
                        height: e.source.asset().height.unwrap(),
                        quarter_turns: e.quarter_turns,
                        opacity: e.opacity,
                        topmost: e.topmost,
                        attachment_asset_id: e
                            .object_assets
                            .get(&e.quarter_turns)
                            .map(|a| a.id.clone()),
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
        ObjectControl::Move { x, y } => {
            if !x.is_finite()
                || !y.is_finite()
                || !(0. ..=1.).contains(&x)
                || !(0. ..=1.).contains(&y)
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
