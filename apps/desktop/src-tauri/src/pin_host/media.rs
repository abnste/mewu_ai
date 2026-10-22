// SPDX-License-Identifier: MPL-2.0
//! Explicit immutable scene-item pins; video keeps a real MP4 and timed ink.
use super::*;
use mewu_core::{SpaceItem, VideoPixelRect, VideoRange};
use std::io::Read;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinVideoAnnotation {
    pub id: String,
    pub interval: VideoRange,
    pub bounds: VideoPixelRect,
    pub image_url: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinVideo {
    pub kind: &'static str,
    pub canvas_presentation: bool,
    pub source_id: String,
    pub source_duration_ticks: Option<u64>,
    pub range: Option<VideoRange>,
    pub annotation_revision: u64,
    pub annotations: Vec<PinVideoAnnotation>,
}

#[tauri::command]
pub async fn pin_scene_item(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    item_id: String,
    expected_asset_id: String,
) -> Result<Option<PinCreated>, String> {
    space_owner(&window)?;
    if !valid_uuid(&scene_id) || !valid_uuid(&item_id) || !valid_uuid(&expected_asset_id) {
        return Err("素材身份无效".into());
    }
    gate(&app)?;
    let origin = {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        ImportedImageOrigin::capture_item(
            &engine.store.snapshot(),
            &scene_id,
            &item_id,
            &expected_asset_id,
        )
        .ok_or("素材或会话已更改")?
    };
    scene_item_result(pin_imported_image(app, origin).await)
}

fn scene_item_result(result: Result<String, String>) -> Result<Option<PinCreated>, String> {
    match result {
        Ok(id) => Ok(Some(PinCreated { id })),
        Err(error) if error == CANCELED => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn dimensions(asset: &Asset) -> Result<(u32, u32), String> {
    if !matches!(asset.kind, AssetKind::Image | AssetKind::Video) {
        return Err("此素材不支持置顶".into());
    }
    let (width, height) = (
        asset.width.ok_or("素材宽度无效")?,
        asset.height.ok_or("素材高度无效")?,
    );
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > u64::from(MAX_PIXELS)
    {
        return Err("素材超过贴图像素限制".into());
    }
    Ok((width, height))
}

fn copy_video(
    asset: &Asset,
    assets_root: &Path,
    destination: &Path,
    cancel: &Cancel,
) -> Result<(), String> {
    let path = assets::verified_path(asset, assets_root)?;
    // Validate the app-owned MP4, then pin its exact identity through streaming.
    let (_, length) = assets::open_video(asset, assets_root)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut input = options.open(&path).map_err(|_| "无法读取视频来源")?;
    let identity = same_file::Handle::from_file(input.try_clone().map_err(|_| "无法检查视频来源")?)
        .map_err(|_| "无法检查视频来源")?;
    let before = input.metadata().map_err(|_| "无法检查视频来源")?;
    if before.len() != length || !before.is_file() {
        return Err("视频来源已更改".into());
    }
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|_| "无法创建置顶视频")?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut remaining = length;
    while remaining > 0 {
        cancel.check()?;
        let wanted = buffer.len().min(remaining as usize);
        let count = input
            .read(&mut buffer[..wanted])
            .map_err(|_| "无法读取视频来源")?;
        if count == 0 {
            return Err("视频来源不完整".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|_| "无法保存置顶视频")?;
        remaining -= count as u64;
    }
    let after = input.metadata().map_err(|_| "无法检查视频来源")?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || identity != same_file::Handle::from_path(&path).map_err(|_| "视频来源已更改")?
    {
        return Err("视频来源已更改".into());
    }
    output
        .flush()
        .and_then(|_| output.sync_all())
        .map_err(|_| "无法保存完整置顶视频")?;
    cancel.check()
}

pub(super) fn stage_video(
    root: &Path,
    assets_root: &Path,
    item: &SpaceItem,
    pixels: OwnedSemaphorePermit,
    cancel: &Cancel,
    pixel_budget: Arc<Semaphore>,
    content_origin: &str,
) -> Result<PinAssetLease, String> {
    dimensions(&item.asset)?;
    if item.asset.kind != AssetKind::Video {
        return Err("视频来源无效".into());
    }
    let root_lock = directory(root)?;
    let root = root.canonicalize().map_err(|_| "置顶目录不可用")?;
    let staging = root.join("pin-staging");
    match std::fs::create_dir(&staging) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("无法创建置顶目录".into()),
    };
    let staging_lock = directory(&staging)?;
    let staging = staging.canonicalize().map_err(|_| "置顶目录不可用")?;
    if staging.parent() != Some(root.as_path()) {
        return Err("置顶目录超出范围".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let path = staging.join(format!("{id}.mp4"));
    let mut guards = vec![Temp(Some(path.clone()))];
    copy_video(&item.asset, assets_root, &path, cancel)?;
    let mut asset = item.asset.clone();
    asset.id = id;
    asset.path = path.to_string_lossy().into_owned();
    let mut media = PinVideo {
        kind: "video",
        canvas_presentation: cfg!(target_os = "macos"),
        source_id: item.asset.id.clone(),
        source_duration_ticks: item
            .video_edit
            .as_ref()
            .map(|edit| edit.source_duration_ticks)
            .or_else(|| {
                item.video_annotations
                    .as_ref()
                    .map(|doc| doc.source_duration_ticks)
            }),
        range: item.video_edit.as_ref().and_then(|edit| edit.range),
        annotation_revision: item
            .video_annotations
            .as_ref()
            .map_or(0, |doc| doc.revision),
        annotations: Vec::new(),
    };
    let mut extra_assets = Vec::new();
    let mut extra_pixels = Vec::new();
    let mut total_bytes = 0usize;
    if let Some(doc) = &item.video_annotations {
        if doc.source_id != item.asset.id
            || Some(doc.source_width) != item.asset.width
            || Some(doc.source_height) != item.asset.height
        {
            return Err("视频批注来源已更改".into());
        }
        for object in &doc.objects {
            cancel.check()?;
            let raster = crate::video_annotation_raster::render_in_source(
                object,
                doc.source_width,
                doc.source_height,
                |reference| {
                    mewu_core::Store::read_video_text_layout(root.join("spaces.db"), reference)
                        .map_err(|error| error.to_string())
                },
                |reference| {
                    mewu_core::Store::read_video_vector_layout(root.join("spaces.db"), reference)
                        .map_err(|error| error.to_string())
                },
            )?;
            total_bytes = total_bytes
                .checked_add(raster.png.len())
                .ok_or("视频批注过大")?;
            if total_bytes > crate::video_annotation_raster::MAX_TOTAL_PNG_BYTES {
                return Err("视频批注超过置顶预算".into());
            }
            let count = raster
                .reference
                .width
                .checked_mul(raster.reference.height)
                .ok_or("视频批注过大")?;
            extra_pixels.push(
                pixel_budget
                    .clone()
                    .try_acquire_many_owned(count)
                    .map_err(|_| "置顶图层总像素已达上限")?,
            );
            let image_id = uuid::Uuid::new_v4().to_string();
            let image_path = staging.join(format!("{image_id}.png"));
            guards.push(Temp(Some(image_path.clone())));
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&image_path)
                .map_err(|_| "无法保存视频批注")?;
            output
                .write_all(&raster.png)
                .and_then(|_| output.sync_all())
                .map_err(|_| "无法保存完整视频批注")?;
            extra_assets.push(Asset {
                capture_coordinates: None,
                id: image_id.clone(),
                name: "视频批注.png".into(),
                kind: AssetKind::Image,
                path: image_path.to_string_lossy().into_owned(),
                width: Some(raster.reference.width),
                height: Some(raster.reference.height),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
            });
            media.annotations.push(PinVideoAnnotation {
                id: object.id.clone(),
                interval: object.interval,
                bounds: raster.bounds,
                image_url: format!("{content_origin}/{image_id}"),
            });
        }
    }
    cancel.check()?;
    assets::open_video(&asset, &staging)?;
    let source = Source {
        asset,
        saved_item: Some(item.clone()),
        media: Some(media),
        extra_assets,
        _extra_pixels: extra_pixels,
        root: staging,
        _directory: staging_lock,
        _root_directory: root_lock,
        _pixels: pixels,
    };
    for guard in &mut guards {
        guard.0 = None;
    }
    Ok(PinAssetLease(Arc::new(source), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_media_pin_cancellation_is_nullable_but_other_failures_stay_errors() {
        assert!(scene_item_result(Err(CANCELED.into())).unwrap().is_none());
        assert_eq!(
            scene_item_result(Ok("synthetic-pin".into()))
                .unwrap()
                .unwrap()
                .id,
            "synthetic-pin"
        );
        for message in ["无法创建置顶视频", "应用正在退出", "已取消贴图：磁盘错误"]
        {
            assert_eq!(
                scene_item_result(Err(message.into())).err().as_deref(),
                Some(message)
            );
        }
    }
    use mewu_core::{
        TimedVideoAnnotation, VideoAnnotationDocument, VideoAnnotationPrimitive, VideoEdit,
        VideoRectStyle, VideoSourceClock,
    };
    struct Fixture {
        root: PathBuf,
        item: SpaceItem,
        bytes: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("mewu-media-pin-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let assets = root.join("assets");
            std::fs::create_dir(&assets).unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let path = assets.join(format!("{id}.mp4"));
            // Container fixture only: this is not a codec/playback acceptance.
            let mut bytes = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom\0\0\0\x08mdat".to_vec();
            bytes.resize(2 * 1024 * 1024 + 31, 0x53);
            std::fs::write(&path, &bytes).unwrap();
            let item = SpaceItem {
                id: uuid::Uuid::new_v4().to_string(),
                asset: Asset {
                    capture_coordinates: None,
                    id: id.clone(),
                    name: "fixture.mp4".into(),
                    kind: AssetKind::Video,
                    path: path.to_string_lossy().into_owned(),
                    width: Some(160),
                    height: Some(90),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                },
                x: 0.2,
                y: 0.3,
                width: 0.4,
                height: 0.3,
                state: None,
                image_annotations: None,
                video_edit: Some(VideoEdit {
                    source_id: id.clone(),
                    source_duration_ticks: 50_000_000,
                    revision: 2,
                    range: Some(VideoRange {
                        start_ticks: 10_000_000,
                        end_ticks: 40_000_000,
                    }),
                    undo: vec![],
                    redo: vec![],
                }),
                video_annotations: Some(VideoAnnotationDocument {
                    version: 1,
                    clock: VideoSourceClock::SourcePlaybackTicks,
                    source_id: id,
                    source_duration_ticks: 50_000_000,
                    source_width: 160,
                    source_height: 90,
                    revision: 3,
                    objects: vec![TimedVideoAnnotation {
                        id: uuid::Uuid::new_v4().to_string(),
                        interval: VideoRange {
                            start_ticks: 20_000_000,
                            end_ticks: 30_000_000,
                        },
                        primitive: VideoAnnotationPrimitive::Rect {
                            bounds: VideoPixelRect {
                                x: 10.,
                                y: 20.,
                                width: 40.,
                                height: 30.,
                            },
                            style: VideoRectStyle {
                                color: "#FF0000".into(),
                                stroke_width: 2.,
                                opacity: 1.,
                            },
                        },
                        origin: None,
                    }],
                    undo: vec![],
                    redo: vec![],
                }),
            };
            Self { root, item, bytes }
        }
        fn stage(&self, budget: Arc<Semaphore>, cancel: &Cancel) -> Result<PinAssetLease, String> {
            let pixels = budget.clone().try_acquire_many_owned(160 * 90).unwrap();
            stage_video(
                &self.root,
                &self.root.join("assets"),
                &self.item,
                pixels,
                cancel,
                budget,
                "http://127.0.0.1:49123",
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.item.asset.path);
            let _ = std::fs::remove_dir(self.root.join("assets"));
            let _ = std::fs::remove_dir(self.root.join("pin-staging"));
            let _ = std::fs::remove_dir(&self.root);
        }
    }
    #[test]
    fn media_pin_stream_copy_and_timed_ink_are_independent_and_last_reader_owns_files() {
        let fixture = Fixture::new();
        let budget = Arc::new(Semaphore::new(100_000));
        let source = fixture.stage(budget.clone(), &Cancel::default()).unwrap();
        let staged = source.asset().clone();
        let image = source.0.extra_assets[0].clone();
        let media = source.0.media.as_ref().unwrap();
        assert_ne!(staged.id, fixture.item.asset.id);
        assert_eq!(std::fs::read(&staged.path).unwrap(), fixture.bytes);
        assert_eq!(
            std::fs::read(&fixture.item.asset.path).unwrap(),
            fixture.bytes
        );
        assert_eq!(media.source_id, fixture.item.asset.id);
        assert_eq!(media.range, fixture.item.video_edit.as_ref().unwrap().range);
        assert_eq!(
            media.annotations[0].interval,
            fixture.item.video_annotations.as_ref().unwrap().objects[0].interval
        );
        assert_eq!(media.annotation_revision, 3);
        assert!(image::open(&image.path).unwrap().width() > 0);
        assert!(media.annotations[0].image_url.ends_with(&image.id));
        let reader = PinAssetLease(source.0.clone(), Some(0));
        drop(source);
        assert!(Path::new(&staged.path).exists());
        assert!(Path::new(&image.path).exists());
        assert!(budget.available_permits() < 100_000);
        drop(reader);
        assert!(!Path::new(&staged.path).exists());
        assert!(!Path::new(&image.path).exists());
        assert!(Path::new(&fixture.item.asset.path).exists());
        assert_eq!(budget.available_permits(), 100_000);
    }
    #[test]
    fn media_pin_cancellation_and_raster_budget_failure_clean_only_staged_files() {
        for canceled in [true, false] {
            let fixture = Fixture::new();
            let budget = Arc::new(Semaphore::new(if canceled { 100_000 } else { 160 * 90 }));
            let cancel = Cancel::default();
            if canceled {
                cancel.cancel();
            }
            assert!(fixture.stage(budget.clone(), &cancel).is_err());
            assert_eq!(
                std::fs::read(&fixture.item.asset.path).unwrap(),
                fixture.bytes
            );
            assert_eq!(
                std::fs::read_dir(fixture.root.join("pin-staging"))
                    .unwrap()
                    .count(),
                0
            );
            assert_eq!(
                budget.available_permits(),
                if canceled { 100_000 } else { 160 * 90 }
            );
        }
    }
}
