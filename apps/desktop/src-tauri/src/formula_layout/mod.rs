// SPDX-License-Identifier: MPL-2.0
//! Native formula worker. No Tauri commands, engine lock, model call or database writes.
//! Only the host can supply Resources and turn this verified result into a core layout.
mod embedded;
mod fonts;
mod protocol;
mod render;
mod supervise;

pub use protocol::{FontIdentity, StyleIdentity};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub const RENDERER_ID: &str =
    "mewu.ratex-svg-path.resvg72.v1@776c1d37bafa3bf445a0ab9377c55fe77f7a0133+resvg-0.48.1";
pub(crate) const MAX_INPUT_CHARS: usize = 2048;
pub(crate) const MAX_SVG: usize = 1024 * 1024;
pub(crate) const MAX_SIDE: u32 = 4096;
pub(crate) const MAX_PIXELS: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_PNG: usize = 8 * 1024 * 1024;
pub(crate) const MAX_CONFIG: usize = 32 * 1024;
pub(crate) const MAX_FRAME: usize = 64 * 1024;

/// Adapt the already validated RichContent::Formula variant, never old text/cell strings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormulaInput {
    pub version: u8,
    pub tex: String,
    pub color: String,
    pub font_size: f64,
}
impl FormulaInput {
    pub fn validate(&self) -> Result<(), String> {
        let c = self.color.as_bytes();
        if self.version != 1
            || self.tex.is_empty()
            || self.tex.chars().count() > MAX_INPUT_CHARS
            || self
                .tex
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err("formula_input_budget".into());
        }
        if c.len() != 7
            || c[0] != b'#'
            || !c[1..].iter().all(u8::is_ascii_hexdigit)
            || !self.font_size.is_finite()
            || !(8.0..=128.0).contains(&self.font_size)
        {
            return Err("formula_style_invalid".into());
        }
        Ok(())
    }
}

/// Construct at startup from app-owned resources, an installed primary font and
/// app-owned cache root. None of these paths belong in model/frontend input.
#[derive(Clone, Debug)]
pub struct Resources {
    pub(crate) font_directory: PathBuf,
    pub(crate) unicode_font: PathBuf,
    pub(crate) work_root: PathBuf,
}
impl Resources {
    pub fn new(
        font_directory: PathBuf,
        unicode_font: PathBuf,
        work_root: PathBuf,
    ) -> Result<Self, String> {
        for path in [&font_directory, &unicode_font, &work_root] {
            if !path.is_absolute() {
                return Err("formula_resources_path".into());
            }
        }
        // Per-job validation/pinning is authoritative; no result is cached by path alone.
        Ok(Self {
            font_directory,
            unicode_font,
            work_root,
        })
    }
    /// Call lazily on the blocking rendering path, not at application startup.
    /// The 19 fonts are part of the binary; no release-side directory required.
    pub fn from_embedded(app_owned_root: PathBuf) -> Result<Self, String> {
        embedded::prepare(app_owned_root)
    }
}

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    active: AtomicBool,
    worker_started: AtomicBool,
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<CancellationState>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    /// Includes preparation and reaper-held real child/Job/IO lifetime, so an
    /// exit ACK must not treat an aborted invoke Future as a released worker.
    pub fn is_active(&self) -> bool {
        self.0.active.load(Ordering::Acquire)
    }
    /// Observation for native lifecycle/tests, not a product UI state.
    pub fn has_started_worker(&self) -> bool {
        self.0.worker_started.load(Ordering::Acquire)
    }
}
struct CancelOnDrop {
    token: Cancellation,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.token.cancel();
        }
    }
}

/// Deliberately no Deserialize or Serialize. Bytes have been read after actual
/// child/Job/IO exit, bounded, hashed, decoded and checked by the parent process.
pub struct FormulaLayout {
    input: FormulaInput,
    fonts: FontIdentity,
    style: StyleIdentity,
    width: u32,
    height: u32,
    png: Vec<u8>,
}
pub struct LayoutParts {
    pub input: FormulaInput,
    pub renderer_id: &'static str,
    pub font_identity: String,
    pub style_identity: String,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}
impl FormulaLayout {
    pub fn into_parts(self) -> Result<LayoutParts, String> {
        Ok(LayoutParts {
            input: self.input,
            renderer_id: RENDERER_ID,
            font_identity: serde_json::to_string(&self.fonts)
                .map_err(|_| "formula_identity_encode")?,
            style_identity: serde_json::to_string(&self.style)
                .map_err(|_| "formula_identity_encode")?,
            width: self.width,
            height: self.height,
            png: self.png,
        })
    }
}

/// Dropping the async caller cancels the detached blocking operation, but does
/// NOT release its actual child slot. Retain Cancellation in the run registry
/// for plugin-stop, source-change and prepare-exit cancellation as well.
pub async fn render_formula(
    input: FormulaInput,
    resources: Resources,
    cancel: Cancellation,
) -> Result<FormulaLayout, String> {
    input.validate()?;
    let mut guard = CancelOnDrop {
        token: cancel.clone(),
        armed: true,
    };
    let result = tokio::task::spawn_blocking(move || supervise::run(input, resources, cancel))
        .await
        .map_err(|_| "formula_host_interrupted")?;
    guard.armed = false;
    result
}

/// Synchronous VisualScope callers must run this outside Engine/plugin locks,
/// on their existing blocking worker. Caller retains and registers `cancel`.
pub fn render_formula_blocking(
    input: FormulaInput,
    resources: Resources,
    cancel: Cancellation,
) -> Result<FormulaLayout, String> {
    supervise::run(input, resources, cancel)
}
pub fn render_formula_embedded_blocking(
    input: FormulaInput,
    app_owned_root: PathBuf,
    cancel: Cancellation,
) -> Result<FormulaLayout, String> {
    supervise::run_embedded(input, app_owned_root, cancel)
}
/// First-use embedding preparation and rendering both run off the UI/async
/// executor thread. A normally completed call leaves a batch token usable.
pub async fn render_formula_embedded(
    input: FormulaInput,
    app_owned_root: PathBuf,
    cancel: Cancellation,
) -> Result<FormulaLayout, String> {
    input.validate()?;
    let mut guard = CancelOnDrop {
        token: cancel.clone(),
        armed: true,
    };
    let result =
        tokio::task::spawn_blocking(move || supervise::run_embedded(input, app_owned_root, cancel))
            .await
            .map_err(|_| "formula_host_interrupted")?;
    guard.armed = false;
    result
}
/// Native prepare-exit: first close the host request gate, cancel registered
/// VisualScopes, then signal this worker. Poll worker_is_active before ACK.
pub fn cancel_active_worker() -> bool {
    supervise::cancel_active()
}
pub fn worker_is_active() -> bool {
    supervise::active()
}
/// Application shutdown only: closes this module's request gate and
/// cancels all registered queued/running requests. Plugin/source cancellation
/// uses its fresh Cancellation token instead and does not close the module.
pub fn cancel_all() -> usize {
    supervise::cancel_all()
}
/// Call on the native exit worker after cancel_all, never while holding Engine
/// or plugin locks. A timeout leaves the real process permit/reaper owned.
pub fn wait_until_idle(timeout: std::time::Duration) -> bool {
    supervise::wait_until_idle(timeout)
}
/// Root lifecycle abort must first restore Host::RUNNING, then call this. It
/// never un-cancels an old token or releases a live worker/reaper permit.
pub fn resume_after_failed_exit() {
    supervise::resume_after_failed_exit();
}
/// Merge these complete notices into the existing third-party document.
pub fn license_documents() -> [&'static str; 4] {
    [
        include_str!("../../resources/formula/licenses/LICENSE"),
        include_str!("../../resources/formula/licenses/THIRD_PARTY_NOTICES.txt"),
        include_str!("../../resources/formula/licenses/KaTeX-fonts-NOTICE.txt"),
        include_str!("../../resources/formula/licenses/SIL-OFL-1.1.txt"),
    ]
}

/// Call before any Tauri initialization or single-instance logic in main.
pub fn worker_main() -> i32 {
    if std::panic::catch_unwind(supervise::worker).is_ok_and(|r| r.is_ok()) {
        0
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_formula_rejects_bad_style_and_old_text_shape() {
        let mut input = FormulaInput {
            version: 1,
            tex: r"\frac{1}{2}".into(),
            color: "#123abc".into(),
            font_size: 40.0,
        };
        assert!(input.validate().is_ok());
        input.font_size = f64::NAN;
        assert!(input.validate().is_err());
        input.font_size = 40.0;
        input.color = "url(a)".into();
        assert!(input.validate().is_err());
        assert!(serde_json::from_str::<FormulaInput>(
            r##"{"version":1,"tex":"x","color":"#000000","fontSize":40,"svg":"<svg/>"}"##
        )
        .is_err());
        assert!(serde_json::from_str::<FormulaInput>(r#"{"text":"x"}"#).is_err());
    }
}
