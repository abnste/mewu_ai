// SPDX-License-Identifier: MPL-2.0
//! A worker owns this model; never hold a shared mutex across Backend calls.
pub use super::host_preferences::{
    CommitReceipt, FileStamp, HostPreferences, LoadedPreferences, PreferencesError,
    PreferencesFile, ShortcutChord,
};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutStatus {
    Active,
    Disabled,
    Fallback,
    Unavailable,
    CleanupPending,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureShortcutState {
    pub revision: u64,
    pub sequence: u64,
    pub configured: Option<ShortcutChord>,
    pub active: Option<ShortcutChord>,
    pub status: ShortcutStatus,
    pub editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationError {
    Occupied,
    Unavailable,
}
impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Occupied => "快捷键被占用",
            Self::Unavailable => "快捷键被占用或无法使用",
        })
    }
}
impl std::error::Error for RegistrationError {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutError {
    Conflict,
    CleanupPending,
    NotAllowed,
    Registration(RegistrationError),
    Preferences(PreferencesError),
}
impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict => f.write_str("快捷键设置已改变，请载入最新设置"),
            Self::CleanupPending => f.write_str("上次快捷键尚未释放，请稍后重试"),
            Self::NotAllowed => f.write_str("当前无法修改截图快捷键"),
            Self::Registration(error) => error.fmt(f),
            Self::Preferences(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for ShortcutError {}
impl From<PreferencesError> for ShortcutError {
    fn from(value: PreferencesError) -> Self {
        Self::Preferences(value)
    }
}

pub trait Backend {
    /// Err means this call acquired no OS registration. Do not parse OS error text.
    fn register(&mut self, chord: &ShortcutChord) -> Result<(), RegistrationError>;
    /// Called only for the exact inactive registration owned by this model.
    /// Cleanup must remain allowed while an accepted operation is draining at exit.
    fn unregister(&mut self, chord: &ShortcutChord) -> Result<(), RegistrationError>;
    /// Must recheck admission immediately before file replacement. A successful
    /// commit is final even if Running/visibility changes while it returns.
    fn persist(
        &mut self,
        expected: &FileStamp,
        next: &HostPreferences,
    ) -> Result<CommitReceipt, PreferencesError>;
    /// A no-op still needs a read-only disk revision fence against another writer.
    fn check_current(&mut self, expected: &FileStamp) -> Result<(), PreferencesError>;
    /// Infallible. Short atomic/poison-recover route replacement only, no OS call.
    /// owned includes inactive registrations so an armed recorder can receive them.
    fn switch_route(&mut self, active: Option<&ShortcutChord>, owned: &[ShortcutChord]);
    /// Infallible. Replace immutable view under a short lock, then emit outside it.
    fn publish(&mut self, state: &CaptureShortcutState);
    fn allowed(&self) -> bool;
}

pub struct ShortcutModel {
    preferences: HostPreferences,
    stamp: FileStamp,
    active: Option<ShortcutChord>,
    stale: Option<ShortcutChord>,
    fault: Option<PreferencesError>,
    durability_warning: bool,
    sequence: u64,
    started: bool,
}
impl ShortcutModel {
    pub fn new(load: Result<LoadedPreferences, PreferencesError>) -> Self {
        let (loaded, fault) = match load {
            Ok(loaded) => (loaded, None),
            Err(error) => (LoadedPreferences::missing(), Some(error)),
        };
        Self {
            preferences: loaded.preferences,
            stamp: loaded.stamp,
            active: None,
            stale: None,
            fault,
            durability_warning: false,
            sequence: 0,
            started: false,
        }
    }
    pub fn view(&self) -> CaptureShortcutState {
        let (status, message) = if let Some(error) = self.fault {
            (ShortcutStatus::Unavailable, Some(error.to_string()))
        } else if self.stale.is_some() {
            (
                ShortcutStatus::CleanupPending,
                Some("另一个截图快捷键尚未释放".into()),
            )
        } else if self.preferences.capture_shortcut.is_none() {
            (ShortcutStatus::Disabled, None)
        } else if self.active.is_none() {
            (
                ShortcutStatus::Unavailable,
                Some("截图快捷键不可用，可从托盘截图".into()),
            )
        } else if self.active != self.preferences.capture_shortcut {
            (
                ShortcutStatus::Fallback,
                Some("配置的快捷键不可用，正在使用备用快捷键".into()),
            )
        } else {
            (ShortcutStatus::Active, None)
        };
        CaptureShortcutState {
            revision: self.preferences.revision,
            sequence: self.sequence,
            configured: if self.fault.is_some() {
                None
            } else {
                self.preferences.capture_shortcut.clone()
            },
            active: self.active.clone(),
            status,
            editable: self.fault.is_none(),
            message: message.or_else(|| {
                self.durability_warning
                    .then(|| "设置已生效，但存储同步状态未确认".into())
            }),
        }
    }
    pub fn owned_chords(&self) -> Vec<ShortcutChord> {
        let mut owned = Vec::with_capacity(2);
        if let Some(active) = &self.active {
            owned.push(active.clone());
        }
        if let Some(stale) = &self.stale {
            if !owned.contains(stale) {
                owned.push(stale.clone());
            }
        }
        owned
    }
    fn route(&self, backend: &mut impl Backend) {
        backend.switch_route(self.active.as_ref(), &self.owned_chords());
    }
    fn publish(&mut self, backend: &mut impl Backend) -> CaptureShortcutState {
        // No realistic process can publish 2^64 views; do not silently wrap and
        // make a stale UI result become newer. Host lifetime ends before this limit.
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("shortcut runtime sequence exhausted");
        let view = self.view();
        backend.publish(&view);
        view
    }
    pub fn startup(&mut self, backend: &mut impl Backend) -> CaptureShortcutState {
        if self.started {
            return self.view();
        }
        self.started = true;
        if self.fault.is_none() && backend.allowed() {
            if let Some(desired) = &self.preferences.capture_shortcut {
                let mut candidates = vec![desired.clone()];
                for fallback in [
                    ShortcutChord::default_capture(),
                    ShortcutChord::fallback_capture(),
                ] {
                    if !candidates.contains(&fallback) {
                        candidates.push(fallback);
                    }
                }
                for candidate in candidates {
                    if !backend.allowed() {
                        break;
                    }
                    if backend.register(&candidate).is_ok() {
                        self.active = Some(candidate);
                        break;
                    }
                }
            }
        }
        // Corrupt/unknown preferences never register a default: the unreadable
        // file may have contained an explicit disabled choice. Tray stays usable.
        self.route(backend);
        self.publish(backend)
    }
    fn clean_stale(&mut self, backend: &mut impl Backend) -> bool {
        let Some(stale) = self.stale.clone() else {
            return true;
        };
        assert_ne!(
            self.active.as_ref(),
            Some(&stale),
            "never unregister current routing"
        );
        if backend.unregister(&stale).is_err() {
            return false;
        }
        self.stale = None;
        self.route(backend);
        true
    }
    fn failed(
        &mut self,
        backend: &mut impl Backend,
        error: ShortcutError,
    ) -> Result<CaptureShortcutState, ShortcutError> {
        self.publish(backend);
        Err(error)
    }
    pub fn set(
        &mut self,
        expected_revision: u64,
        shortcut: Option<ShortcutChord>,
        backend: &mut impl Backend,
    ) -> Result<CaptureShortcutState, ShortcutError> {
        if let Some(error) = self.fault {
            return Err(error.into());
        }
        if let Some(chord) = &shortcut {
            chord.validate()?;
        }
        if !self.started || !backend.allowed() {
            return Err(ShortcutError::NotAllowed);
        }
        if expected_revision != self.preferences.revision {
            return Err(ShortcutError::Conflict);
        }

        // Loaded same config/active can be a no-op or an exact cleanup retry.
        // Missing rev0 is deliberately persisted on explicit Save. Fallback is
        // not a no-op: retry registering the configured combination itself.
        if shortcut == self.preferences.capture_shortcut
            && shortcut == self.active
            && !matches!(self.stamp, FileStamp::Missing)
        {
            if let Err(error) = backend.check_current(&self.stamp) {
                self.fault = Some(error);
                return self.failed(backend, error.into());
            }
            let before = self.stale.clone();
            self.clean_stale(backend);
            return Ok(if before != self.stale {
                self.publish(backend)
            } else {
                self.view()
            });
        }
        let next_revision = self
            .preferences
            .revision
            .checked_add(1)
            .ok_or(PreferencesError::RevisionExhausted)?;

        // Reusing the stale key consumes its existing registration. Reusing the
        // active key needs no new slot. All other changes must clear stale first.
        let reuses_active = shortcut.is_some() && shortcut == self.active;
        let reuses_stale = shortcut.is_some() && shortcut == self.stale;
        if self.stale.is_some() && !reuses_active && !reuses_stale && !self.clean_stale(backend) {
            return self.failed(backend, ShortcutError::CleanupPending);
        }
        let mut newly_registered = false;
        if let Some(candidate) = &shortcut {
            if !reuses_active && !reuses_stale {
                if !backend.allowed() {
                    return self.failed(backend, ShortcutError::NotAllowed);
                }
                if let Err(error) = backend.register(candidate) {
                    return self.failed(backend, ShortcutError::Registration(error));
                }
                // Track it immediately, before any callback or persistence error.
                self.stale = Some(candidate.clone());
                newly_registered = true;
                self.route(backend);
            }
        }
        if !backend.allowed() {
            if newly_registered {
                self.clean_stale(backend);
            }
            return self.failed(backend, ShortcutError::NotAllowed);
        }
        let next = HostPreferences {
            version: 1,
            revision: next_revision,
            capture_shortcut: shortcut.clone(),
        };
        let receipt = match backend.persist(&self.stamp, &next) {
            Ok(receipt) => receipt,
            Err(error) => {
                if newly_registered {
                    self.clean_stale(backend);
                }
                if matches!(
                    error,
                    PreferencesError::CommitIndeterminate
                        | PreferencesError::Conflict
                        | PreferencesError::Corrupt
                        | PreferencesError::UnsupportedVersion
                        | PreferencesError::TooLarge
                        | PreferencesError::UnsafePath
                        | PreferencesError::Access
                ) {
                    self.fault = Some(error);
                }
                return self.failed(backend, error.into());
            }
        };
        // Commit boundary. Admission changes cannot turn a persisted setting into
        // a fake rollback or cause its registered key to be forgotten.
        let old = self.active.clone();
        self.preferences = next;
        self.stamp = receipt.stamp;
        self.durability_warning = receipt.durability_warning;
        if self.stale == shortcut {
            self.stale = None;
        }
        self.active = shortcut;
        if old != self.active {
            if let Some(old) = old {
                assert!(self.stale.is_none());
                self.stale = Some(old);
            }
        }
        self.route(backend);
        self.publish(backend); // new active is visible before old-key cleanup
        let had_stale = self.stale.is_some();
        let cleaned = self.clean_stale(backend);
        Ok(if had_stale && cleaned {
            self.publish(backend)
        } else {
            self.view()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[derive(Default)]
    struct Fake {
        keys: HashSet<ShortcutChord>,
        blocked: HashSet<ShortcutChord>,
        cleanup_fails: HashSet<ShortcutChord>,
        bytes: Option<Vec<u8>>,
        fail_save: bool,
        cancel_after_register: bool,
        cancel_after_persist: bool,
        allowed: bool,
        route: Option<ShortcutChord>,
        events: Vec<&'static str>,
        published: Vec<CaptureShortcutState>,
    }
    impl Backend for Fake {
        fn register(&mut self, chord: &ShortcutChord) -> Result<(), RegistrationError> {
            self.events.push("register");
            if self.blocked.contains(chord) || self.keys.contains(chord) {
                return Err(RegistrationError::Occupied);
            }
            self.keys.insert(chord.clone());
            if self.cancel_after_register {
                self.allowed = false;
            }
            Ok(())
        }
        fn unregister(&mut self, chord: &ShortcutChord) -> Result<(), RegistrationError> {
            self.events.push("unregister");
            assert_ne!(self.route.as_ref(), Some(chord));
            if self.cleanup_fails.contains(chord) {
                return Err(RegistrationError::Unavailable);
            }
            assert!(
                self.keys.remove(chord),
                "only remove a registration acquired by this fake"
            );
            Ok(())
        }
        fn persist(
            &mut self,
            _: &FileStamp,
            next: &HostPreferences,
        ) -> Result<CommitReceipt, PreferencesError> {
            self.events.push("persist");
            if self.fail_save {
                return Err(PreferencesError::WriteFailed);
            }
            let bytes = serde_json::to_vec(next).unwrap();
            self.bytes = Some(bytes.clone());
            if self.cancel_after_persist {
                self.allowed = false;
            }
            Ok(CommitReceipt {
                stamp: FileStamp::Present(bytes),
                durability_warning: false,
            })
        }
        fn switch_route(&mut self, active: Option<&ShortcutChord>, owned: &[ShortcutChord]) {
            self.events.push("route");
            assert!(owned.len() <= 2);
            assert!(owned.iter().all(|key| self.keys.contains(key)));
            self.route = active.cloned();
        }
        fn check_current(&mut self, expected: &FileStamp) -> Result<(), PreferencesError> {
            let actual = self
                .bytes
                .clone()
                .map(FileStamp::Present)
                .unwrap_or(FileStamp::Missing);
            if actual == *expected {
                Ok(())
            } else {
                Err(PreferencesError::Conflict)
            }
        }
        fn publish(&mut self, state: &CaptureShortcutState) {
            self.events.push("publish");
            self.published.push(state.clone());
        }
        fn allowed(&self) -> bool {
            self.allowed
        }
    }
    fn chord(letter: char) -> ShortcutChord {
        ShortcutChord {
            code: format!("Key{letter}"),
            ctrl: true,
            shift: false,
            alt: false,
        }
    }
    fn setup() -> (ShortcutModel, Fake) {
        let mut model = ShortcutModel::new(Ok(LoadedPreferences::missing()));
        let mut backend = Fake {
            allowed: true,
            ..Default::default()
        };
        model.startup(&mut backend);
        (model, backend)
    }
    #[test]
    fn conflict_and_failed_save_keep_old_routing_and_exact_bounded_ownership() {
        let (mut model, mut b) = setup();
        let old = b.route.clone();
        b.blocked.insert(chord('A'));
        assert!(model.set(0, Some(chord('A')), &mut b).is_err());
        assert_eq!(b.route, old);
        assert!(b.bytes.is_none());
        b.fail_save = true;
        b.cleanup_fails.insert(chord('B'));
        assert!(model.set(0, Some(chord('B')), &mut b).is_err());
        assert_eq!(b.route, old);
        assert_eq!(model.owned_chords().len(), 2);
        assert_eq!(
            model.set(0, Some(chord('C')), &mut b),
            Err(ShortcutError::CleanupPending)
        );
        assert_eq!(model.owned_chords().len(), 2);
        assert!(!b.keys.contains(&chord('C')));
    }
    #[test]
    fn post_commit_cleanup_error_is_success_and_stale_reuse_never_registers_twice() {
        let (mut model, mut b) = setup();
        let old = b.route.clone().unwrap();
        b.cleanup_fails.insert(old.clone());
        let state = model.set(0, Some(chord('A')), &mut b).unwrap();
        assert_eq!(state.revision, 1);
        assert_eq!(state.status, ShortcutStatus::CleanupPending);
        assert_eq!(b.route, Some(chord('A')));
        let before = b.events.iter().filter(|e| **e == "register").count();
        let restored = model.set(1, Some(old.clone()), &mut b).unwrap();
        assert_eq!(restored.active, Some(old));
        assert_eq!(restored.revision, 2);
        assert_eq!(
            b.events.iter().filter(|e| **e == "register").count(),
            before
        );
        assert_eq!(b.keys.len(), 1);
    }
    #[test]
    fn disable_persists_null_and_rejects_old_route_even_when_unregister_fails() {
        let (mut model, mut b) = setup();
        b.cleanup_fails.insert(b.route.clone().unwrap());
        let state = model.set(0, None, &mut b).unwrap();
        assert_eq!(state.configured, None);
        assert_eq!(b.route, None);
        let load = LoadedPreferences::from_bytes(b.bytes.clone().unwrap()).unwrap();
        let mut restarted = ShortcutModel::new(Ok(load));
        let mut backend = Fake {
            allowed: true,
            ..Default::default()
        };
        assert_eq!(
            restarted.startup(&mut backend).status,
            ShortcutStatus::Disabled
        );
        assert!(backend.keys.is_empty());
    }
    #[test]
    fn missing_explicit_default_is_saved_and_existing_same_value_is_noop() {
        let (mut model, mut b) = setup();
        assert_eq!(
            model
                .set(0, Some(ShortcutChord::default_capture()), &mut b)
                .unwrap()
                .revision,
            1
        );
        let count = b.events.len();
        assert_eq!(
            model
                .set(1, Some(ShortcutChord::default_capture()), &mut b)
                .unwrap()
                .revision,
            1
        );
        assert_eq!(b.events.len(), count);
        assert_eq!(model.set(0, None, &mut b), Err(ShortcutError::Conflict));
    }
    #[test]
    fn fallback_does_not_overwrite_choice_and_same_desired_retries_registration() {
        let desired = chord('A');
        let load = LoadedPreferences::from_bytes(
            serde_json::to_vec(&HostPreferences {
                version: 1,
                revision: 4,
                capture_shortcut: Some(desired.clone()),
            })
            .unwrap(),
        )
        .unwrap();
        let mut model = ShortcutModel::new(Ok(load));
        let mut b = Fake {
            allowed: true,
            ..Default::default()
        };
        b.blocked.insert(desired.clone());
        let state = model.startup(&mut b);
        assert_eq!(state.status, ShortcutStatus::Fallback);
        assert_eq!(state.configured, Some(desired.clone()));
        assert!(model.set(4, Some(desired.clone()), &mut b).is_err());
        b.blocked.clear();
        assert_eq!(
            model.set(4, Some(desired.clone()), &mut b).unwrap().active,
            Some(desired)
        );
    }
    #[test]
    fn committed_route_is_published_before_cleanup_and_corrupt_file_is_readonly() {
        let (mut model, mut b) = setup();
        b.events.clear();
        let state = model.set(0, Some(chord('A')), &mut b).unwrap();
        let persist = b.events.iter().position(|e| *e == "persist").unwrap();
        assert_eq!(
            &b.events[persist..persist + 4],
            &["persist", "route", "publish", "unregister"]
        );
        assert!(b
            .published
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
        assert_eq!(state.active, Some(chord('A')));
        let mut corrupt = ShortcutModel::new(Err(PreferencesError::Corrupt));
        let mut b = Fake {
            allowed: true,
            ..Default::default()
        };
        let state = corrupt.startup(&mut b);
        assert!(!state.editable);
        assert!(b.keys.is_empty());
        assert!(corrupt.set(0, None, &mut b).is_err());
        assert!(b.bytes.is_none());
    }
    #[test]
    fn same_value_still_checks_other_writer_and_never_reports_a_stale_success() {
        let (mut model, mut b) = setup();
        let desired = Some(ShortcutChord::default_capture());
        model.set(0, desired.clone(), &mut b).unwrap();
        b.bytes = Some(
            serde_json::to_vec(&HostPreferences {
                version: 1,
                revision: 2,
                capture_shortcut: None,
            })
            .unwrap(),
        );
        let external = b.bytes.clone();
        assert_eq!(
            model.set(1, desired.clone(), &mut b),
            Err(ShortcutError::Preferences(PreferencesError::Conflict))
        );
        assert_eq!(b.bytes, external);
        assert_eq!(b.route, desired);
        assert!(!model.view().editable);
    }
    #[test]
    fn admission_loss_before_commit_rolls_back_candidate_but_after_commit_finishes_transition() {
        let (mut model, mut b) = setup();
        let old = b.route.clone();
        b.cancel_after_register = true;
        assert_eq!(
            model.set(0, Some(chord('A')), &mut b),
            Err(ShortcutError::NotAllowed)
        );
        assert_eq!(b.route, old);
        assert!(b.bytes.is_none());
        assert_eq!(b.keys.len(), 1);
        b.allowed = true;
        b.cancel_after_register = false;
        b.cancel_after_persist = true;
        let saved = model.set(0, Some(chord('B')), &mut b).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.active, Some(chord('B')));
        assert_eq!(b.keys.len(), 1);
        assert!(!b.allowed);
    }
}
