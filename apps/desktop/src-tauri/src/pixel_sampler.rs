// SPDX-License-Identifier: MPL-2.0
//! A bounded, read-only view of the active clean capture. Artifact-origin CORS
//! stays unchanged; only the first-party space can request a 25-pixel sample.
use crate::{assets, recording, Host};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{DynamicImage, Rgba, RgbaImage};
use mewu_core::{Asset, AssetKind, Snapshot};
use serde::Serialize;
use std::{
    io::Cursor,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;

const SAMPLE_SIZE: u32 = 25;
const RADIUS: i64 = 12;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SAMPLE_BYTES: usize = 16 * 1024;
const IDLE_TTL: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointerSample {
    scene_id: String,
    background_id: String,
    x: u32,
    y: u32,
    global_x: i64,
    global_y: i64,
    hex: String,
    data_url: String,
}

#[derive(Clone, Debug, PartialEq)]
struct SourceKey {
    scene_id: String,
    asset: Asset,
}

struct CachedSource {
    image: RgbaImage,
    last_used: Instant,
}

#[derive(Default)]
struct CacheState {
    generation: u64,
    // Keep the target while decoding as well as when pixels are resident.
    // Reconcile must invalidate a pending source, not just a completed image.
    target: Option<SourceKey>,
    entry: Option<CachedSource>,
}

impl CacheState {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.target = None;
        self.entry = None;
    }

    fn begin(&mut self, key: &SourceKey) -> u64 {
        if self.target.as_ref() != Some(key) {
            self.invalidate();
            self.target = Some(key.clone());
        }
        self.generation
    }

    fn matches(&self, key: &SourceKey, generation: u64) -> bool {
        self.generation == generation && self.target.as_ref() == Some(key)
    }

    fn expire(&mut self, now: Instant) {
        if self
            .entry
            .as_ref()
            .is_some_and(|entry| now.saturating_duration_since(entry.last_used) >= IDLE_TTL)
        {
            self.invalidate();
        }
    }

    fn reconcile(&mut self, snapshot: &Snapshot) {
        let Some(key) = self.target.as_ref() else {
            return;
        };
        let current = snapshot.scenes.iter().find(|scene| {
            scene.id == snapshot.active_scene_id
                && scene.id == key.scene_id
                && !scene.closed
                && !scene.frozen
        });
        if current.and_then(|scene| scene.background.as_ref()) != Some(&key.asset) {
            self.invalidate();
        }
    }
}

struct Inner {
    cache: Mutex<CacheState>,
    requests: Arc<Semaphore>,
    decoder: Arc<Semaphore>,
    watchdog_started: AtomicBool,
}

pub struct PixelSampler {
    inner: Arc<Inner>,
}

impl Default for PixelSampler {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                cache: Mutex::new(CacheState::default()),
                requests: Arc::new(Semaphore::new(2)),
                decoder: Arc::new(Semaphore::new(1)),
                watchdog_started: AtomicBool::new(false),
            }),
        }
    }
}

impl PixelSampler {
    pub fn invalidate(&self) {
        if let Ok(mut cache) = self.inner.cache.lock() {
            cache.invalidate();
        }
    }

    fn start_watchdog(&self) {
        if self.inner.watchdog_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        // One task per app state, not per mouse event. It retains neither the
        // AppHandle nor the screenshot while sleeping, and also clears an idle
        // cache when no subsequent request arrives.
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                if let Ok(mut cache) = inner.cache.lock() {
                    cache.expire(Instant::now());
                };
            }
        });
    }
}

pub fn invalidate(app: &AppHandle) {
    if let Some(sampler) = app.try_state::<PixelSampler>() {
        sampler.invalidate();
    }
}

pub fn reconcile(app: &AppHandle, snapshot: &Snapshot) {
    if let Some(sampler) = app.try_state::<PixelSampler>() {
        if let Ok(mut cache) = sampler.inner.cache.lock() {
            cache.reconcile(snapshot);
        };
    }
}

impl Inner {
    fn fail(&self, key: &SourceKey, generation: u64) {
        if let Ok(mut cache) = self.cache.lock() {
            if cache.matches(key, generation) {
                cache.invalidate();
            }
        }
    }

    fn sample_or_load(
        &self,
        key: &SourceKey,
        generation: u64,
        x: u32,
        y: u32,
        load: impl FnOnce() -> Result<RgbaImage, String>,
    ) -> Result<RgbaImage, String> {
        {
            let mut cache = self.cache.lock().map_err(|_| "取色缓存不可用")?;
            if !cache.matches(key, generation) {
                return Err("截图已变更".into());
            }
            if let Some(entry) = cache.entry.as_mut() {
                let result = sample_pixels(&entry.image, x, y)?;
                entry.last_used = Instant::now();
                return Ok(result);
            }
        }
        // There is one decoder permit, owned by the caller's blocking worker.
        // No cache or Engine lock is held across disk access/full-image decode.
        let image = load()?;
        validate_source(&key.asset, image.width(), image.height(), x, y)?;
        let mut cache = self.cache.lock().map_err(|_| "取色缓存不可用")?;
        if !cache.matches(key, generation) {
            return Err("截图已变更".into());
        }
        let result = sample_pixels(&image, x, y)?;
        cache.entry = Some(CachedSource {
            image,
            last_used: Instant::now(),
        });
        Ok(result)
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("截图尺寸超出取色范围".into());
    }
    Ok(())
}

fn validate_source(asset: &Asset, width: u32, height: u32, x: u32, y: u32) -> Result<(), String> {
    validate_dimensions(width, height)?;
    if asset.kind != AssetKind::Image
        || asset.width != Some(width)
        || asset.height != Some(height)
        || asset.origin_x.is_none()
        || asset.origin_y.is_none()
    {
        return Err("截图像素信息不一致".into());
    }
    if x >= width || y >= height {
        return Err("取色位置超出截图".into());
    }
    Ok(())
}

fn decode_source(asset: &Asset, root: &Path, x: u32, y: u32) -> Result<RgbaImage, String> {
    let path = assets::verified_path(asset, root)?;
    let metadata = std::fs::metadata(&path).map_err(|_| "无法读取截图")?;
    if !metadata.is_file() || !(12..=MAX_SOURCE_BYTES).contains(&metadata.len()) {
        return Err("截图文件为空或过大".into());
    }
    let (width, height) = image::image_dimensions(&path).map_err(|_| "无法读取截图尺寸")?;
    validate_source(asset, width, height, x, y)?;
    // Reuse the existing canonical-path/decoder limits. Move RGBA captures
    // directly into the cache; other bounded formats can require a temporary
    // conversion allocation. 128 MiB is the cache cap, not a process-memory cap.
    let image = assets::image(asset, root)?;
    validate_source(asset, image.width(), image.height(), x, y)?;
    Ok(image.into_rgba8())
}

fn sample_pixels(source: &RgbaImage, x: u32, y: u32) -> Result<RgbaImage, String> {
    if x >= source.width() || y >= source.height() {
        return Err("取色位置超出截图".into());
    }
    let mut sample = RgbaImage::new(SAMPLE_SIZE, SAMPLE_SIZE);
    for row in 0..SAMPLE_SIZE {
        for column in 0..SAMPLE_SIZE {
            let source_x = i64::from(x) + i64::from(column) - RADIUS;
            let source_y = i64::from(y) + i64::from(row) - RADIUS;
            if source_x < 0
                || source_y < 0
                || source_x >= i64::from(source.width())
                || source_y >= i64::from(source.height())
            {
                continue;
            }
            let pixel = source.get_pixel(source_x as u32, source_y as u32);
            let alpha = u32::from(pixel[3]);
            let white = 255 * (255 - alpha) + 127;
            // Captures are opaque. If a historical source contains alpha,
            // flatten actual source pixels onto white for both HEX and preview.
            // Missing neighbors remain transparent and never move the center.
            sample.put_pixel(
                column,
                row,
                Rgba([
                    ((u32::from(pixel[0]) * alpha + white) / 255) as u8,
                    ((u32::from(pixel[1]) * alpha + white) / 255) as u8,
                    ((u32::from(pixel[2]) * alpha + white) / 255) as u8,
                    255,
                ]),
            );
        }
    }
    Ok(sample)
}

fn encode_sample(
    key: &SourceKey,
    x: u32,
    y: u32,
    sample: RgbaImage,
) -> Result<PointerSample, String> {
    let center = *sample.get_pixel(RADIUS as u32, RADIUS as u32);
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(sample)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "无法生成取色预览")?;
    let bytes = png.into_inner();
    if bytes.len() > MAX_SAMPLE_BYTES {
        return Err("取色预览过大".into());
    }
    Ok(PointerSample {
        scene_id: key.scene_id.clone(),
        background_id: key.asset.id.clone(),
        x,
        y,
        global_x: i64::from(key.asset.origin_x.ok_or("截图缺少坐标")?) + i64::from(x),
        global_y: i64::from(key.asset.origin_y.ok_or("截图缺少坐标")?) + i64::from(y),
        hex: format!("#{:02X}{:02X}{:02X}", center[0], center[1], center[2]),
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
    })
}

fn ensure_window(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "space" {
        return Err("此窗口不能读取截图像素".into());
    }
    if !window.is_visible().map_err(|_| "空间窗口不可用")?
        || !window.is_focused().map_err(|_| "空间窗口不可用")?
    {
        return Err("截图空间未在前台".into());
    }
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("截图空间正在切换".into());
    }
    recording::ensure_idle(app)
}

#[tauri::command]
pub async fn get_pointer_sample(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    background_id: String,
    x: u32,
    y: u32,
) -> Result<PointerSample, String> {
    ensure_window(&app, &window)?;
    let sampler = app.state::<PixelSampler>();
    let request = sampler
        .inner
        .requests
        .clone()
        .try_acquire_owned()
        .map_err(|_| "取色预览繁忙")?;
    let decoder = sampler
        .inner
        .decoder
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "取色预览不可用")?;
    ensure_window(&app, &window)?;
    let host = app.state::<Host>();
    let (key, generation) = {
        let engine = host.lock()?;
        host.exit.ensure_running()?;
        recording::ensure_idle(&app)?;
        let asset = engine
            .store
            .active_background_for_preview(&scene_id, &background_id)
            .map_err(|error| error.to_string())?;
        validate_source(
            &asset,
            asset.width.ok_or("截图缺少尺寸")?,
            asset.height.ok_or("截图缺少尺寸")?,
            x,
            y,
        )?;
        let key = SourceKey { scene_id, asset };
        // The only nested lock order is Engine -> sampler. Workers and idle
        // cleanup never acquire Engine while retaining this cache mutex.
        let generation = sampler
            .inner
            .cache
            .lock()
            .map_err(|_| "取色缓存不可用")?
            .begin(&key);
        (key, generation)
    };
    sampler.start_watchdog();
    let root = host.assets.clone();
    let inner = sampler.inner.clone();
    let worker_key = key.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (_request, _decoder) = (request, decoder);
        let result = inner
            .sample_or_load(&worker_key, generation, x, y, || {
                decode_source(&worker_key.asset, &root, x, y)
            })
            .and_then(|sample| encode_sample(&worker_key, x, y, sample));
        if result.is_err() {
            inner.fail(&worker_key, generation);
        }
        result
    })
    .await
    .map_err(|_| "取色预览未完成".to_owned())
    .and_then(|result| result);
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            sampler.inner.fail(&key, generation);
            return Err(error);
        }
    };
    let current = (|| {
        ensure_window(&app, &window)?;
        let engine = host.lock()?;
        host.exit.ensure_running()?;
        let asset = engine
            .store
            .active_background_for_preview(&key.scene_id, &key.asset.id)
            .map_err(|error| error.to_string())?;
        if asset != key.asset
            || !sampler
                .inner
                .cache
                .lock()
                .map_err(|_| "取色缓存不可用")?
                .matches(&key, generation)
        {
            return Err("截图已变更".into());
        }
        Ok(())
    })();
    if let Err(error) = current {
        sampler.inner.fail(&key, generation);
        return Err(error);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Barrier};

    fn key(width: u32, height: u32) -> SourceKey {
        SourceKey {
            scene_id: "scene-a".into(),
            asset: Asset {
                id: "capture-a".into(),
                name: "capture.png".into(),
                kind: AssetKind::Image,
                path: "unused/capture.png".into(),
                width: Some(width),
                height: Some(height),
                origin_x: Some(-1920),
                origin_y: Some(-1080),
                scale_factor: Some(1.75),
            },
        }
    }

    #[test]
    fn corners_keep_center_and_missing_neighbors_transparent() {
        let source = RgbaImage::from_fn(30, 20, |x, y| Rgba([x as u8, y as u8, 15, 255]));
        for (x, y) in [(0, 0), (29, 0), (0, 19), (29, 19), (15, 10)] {
            let sample = sample_pixels(&source, x, y).unwrap();
            assert_eq!(sample.dimensions(), (25, 25));
            assert_eq!(sample.get_pixel(12, 12), source.get_pixel(x, y));
            for row in 0..25 {
                for column in 0..25 {
                    let sx = i64::from(x) + i64::from(column) - 12;
                    let sy = i64::from(y) + i64::from(row) - 12;
                    if !(0..30).contains(&sx) || !(0..20).contains(&sy) {
                        assert_eq!(*sample.get_pixel(column, row), Rgba([0, 0, 0, 0]));
                    } else {
                        assert_eq!(
                            sample.get_pixel(column, row),
                            source.get_pixel(sx as u32, sy as u32)
                        );
                    }
                }
            }
        }
        assert!(sample_pixels(&source, 30, 0).is_err());
    }

    #[test]
    fn one_pixel_alpha_hex_png_and_negative_global_coordinates_agree() {
        let source = RgbaImage::from_pixel(1, 1, Rgba([0, 128, 255, 128]));
        let sample = sample_pixels(&source, 0, 0).unwrap();
        assert_eq!(sample.pixels().filter(|pixel| pixel[3] != 0).count(), 1);
        assert_eq!(*sample.get_pixel(12, 12), Rgba([127, 191, 255, 255]));
        let result = encode_sample(&key(1, 1), 0, 0, sample).unwrap();
        assert_eq!(result.hex, "#7FBFFF");
        assert_eq!((result.global_x, result.global_y), (-1920, -1080));
        let bytes = STANDARD
            .decode(
                result
                    .data_url
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert_eq!(*decoded.get_pixel(12, 12), Rgba([127, 191, 255, 255]));
        let mut large = key(2, 1);
        large.asset.origin_x = Some(i32::MAX);
        let result = encode_sample(
            &large,
            1,
            0,
            sample_pixels(&RgbaImage::new(2, 1), 1, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(result.global_x, i64::from(i32::MAX) + 1);
    }

    #[test]
    fn same_source_decodes_once_and_full_asset_identity_changes_generation() {
        let sampler = PixelSampler::default();
        let key = key(2, 2);
        let generation = sampler.inner.cache.lock().unwrap().begin(&key);
        let loads = AtomicUsize::new(0);
        for (x, y) in [(0, 0), (1, 0), (1, 1)] {
            sampler
                .inner
                .sample_or_load(&key, generation, x, y, || {
                    loads.fetch_add(1, Ordering::Relaxed);
                    Ok(RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255])))
                })
                .unwrap();
        }
        assert_eq!(loads.load(Ordering::Relaxed), 1);
        let mut changed = key.clone();
        changed.asset.origin_x = Some(-2560);
        let mut cache = sampler.inner.cache.lock().unwrap();
        assert_ne!(cache.begin(&changed), generation);
        assert!(cache.entry.is_none());
        assert!(!cache.matches(&key, generation));
    }

    #[test]
    fn pending_decode_invalidation_cannot_repopulate_or_release_permit_early() {
        let sampler = PixelSampler::default();
        let key = key(1, 1);
        let generation = sampler.inner.cache.lock().unwrap().begin(&key);
        let permit = sampler.inner.decoder.clone().try_acquire_owned().unwrap();
        let entered = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let worker = {
            let inner = sampler.inner.clone();
            let entered = entered.clone();
            let resume = resume.clone();
            std::thread::spawn(move || {
                let _permit = permit;
                inner.sample_or_load(&key, generation, 0, 0, || {
                    entered.wait();
                    resume.wait();
                    Ok(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])))
                })
            })
        };
        entered.wait();
        sampler.invalidate();
        assert!(sampler.inner.decoder.clone().try_acquire_owned().is_err());
        resume.wait();
        assert!(worker.join().unwrap().is_err());
        assert!(sampler.inner.cache.lock().unwrap().entry.is_none());
        assert!(sampler.inner.decoder.clone().try_acquire_owned().is_ok());
    }

    #[test]
    fn idle_expiry_releases_buffer_and_failure_cannot_erase_new_generation() {
        let sampler = PixelSampler::default();
        let key = key(1, 1);
        let old = sampler.inner.cache.lock().unwrap().begin(&key);
        sampler
            .inner
            .sample_or_load(&key, old, 0, 0, || Ok(RgbaImage::new(1, 1)))
            .unwrap();
        let mut cache = sampler.inner.cache.lock().unwrap();
        let used = cache.entry.as_ref().unwrap().last_used;
        cache.expire(used + IDLE_TTL - Duration::from_millis(1));
        assert!(cache.entry.is_some());
        cache.expire(used + IDLE_TTL);
        assert!(cache.entry.is_none());
        assert!(cache.target.is_none());
        let new = cache.begin(&key);
        drop(cache);
        sampler.inner.fail(&key, old);
        assert!(sampler.inner.cache.lock().unwrap().matches(&key, new));
        sampler.inner.fail(&key, new);
        assert!(sampler.inner.cache.lock().unwrap().target.is_none());
    }

    #[test]
    fn reconcile_preserves_draft_updates_but_revokes_pending_and_completed_sources() {
        let store = mewu_core::Store::open_in_memory().unwrap();
        let mut snapshot = store.snapshot();
        let mut key = key(1, 1);
        key.scene_id = snapshot.active_scene_id.clone();
        snapshot.scenes[0].background = Some(key.asset.clone());
        let mut cache = CacheState::default();
        let generation = cache.begin(&key);
        snapshot.scenes[0].draft = "another streamed draft".into();
        cache.reconcile(&snapshot);
        assert!(cache.matches(&key, generation));
        // No pixels have arrived: pending decode identity must still be revoked.
        snapshot.scenes[0].frozen = true;
        cache.reconcile(&snapshot);
        assert!(!cache.matches(&key, generation));
        snapshot.scenes[0].frozen = false;
        let generation = cache.begin(&key);
        cache.entry = Some(CachedSource {
            image: RgbaImage::new(1, 1),
            last_used: Instant::now(),
        });
        snapshot.scenes[0].background.as_mut().unwrap().origin_y = Some(-1440);
        cache.reconcile(&snapshot);
        assert!(!cache.matches(&key, generation));
        assert!(cache.entry.is_none());
        snapshot.scenes[0].background = Some(key.asset.clone());
        let generation = cache.begin(&key);
        snapshot.active_scene_id = "scene-b".into();
        cache.reconcile(&snapshot);
        assert!(!cache.matches(&key, generation));
    }

    #[test]
    fn bounded_decode_rejects_missing_corrupt_and_metadata_mismatch_without_fallback() {
        struct Temporary(std::path::PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("capture.png"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let root =
            Temporary(std::env::temp_dir().join(format!("mewu-picker-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&root.0).unwrap();
        let path = root.0.join("capture.png");
        let mut key = key(3, 2);
        key.asset.path = path.to_string_lossy().into_owned();
        assert!(decode_source(&key.asset, &root.0, 0, 0).is_err());
        std::fs::write(&path, b"invalid PNG, not a screenshot").unwrap();
        assert!(decode_source(&key.asset, &root.0, 0, 0).is_err());
        RgbaImage::from_pixel(3, 2, Rgba([1, 2, 15, 255]))
            .save(&path)
            .unwrap();
        assert_eq!(
            decode_source(&key.asset, &root.0, 0, 0)
                .unwrap()
                .dimensions(),
            (3, 2)
        );
        let sampler = PixelSampler::default();
        let generation = sampler.inner.cache.lock().unwrap().begin(&key);
        sampler
            .inner
            .sample_or_load(&key, generation, 0, 0, || {
                decode_source(&key.asset, &root.0, 0, 0)
            })
            .unwrap();
        key.asset.width = Some(2);
        let generation = sampler.inner.cache.lock().unwrap().begin(&key);
        assert!(sampler
            .inner
            .sample_or_load(&key, generation, 0, 0, || decode_source(
                &key.asset, &root.0, 0, 0
            ))
            .is_err());
        sampler.inner.fail(&key, generation);
        let cache = sampler.inner.cache.lock().unwrap();
        assert!(cache.entry.is_none());
        assert!(cache.target.is_none());
    }

    #[test]
    fn metadata_and_allocation_preflight_rejects_invalid_sources() {
        for (width, height) in [
            (0, 1),
            (1, 0),
            (16385, 1),
            (8192, 8192),
            (u32::MAX, u32::MAX),
        ] {
            assert!(validate_dimensions(width, height).is_err());
        }
        assert!(validate_dimensions(8192, 4096).is_ok());
        let key = key(3, 2);
        assert!(validate_source(&key.asset, 3, 2, 2, 1).is_ok());
        assert!(validate_source(&key.asset, 2, 3, 0, 0).is_err());
        assert!(validate_source(&key.asset, 3, 2, 3, 0).is_err());
        let mut no_origin = key.asset.clone();
        no_origin.origin_x = None;
        assert!(validate_source(&no_origin, 3, 2, 0, 0).is_err());
    }
}
