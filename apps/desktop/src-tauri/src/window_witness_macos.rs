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
