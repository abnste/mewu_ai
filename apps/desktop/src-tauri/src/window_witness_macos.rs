// SPDX-License-Identifier: MPL-2.0
//! Bounded CoreGraphics witnesses for first-party windows.
//!
//! Resolve handles without host/store locks. Revoke the owner on Destroyed,
//! before taking those locks, and before permitting a replacement of its label.
//! Quartz names can be absent or differ from AppKit titles; those cases fail
//! closed. PID/title/CGWindowID are observations, not a creation fingerprint.
#![cfg(target_os = "macos")]

use core_foundation::{
    array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef},
    base::{CFGetTypeID, CFRange, CFType, CFTypeID, CFTypeRef, TCFType},
    dictionary::{CFDictionaryGetTypeID, CFDictionaryGetValue, CFDictionaryRef},
    number::{
        kCFNumberSInt64Type, CFBooleanGetTypeID, CFBooleanGetValue, CFNumberGetTypeID,
        CFNumberGetValue, CFNumberIsFloatType,
    },
    string::{
        kCFStringEncodingUTF8, CFStringGetBytes, CFStringGetLength, CFStringGetTypeID, CFStringRef,
    },
};
use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::{Mutex, OnceLock},
};
use tauri::WebviewWindow;

const MAX_WINDOWS: usize = 4096;
const MAX_OWNERS: usize = 64;
const MAX_LABEL_BYTES: usize = 256;
const MAX_TITLE_BYTES: usize = 4096;
const UNAVAILABLE: &str = "窗口身份不可用，请重新打开空间";
const OPTION_ALL: u32 = 0;
const OPTION_ON_SCREEN_ONLY: u32 = 1;
const OPTION_ON_SCREEN_BELOW_WINDOW: u32 = 1 << 2;
const OPTION_INCLUDING_WINDOW: u32 = 1 << 3;

// Copy returns owned CF objects; only dictionary kCGWindowNumber supplies IDs.
// https://developer.apple.com/documentation/coregraphics/cgwindowlistcopywindowinfo(_:_:)
// https://developer.apple.com/documentation/coregraphics/kcgwindownumber
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(options: u32, relative_to_window: u32) -> CFArrayRef;
    static kCGWindowNumber: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowName: CFStringRef;
    static kCGWindowIsOnscreen: CFStringRef;
    #[cfg(debug_assertions)]
    static kCGWindowLayer: CFStringRef;
}

#[cfg(debug_assertions)]
fn diagnostics_enabled() -> bool {
    std::env::var("MEWU_MAC_WINDOW_DIAGNOSTICS").is_ok_and(|value| value == "1")
}

/// Preserve the original error and policy; diagnostics never supply identity.
fn diagnostic_stage<T>(
    _label: &str,
    _phase: &str,
    _step: &str,
    result: Result<T, String>,
) -> Result<T, String> {
    result.map_err(|error| {
        #[cfg(debug_assertions)]
        if diagnostics_enabled() {
            eprintln!(
                "[mewu-window-witness] own_pid={} label={_label:?} phase={_phase} step={_step} error={error:?}",
                std::process::id()
            );
        }
        error
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeIdentity {
    id: u32,
    pid: i32,
    title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Witness {
    native: NativeIdentity,
    label: String,
    generation: u64,
}

/// Admission for observing a capture window, never an action authorization.
/// A window that starts hidden may not yet have a Quartz window number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureWitness {
    label: String,
    generation: u64,
    pid: i32,
    title: String,
    native_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CaptureObservation {
    id: u32,
    onscreen: bool,
}

#[derive(Default)]
struct Owner {
    generation: u64,
    current: Option<usize>,
}

#[derive(Default)]
struct Registry {
    last_token: usize,
    owners: HashMap<String, Owner>,
    witnesses: HashMap<usize, Witness>,
    failed: bool,
}

impl Registry {
    fn fail(&mut self) -> String {
        self.failed = true;
        self.witnesses.clear();
        UNAVAILABLE.into()
    }

    fn begin(&mut self, label: &str) -> Result<u64, String> {
        if self.failed || !valid_text(label, MAX_LABEL_BYTES) {
            return Err(UNAVAILABLE.into());
        }
        if !self.owners.contains_key(label) && self.owners.len() >= MAX_OWNERS {
            return Err(self.fail());
        }
        Ok(self.owners.entry(label.into()).or_default().generation)
    }

    fn revoke(&mut self, label: &str) {
        if self.failed || !self.owners.contains_key(label) {
            return;
        }
        let owner = self.owners.get_mut(label).unwrap();
        if let Some(token) = owner.current.take() {
            self.witnesses.remove(&token);
        }
        match owner.generation.checked_add(1) {
            Some(next) => owner.generation = next,
            None => {
                self.fail();
            }
        }
    }

    fn resolve(
        &mut self,
        label: &str,
        generation: u64,
        native: NativeIdentity,
    ) -> Result<usize, String> {
        if self.failed {
            return Err(UNAVAILABLE.into());
        }
        let owner = self.owners.get(label).ok_or(UNAVAILABLE)?;
        if owner.generation != generation {
            return Err(UNAVAILABLE.into());
        }
        if let Some(token) = owner.current {
            if self
                .witnesses
                .get(&token)
                .is_some_and(|witness| witness.generation == generation && witness.native == native)
            {
                return Ok(token);
            }
            // A changed ID or title never rebinds an old token to the new window.
            self.revoke(label);
            if self.failed {
                return Err(UNAVAILABLE.into());
            }
        }
        let token = self.last_token.checked_add(1).ok_or_else(|| self.fail())?;
        self.last_token = token;
        let owner = self.owners.get_mut(label).ok_or(UNAVAILABLE)?;
        let witness = Witness {
            native,
            label: label.into(),
            generation: owner.generation,
        };
        owner.current = Some(token);
        self.witnesses.insert(token, witness);
        Ok(token)
    }

    fn get(&self, token: usize) -> Option<&Witness> {
        if self.failed || token == 0 {
            return None;
        }
        let witness = self.witnesses.get(&token)?;
        let owner = self.owners.get(&witness.label)?;
        (owner.current == Some(token) && owner.generation == witness.generation).then_some(witness)
    }

    fn capture_current(&self, witness: &CaptureWitness) -> bool {
        !self.failed
            && self
                .owners
                .get(&witness.label)
                .is_some_and(|owner| owner.generation == witness.generation)
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

/// May query Tauri's native title; call only where no host/store locks are held.
/// Returns a stable token for repeated calls on the same active native witness.
pub(crate) fn window_handle(window: &WebviewWindow) -> Result<usize, String> {
    let label = window.label().to_owned();
    if !valid_text(&label, MAX_LABEL_BYTES) {
        return Err(UNAVAILABLE.into());
    }
    // Register the admission before querying the UI, so a Destroyed hook can
    // invalidate it while title/WindowServer observations are in flight.
    let generation = registry().lock().map_err(|_| UNAVAILABLE)?.begin(&label)?;
    let title = window.title().map_err(|_| UNAVAILABLE)?;
    if !valid_text(&title, MAX_TITLE_BYTES) {
        return Err(UNAVAILABLE.into());
    }
    let pid = i32::try_from(std::process::id()).map_err(|_| UNAVAILABLE)?;
    let list = WindowList::copy(OPTION_ALL, 0)?;
    let mut candidate = None;
    for index in 0..list.count {
        let entry = list.entry(index)?;
        if entry.pid()? != pid {
            continue;
        }
        let Some(name) = entry.title()? else {
            continue;
        };
        if name != title {
            continue;
        }
        if candidate.is_some() {
            return Err("窗口标题不唯一，无法确认空间窗口".into());
        }
        let identity = NativeIdentity {
            id: entry.id()?,
            pid,
            title: name,
        };
        if !entry.onscreen()? {
            return Err("窗口已关闭或收起".into());
        }
        candidate = Some(identity);
    }
    let native = candidate.ok_or(UNAVAILABLE)?;
    // CF enumeration and title access never run while the registry is locked.
    drop(list);
    registry()
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .resolve(&label, generation, native)
}

/// Acquire before hiding, without host/store locks: title() may visit the UI.
/// Hidden or not-yet-listed windows are admitted without granting a code token.
pub(crate) fn capture_witness(window: &WebviewWindow) -> Result<CaptureWitness, String> {
    let label = window.label().to_owned();
    let generation = {
        let mut state = diagnostic_stage(
            &label,
            "admit",
            "registry.lock",
            registry().lock().map_err(|_| UNAVAILABLE.into()),
        )?;
        diagnostic_stage(&label, "admit", "registry.begin", state.begin(&label))?
    };
    let title = diagnostic_stage(
        &label,
        "admit",
        "tauri.title",
        window.title().map_err(|_| UNAVAILABLE.into()),
    )?;
    if !valid_text(&title, MAX_TITLE_BYTES) {
        return diagnostic_stage(&label, "admit", "title.validate", Err(UNAVAILABLE.into()));
    }
    let pid = i32::try_from(std::process::id()).map_err(|_| UNAVAILABLE)?;
    let mut witness = CaptureWitness {
        label,
        generation,
        pid,
        title,
        native_id: None,
    };
    // All includes offscreen windows; neither an empty list nor an offscreen
    // entry grants action authority. Only the checked dictionary supplies IDs.
    let list = diagnostic_stage(
        &witness.label,
        "admit.all",
        "list.copy",
        WindowList::copy(OPTION_ALL, 0),
    )?;
    witness.native_id = capture_match(&list, &witness, false, "admit.all")?.map(|entry| entry.id);
    drop(list);
    if !registry()
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .capture_current(&witness)
    {
        return diagnostic_stage(
            &witness.label,
            "admit",
            "generation.after",
            Err(UNAVAILABLE.into()),
        );
    }
    Ok(witness)
}

/// The admitted ID, if known and not revoked. This is an observation, not a
/// capability: callers must also verify hidden() and the filter's live owner.
pub(crate) fn window_id(witness: &CaptureWitness) -> Option<u32> {
    registry()
        .lock()
        .ok()
        .filter(|state| state.capture_current(witness))
        .and(witness.native_id)
}

/// Pure generation fence; safe during final Store adoption because it neither
/// queries WindowServer nor dispatches the main loop.
pub(crate) fn current(witness: &CaptureWitness) -> Result<(), String> {
    if registry()
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .capture_current(witness)
    {
        Ok(())
    } else {
        Err(UNAVAILABLE.into())
    }
}

/// Build a filter hint from the admitted identity, never a fresh UI title.
/// The capture backend must still check the live SCWindow's PID/ID/title.
pub(crate) fn exclusion(
    witness: &CaptureWitness,
) -> Result<Option<crate::capture_macos::ExcludedWindow>, String> {
    if !registry()
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .capture_current(witness)
    {
        return Err(UNAVAILABLE.into());
    }
    let excluded = witness
        .native_id
        .map(|id| crate::capture_macos::ExcludedWindow {
            id,
            title: witness.title.clone(),
        });
    // Construct outside the registry lock, then reject a Destroyed/replacement
    // that occurred in the meantime, including admissions with no Quartz ID.
    if !registry()
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .capture_current(witness)
    {
        return Err(UNAVAILABLE.into());
    }
    Ok(excluded)
}

/// Fresh WindowServer verification, without dispatching to the UI thread.
/// A queued Tauri hide() alone must never be treated as this result.
pub(crate) fn hidden(witness: &CaptureWitness) -> Result<bool, String> {
    if !registry()
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .capture_current(witness)
    {
        return Err(UNAVAILABLE.into());
    }
    // Check even offscreen reuse of the old number before querying the current
    // onscreen set. A missing old ID is safe only if no matching owner is shown.
    let all = diagnostic_stage(
        &witness.label,
        "hidden.all",
        "list.copy",
        WindowList::copy(OPTION_ALL, 0),
    )?;
    capture_match(&all, witness, false, "hidden.all")?;
    drop(all);
    // Apple specifies NullWindowID for OnScreenOnly. The last observation is
    // the onscreen set, so a completed hide between these calls can succeed.
    // https://developer.apple.com/documentation/coregraphics/cgwindowlistoption/optiononscreenonly
    let onscreen = diagnostic_stage(
        &witness.label,
        "hidden.onscreen",
        "list.copy",
        WindowList::copy(OPTION_ON_SCREEN_ONLY, 0),
    )?;
    let observed_hidden = capture_match(&onscreen, witness, true, "hidden.onscreen")?.is_none();
    drop(onscreen);
    if !registry()
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .capture_current(witness)
    {
        return Err(UNAVAILABLE.into());
    }
    Ok(observed_hidden)
}

fn capture_match(
    list: &WindowList,
    witness: &CaptureWitness,
    onscreen_only: bool,
    phase: &str,
) -> Result<Option<CaptureObservation>, String> {
    let mut scan = CaptureScan::new(witness, onscreen_only);
    #[cfg(debug_assertions)]
    let mut diagnostic_rows = 0;
    for index in 0..list.count {
        let entry = diagnostic_stage(&witness.label, phase, "entry.dictionary", list.entry(index))?;
        let id = diagnostic_stage(&witness.label, phase, "entry.id", entry.id())?;
        let pid = diagnostic_stage(&witness.label, phase, "entry.pid", entry.pid())?;
        if pid != witness.pid && Some(id) != witness.native_id {
            continue;
        }
        #[cfg(debug_assertions)]
        if pid == witness.pid && diagnostic_rows < MAX_OWNERS {
            diagnostic_own_entry(witness, phase, id, pid, &entry);
            diagnostic_rows += 1;
        }
        let title = diagnostic_stage(&witness.label, phase, "entry.title", entry.title())?;
        let onscreen = diagnostic_stage(&witness.label, phase, "entry.onscreen", entry.onscreen())?;
        diagnostic_stage(
            &witness.label,
            phase,
            "match.consider",
            scan.consider(id, pid, title.as_deref(), onscreen),
        )?;
    }
    Ok(scan.candidate)
}

#[cfg(debug_assertions)]
fn diagnostic_own_entry(
    witness: &CaptureWitness,
    phase: &str,
    id: u32,
    pid: i32,
    entry: &Dictionary<'_>,
) {
    if !diagnostics_enabled() || u32::try_from(pid).ok() != Some(std::process::id()) {
        return;
    }
    // Only current-process metadata is inspected here. No title content, owner
    // name, bounds, or foreign/reused owner's metadata is logged.
    let raw_type = |value: CFTypeRef| {
        // SAFETY: Values are borrowed from the retained, checked dictionary.
        (!value.is_null()).then(|| unsafe { CFGetTypeID(value) })
    };
    // SAFETY: Public, linked CoreGraphics keys are immortal CFStrings.
    let (name, onscreen, layer) = unsafe {
        (
            entry.value(kCGWindowName),
            entry.value(kCGWindowIsOnscreen),
            entry.value(kCGWindowLayer),
        )
    };
    let title = entry.title();
    let parsed_title = title.as_ref().ok().and_then(|value| value.as_deref());
    eprintln!(
        "[mewu-window-witness] own_pid={pid} label={:?} phase={phase} generation={} target_id={:?} id={id} has_title={} title_ok={} title_bytes={:?} matches_target={} title_cf_type={:?} onscreen={:?} onscreen_cf_type={:?} cf_boolean_type={} layer={:?} layer_cf_type={:?}",
        witness.label,
        witness.generation,
        witness.native_id,
        !name.is_null(),
        title.is_ok(),
        parsed_title.map(str::len),
        parsed_title == Some(witness.title.as_str()),
        raw_type(name),
        entry.onscreen().ok(),
        raw_type(onscreen),
        // SAFETY: Public CF type ID getter takes no arguments.
        unsafe { CFBooleanGetTypeID() },
        integer(layer).ok(),
        raw_type(layer),
    );
}

struct CaptureScan<'a> {
    witness: &'a CaptureWitness,
    onscreen_only: bool,
    candidate: Option<CaptureObservation>,
}

impl<'a> CaptureScan<'a> {
    fn new(witness: &'a CaptureWitness, onscreen_only: bool) -> Self {
        Self {
            witness,
            onscreen_only,
            candidate: None,
        }
    }

    fn consider(
        &mut self,
        id: u32,
        pid: i32,
        title: Option<&str>,
        onscreen: bool,
    ) -> Result<(), String> {
        let known_id = self.witness.native_id == Some(id);
        if known_id && (pid != self.witness.pid || title != Some(self.witness.title.as_str())) {
            return Err("截图窗口编号已被替换，请重新打开空间".into());
        }
        if pid != self.witness.pid {
            return Ok(());
        }
        // Quartz names are optional. An unnamed onscreen window belonging to
        // us could be the capture window, so its absence cannot prove hidden.
        // https://developer.apple.com/documentation/coregraphics/kcgwindowname
        if title.is_none_or(str::is_empty) {
            return if onscreen {
                Err(UNAVAILABLE.into())
            } else {
                Ok(())
            };
        }
        if title != Some(self.witness.title.as_str()) {
            return Ok(());
        }
        if self.candidate.is_some() {
            return Err("窗口标题不唯一，无法确认截图窗口".into());
        }
        if self
            .witness
            .native_id
            .is_some_and(|expected| expected != id)
        {
            return Err("截图窗口身份已改变，请重新打开空间".into());
        }
        if self.onscreen_only && !onscreen {
            return Err(UNAVAILABLE.into());
        }
        self.candidate = Some(CaptureObservation { id, onscreen });
        Ok(())
    }
}

/// Performs a fresh WindowServer query without dispatching to the UI thread.
pub(crate) fn visible(token: usize) -> bool {
    let witness = match registry().lock() {
        Ok(state) => match state.get(token) {
            Some(witness) => witness.clone(),
            None => return false,
        },
        Err(_) => return false,
    };
    let observed = observe(&witness.native).unwrap_or(false);
    let Ok(mut state) = registry().lock() else {
        return false;
    };
    if state.get(token) != Some(&witness) {
        return false;
    }
    if !observed {
        state.revoke(&witness.label);
    }
    observed
}

/// Call at the start of the owner's Destroyed hook, before taking host locks.
pub(crate) fn revoke_owner(label: &str) {
    if let Ok(mut state) = registry().lock() {
        state.revoke(label);
    }
}

fn observe(native: &NativeIdentity) -> Result<bool, String> {
    // Apple requires IncludingWindow to accompany an above/below list option.
    // https://developer.apple.com/documentation/coregraphics/cgwindowlistoption/optionincludingwindow
    let list = WindowList::copy(
        OPTION_INCLUDING_WINDOW | OPTION_ON_SCREEN_BELOW_WINDOW,
        native.id,
    )?;
    let mut found = None;
    for index in 0..list.count {
        let entry = list.entry(index)?;
        if entry.id()? != native.id {
            continue;
        }
        if found.is_some() {
            return Err(UNAVAILABLE.into());
        }
        found = Some(
            entry.pid()? == native.pid
                && entry.title()?.as_deref() == Some(native.title.as_str())
                && entry.onscreen()?,
        );
    }
    Ok(found.unwrap_or(false))
}

struct WindowList {
    _owned: CFType,
    raw: CFArrayRef,
    count: usize,
}

impl WindowList {
    fn copy(options: u32, window_id: u32) -> Result<Self, String> {
        // Metadata permission is checked only; this module never prompts.
        crate::capture_permission_macos::preflight()?;
        // SAFETY: Public API with bounded scalar arguments; Copy owns its result.
        let raw = unsafe { CGWindowListCopyWindowInfo(options, window_id) };
        if raw.is_null() {
            return Err(UNAVAILABLE.into());
        }
        // SAFETY: Nonnull Copy result follows the CF Create Rule. CFType releases
        // it exactly once on every success/error path; children remain borrowed.
        let owned = unsafe { CFType::wrap_under_create_rule(raw.cast()) };
        // SAFETY: The retained object is a live CF object; check before array use.
        if unsafe { CFGetTypeID(owned.as_CFTypeRef()) != CFArrayGetTypeID() } {
            return Err(UNAVAILABLE.into());
        }
        // SAFETY: Runtime type has been checked and owned keeps the array alive.
        let count = usize::try_from(unsafe { CFArrayGetCount(raw) }).map_err(|_| UNAVAILABLE)?;
        if count > MAX_WINDOWS {
            return Err("窗口信息超过当前限制".into());
        }
        Ok(Self {
            _owned: owned,
            raw,
            count,
        })
    }

    fn entry(&self, index: usize) -> Result<Dictionary<'_>, String> {
        if index >= self.count {
            return Err(UNAVAILABLE.into());
        }
        // SAFETY: index is bounded by the checked native count and list is alive.
        let raw = unsafe { CFArrayGetValueAtIndex(self.raw, index as isize) };
        // SAFETY: A nonnull CF array element is a live CF object.
        require_type(raw, unsafe { CFDictionaryGetTypeID() })?;
        Ok(Dictionary {
            raw: raw.cast(),
            _list: PhantomData,
        })
    }
}

struct Dictionary<'a> {
    raw: CFDictionaryRef,
    _list: PhantomData<&'a WindowList>,
}

impl Dictionary<'_> {
    fn value(&self, key: CFStringRef) -> CFTypeRef {
        // SAFETY: Dictionary type/lifetime checked by entry(); framework keys
        // are immortal CFStrings. No returned value is retained or released.
        unsafe { CFDictionaryGetValue(self.raw, key.cast()) }
    }

    fn id(&self) -> Result<u32, String> {
        // SAFETY: Linked public CoreGraphics CFString constant.
        let id = u32::try_from(integer(self.value(unsafe { kCGWindowNumber }))?)
            .map_err(|_| UNAVAILABLE)?;
        (id != 0).then_some(id).ok_or_else(|| UNAVAILABLE.into())
    }

    fn pid(&self) -> Result<i32, String> {
        // SAFETY: Linked public CoreGraphics CFString constant.
        let pid = i32::try_from(integer(self.value(unsafe { kCGWindowOwnerPID }))?)
            .map_err(|_| UNAVAILABLE)?;
        (pid >= 0).then_some(pid).ok_or_else(|| UNAVAILABLE.into())
    }

    fn title(&self) -> Result<Option<String>, String> {
        // SAFETY: Linked public CoreGraphics CFString constant.
        let value = self.value(unsafe { kCGWindowName });
        if value.is_null() {
            return Ok(None);
        }
        bounded_string(value).map(Some)
    }

    fn onscreen(&self) -> Result<bool, String> {
        // SAFETY: Linked public constant and runtime CFBoolean type check.
        let value = self.value(unsafe { kCGWindowIsOnscreen });
        require_type(value, unsafe { CFBooleanGetTypeID() })?;
        // SAFETY: Runtime type checked and dictionary keeps the value alive.
        Ok(unsafe { CFBooleanGetValue(value.cast()) })
    }
}

fn require_type(value: CFTypeRef, expected: CFTypeID) -> Result<(), String> {
    // SAFETY: Callers provide borrowed CF objects from retained native containers.
    if value.is_null() || unsafe { CFGetTypeID(value) } != expected {
        Err(UNAVAILABLE.into())
    } else {
        Ok(())
    }
}

fn integer(value: CFTypeRef) -> Result<i64, String> {
    // SAFETY: Public type ID getter takes no arguments.
    require_type(value, unsafe { CFNumberGetTypeID() })?;
    // SAFETY: Type checked. Float values and inexact conversions are rejected.
    if unsafe { CFNumberIsFloatType(value.cast()) } != 0 {
        return Err(UNAVAILABLE.into());
    }
    let mut output = 0i64;
    // SAFETY: output is correctly sized/aligned for the requested SInt64 type.
    if unsafe {
        CFNumberGetValue(
            value.cast(),
            kCFNumberSInt64Type,
            (&mut output as *mut i64).cast(),
        )
    } {
        Ok(output)
    } else {
        Err(UNAVAILABLE.into())
    }
}

fn bounded_string(value: CFTypeRef) -> Result<String, String> {
    // SAFETY: Public type ID getter takes no arguments.
    require_type(value, unsafe { CFStringGetTypeID() })?;
    // SAFETY: Runtime string type checked and caller retains its parent list.
    let length = unsafe { CFStringGetLength(value.cast()) };
    if length < 0 || length as usize > MAX_TITLE_BYTES {
        return Err(UNAVAILABLE.into());
    }
    let mut bytes = [0u8; MAX_TITLE_BYTES];
    let mut used = 0isize;
    // SAFETY: Full checked UTF-16 range, fixed output buffer and valid out pointer.
    // GetBytes preserves embedded NULs, unlike truncating a C-string conversion.
    let converted = unsafe {
        CFStringGetBytes(
            value.cast(),
            CFRange {
                location: 0,
                length,
            },
            kCFStringEncodingUTF8,
            0,
            0,
            bytes.as_mut_ptr(),
            bytes.len() as isize,
            &mut used,
        )
    };
    let used = usize::try_from(used).map_err(|_| UNAVAILABLE)?;
    if converted != length || used > bytes.len() {
        return Err(UNAVAILABLE.into());
    }
    let output = String::from_utf8(bytes[..used].to_vec()).map_err(|_| UNAVAILABLE)?;
    // An unrelated untitled window may legitimately carry an empty CFString;
    // it cannot match the nonempty title required by window_handle().
    if output.len() > MAX_TITLE_BYTES || output.chars().any(char::is_control) {
        return Err(UNAVAILABLE.into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(id: u32) -> NativeIdentity {
        NativeIdentity {
            id,
            pid: 42,
            title: "Mewu".into(),
        }
    }

    fn capture(generation: u64, native_id: Option<u32>) -> CaptureWitness {
        CaptureWitness {
            label: "space".into(),
            generation,
            pid: 42,
            title: "Mewu".into(),
            native_id,
        }
    }

    #[test]
    fn capture_admission_never_grants_action_authority_and_destroyed_invalidates_it() {
        let mut state = Registry::default();
        let generation = state.begin("space").unwrap();
        let unlisted = capture(generation, None);
        let listed = capture(generation, Some(10));
        assert!(state.capture_current(&unlisted));
        assert!(state.capture_current(&listed));
        assert!(state.witnesses.is_empty());
        assert!(state.get(1).is_none());
        state.revoke("space");
        assert!(!state.capture_current(&unlisted));
        assert!(!state.capture_current(&listed));
        let replacement = capture(state.begin("space").unwrap(), Some(10));
        assert!(state.capture_current(&replacement));
        assert!(!state.capture_current(&listed));
    }

    #[test]
    fn action_identity_replacement_also_revokes_old_capture_admission() {
        let mut state = Registry::default();
        let generation = state.begin("space").unwrap();
        state.resolve("space", generation, native(10)).unwrap();
        let old_capture = capture(generation, Some(10));
        assert!(state.capture_current(&old_capture));
        state.resolve("space", generation, native(11)).unwrap();
        assert!(!state.capture_current(&old_capture));
    }

    #[test]
    fn unlisted_capture_is_hidden_until_its_own_exact_title_is_onscreen() {
        let witness = capture(0, None);
        let mut scan = CaptureScan::new(&witness, true);
        scan.consider(20, 42, Some("Mewu Settings"), true).unwrap();
        scan.consider(30, 84, Some("Mewu"), true).unwrap();
        assert!(scan.candidate.is_none());
        scan.consider(10, 42, Some("Mewu"), true).unwrap();
        assert_eq!(
            scan.candidate,
            Some(CaptureObservation {
                id: 10,
                onscreen: true
            })
        );
    }

    #[test]
    fn offscreen_admission_retains_actual_id_and_reuse_fails_even_offscreen() {
        let witness = capture(0, Some(10));
        let mut scan = CaptureScan::new(&witness, false);
        scan.consider(10, 42, Some("Mewu"), false).unwrap();
        assert_eq!(
            scan.candidate,
            Some(CaptureObservation {
                id: 10,
                onscreen: false
            })
        );
        for (pid, title) in [(84, Some("Mewu")), (42, Some("Settings")), (42, None)] {
            let mut reused = CaptureScan::new(&witness, false);
            assert!(reused.consider(10, pid, title, false).is_err());
        }
    }

    #[test]
    fn duplicate_or_changed_capture_identity_never_proves_hidden() {
        let witness = capture(0, None);
        let mut duplicate = CaptureScan::new(&witness, false);
        duplicate.consider(10, 42, Some("Mewu"), false).unwrap();
        assert!(duplicate.consider(11, 42, Some("Mewu"), false).is_err());
        let known = capture(0, Some(10));
        let mut changed = CaptureScan::new(&known, false);
        assert!(changed.consider(11, 42, Some("Mewu"), false).is_err());
    }

    #[test]
    fn unnamed_own_onscreen_or_inconsistent_onscreen_metadata_fails_closed() {
        let witness = capture(0, None);
        for title in [None, Some("")] {
            let mut visible = CaptureScan::new(&witness, true);
            assert!(visible.consider(10, 42, title, true).is_err());
            let mut absent = CaptureScan::new(&witness, false);
            absent.consider(10, 42, title, false).unwrap();
            assert!(absent.candidate.is_none());
        }
        let mut inconsistent = CaptureScan::new(&witness, true);
        assert!(inconsistent.consider(10, 42, Some("Mewu"), false).is_err());
    }

    #[test]
    fn destroyed_owner_rejects_inflight_admission_and_old_token() {
        let mut state = Registry::default();
        let generation = state.begin("space").unwrap();
        let old = state.resolve("space", generation, native(10)).unwrap();
        state.revoke("space");
        assert!(state.resolve("space", generation, native(10)).is_err());
        assert!(state.get(old).is_none());
        let generation = state.begin("space").unwrap();
        let new = state.resolve("space", generation, native(10)).unwrap();
        assert_ne!(old, new);
    }

    #[test]
    fn repeated_admission_is_stable_and_replacement_cannot_rebind_token() {
        let mut state = Registry::default();
        let generation = state.begin("space").unwrap();
        let old = state.resolve("space", generation, native(10)).unwrap();
        assert_eq!(state.resolve("space", generation, native(10)).unwrap(), old);
        let new = state.resolve("space", generation, native(11)).unwrap();
        assert_ne!(old, new);
        assert!(state.get(old).is_none());
        assert!(state.resolve("space", generation, native(10)).is_err());
    }

    #[test]
    fn exhausted_tokens_fail_closed_without_wrapping_or_reusing_handles() {
        let mut state = Registry::default();
        let generation = state.begin("space").unwrap();
        let old = state.resolve("space", generation, native(10)).unwrap();
        state.last_token = usize::MAX;
        assert!(state.resolve("space", generation, native(11)).is_err());
        assert!(state.get(old).is_none());
        assert!(state.begin("space").is_err());
    }
}
