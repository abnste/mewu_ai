// SPDX-License-Identifier: MPL-2.0
//! Actual offline Store + compositor + image preparation. No model or desktop.
use super::*;
use mewu_core::*;
use sha2::{Digest, Sha256};
struct Fixture(PathBuf);
impl Drop for Fixture { fn drop(&mut self) {
    let (Ok(root),Ok(parent))=(self.0.canonicalize(),std::env::temp_dir().canonicalize())else{return};
    if root.parent()==Some(parent.as_path())&&root.file_name().and_then(|name|name.to_str()).and_then(|name|name.strip_prefix("mewu-image-ink-")).is_some_and(|id|uuid::Uuid::parse_str(id).is_ok()){let _=std::fs::remove_dir_all(root);}
} }
fn uid()->String { uuid::Uuid::new_v4().to_string() }
#[test]
fn imported_image_ink_reference_pin_compositor_share_pixels_and_preserve_original_bytes() {
    let root=Fixture(std::env::temp_dir().join(format!("mewu-image-ink-{}",uid())));let assets=root.0.join("assets");std::fs::create_dir_all(&assets).unwrap();
    let mut store=Store::open(&root.0.join("spaces.db")).unwrap();let scene_id=store.snapshot().active_scene_id;
    let asset_id=uid();let source=assets.join(format!("{asset_id}.png"));let original=image::RgbaImage::from_pixel(640,400,image::Rgba([255,255,255,255]));original.save(&source).unwrap();let original_hash=Sha256::digest(std::fs::read(&source).unwrap());
    let asset=Asset { id:asset_id.clone(),name:"synthetic-exam.png".into(),kind:AssetKind::Image,path:source.to_string_lossy().into_owned(),width:Some(640),height:Some(400),origin_x:None,origin_y:None,scale_factor:None,capture_coordinates:None };
    store.add_asset(&scene_id,asset.clone()).unwrap();let item=store.scene(&scene_id).unwrap().items[0].clone();
    let line=Drawing { id:uid(),kind:DrawingKind::Line,color:"#ff0000".into(),stroke_width:8.,points:vec![DrawingPoint{x:80.,y:200.},DrawingPoint{x:560.,y:200.}],text:None,font_size:None, text_style: None,origin:None,rich:None , text_runs: None, };
    store.apply(SceneCommand::EditImageAnnotations { scene_id:scene_id.clone(),item_id:item.id.clone(),source_asset_id:asset_id,expected_revision:0,action:ImageAnnotationEdit::AddDrawing { drawing:line } }).unwrap();
    store.apply(SceneCommand::SetRefs { scene_id:scene_id.clone(),refs:vec![Reference { kind:ReferenceKind::Item,id:item.id.clone() }] }).unwrap();
    store.apply(SceneCommand::SetDraft { scene_id:scene_id.clone(),draft:"在图片空白中作答".into() }).unwrap();
    let scene=store.scene(&scene_id).unwrap();let current=&scene.items[0];assert_eq!(current.asset,asset);assert!(scene.background.is_none()&&scene.regions.is_empty());
    let composite=image_item(current,&assets).unwrap().into_rgba8();assert_eq!(*composite.get_pixel(320,200),image::Rgba([255,0,0,255]));assert_eq!(*composite.get_pixel(320,100),image::Rgba([255,255,255,255]));
    let pin_pixels=compose_image_item(current,original.clone(),&assets).unwrap();assert_eq!(composite,pin_pixels);
    let prepared=crate::ai::prepare_scene_visual_attachments(scene,&assets,crate::visual_annotation_input::VisualCoordinateProfile::ExplicitNormalized1000V2).unwrap();
    let target=&prepared.visual_input().unwrap().manifest().targets[0];assert_eq!(target.image_item_id.as_deref(),Some(item.id.as_str()));assert!(target.region_id.is_empty());assert_eq!(target.source,image_annotation_source_fence(current).unwrap());
    assert_eq!(target.source.source_size,VisualPixelSize {width:640,height:400});assert_eq!(target.crop,VisualPixelRect {x:0,y:0,width:640,height:400});
    assert_eq!(original_hash,Sha256::digest(std::fs::read(&source).unwrap()));
    let mut changed=current.clone();changed.asset.width=Some(639);assert!(compose_image_item(&changed,original,&assets).is_err());
}
