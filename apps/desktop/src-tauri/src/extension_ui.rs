// SPDX-License-Identifier: MPL-2.0
//! Plugin content uses the same isolated, movable HTML objects as generated
//! content. A widget never receives Tauri IPC or a main-window DOM reference.
use crate::Host;
use serde_json::{json, Value};
use std::io::Write;
use tauri::{AppHandle, Manager};

pub(crate) fn render(
    app: &AppHandle,
    scene_id: &str,
    plugin_id: &str,
    revision: u64,
    html: &str,
    width: u32,
    height: u32,
) -> Result<Value, String> {
    if html.len() > 256 * 1024 {
        return Err("插件界面过大".into());
    }
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    {
        let engine = host.lock()?;
        let scene = engine
            .store
            .scenes()
            .iter()
            .find(|s| s.id == scene_id)
            .ok_or("会话不存在")?;
        if scene.closed || engine.store.snapshot().active_scene_id != scene_id {
            return Err("界面所属会话已切换".into());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let path = host.assets.join(format!("{id}.html"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "无法保存插件界面")?;
    if file
        .write_all(html.as_bytes())
        .and_then(|_| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err("无法保存插件界面".into());
    }
    drop(file);
    let asset = mewu_core::Asset {
        id: id.clone(),
        name: format!("{plugin_id}.html"),
        kind: mewu_core::AssetKind::Html,
        path: path.to_string_lossy().into_owned(),
        width: Some(width.clamp(80, 1600)),
        height: Some(height.clamp(40, 1200)),
        origin_x: None,
        origin_y: None,
        scale_factor: None,
        capture_coordinates: None,
    };
    let result = (|| {
        let extensions = app.state::<crate::extension_host::ExtensionHost>();
        let registry = extensions.store()?;
        let record = registry.get(plugin_id)?;
        if record.state != "enabled" || record.revision != revision {
            return Err("插件已停用或版本已变化".into());
        }
        let mut engine = host.lock()?;
        host.exit.ensure_new_operation()?;
        if engine.store.snapshot().active_scene_id != scene_id {
            return Err("界面所属会话已切换".to_string());
        }
        let snapshot = engine
            .store
            .add_asset(scene_id, asset)
            .map_err(|e| e.to_string())?;
        crate::publish(app, &snapshot);
        Ok(json!({"assetId":id,"sceneId":scene_id}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}
