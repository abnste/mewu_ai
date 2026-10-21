// SPDX-License-Identifier: MPL-2.0
//! Native object-library adapter for executable plugins. No UI is installed here.
use crate::Host;
use mewu_core::{Asset, AssetKind, SpaceItem, ObjectLibrary, ObjectTemplate};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io::Write, sync::{Arc,atomic::{AtomicBool, Ordering}}};
use tauri::{AppHandle, Manager, Emitter};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Source {
    Item {
        #[serde(rename = "sceneId")]
        scene_id: String,
        #[serde(rename = "itemId")]
        item_id: String,
    },
    Region {
        #[serde(rename = "sceneId")]
        scene_id: String,
        #[serde(rename = "regionId")]
        region_id: String,
    },
    Pin {
        id: String,
    },
    Drawings {
        #[serde(rename = "sceneId")]
        scene_id: String,
        #[serde(rename = "regionId")]
        region_id: Option<String>,
        #[serde(rename = "itemId")]
        item_id: Option<String>,
        #[serde(rename = "drawingIds")]
        drawing_ids: Vec<String>,
    },
}

/// Issued only by the first-party space view for a live registered UI service.
/// It cannot be minted by plugin JS, Agent tools, bindings or services.call.
pub(crate) struct Access {
    pub plugin_id: String,
    pub revision: u64,
    pub scene_id: String,
    pub canceled: Arc<AtomicBool>,
    pub source: Option<Source>,
    pub surface_id: String,
    pub service_id: String,
}
impl Access {
    pub(crate) fn with_store<T>(&self, app:&AppHandle, action:impl FnOnce(&mut mewu_core::Store)->Result<T,String>) -> Result<T,String> {
        let extensions=app.state::<crate::extension_host::ExtensionHost>();
        let registry=extensions.store()?;
        let record=registry.get(&self.plugin_id)?;
        if self.canceled.load(Ordering::SeqCst) || record.state!="enabled" || record.revision!=self.revision
            || !record.manifest.permissions.iter().any(|p|p=="objects.library") { return Err("对象库授权已撤销".into()); }
        record.ensure_supported()?;
        registry.validate_dependencies(&record.manifest)?;
        let detached=extensions.surfaces.window_owner(&self.surface_id).is_some();
        extensions.surfaces.with_owner(&self.surface_id,&crate::extension_surfaces::Owner {
            plugin:self.plugin_id.clone(),revision:self.revision,service:self.service_id.clone(),scene:self.scene_id.clone()
        },||{
            let host=app.state::<Host>(); host.exit.ensure_new_operation()?;
            let mut engine=host.lock()?;
            let scene=engine.store.active_scene().map_err(|e|e.to_string())?;
            if !detached&&(scene.id!=self.scene_id || scene.closed || scene.frozen || scene.run.as_ref().is_some_and(|r|r.status==mewu_core::RunStatus::Running)) { return Err("对象库所属会话已变化".into()); }
            // Registry -> surface -> engine stays held through the transaction.
            action(&mut engine.store)
        })
    }
}
pub(crate) fn is_surface(service:&mewu_extension::ServiceRegistration)->bool {
    service.kind==mewu_extension::ServiceKind::Ui && service.metadata.as_ref().is_some_and(|m|
        m.get("surface").and_then(Value::as_str)==Some("space-overlay"))
}
pub(crate) fn is_settings_surface(service:&mewu_extension::ServiceRegistration)->bool {
    service.kind==mewu_extension::ServiceKind::Ui && service.metadata.as_ref().is_some_and(|m|
        m.get("surface").and_then(Value::as_str)==Some("settings"))
}
pub(crate) fn describe()->Value { json!({"version":1,"capabilities":["ui.surfaces","ui.objects","objects.library","objects.preview"]}) }
#[derive(Clone, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct Commit {
    pub expected_revision:u64,
    pub state:Value,
    #[serde(default)] pub remove:Vec<String>,
    pub capture_key:Option<String>,
}
pub(crate) fn bridge(app:&AppHandle, access:&mut Access, method:&str, params:Value)->Result<Value,String> {
    match method {
        "objects.library.preview"=>{
            #[derive(Deserialize)]
            #[serde(rename_all="camelCase",deny_unknown_fields)]
            struct Preview { expected_revision:u64,id:String }
            let input:Preview=serde_json::from_value(params).map_err(|_|"预览参数无效")?;
            let item=access.with_store(app,|s|{
                let library=s.object_library(&access.plugin_id);
                if library.revision!=input.expected_revision{return Err("对象库已变化".into());}
                Ok(library.objects.get(&input.id).ok_or("对象不存在")?.item.clone())
            })?;
            let root=app.state::<Host>().assets.clone();
            if matches!(item.asset.kind,AssetKind::Html|AssetKind::Svg){
                let html=crate::assets::read_text(&item.asset,&root)?;
                access.with_store(app,|s|{
                    let library=s.object_library(&access.plugin_id);
                    if library.revision!=input.expected_revision||library.objects.get(&input.id).map(|v|&v.item)!=Some(&item){return Err("对象已变化".into());}Ok(())
                })?;
                return Ok(json!({"image":null,"document":{"html":html,"width":item.asset.width.unwrap_or(640),"height":item.asset.height.unwrap_or(480)}}));
            }
            if item.asset.kind!=AssetKind::Image{return Ok(json!({"image":null}));}
            let image=crate::assets::image_item(&item,&root)?.thumbnail(240,180);
            let mut bytes=std::io::Cursor::new(Vec::new());
            image.write_to(&mut bytes,image::ImageFormat::Png).map_err(|_|"无法生成预览")?;
            access.with_store(app,|s|{
                let library=s.object_library(&access.plugin_id);
                if library.revision!=input.expected_revision||library.objects.get(&input.id).map(|v|&v.item)!=Some(&item){return Err("对象已变化".into());}
                Ok(())
            })?;
            use base64::Engine as _;
            Ok(json!({"image":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()))}))
        },
        "objects.library.get"=>access.with_store(app,|s|Ok(s.object_library(&access.plugin_id).catalogue())),
        "objects.library.commit"=>{
            let commit:Commit=serde_json::from_value(params).map_err(|_|"对象事务参数无效")?;
            let result=if commit.capture_key.is_some() {
                let source=access.source.take().ok_or("没有当前用户拖入的对象，或该对象已被消费")?;
                capture(app,access,source,commit)?
            } else {
                access.with_store(app,|s|s.commit_objects(&access.plugin_id,commit.expected_revision,commit.state,&commit.remove,None).map_err(|e|e.to_string()))?
            };
            let _=app.emit_to("space","extension-objects-changed",json!({"pluginId":access.plugin_id,"revision":result.revision}));
            Ok(result.catalogue())
        },
        "objects.library.restore"=>{
            #[derive(Deserialize)]
            #[serde(rename_all="camelCase",deny_unknown_fields)]
            struct Restore { expected_revision:u64,id:String,x:f64,y:f64 }
            let value:Restore=serde_json::from_value(params).map_err(|_|"对象恢复参数无效")?;
            let snapshot=access.with_store(app,|s|{
                let scene=s.active_scene().map_err(|e|e.to_string())?;
                if scene.id!=access.scene_id||scene.closed||scene.frozen{return Err("请回到原截图空间取出对象".into());}
                s.restore_object(&access.plugin_id,&value.id,value.expected_revision,&access.scene_id,value.x,value.y).map_err(|e|e.to_string())
            })?;
            crate::publish_owned(app,snapshot);
            Ok(json!({"restored":true}))
        },
        _=>Err("未知对象操作".into())
    }
}
fn drawing_image(
    width: u32,
    height: u32,
    drawings: Vec<mewu_core::Drawing>,
    root: &std::path::Path,
) -> Result<image::DynamicImage, String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 33_554_432 {
        return Err("绘制对象尺寸无效".into());
    }
    let source = image::DynamicImage::ImageRgba8(image::RgbaImage::new(width, height));
    let region = mewu_core::Region {
        x: 0.,
        y: 0.,
        width: width.into(),
        height: height.into(),
        drawings,
        ..Default::default()
    };
    let image =
        crate::drawing_render::crop_region_with_layouts(&source, &region, None, |reference| {
            crate::rich_annotation_host::load_raster(root, reference)
        })?
        .into_rgba8();
    let mut bounds = (width, height, 0, 0);
    let mut found = false;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] != 0 {
            found = true;
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        }
    }
    if !found {
        return Err("此对象没有可收藏的可见内容".into());
    }
    Ok(image::DynamicImage::ImageRgba8(
        image::imageops::crop_imm(
            &image,
            bounds.0,
            bounds.1,
            bounds.2 - bounds.0 + 1,
            bounds.3 - bounds.1 + 1,
        )
        .to_image(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_drawing_preserves_transparency_and_crops_visible_ink() {
        let drawing = mewu_core::Drawing {
            id: uuid::Uuid::new_v4().to_string(),
            kind: mewu_core::DrawingKind::Rect,
            color: "#ff0000".into(),
            stroke_width: 2.,
            points: vec![
                mewu_core::DrawingPoint { x: 20., y: 20. },
                mewu_core::DrawingPoint { x: 70., y: 70. },
            ],
            text: None, font_size: None, text_style: Default::default(),
            origin: None, rich: None,
        };
        let image = drawing_image(100,100,vec![drawing],std::path::Path::new("unused-for-vector")).unwrap().into_rgba8();
        assert!(image.width()<60 && image.height()<60);
        assert!(image.pixels().any(|pixel|pixel[3]==0));
        assert!(image.pixels().any(|pixel|pixel[0]>200 && pixel[1]<50 && pixel[3]>200));
    }
    #[test]
    fn capture_rejects_empty_oversized_and_path_supplied_sources() {
        assert!(drawing_image(100,100,vec![],std::path::Path::new("unused")).is_err());
        assert!(drawing_image(32768,32768,vec![],std::path::Path::new("unused")).is_err());
        assert!(serde_json::from_value::<Source>(json!({"kind":"item","sceneId":"a","itemId":"b","path":"file"})).is_err());
    }
}
pub(crate) fn template(asset: Asset) -> SpaceItem {
    SpaceItem {
        id: uuid::Uuid::new_v4().to_string(),
        asset,
        x: 0.3,
        y: 0.2,
        width: 0.36,
        height: 0.5,
        state: None,
        video_edit: None,
        video_annotations: None,
        image_annotations: None,
    }
}

fn capture(app:&AppHandle, access:&Access, source:Source, commit:Commit)->Result<ObjectLibrary,String> {
    if let Source::Pin{id}=source {
        return tauri::async_runtime::block_on(crate::pin_host::capture_extension_pin(app.clone(),id,access,commit));
    }
    if let Source::Item{scene_id,item_id}=source {
        if scene_id!=access.scene_id {return Err("对象不属于当前会话".into());}
        return access.with_store(app,|s|{
            let template=s.capture_object(&scene_id,&item_id).map_err(|e|e.to_string())?;
            s.commit_objects(&access.plugin_id,commit.expected_revision,commit.state,&commit.remove,Some((commit.capture_key.ok_or("缺少对象标识")?,template))).map_err(|e|e.to_string())
        });
    }
    let original=access.with_store(app,|s|s.active_scene().cloned().map_err(|e|e.to_string()))?;
    let host=app.state::<Host>();
    let image=match source {
        Source::Region{scene_id,region_id}=>{
            if scene_id!=original.id {return Err("选区不属于当前会话".into());}
            crate::assets::crop(&original,&region_id,&host.assets)?
        },
        Source::Drawings{scene_id,region_id,item_id,drawing_ids}=>{
            if scene_id!=original.id || drawing_ids.is_empty() || drawing_ids.len()>256 || region_id.is_some()==item_id.is_some() {return Err("绘制对象来源无效".into());}
            let (asset,drawings)=if let Some(id)=region_id {
                let region=original.regions.iter().find(|r|r.id==id).ok_or("选区已移除")?;
                (region.image_override.as_ref().or(original.background.as_ref()).ok_or("图片已移除")?,&region.drawings)
            } else {
                let item=original.items.iter().find(|i|Some(&i.id)==item_id.as_ref()).ok_or("图片已移除")?;
                (&item.asset,&item.image_annotations.as_ref().ok_or("标注已移除")?.drawings)
            };
            let selected:Vec<_>=drawings.iter().filter(|d|drawing_ids.contains(&d.id)).cloned().collect();
            if selected.len()!=drawing_ids.len(){return Err("绘制对象已变化".into());}
            drawing_image(asset.width.ok_or("图片宽度无效")?,asset.height.ok_or("图片高度无效")?,selected,&host.assets)?
        },
        _=>unreachable!()
    };
    let id=uuid::Uuid::new_v4().to_string(); let path=host.assets.join(format!("{id}.png"));
    let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|_|"无法保存收藏")?;
    let result=(||{
        let mut encoded=std::io::Cursor::new(Vec::new());
        image.write_to(&mut encoded,image::ImageFormat::Png).map_err(|_|"无法生成收藏图片")?;
        file.write_all(encoded.get_ref()).and_then(|_|file.sync_all()).map_err(|_|"无法保存收藏")?;
        let asset=Asset{id,name:"收藏.png".into(),kind:AssetKind::Image,path:path.to_string_lossy().into_owned(),width:Some(image.width()),height:Some(image.height()),origin_x:None,origin_y:None,scale_factor:None,capture_coordinates:None};
        access.with_store(app,|s|{
            if s.active_scene().map_err(|e|e.to_string())?!=&original {return Err("对象已变化，请重新收藏".into());}
            s.commit_objects(&access.plugin_id,commit.expected_revision,commit.state,&commit.remove,
                Some((commit.capture_key.ok_or("缺少对象标识")?,ObjectTemplate{item:template(asset),board:None}))).map_err(|e|e.to_string())
        })
    })();
    drop(file); if result.is_err(){let _=std::fs::remove_file(&path);} result
}
