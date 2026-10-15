// SPDX-License-Identifier: MPL-2.0
//! Plugin content uses the same isolated, movable HTML objects as generated
//! content. A widget never receives Tauri IPC or a main-window DOM reference.
use crate::Host;
use serde_json::{json, Value};
use std::io::Write;
use tauri::{AppHandle, Manager};

fn logical_viewport(size: tauri::PhysicalSize<u32>, scale: f64) -> Result<(u32, u32), String> {
    if !scale.is_finite() || scale <= 0. {
        return Err("无法读取空间缩放".into());
    }
    let logical = size.to_logical::<u32>(scale);
    if !(2..=32768).contains(&logical.width) || !(2..=32768).contains(&logical.height) {
        return Err("无法读取空间尺寸".into());
    }
    Ok((logical.width, logical.height))
}

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
    // Tauri reports physical client pixels; plugin width/height are CSS pixels.
    // Native window queries must precede Engine/registry locks (UI dispatch).
    let window = app.get_webview_window("space").ok_or("空间窗口不可用")?;
    let (viewport_width, viewport_height) = logical_viewport(
        window.inner_size().map_err(|_| "无法读取空间尺寸")?,
        window.scale_factor().map_err(|_| "无法读取空间缩放")?,
    )?;
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
            .add_html_asset_in_viewport(scene_id, asset, viewport_width, viewport_height)
            .map_err(|e| e.to_string())?;
        crate::publish(app, &snapshot);
        Ok(json!({"assetId":id,"sceneId":scene_id}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::logical_viewport;
    use tauri::PhysicalSize;

    #[test]
    fn native_physical_viewport_maps_to_plugin_css_pixels() {
        for (width, height, scale, expected) in [
            (1920, 1080, 1.25, (1536, 864)),
            (2304, 1296, 1.5, (1536, 864)),
            (3072, 1728, 2., (1536, 864)),
            (1536, 864, 1., (1536, 864)),
            (1080, 1920, 1.25, (864, 1536)),
        ] {
            assert_eq!(
                logical_viewport(PhysicalSize::new(width, height), scale).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn invalid_native_scale_or_viewport_is_rejected_before_file_creation() {
        for scale in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(logical_viewport(PhysicalSize::new(1920, 1080), scale).is_err());
        }
        for size in [(0, 1080), (1920, 1), (32769, 1080)] {
            assert!(logical_viewport(PhysicalSize::new(size.0, size.1), 1.).is_err());
        }
    }
}
