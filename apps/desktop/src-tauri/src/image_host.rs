// SPDX-License-Identifier: MPL-2.0
//! A single image export owns its real worker, source leases and publication.
//! The Windows host uses typed PNG/JPEG selection; other platforms retain PNG.
use crate::{
    assets,
    export_variant_dialog::{self, ExportVariant, Media, Outcome as VariantOutcome},
    image_export::{self, ImageDestination},
    image_save_dialog::{self, ImageSaveName, SaveDialogOutcome},
    Host,
};
use mewu_core::{
    Asset, AssetKind, Drawing, OcrTarget, Region, RegionGeometry, SavedTranslation, Snapshot,
};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Notify;

pub const CANCELED: &str = image_export::CANCELED;
pub fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELED.into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct TranslationPixels {
    background_id: String,
    source_id: String,
    drawing_revision: u64,
    dimensions: (u32, u32),
    overlay: Asset,
}
impl TranslationPixels {
    fn new(value: &SavedTranslation) -> Self {
        Self {
            background_id: value.background_id.clone(),
            source_id: value.source_id.clone(),
            drawing_revision: value.drawing_revision,
            dimensions: (value.document.width, value.document.height),
            overlay: value.overlay.clone(),
        }
    }
}
/// Only fields determining rendered pixels/placement. OCR selections, drawing
/// history, messages, refs, draft, run and Agent changes do not cancel exports.
#[derive(Clone, Debug, PartialEq)]
struct RegionOrigin {
    scene_id: String,
    background: Asset,
    region_id: String,
    geometry: RegionGeometry,
    image_override: Option<Asset>,
    drawing_revision: u64,
    drawings: Vec<Drawing>,
    translation: Option<TranslationPixels>,
}
impl RegionOrigin {
    fn new(scene_id: &str, background: &Asset, region: &Region) -> Self {
        Self {
            scene_id: scene_id.into(),
            background: background.clone(),
            region_id: region.id.clone(),
            geometry: RegionGeometry::from(region),
            image_override: region.image_override.clone(),
            drawing_revision: region.drawing_revision,
            drawings: region.drawings.clone(),
            translation: region.translation.as_ref().map(TranslationPixels::new),
        }
    }
    fn matches_parts(&self, background: &Asset, region: &Region) -> bool {
        self.background == *background
            && self.region_id == region.id
            && self.geometry == RegionGeometry::from(region)
            && self.image_override == region.image_override
            && self.drawing_revision == region.drawing_revision
            && self.drawings == region.drawings
            && self.translation == region.translation.as_ref().map(TranslationPixels::new)
    }
    fn matches_snapshot(&self, snapshot: &Snapshot) -> bool {
        snapshot.active_scene_id == self.scene_id
            && snapshot.scenes.iter().any(|scene| {
                scene.id == self.scene_id
                    && !scene.closed
                    && !scene.frozen
                    && scene.background.as_ref().is_some_and(|background| {
                        scene
                            .regions
                            .iter()
                            .find(|region| region.id == self.region_id)
                            .is_some_and(|region| self.matches_parts(background, region))
                    })
            })
    }
    fn target(&self) -> OcrTarget {
        OcrTarget {
            scene_id: self.scene_id.clone(),
            region_id: self.region_id.clone(),
            background_id: self.background.id.clone(),
            drawing_revision: self.drawing_revision,
            x: self.geometry.x,
            y: self.geometry.y,
            width: self.geometry.width,
            height: self.geometry.height,
        }
    }
}
struct Pending {
    id: String,
    owner: String,
    region: Option<RegionOrigin>,
    cancel: Arc<AtomicBool>,
    publishing: bool,
}
#[derive(Default)]
struct Inner {
    pending: Mutex<Option<Pending>>,
    changed: Notify,
}
#[derive(Clone, Default)]
pub struct Registry(Arc<Inner>);
impl Registry {
    fn lock(&self) -> MutexGuard<'_, Option<Pending>> {
        self.0
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn begin(
        &self,
        owner: String,
        region: Option<RegionOrigin>,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(Work, Arc<AtomicBool>), String> {
        let mut pending = self.lock();
        if pending.is_some() {
            return Err("图片正在导出".into());
        }
        // Only atomics and native IsWindow/IsWindowVisible here. The caller
        // already owns Engine for region identity; never reacquire it here.
        admit()?;
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        *pending = Some(Pending {
            id: id.clone(),
            owner,
            region,
            cancel: cancel.clone(),
            publishing: false,
        });
        Ok((
            Work {
                registry: self.clone(),
                id,
                cancel: cancel.clone(),
            },
            cancel,
        ))
    }
    pub fn begin_pin(
        &self,
        owner: String,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(Work, Arc<AtomicBool>), String> {
        self.begin(owner, None, admit)
    }
    fn cancel_where(&self, predicate: impl FnOnce(&Pending) -> bool) {
        if let Some(pending) = self.lock().as_ref() {
            if !pending.publishing && predicate(pending) {
                pending.cancel.store(true, Ordering::Release);
            }
        }
        self.0.changed.notify_waiters();
    }
    pub fn cancel_owner(&self, owner: &str) {
        self.cancel_where(|pending| pending.owner == owner);
    }
    pub fn cancel_all(&self) {
        self.cancel_where(|_| true);
    }
    fn cancel_id(&self, id: &str) {
        self.cancel_where(|pending| pending.id == id);
    }
    fn check(&self, id: &str, admit: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let pending = self.lock();
        let pending = pending.as_ref().filter(|p| p.id == id).ok_or(CANCELED)?;
        if pending.cancel.load(Ordering::Acquire) {
            return Err(CANCELED.into());
        }
        if pending.publishing {
            return Err("图片已开始保存".into());
        }
        admit()
    }
    fn reconcile(&self, snapshot: &Snapshot) {
        self.cancel_where(|pending| {
            pending
                .region
                .as_ref()
                .is_some_and(|r| !r.matches_snapshot(snapshot))
        });
    }
    pub async fn drain(&self, timeout: Duration) -> Result<(), String> {
        self.cancel_all();
        tokio::time::timeout(timeout, async {
            loop {
                let changed = self.0.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.lock().is_none() {
                    return;
                }
                changed.await;
            }
        })
        .await
        .map_err(|_| "图片保存尚未结束，请稍后重试".to_string())
    }
}

/// Single owner; never clone this into the invoke future. Move into the actual
/// blocking closure before its first await. Drop releases only this exact flight.
pub struct Work {
    registry: Registry,
    id: String,
    cancel: Arc<AtomicBool>,
}
impl Work {
    pub fn cancel_on_drop(&self) -> InvokeCancel {
        InvokeCancel {
            registry: self.registry.clone(),
            id: self.id.clone(),
        }
    }
    pub fn check_cancel(&self) -> Result<(), String> {
        check_cancel(&self.cancel)
    }
    pub async fn cancelled(&self) {
        loop {
            let changed = self.registry.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.cancel.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
    pub fn accept_publication(
        &self,
        admit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut state = self.registry.lock();
        let pending = state.as_mut().filter(|p| p.id == self.id).ok_or(CANCELED)?;
        if pending.cancel.load(Ordering::Acquire) {
            return Err(CANCELED.into());
        }
        if pending.publishing {
            return Err("图片已开始保存".into());
        }
        admit()?;
        pending.publishing = true;
        Ok(())
    }
    /// Exit/owner cancellation is a normal settled operation, not a failed save.
    /// Once publication was accepted, errors must always remain visible.
    pub fn settle_result(&self, result: Result<(), String>) -> Result<(), String> {
        let pending = self.registry.lock();
        if pending
            .as_ref()
            .is_some_and(|p| p.id == self.id && !p.publishing && p.cancel.load(Ordering::Acquire))
        {
            Ok(())
        } else {
            result
        }
    }
    /// Only a completed publication is success for copy-and-close callers.
    /// Picker cancellation and revoked work never authorize closing a capture.
    fn settle_export_result(&self, result: Result<(), String>) -> Result<bool, String> {
        let pending = self.registry.lock();
        let pending = pending
            .as_ref()
            .filter(|p| p.id == self.id)
            .ok_or(CANCELED)?;
        if pending.publishing {
            result.map(|()| true)
        } else if pending.cancel.load(Ordering::Acquire) {
            Ok(false)
        } else {
            result.map(|()| false)
        }
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        let mut pending = self.registry.lock();
        if pending.as_ref().is_some_and(|p| p.id == self.id) {
            *pending = None;
        }
        drop(pending);
        self.registry.0.changed.notify_waiters();
    }
}
pub struct InvokeCancel {
    registry: Registry,
    id: String,
}
impl Drop for InvokeCancel {
    fn drop(&mut self) {
        self.registry.cancel_id(&self.id);
    }
}

/// Windows read sharing only: an existing incompatible writer prevents the
/// lease, and replacement/writes cannot start while the lease remains alive.
pub struct SourceLease {
    path: PathBuf,
    file: File,
}
impl SourceLease {
    pub fn open(asset: &Asset, root: &Path) -> Result<Self, String> {
        if asset.kind != AssetKind::Image {
            return Err("图片来源无效".into());
        }
        let path = assets::verified_path(asset, root)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let file = options.open(&path).map_err(|_| "图片正在使用或无法读取")?;
        let metadata = file.metadata().map_err(|_| "无法检查图片")?;
        if !metadata.is_file() || metadata.len() > image_export::MAX_ENCODED_BYTES {
            return Err("图片来源超过大小限制".into());
        }
        let result = Self { path, file };
        result.verify()?;
        Ok(result)
    }
    /// Decode the already leased file, never a second path-based open. Check
    /// the captured dimensions before allocating pixels and again afterwards.
    pub(crate) fn decode_rgba(&mut self, asset: &Asset) -> Result<image::RgbaImage, String> {
        use std::io::{BufReader, Seek, SeekFrom};
        self.verify()?;
        let expected = (
            asset.width.ok_or("图片宽度无效")?,
            asset.height.ok_or("图片高度无效")?,
        );
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "无法读取图片")?;
        let dimensions = image::ImageReader::new(BufReader::new(&mut self.file))
            .with_guessed_format()
            .map_err(|_| "图片格式无效")?
            .into_dimensions()
            .map_err(|_| "无法检查图片尺寸")?;
        if dimensions != expected
            || dimensions.0 == 0
            || dimensions.1 == 0
            || dimensions.0 > 16384
            || dimensions.1 > 16384
            || u64::from(dimensions.0) * u64::from(dimensions.1) > 32 * 1024 * 1024
        {
            return Err("图片像素尺寸已变更或超过贴图限制".into());
        }
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "无法读取图片")?;
        let mut reader = image::ImageReader::new(BufReader::new(&mut self.file))
            .with_guessed_format()
            .map_err(|_| "图片格式无效")?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().map_err(|_| "无法解码图片")?.into_rgba8();
        self.verify()?;
        if image.dimensions() != expected {
            return Err("图片像素尺寸已变更".into());
        }
        Ok(image)
    }
    fn identity(&self) -> Result<same_file::Handle, String> {
        same_file::Handle::from_file(self.file.try_clone().map_err(|_| "无法读取图片")?)
            .map_err(|_| "无法检查图片来源".into())
    }
    pub(crate) fn verify(&self) -> Result<(), String> {
        let current = same_file::Handle::from_path(&self.path).map_err(|_| "图片来源已变化")?;
        if current != self.identity()? {
            return Err("图片来源已变化".into());
        }
        Ok(())
    }
}
/// File I/O: always outside Engine/registry/Work.commit locks. Caller retains
/// the destination directory lease and every source lease through final rename.
pub fn protect_destination(
    destination: &ImageDestination,
    managed_root: &Path,
    sources: &[SourceLease],
) -> Result<(), String> {
    let protected = managed_root.canonicalize().map_err(|_| "应用目录不可用")?;
    if destination
        .path()
        .parent()
        .is_some_and(|parent| parent.starts_with(&protected))
    {
        return Err("请选择应用数据目录之外的位置".into());
    }
    for source in sources {
        source.verify()?;
    }
    match fs::symlink_metadata(destination.path()) {
        Ok(_) => {
            if destination
                .path()
                .canonicalize()
                .map_err(|_| "无法检查保存位置")?
                .starts_with(&protected)
            {
                return Err("请选择应用数据目录之外的位置".into());
            }
            let target =
                same_file::Handle::from_path(destination.path()).map_err(|_| "无法检查保存位置")?;
            for source in sources {
                if target == source.identity()? {
                    return Err("不能覆盖图片来源".into());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err("无法检查保存位置".into()),
    }
    Ok(())
}

pub(crate) fn window_handle(window: &WebviewWindow) -> Result<usize, String> {
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|h| h.0 as usize)
            .map_err(|_| "图片窗口不可用".into())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("此平台暂不支持图片保存".into())
    }
}
pub(crate) fn ensure_visible(handle: usize) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        if handle != 0 && IsWindow(handle as _) != 0 && IsWindowVisible(handle as _) != 0 {
            return Ok(());
        }
    }
    #[cfg(not(windows))]
    let _ = handle;
    Err("图片窗口已关闭或收起".into())
}
fn atomic_region_gate(host: &Host, handle: usize) -> Result<(), String> {
    host.exit.ensure_running()?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("截图正在切换".into());
    }
    ensure_visible(handle)
}
fn region_current(host: &Host, origin: &RegionOrigin) -> Result<(), String> {
    let engine = host.lock()?;
    let (background, region) = engine
        .store
        .region_for_pin(&origin.target())
        .map_err(|e| e.to_string())?;
    if !origin.matches_parts(&background, &region) {
        return Err("图片来源已更改，请重新保存".into());
    }
    Ok(())
}
fn region_allowed(
    app: &AppHandle,
    registry: &Registry,
    id: &str,
    origin: &RegionOrigin,
    handle: usize,
) -> Result<(), String> {
    let host = app.state::<Host>();
    // This helper retains Engine through registry admission. No filesystem or
    // Tauri HWND getter in this scope; Windows visibility is a direct API read.
    let engine = host.lock()?;
    let (background, region) = engine
        .store
        .region_for_pin(&origin.target())
        .map_err(|e| e.to_string())?;
    if !origin.matches_parts(&background, &region) {
        return Err("图片来源已更改，请重新保存".into());
    }
    registry.check(id, || atomic_region_gate(&host, handle))
}
fn accept_region(
    app: &AppHandle,
    work: &Work,
    origin: &RegionOrigin,
    handle: usize,
) -> Result<(), String> {
    let host = app.state::<Host>();
    let engine = host.lock()?;
    let (background, region) = engine
        .store
        .region_for_pin(&origin.target())
        .map_err(|e| e.to_string())?;
    if !origin.matches_parts(&background, &region) {
        return Err("图片来源已更改，请重新保存".into());
    }
    work.accept_publication(|| atomic_region_gate(&host, handle))
}

fn has_annotations(region: &Region) -> bool {
    !region.drawings.is_empty() || region.translation.is_some()
}

fn region_for_export(region: &Region, variant: ExportVariant) -> Region {
    let mut output = region.clone();
    if variant == ExportVariant::Clean {
        output.drawings.clear();
        output.translation = None;
    }
    output
}

pub async fn export_region(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    region_id: String,
    clipboard: bool,
) -> Result<bool, String> {
    if window.label() != "space" {
        return Err("此窗口不能导出截图".into());
    }
    let handle = window_handle(&window)?; // Main-thread getter outside shared locks.
    let (work, cancel, origin, background, region, assets_root, managed_root) = {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        atomic_region_gate(&host, handle)?;
        // Existing IPC has IDs only; current Store has no two-ID region reader.
        // Only initial admission uses one snapshot. Final checks use region_for_pin.
        let snapshot = engine.store.snapshot();
        let scene = snapshot
            .scenes
            .iter()
            .find(|s| s.id == scene_id)
            .ok_or("场景不存在")?;
        if snapshot.active_scene_id != scene_id || scene.closed || scene.frozen {
            return Err("空间已切换".into());
        }
        let background = scene.background.as_ref().ok_or("尚未截图")?.clone();
        let mut region = scene
            .regions
            .iter()
            .find(|r| r.id == region_id)
            .ok_or("选区不存在")?
            .clone();
        let origin = RegionOrigin::new(&scene_id, &background, &region);
        region.drawing_history = Default::default();
        region.ocr = None;
        let (work, cancel) =
            host.image_exports
                .begin("space".into(), Some(origin.clone()), || {
                    atomic_region_gate(&host, handle)
                })?;
        (
            work,
            cancel,
            origin,
            background,
            region,
            host.assets.clone(),
            host.root.clone(),
        )
    };
    let guard = work.cancel_on_drop();
    // Dropping this invoke future drops only guard. The detached blocking job
    // still owns Work, source handles, picker thread and final I/O to completion.
    let result = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            work.check_cancel()?;
            let mut sources = vec![SourceLease::open(
                region.image_override.as_ref().unwrap_or(&background),
                &assets_root,
            )?];
            region_current(&app.state::<Host>(), &origin)?;
            let variant = if clipboard {
                ExportVariant::Annotated
            } else {
                let picker_app = app.clone();
                let picker_registry = work.registry.clone();
                let picker_id = work.id.clone();
                let picker_origin = origin.clone();
                match export_variant_dialog::pick_variant(
                    handle as isize,
                    Media::Image,
                    has_annotations(&region),
                    cancel.clone(),
                    move || {
                        region_allowed(
                            &picker_app,
                            &picker_registry,
                            &picker_id,
                            &picker_origin,
                            handle,
                        )
                    },
                )? {
                    VariantOutcome::Selected(variant) => variant,
                    VariantOutcome::UserCanceled | VariantOutcome::RequestCanceled => return Ok(()),
                }
            };
            region_allowed(&app, &work.registry, &work.id, &origin, handle)?;
            let output_region = region_for_export(&region, variant);
            if let Some(saved) = &output_region.translation {
                sources.push(SourceLease::open(&saved.overlay, &assets_root)?);
            }
            work.check_cancel()?;
            let image =
                assets::crop_from_parts(&background, &output_region, &assets_root)?.into_rgba8();
            for source in &sources {
                source.verify()?;
            }
            work.check_cancel()?;
            if clipboard {
                let mut board = arboard::Clipboard::new().map_err(|_| "无法打开剪贴板")?;
                accept_region(&app, &work, &origin, handle)?;
                // Acceptance locks have been released before native clipboard I/O.
                board
                    .set_image(arboard::ImageData {
                        width: image.width() as usize,
                        height: image.height() as usize,
                        bytes: std::borrow::Cow::Owned(image.into_raw()),
                    })
                    .map_err(|_| "无法复制图片".to_string())?;
                return Ok(());
            }
            let picker_app = app.clone();
            let picker_registry = work.registry.clone();
            let picker_id = work.id.clone();
            let picker_origin = origin.clone();
            let default_format = image_save_dialog::preference_format(&app)?;
            let selected = image_save_dialog::pick_image_destination_with_default(
                handle as isize,
                ImageSaveName::Screenshot,
                default_format,
                cancel.clone(),
                move || {
                    region_allowed(
                        &picker_app,
                        &picker_registry,
                        &picker_id,
                        &picker_origin,
                        handle,
                    )
                },
            )
            .map_err(|error| error.to_string())?;
            let (path, format) = match selected {
                SaveDialogOutcome::Selected { path, format } => (path, format),
                SaveDialogOutcome::UserCanceled | SaveDialogOutcome::RequestCanceled => {
                    return Ok(())
                }
            };
            work.check_cancel()?;
            let destination = ImageDestination::prepare(&path, format)?;
            protect_destination(&destination, &managed_root, &sources)?;
            image_export::save_image_atomic(
                format,
                &image,
                &destination,
                &cancel,
                |temp, target| {
                    // Disk/source checks before short Engine->registry acceptance.
                    protect_destination(&destination, &managed_root, &sources)?;
                    accept_region(&app, &work, &origin, handle)?;
                    fs::rename(temp, target).map_err(|_| "无法完成图片保存，原文件未被覆盖".into())
                },
            )
        })();
        work.settle_export_result(result)
    })
    .await
    .map_err(|_| "图片导出中断")?;
    drop(guard);
    result
}

pub fn reconcile(app: &AppHandle, snapshot: &Snapshot) {
    if let Some(host) = app.try_state::<Host>() {
        host.image_exports.reconcile(snapshot);
    }
}
pub fn cancel_owner(app: &AppHandle, owner: &str) {
    if let Some(host) = app.try_state::<Host>() {
        host.image_exports.cancel_owner(owner);
    }
}
pub fn cancel_all(app: &AppHandle) {
    if let Some(host) = app.try_state::<Host>() {
        host.image_exports.cancel_all();
    }
}
pub async fn drain(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    let registry = app.state::<Host>().image_exports.clone();
    registry.drain(timeout).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_export_strips_only_output_layers_and_keeps_original_pixel_source_and_fence() {
        struct Fixture {
            root: PathBuf,
            files: Vec<PathBuf>,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                for file in &self.files {
                    let _ = fs::remove_file(file);
                }
                let _ = fs::remove_dir(&self.root);
            }
        }
        let mut fixture = Fixture {
            root: std::env::temp_dir().join(format!("mewu-image-variant-{}", uuid::Uuid::new_v4())),
            files: vec![],
        };
        fs::create_dir(&fixture.root).unwrap();
        let mut image_asset = |width, height, pixel| {
            let id = uuid::Uuid::new_v4().to_string();
            let path = fixture.root.join(format!("{id}.png"));
            image::RgbaImage::from_pixel(width, height, image::Rgba(pixel))
                .save(&path)
                .unwrap();
            fixture.files.push(path.clone());
            Asset {
                id,
                kind: AssetKind::Image,
                name: "synthetic.png".into(),
                path: path.to_string_lossy().into(),
                width: Some(width),
                height: Some(height),
                origin_x: None,
                origin_y: None,
                scale_factor: None,
            }
        };
        let background = image_asset(16, 16, [10, 20, 30, 255]);
        let replacement = image_asset(8, 12, [120, 130, 140, 255]);
        let overlay = image_asset(8, 12, [0, 180, 0, 255]);
        let mut region = Region {
            id: "region".into(),
            x: 100.,
            y: 200.,
            width: 80.,
            height: 120.,
            image_override: Some(replacement.clone()),
            drawing_revision: 9,
            ..Default::default()
        };
        region.drawings.push(Drawing {
            id: "drawing".into(),
            kind: mewu_core::DrawingKind::Line,
            color: "#ff0000".into(),
            stroke_width: 2.,
            points: vec![
                mewu_core::DrawingPoint { x: 1., y: 1. },
                mewu_core::DrawingPoint { x: 6., y: 9. },
            ],
            text: None,
            font_size: None,
            origin: None,
            rich: None,
        });
        region.translation = Some(SavedTranslation {
            background_id: background.id.clone(),
            source_id: replacement.id.clone(),
            drawing_revision: 9,
            document: mewu_core::TranslationDocument {
                version: 1,
                target_language: "zh-Hans".into(),
                width: 8,
                height: 12,
                text_angle: None,
                lines: vec![],
            },
            overlay,
            selection: mewu_core::OcrDocument {
                engine: "synthetic".into(),
                language: "zh-Hans".into(),
                width: 8,
                height: 12,
                text_angle: None,
                lines: vec![],
            },
        });
        let before = region.clone();
        let origin = RegionOrigin::new("scene", &background, &region);
        assert!(has_annotations(&region));
        let clean = region_for_export(&region, ExportVariant::Clean);
        assert!(!has_annotations(&clean));
        assert_eq!(clean.image_override, Some(replacement.clone()));
        assert_eq!(clean.drawing_revision, region.drawing_revision);
        assert_eq!(RegionGeometry::from(&clean), RegionGeometry::from(&region));
        assert_eq!(clean.drawing_history, region.drawing_history);
        let pixels = assets::crop_from_parts(&background, &clean, &fixture.root)
            .unwrap()
            .into_rgba8();
        assert_eq!(pixels.dimensions(), (8, 12));
        assert!(pixels.pixels().all(|pixel| pixel.0 == [120, 130, 140, 255]));
        let annotated = region_for_export(&region, ExportVariant::Annotated);
        assert_eq!(annotated, before);
        assert_ne!(
            assets::crop_from_parts(&background, &annotated, &fixture.root)
                .unwrap()
                .into_rgba8(),
            pixels
        );
        assert_eq!(region, before);
        assert!(origin.matches_parts(&background, &region));
        assert!(!origin.matches_parts(&background, &clean));
    }
    #[test]
    fn only_successful_publication_allows_copy_and_close() {
        let registry = Registry::default();
        let (canceled, _) = registry.begin_pin("space".into(), || Ok(())).unwrap();
        registry.cancel_owner("space");
        assert_eq!(
            canceled.settle_export_result(Err(CANCELED.into())),
            Ok(false)
        );
        assert_eq!(canceled.settle_export_result(Ok(())), Ok(false));
        drop(canceled);

        let (pending, _) = registry.begin_pin("space".into(), || Ok(())).unwrap();
        // Closing the picker successfully is not an exported image.
        assert_eq!(pending.settle_export_result(Ok(())), Ok(false));
        assert_eq!(
            pending.settle_export_result(Err("decode failed".into())),
            Err("decode failed".into())
        );
        pending.accept_publication(|| Ok(())).unwrap();
        registry.cancel_all();
        assert_eq!(pending.settle_export_result(Ok(())), Ok(true));
        assert_eq!(
            pending.settle_export_result(Err("clipboard failed".into())),
            Err("clipboard failed".into())
        );
    }
    #[test]
    fn invoke_drop_cancels_but_real_worker_holds_slot_until_exit() {
        let registry = Registry::default();
        let (work, cancel) = registry.begin_pin("pin-first".into(), || Ok(())).unwrap();
        let guard = work.cancel_on_drop();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            done_rx.recv().unwrap();
            assert_eq!(work.settle_result(Err(CANCELED.into())), Ok(()));
            drop(work);
        });
        ready_rx.recv().unwrap();
        drop(guard);
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry.begin_pin("pin-second".into(), || Ok(())).is_err());
        done_tx.send(()).unwrap();
        thread.join().unwrap();
        assert!(registry.begin_pin("pin-second".into(), || Ok(())).is_ok());
    }
    #[test]
    fn admission_and_publication_are_linearized_with_cancel_and_do_not_swallow_io_failure() {
        let registry = Registry::default();
        assert!(registry
            .begin_pin("pin".into(), || Err("exiting".into()))
            .is_err());
        let (work, cancel) = registry.begin_pin("pin".into(), || Ok(())).unwrap();
        let guard = work.cancel_on_drop();
        work.accept_publication(|| {
            assert!(matches!(
                registry.0.pending.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Ok(())
        })
        .unwrap();
        drop(guard);
        registry.cancel_owner("pin");
        registry.cancel_all();
        assert!(!cancel.load(Ordering::Acquire));
        assert!(registry.begin_pin("other".into(), || Ok(())).is_err());
        assert_eq!(
            work.settle_result(Err("disk failed".into())),
            Err("disk failed".into())
        );
        drop(work);
        let (next, _) = registry.begin_pin("pin".into(), || Ok(())).unwrap();
        registry.cancel_owner("different");
        assert!(next.check_cancel().is_ok());
        registry.cancel_owner("pin");
        assert!(next.accept_publication(|| Ok(())).is_err());
    }
    #[test]
    fn pixel_identity_ignores_ocr_history_but_detects_source_and_geometry_changes() {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id.clone();
        let asset = Asset {
            id: uuid::Uuid::new_v4().to_string(),
            kind: AssetKind::Image,
            name: "test.png".into(),
            path: "synthetic.png".into(),
            width: Some(800),
            height: Some(600),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        store.set_background(&scene_id, asset.clone()).unwrap();
        let region = Region {
            id: uuid::Uuid::new_v4().to_string(),
            x: 10.,
            y: 20.,
            width: 100.,
            height: 100.,
            ..Default::default()
        };
        store
            .apply(mewu_core::SceneCommand::AddRegion {
                scene_id: scene_id.clone(),
                region: region.clone(),
            })
            .unwrap();
        let origin = RegionOrigin::new(&scene_id, &asset, &region);
        let mut snapshot = store.snapshot();
        let scene = snapshot
            .scenes
            .iter_mut()
            .find(|s| s.id == scene_id)
            .unwrap();
        scene.draft = "unrelated conversation draft".into();
        scene.regions[0].ocr = Some(mewu_core::RegionOcr {
            background_id: asset.id.clone(),
            drawing_revision: region.drawing_revision,
            document: mewu_core::OcrDocument {
                engine: "test".into(),
                language: "en".into(),
                width: 100,
                height: 100,
                text_angle: None,
                lines: vec![],
            },
        });
        assert!(origin.matches_snapshot(&snapshot));
        let scene = snapshot
            .scenes
            .iter_mut()
            .find(|s| s.id == scene_id)
            .unwrap();
        scene.regions[0].x += 1.;
        assert!(!origin.matches_snapshot(&snapshot));
        let mut changed = asset.clone();
        changed.path = "different.png".into();
        assert!(!origin.matches_parts(&changed, &region));
    }
    #[test]
    fn destination_protection_rejects_private_tree_and_external_hardlink_to_source() {
        struct TestRoot {
            path: PathBuf,
            parent: PathBuf,
            leaf: String,
        }
        impl Drop for TestRoot {
            fn drop(&mut self) {
                if let Ok(path) = self.path.canonicalize() {
                    if path == self.parent.join(&self.leaf)
                        && path.parent() == Some(self.parent.as_path())
                        && self
                            .leaf
                            .strip_prefix("mewu-image-host-test-")
                            .is_some_and(|v| uuid::Uuid::parse_str(v).is_ok())
                    {
                        let _ = fs::remove_dir_all(path);
                    }
                }
            }
        }
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let leaf = format!("mewu-image-host-test-{}", uuid::Uuid::new_v4());
        let path = parent.join(&leaf);
        fs::create_dir(&path).unwrap();
        let root = TestRoot { path, parent, leaf };
        let managed = root.path.join("managed");
        let assets_root = managed.join("assets");
        let outside = root.path.join("outside");
        fs::create_dir(&managed).unwrap();
        fs::create_dir(&assets_root).unwrap();
        fs::create_dir(&outside).unwrap();
        let source_path = assets_root.join("synthetic.png");
        fs::write(&source_path, b"immutable image fixture").unwrap();
        let alias = outside.join("alias.png");
        fs::hard_link(&source_path, &alias).unwrap();
        let asset = Asset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "synthetic.png".into(),
            kind: AssetKind::Image,
            path: source_path.to_str().unwrap().into(),
            width: Some(1),
            height: Some(1),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let sources = vec![SourceLease::open(&asset, &assets_root).unwrap()];
        let private =
            ImageDestination::prepare(&managed.join("new.png"), image_export::ImageSaveFormat::Png)
                .unwrap();
        let alias = ImageDestination::prepare(&alias, image_export::ImageSaveFormat::Png).unwrap();
        let allowed =
            ImageDestination::prepare(&outside.join("new.png"), image_export::ImageSaveFormat::Png)
                .unwrap();
        assert!(protect_destination(&private, &managed, &sources).is_err());
        assert!(protect_destination(&alias, &managed, &sources).is_err());
        assert!(protect_destination(&allowed, &managed, &sources).is_ok());
        assert_eq!(fs::read(&source_path).unwrap(), b"immutable image fixture");
        #[cfg(windows)]
        {
            assert!(OpenOptions::new().write(true).open(&source_path).is_err());
        }
    }
}
