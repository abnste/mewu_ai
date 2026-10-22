// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0

//! Revocable, host-attested access to one real desktop region. This is not a
//! general input injection endpoint. Only the host constructs RegionSource,
//! following an explicit user command; plugin manifests cannot mint grants.
//! All blocking operations must run off the UI thread. Overlay hooks marshal
//! their own window changes to the UI thread and must not take an Engine lock.

use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use mewu_core::{
    Asset, AssetKind, CaptureCoordinateSpace, CaptureCoordinates, Region, RegionGeometry,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::File,
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const MAX_REGION_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SESSION: Duration = Duration::from_secs(10 * 60);
static ACTIVE_REGION: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub(crate) struct RegionError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl RegionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for RegionError {}
type Result<T> = std::result::Result<T, RegionError>;

/// Deliberately NOT Deserialize. These are trusted host state, not plugin args.
pub(crate) struct RegionSource {
    pub(crate) scene_id: String,
    pub(crate) background: Asset,
    pub(crate) selection: Region,
    pub(crate) asset_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionBinding {
    pub(crate) scene_id: String,
    pub(crate) background_id: String,
    pub(crate) region_id: String,
    pub(crate) asset_sha256: String,
    pub(crate) source_width: u32,
    pub(crate) source_height: u32,
    pub(crate) selection: RegionGeometry,
    pub(crate) drawing_revision: u64,
    pub(crate) coordinates: CaptureCoordinates,
}

pub(crate) trait RegionOverlayHooks: Send + Sync {
    /// Recheck the current scene/asset/selection/revision AND the user's live run
    /// authority. This must fail after disable, unload, stop or run replacement.
    fn validate_source(&self, binding: &RegionBinding) -> std::result::Result<(), String>;
    /// Hide every owned overlay intersecting the region; wait for compositor
    /// completion before returning. On error, compensate any partial hiding.
    fn hide_owned_overlays(&self) -> std::result::Result<(), String>;
    /// Restore the same owned windows, without stealing focus from the target.
    fn restore_owned_overlays(&self);
}

struct HiddenOverlays {
    hooks: Arc<dyn RegionOverlayHooks>,
    hidden: AtomicBool,
}
impl HiddenOverlays {
    fn enter(hooks: Arc<dyn RegionOverlayHooks>) -> Result<Self> {
        hooks
            .hide_owned_overlays()
            .map_err(|e| RegionError::new("overlay_hide_failed", e))?;
        Ok(Self { hooks, hidden: AtomicBool::new(true) })
    }
    fn restore(&self) {
        if self.hidden.swap(false, Ordering::SeqCst) {
            self.hooks.restore_owned_overlays();
        }
    }
}
impl Drop for HiddenOverlays {
    fn drop(&mut self) {
        self.restore();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RegionPoint {
    pub(crate) x: u32,
    pub(crate) y: u32,
}

/// No arbitrary virtual keycodes, shell shortcuts or modifier chords.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RegionKey {
    Enter,
    Backspace,
    Delete,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum RegionAction {
    Click { x: u32, y: u32 },
    DoubleClick { x: u32, y: u32 },
    Drag { path: Vec<RegionPoint> },
    Scroll { x: u32, y: u32, delta_y: i32 },
    Type { text: String },
    Key { key: RegionKey },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionSessionInfo {
    pub(crate) id: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) binding: RegionBinding,
    pub(crate) capabilities: Vec<&'static str>,
    pub(crate) target: String,
}

pub(crate) struct RegionFrame {
    pub(crate) png: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) sha256: String,
}

#[cfg(windows)]
#[derive(Clone)]
pub(crate) struct ForegroundIdentity(platform::Target);

#[cfg(windows)]
pub(crate) fn foreground_identity() -> Option<ForegroundIdentity> {
    let _dpi = platform::dpi_scope().ok()?;
    platform::foreground_identity().map(ForegroundIdentity)
}

#[cfg(windows)]
pub(crate) fn foreground_matches(expected: &ForegroundIdentity) -> bool {
    let Ok(_dpi) = platform::dpi_scope() else { return false };
    platform::foreground_matches(&expected.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Viewport {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl Viewport {
    fn from_source(source: &RegionSource) -> Result<(Self, RegionBinding)> {
        let invalid = || RegionError::new("region_source_invalid", "请选择原始截图中的有效区域");
        let coordinates = source.background.capture_coordinates.ok_or_else(invalid)?;
        let width = source.background.width.ok_or_else(invalid)?;
        let height = source.background.height.ok_or_else(invalid)?;
        if source.background.kind != AssetKind::Image
            || source.selection.image_override.is_some()
            || !coordinates.valid_for(width, height)
            || source.asset_sha256.len() != 64
            || !source.asset_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || source.scene_id.is_empty()
            || source.background.id.is_empty()
            || source.selection.id.is_empty()
        {
            return Err(invalid());
        }
        // Use only whole pixels INSIDE the selection; rounding must never grant
        // an extra strip outside what the user selected.
        let r = &source.selection;
        if ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
            || r.x < 0.
            || r.y < 0.
            || r.width <= 0.
            || r.height <= 0.
            || r.x + r.width > f64::from(width)
            || r.y + r.height > f64::from(height)
        {
            return Err(invalid());
        }
        let left = r.x.ceil() as u32;
        let top = r.y.ceil() as u32;
        let right = (r.x + r.width).floor() as u32;
        let bottom = (r.y + r.height).floor() as u32;
        let region_width = right
            .checked_sub(left)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        let region_height = bottom
            .checked_sub(top)
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        if u64::from(region_width) * u64::from(region_height) > MAX_REGION_PIXELS {
            return Err(RegionError::new(
                "region_too_large",
                "操作区域不能超过 4 Mi 像素",
            ));
        }
        let (native_x, native_y) = coordinates
            .point(width, height, left, top)
            .ok_or_else(invalid)?;
        // Physical-pixel platforms retain their existing desktop viewport.
        // Quartz uses a local raster viewport and the attested transform below;
        // never truncate Retina points or multiply a global origin by scale.
        let (x,y) = match coordinates.space {
            CaptureCoordinateSpace::PhysicalPixels => (native_x,native_y),
            CaptureCoordinateSpace::QuartzPoints|CaptureCoordinateSpace::PortalLogical => (f64::from(left),f64::from(top)),
        };
        if x < f64::from(i32::MIN)
            || y < f64::from(i32::MIN)
            || x + f64::from(region_width) > f64::from(i32::MAX)
            || y + f64::from(region_height) > f64::from(i32::MAX)
        {
            return Err(invalid());
        }
        let viewport = Self {
            x: x as i32,
            y: y as i32,
            width: region_width,
            height: region_height,
        };
        let binding = RegionBinding {
            scene_id: source.scene_id.clone(),
            background_id: source.background.id.clone(),
            region_id: source.selection.id.clone(),
            asset_sha256: source.asset_sha256.to_ascii_lowercase(),
            source_width: width,
            source_height: height,
            selection: RegionGeometry::from(r),
            drawing_revision: r.drawing_revision,
            coordinates,
        };
        Ok((viewport, binding))
    }

    fn point(self, point: RegionPoint) -> Result<(i32, i32)> {
        if point.x >= self.width || point.y >= self.height {
            return Err(RegionError::new(
                "region_point_outside",
                "操作坐标超出授权区域",
            ));
        }
        Ok((self.x + point.x as i32, self.y + point.y as i32))
    }
    fn contains_rect(self, x: i32, y: i32, right: i32, bottom: i32) -> bool {
        x < right
            && y < bottom
            && x >= self.x
            && y >= self.y
            && i64::from(right) <= i64::from(self.x) + i64::from(self.width)
            && i64::from(bottom) <= i64::from(self.y) + i64::from(self.height)
    }

    fn native_point(self, binding: &RegionBinding, point: RegionPoint) -> Result<(f64,f64)> {
        self.point(point)?;
        binding.coordinates.point(binding.source_width,binding.source_height,
            binding.selection.x.ceil() as u32 + point.x,
            binding.selection.y.ceil() as u32 + point.y)
            .ok_or_else(|| RegionError::new("region_coordinates_unavailable","操作区域坐标无效"))
    }

    fn native_rect(self,binding:&RegionBinding)->Result<[f64;4]> {
        let (x,y)=self.native_point(binding,RegionPoint{x:0,y:0})?;
        let right=x+f64::from(self.width)*binding.coordinates.width/f64::from(binding.source_width);
        let bottom=y+f64::from(self.height)*binding.coordinates.height/f64::from(binding.source_height);
        Ok([x,y,right,bottom])
    }
}

fn source_hash(path: &str) -> Result<String> {
    let mut file = File::open(path)
        .map_err(|_| RegionError::new("region_source_missing", "截图文件已不可用"))?;
    let length = file
        .metadata()
        .map_err(|_| RegionError::new("region_source_missing", "截图文件已不可用"))?
        .len();
    if length == 0 || length > MAX_SOURCE_BYTES {
        return Err(RegionError::new(
            "region_source_invalid",
            "截图文件大小无效",
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut read = 0u64;
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| RegionError::new("region_source_missing", "无法读取截图文件"))?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > MAX_SOURCE_BYTES {
            return Err(RegionError::new(
                "region_source_invalid",
                "截图文件大小无效",
            ));
        }
        hasher.update(&buffer[..n]);
    }
    if read != length {
        return Err(RegionError::new("region_source_changed", "截图文件已改变"));
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) struct RegionSession {
    info: RegionSessionInfo,
    source_path: String,
    viewport: Viewport,
    topology: Vec<crate::capture::MonitorInfo>,
    target: platform::Target,
    hooks: Arc<dyn RegionOverlayHooks>,
    overlays: HiddenOverlays,
    created: Instant,
    revoked: AtomicBool,
    owns_slot: AtomicBool,
    button_down: AtomicBool,
    observed: AtomicBool,
    operation: Mutex<()>,
}

impl RegionSession {
    pub(crate) fn supported() -> bool {
        cfg!(any(windows,target_os="macos"))
            && crate::extension_store::region_platform_supported(mewu_extension::current_plugin_platform())
    }

    pub(crate) fn begin(
        source: RegionSource,
        hooks: Arc<dyn RegionOverlayHooks>,
    ) -> Result<Arc<Self>> {
        if !Self::supported() {
            return Err(RegionError::new(
                "region_platform_unsupported",
                "当前平台尚不支持区域自动操作",
            ));
        }
        let (viewport, binding) = Viewport::from_source(&source)?;
        hooks
            .validate_source(&binding)
            .map_err(|e| RegionError::new("region_authority_revoked", e))?;
        if source_hash(&source.background.path)? != binding.asset_sha256 {
            return Err(RegionError::new("region_source_changed", "截图文件已改变"));
        }
        if ACTIVE_REGION
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(RegionError::new("region_busy", "已有区域操作正在运行"));
        }
        // Keep this guard until Arc owns the slot, including all fallible hooks.
        struct Slot(bool);
        impl Drop for Slot {
            fn drop(&mut self) {
                if self.0 {
                    ACTIVE_REGION.store(false, Ordering::SeqCst);
                }
            }
        }
        let mut slot = Slot(true);
        // Keep the real application visible throughout observation/model/action
        // turns. Re-showing the immutable capture after each call both flickers
        // and conceals every successful action behind an outdated screenshot.
        // This owned guard also restores overlays if binding fails below.
        let overlays = HiddenOverlays::enter(hooks.clone())?;
        let _dpi = platform::dpi_scope()?;
        let topology = topology()?;
        validate_monitor(&binding, viewport, &topology)?;
        let target = platform::bind(viewport, &binding)?;
        platform::neutral_input()?;
        let info = RegionSessionInfo {
            id: uuid::Uuid::new_v4().to_string(),
            width: viewport.width,
            height: viewport.height,
            binding,
            capabilities: vec![
                "observe",
                "click",
                "doubleClick",
                "drag",
                "scroll",
                "type",
                "key",
            ],
            target: platform::description(&target),
        };
        let session = Arc::new(Self {
            info,
            source_path: source.background.path,
            viewport,
            topology,
            target,
            hooks: hooks.clone(),
            overlays,
            created: Instant::now(),
            revoked: AtomicBool::new(false),
            owns_slot: AtomicBool::new(true),
            button_down: AtomicBool::new(false),
            observed: AtomicBool::new(false),
            operation: Mutex::new(()),
        });
        slot.0 = false;
        session.check()?;
        #[cfg(target_os="macos")]
        {
            let weak=Arc::downgrade(&session);
            if std::thread::Builder::new().name("mewu-region-stop".into()).spawn(move||{
                loop {
                    let Some(session)=weak.upgrade() else {break};
                    if session.revoked.load(Ordering::SeqCst){break}
                    if platform::stop_requested()||session.created.elapsed()>MAX_SESSION {
                        session.revoke();break;
                    }
                    drop(session);
                    std::thread::sleep(Duration::from_millis(10));
                }
            }).is_err(){session.revoke();return Err(RegionError::new("region_stop_unavailable","无法启用区域停止控制"))}
        }
        Ok(session)
    }

    pub(crate) fn info(&self) -> RegionSessionInfo {
        self.info.clone()
    }

    fn ensure_live(&self) -> Result<()> {
        if self.revoked.load(Ordering::SeqCst) || self.created.elapsed() > MAX_SESSION {
            return Err(RegionError::new(
                "region_authority_revoked",
                "区域操作已停止",
            ));
        }
        Ok(())
    }

    fn check(&self) -> Result<()> {
        let result = self.check_inner();
        if result.is_err() {
            self.revoke();
        }
        result
    }

    fn check_inner(&self) -> Result<()> {
        if platform::stop_requested() {
            self.revoke();
        }
        if self.revoked.load(Ordering::SeqCst) || self.created.elapsed() > MAX_SESSION {
            return Err(RegionError::new(
                "region_authority_revoked",
                "区域操作已停止",
            ));
        }
        self.hooks
            .validate_source(&self.info.binding)
            .map_err(|e| RegionError::new("region_authority_revoked", e))?;
        if topology()? != self.topology {
            return Err(RegionError::new(
                "region_display_changed",
                "显示器设置已改变，请重新选择区域",
            ));
        }
        if source_hash(&self.source_path)? != self.info.binding.asset_sha256 {
            return Err(RegionError::new("region_source_changed", "截图文件已改变"));
        }
        platform::validate(&self.target, self.viewport)
    }

    pub(crate) fn observe(&self) -> Result<RegionFrame> {
        self.ensure_live()?;
        let _operation = self
            .operation
            .lock()
            .map_err(|_| RegionError::new("region_stopped", "区域操作已停止"))?;
        self.ensure_live()?;
        let _dpi = platform::dpi_scope()?;
        self.check()?;
        let pixels = platform::capture(self)?;
        self.check()?;
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
                ExtendedColorType::Rgba8,
            )
            .map_err(|_| RegionError::new("region_capture_failed", "无法编码区域截图"))?;
        self.check()?;
        self.observed.store(true, Ordering::SeqCst);
        Ok(RegionFrame {
            sha256: format!("{:x}", Sha256::digest(&png)),
            png,
            width: pixels.width(),
            height: pixels.height(),
        })
    }

    pub(crate) fn execute(&self, action: RegionAction) -> Result<()> {
        self.ensure_live()?;
        let _operation = self
            .operation
            .lock()
            .map_err(|_| RegionError::new("region_stopped", "区域操作已停止"))?;
        self.ensure_live()?;
        validate_action(self.viewport, &action)?;
        if self
            .observed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(RegionError::new(
                "region_observation_required",
                "请先观察当前区域，再执行下一步操作",
            ));
        }
        let _dpi = platform::dpi_scope()?;
        self.check()?;
        platform::neutral_input()?;
        let result = platform::execute(self, action);
        // Mouse-up is cleanup, so it remains allowed even after authority loss.
        if self.button_down.load(Ordering::SeqCst) {
            self.release_button();
        }
        if let Err(error) = result {
            self.revoke();
            return Err(error);
        }
        self.check()
    }

    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        self.release_button();
        self.overlays.restore();
        if self.owns_slot.swap(false, Ordering::SeqCst) {
            ACTIVE_REGION.store(false, Ordering::SeqCst);
        }
    }
    fn release_button(&self) {
        platform::release_button(&self.button_down);
    }
}
impl Drop for RegionSession {
    fn drop(&mut self) {
        self.revoke();
    }
}

fn validate_action(viewport: Viewport, action: &RegionAction) -> Result<()> {
    match action {
        RegionAction::Click { x, y }
        | RegionAction::DoubleClick { x, y }
        | RegionAction::Scroll { x, y, .. } => {
            viewport.point(RegionPoint { x: *x, y: *y })?;
        }
        RegionAction::Drag { path } => {
            if !(2..=256).contains(&path.len()) {
                return Err(RegionError::new(
                    "region_path_invalid",
                    "拖动路径需要 2 至 256 个坐标",
                ));
            }
            for &point in path {
                viewport.point(point)?;
            }
            let travel: u64 = path
                .windows(2)
                .map(|p| u64::from(p[0].x.abs_diff(p[1].x).max(p[0].y.abs_diff(p[1].y))))
                .sum();
            if travel > 8192 {
                return Err(RegionError::new("region_path_invalid", "单次拖动路径过长"));
            }
        }
        RegionAction::Type { text } => {
            if text.is_empty() || text.len() > 4096 || text.chars().any(|c| c.is_control()) {
                return Err(RegionError::new(
                    "region_text_invalid",
                    "请输入不含控制字符的短文本",
                ));
            }
        }
        RegionAction::Key { .. } => (),
    }
    if let RegionAction::Scroll { delta_y, .. } = action {
        if *delta_y == 0 || !(-1200..=1200).contains(delta_y) {
            return Err(RegionError::new(
                "region_scroll_invalid",
                "滚动量超出单次范围",
            ));
        }
    }
    Ok(())
}

fn topology() -> Result<Vec<crate::capture::MonitorInfo>> {
    let mut monitors = crate::capture::list_monitors()
        .map_err(|e| RegionError::new("region_display_changed", e))?;
    if monitors.is_empty() || monitors.len() > 32 {
        return Err(RegionError::new(
            "region_display_changed",
            "显示器配置不可用",
        ));
    }
    monitors.sort_by_key(|m| m.id);
    if monitors.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(RegionError::new(
            "region_display_changed",
            "显示器标识不唯一",
        ));
    }
    Ok(monitors)
}

fn validate_monitor(
    binding: &RegionBinding,
    viewport: Viewport,
    monitors: &[crate::capture::MonitorInfo],
) -> Result<()> {
    let c = binding.coordinates;
    if !monitors.iter().any(|m| {
        matches!((m.coordinate_space,c.space),
            (crate::capture::CoordinateSpace::PhysicalPixels,CaptureCoordinateSpace::PhysicalPixels)
            |(crate::capture::CoordinateSpace::LogicalPoints,CaptureCoordinateSpace::QuartzPoints))
            && c.origin_x == f64::from(m.origin_x)
            && c.origin_y == f64::from(m.origin_y)
            && c.width == f64::from(m.display_width)
            && c.height == f64::from(m.display_height)
            && viewport.native_rect(binding).is_ok_and(|[x,y,right,bottom]| {
                x >= c.origin_x && y >= c.origin_y && right <= c.origin_x+c.width && bottom <= c.origin_y+c.height
            })
    }) {
        return Err(RegionError::new(
            "region_display_changed",
            "截图与当前显示器坐标不一致，请重新截图",
        ));
    }
    Ok(())
}

#[cfg(not(any(windows,target_os="macos")))]
mod platform {
    use super::*;
    pub(super) struct Target;
    pub(super) fn dpi_scope() -> Result<()> {
        Err(RegionError::new(
            "region_platform_unsupported",
            "当前平台尚不支持区域自动操作",
        ))
    }
    pub(super) fn bind(_: Viewport, _: &RegionBinding) -> Result<Target> {
        Err(RegionError::new(
            "region_platform_unsupported",
            "当前平台尚不支持区域自动操作",
        ))
    }
    pub(super) fn description(_: &Target) -> String {
        String::new()
    }
    pub(super) fn capture(_: &RegionSession) -> Result<image::RgbaImage> {
        Err(RegionError::new("region_platform_unsupported","当前平台尚不支持区域自动操作"))
    }
    pub(super) fn validate(_: &Target, _: Viewport) -> Result<()> {
        dpi_scope()
    }
    pub(super) fn neutral_input() -> Result<()> {
        dpi_scope()
    }
    pub(super) fn execute(_: &RegionSession, _: RegionAction) -> Result<()> {
        dpi_scope()
    }
    pub(super) fn release_button(_: &AtomicBool) {}
    pub(super) fn stop_requested() -> bool {
        false
    }
    #[cfg(test)]
    pub(super) fn fixture_target() -> Target {
        Target
    }
}

#[cfg(windows)]
#[path = "extension_region_windows.rs"]
mod platform;

#[cfg(target_os="macos")]
#[path="extension_region_macos.rs"]
mod platform;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_lease_stays_hidden_between_turns_and_restores_once_on_stop_or_unwind() {
        use std::sync::atomic::AtomicUsize;
        struct Hooks { hides: AtomicUsize, restores: AtomicUsize }
        impl RegionOverlayHooks for Hooks {
            fn validate_source(&self, _: &RegionBinding) -> std::result::Result<(), String> { Ok(()) }
            fn hide_owned_overlays(&self) -> std::result::Result<(), String> { self.hides.fetch_add(1, Ordering::SeqCst); Ok(()) }
            fn restore_owned_overlays(&self) { self.restores.fetch_add(1, Ordering::SeqCst); }
        }
        let hooks = Arc::new(Hooks { hides: AtomicUsize::new(0), restores: AtomicUsize::new(0) });
        let lease = HiddenOverlays::enter(hooks.clone()).unwrap();
        assert_eq!(hooks.hides.load(Ordering::SeqCst), 1);
        assert_eq!(hooks.restores.load(Ordering::SeqCst), 0);
        lease.restore(); lease.restore(); drop(lease);
        assert_eq!(hooks.restores.load(Ordering::SeqCst), 1);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lease = HiddenOverlays::enter(hooks.clone()).unwrap();
            panic!("binding failed");
        }));
        assert_eq!(hooks.hides.load(Ordering::SeqCst), 2);
        assert_eq!(hooks.restores.load(Ordering::SeqCst), 2);
    }

    fn source() -> RegionSource {
        RegionSource {
            scene_id: "scene".into(),
            asset_sha256: "a".repeat(64),
            background: Asset {
                id: "capture".into(),
                name: "Screen".into(),
                kind: AssetKind::Image,
                path: "not-read-by-pure-tests".into(),
                width: Some(1920),
                height: Some(1080),
                origin_x: Some(-1920),
                origin_y: Some(-120),
                scale_factor: Some(1.5),
                capture_coordinates: Some(CaptureCoordinates {
                    space: CaptureCoordinateSpace::PhysicalPixels,
                    origin_x: -1920.,
                    origin_y: -120.,
                    width: 1920.,
                    height: 1080.,
                }),
            },
            selection: Region {
                id: "region".into(),
                x: 100.2,
                y: 20.2,
                width: 200.4,
                height: 100.4,
                ..Default::default()
            },
        }
    }

    #[test]
    fn viewport_uses_physical_origin_without_reapplying_dpi_and_rounds_inward() {
        let (viewport, _) = Viewport::from_source(&source()).unwrap();
        assert_eq!(
            viewport,
            Viewport {
                x: -1819,
                y: -99,
                width: 199,
                height: 99
            }
        );
        assert_eq!(
            viewport.point(RegionPoint { x: 198, y: 98 }).unwrap(),
            (-1621, -1)
        );
        assert!(viewport.point(RegionPoint { x: 199, y: 0 }).is_err());
    }

    #[test]
    fn imported_derived_long_and_non_finite_regions_cannot_mint_authority() {
        let mut s = source();
        s.background.capture_coordinates = None;
        assert!(Viewport::from_source(&s).is_err());
        let mut s = source();
        s.selection.image_override = Some(s.background.clone());
        assert!(Viewport::from_source(&s).is_err());
        let mut s = source();
        s.background.height = Some(5000);
        assert!(Viewport::from_source(&s).is_err());
        for n in [f64::NAN, f64::INFINITY, -1.] {
            let mut s = source();
            s.selection.x = n;
            assert!(Viewport::from_source(&s).is_err());
        }
        let mut s = source();
        s.selection.x = 1919.;
        s.selection.width = 2.;
        assert!(Viewport::from_source(&s).is_err());
    }

    #[test]
    fn retina_region_maps_pixels_to_points_without_scaling_global_origin() {
        let mut source=source();
        source.background.capture_coordinates=Some(CaptureCoordinates {
            space:CaptureCoordinateSpace::QuartzPoints,origin_x:-960.,origin_y:-60.,width:960.,height:540.
        });
        let (v,b)=Viewport::from_source(&source).unwrap();
        assert_eq!((v.width,v.height),(199,99));
        assert_eq!(v.native_point(&b,RegionPoint{x:0,y:0}).unwrap(),(-909.5,-49.5));
        assert_eq!(v.native_point(&b,RegionPoint{x:198,y:98}).unwrap(),(-810.5,-0.5));
        assert_eq!(v.native_rect(&b).unwrap(),[-909.5,-49.5,-810.,0.]);
        assert!(v.native_point(&b,RegionPoint{x:199,y:0}).is_err());
    }

    #[test]
    fn drag_validates_every_point_not_just_endpoints_and_rejects_bad_text() {
        let (viewport, _) = Viewport::from_source(&source()).unwrap();
        let p = |x, y| RegionPoint { x, y };
        assert!(validate_action(
            viewport,
            &RegionAction::Drag {
                path: vec![p(1, 1), p(500, 1), p(2, 2)]
            }
        )
        .is_err());
        assert!(validate_action(
            viewport,
            &RegionAction::Drag {
                path: vec![p(1, 1), p(2, 2)]
            }
        )
        .is_ok());
        for text in ["", "a\n", "\u{0000}", "\u{001b}[2J"] {
            assert!(validate_action(viewport, &RegionAction::Type { text: text.into() }).is_err());
        }
        assert!(validate_action(
            viewport,
            &RegionAction::Type {
                text: "你好，region".into()
            }
        )
        .is_ok());
        assert!(serde_json::from_str::<RegionAction>(r#"{"kind":"key","key":"win"}"#).is_err());
        assert!(serde_json::from_str::<RegionAction>(
            r#"{"kind":"click","x":1,"y":2,"command":"shell"}"#
        )
        .is_err());
    }

    #[test]
    fn keyboard_focus_rect_must_be_inside_the_grant() {
        let v = Viewport {
            x: -30,
            y: 20,
            width: 100,
            height: 80,
        };
        assert!(v.contains_rect(-20, 30, 60, 99));
        assert!(!v.contains_rect(-40, 30, 60, 99));
        assert!(!v.contains_rect(-20, 30, 71, 99));
        assert!(!v.contains_rect(-20, 30, -20, 99));
    }

    #[test]
    fn revoked_session_does_not_hide_overlays_open_source_files_or_inject_input() {
        struct NoOperations;
        impl RegionOverlayHooks for NoOperations {
            fn validate_source(&self, _: &RegionBinding) -> std::result::Result<(), String> {
                panic!("must not inspect a revoked source")
            }
            fn hide_owned_overlays(&self) -> std::result::Result<(), String> {
                panic!("must not hide overlays after stop")
            }
            fn restore_owned_overlays(&self) {
                panic!("no overlays were hidden")
            }
        }
        let (viewport, binding) = Viewport::from_source(&source()).unwrap();
        let session = RegionSession {
            info: RegionSessionInfo {
                id: "revoked-fixture".into(),
                width: viewport.width,
                height: viewport.height,
                binding,
                capabilities: vec![],
                target: String::new(),
            },
            source_path: "must-not-read".into(),
            viewport,
            topology: vec![],
            target: platform::fixture_target(),
            hooks: Arc::new(NoOperations),
            overlays: HiddenOverlays { hooks: Arc::new(NoOperations), hidden: AtomicBool::new(false) },
            created: Instant::now(),
            revoked: AtomicBool::new(false),
            owns_slot: AtomicBool::new(false),
            button_down: AtomicBool::new(false),
            observed: AtomicBool::new(true),
            operation: Mutex::new(()),
        };
        session.revoke();
        assert_eq!(
            session.observe().err().unwrap().code,
            "region_authority_revoked"
        );
        assert_eq!(
            session
                .execute(RegionAction::Click { x: 1, y: 1 })
                .err()
                .unwrap()
                .code,
            "region_authority_revoked"
        );
        assert_eq!(
            session
                .execute(RegionAction::Type {
                    text: "do not type".into()
                })
                .err()
                .unwrap()
                .code,
            "region_authority_revoked"
        );
    }
}
