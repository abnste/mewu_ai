// SPDX-License-Identifier: MPL-2.0
//! A temporary region observation may restore only its original visible scene.
use mewu_core::Snapshot;
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Clone)]
pub(crate) struct RestoreSource {
    pub(crate) scene_id: String,
    pub(crate) background_id: String,
}

pub(crate) fn source_is_current(source: &RestoreSource, snapshot: &Snapshot) -> bool {
    snapshot.active_scene_id == source.scene_id
        && snapshot.scenes.iter().any(|scene| {
            scene.id == source.scene_id
                && !scene.closed
                && !scene.frozen
                && scene.background.as_ref().map(|asset| &asset.id) == Some(&source.background_id)
        })
}

/// Shared by native restoration and the race tests. Snapshot reads happen only
/// after owning the same transition used by explicit close/freeze/capture.
pub(crate) fn restore_if_current(
    source: &RestoreSource,
    capturing: &Arc<AtomicBool>,
    exit: &crate::lifecycle::ExitState,
    snapshot: impl FnOnce() -> Option<Snapshot>,
    restore: impl FnOnce(),
) -> bool {
    let Some(_transition) = crate::SpaceTransition::try_begin(capturing) else {
        return false;
    };
    if snapshot().is_none_or(|snapshot| !source_is_current(source, &snapshot)) {
        return false;
    }
    exit.with_window_restore(restore)
}

pub(crate) fn finalize_if_current(
    source: &RestoreSource,
    capturing: &Arc<AtomicBool>,
    exit: &crate::lifecycle::ExitState,
    snapshot: impl FnOnce() -> Option<Snapshot>,
    original_focus_owner: impl FnOnce() -> bool,
    focus: impl FnOnce(),
) -> bool {
    let mut focused = false;
    restore_if_current(source, capturing, exit, snapshot, || {
        if original_focus_owner() {
            focus();
            focused = true;
        }
    });
    focused
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RestoreStatus {
    Busy,
    Exit,
    Applied,
}

/// Desktop pins have their own live-instance policy and survive a closed
/// source scene. Both groups still serialize against explicit transitions/exit.
pub(crate) fn restore_groups(
    source: &RestoreSource,
    capturing: &Arc<AtomicBool>,
    exit: &crate::lifecycle::ExitState,
    snapshot: impl FnOnce() -> Option<Snapshot>,
    restore: impl FnOnce(bool),
) -> RestoreStatus {
    let Some(_transition) = crate::SpaceTransition::try_begin(capturing) else {
        return RestoreStatus::Busy;
    };
    let scene_current = snapshot().is_some_and(|snapshot| source_is_current(source, &snapshot));
    if exit.with_window_restore(|| restore(scene_current)) {
        RestoreStatus::Applied
    } else {
        RestoreStatus::Exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{Asset, AssetKind, SceneCommand, Store};

    fn source() -> (Store, RestoreSource) {
        let mut store = Store::open_in_memory().unwrap();
        let scene_id = store.snapshot().active_scene_id;
        let background_id = uuid::Uuid::new_v4().to_string();
        store
            .set_background(
                &scene_id,
                Asset {
                    id: background_id.clone(),
                    name: "capture.png".into(),
                    kind: AssetKind::Image,
                    path: "assets/capture.png".into(),
                    width: Some(640),
                    height: Some(480),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                    capture_coordinates: None,
                },
            )
            .unwrap();
        (
            store,
            RestoreSource {
                scene_id,
                background_id,
            },
        )
    }

    #[test]
    fn closed_frozen_and_replaced_scenes_cannot_be_restored_by_a_late_observation() {
        let (mut store, source) = source();
        assert!(source_is_current(&source, &store.snapshot()));
        store
            .apply(SceneCommand::FreezeScene {
                scene_id: source.scene_id.clone(),
            })
            .unwrap();
        assert!(!source_is_current(&source, &store.snapshot()));
        store
            .apply(SceneCommand::ActivateScene {
                scene_id: source.scene_id.clone(),
            })
            .unwrap();
        assert!(source_is_current(&source, &store.snapshot()));
        store
            .apply(SceneCommand::CloseScene {
                scene_id: source.scene_id.clone(),
            })
            .unwrap();
        assert!(!source_is_current(&source, &store.snapshot()));
    }

    #[test]
    fn another_capture_in_the_same_scene_revokes_the_old_restore_source() {
        let (mut store, source) = source();
        let mut background = store
            .snapshot()
            .scenes
            .iter()
            .find(|scene| scene.id == source.scene_id)
            .unwrap()
            .background
            .clone()
            .unwrap();
        background.id = uuid::Uuid::new_v4().to_string();
        store.set_background(&source.scene_id, background).unwrap();
        assert!(!source_is_current(&source, &store.snapshot()));
    }

    #[test]
    fn production_restore_policy_keeps_close_out_until_the_actual_show_finishes() {
        let (mut store, source) = source();
        let capturing = Arc::new(AtomicBool::new(false));
        let exit = crate::lifecycle::ExitState::default();
        let mut shows = 0;
        assert!(restore_if_current(
            &source,
            &capturing,
            &exit,
            || Some(store.snapshot()),
            || {
                assert!(crate::SpaceTransition::try_begin(&capturing).is_none());
                shows += 1;
            }
        ));
        let close = crate::SpaceTransition::try_begin(&capturing).unwrap();
        store
            .apply(SceneCommand::CloseScene {
                scene_id: source.scene_id.clone(),
            })
            .unwrap();
        drop(close);
        assert!(!restore_if_current(
            &source,
            &capturing,
            &exit,
            || Some(store.snapshot()),
            || shows += 1
        ));
        assert_eq!(shows, 1);
    }

    #[test]
    fn a_pending_explicit_transition_prevents_even_reading_a_late_restore_snapshot() {
        let (_, source) = source();
        let capturing = Arc::new(AtomicBool::new(false));
        let exit = crate::lifecycle::ExitState::default();
        let _close = crate::SpaceTransition::try_begin(&capturing).unwrap();
        assert!(!restore_if_current(
            &source,
            &capturing,
            &exit,
            || panic!("must not read old scene while an explicit transition owns it"),
            || panic!("must not restore over an explicit transition")
        ));
    }

    #[test]
    fn final_focus_policy_only_admits_the_matching_live_run_and_foreground() {
        let (mut store, source) = source();
        let capturing = Arc::new(AtomicBool::new(false));
        let exit = crate::lifecycle::ExitState::default();
        let mut focuses = 0;
        for (current_run, same_space, same_foreground) in [
            (true, true, true),
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert_eq!(
                finalize_if_current(
                    &source,
                    &capturing,
                    &exit,
                    || Some(store.snapshot()),
                    || current_run && same_space && same_foreground,
                    || focuses += 1
                ),
                current_run && same_space && same_foreground
            );
        }
        store
            .apply(SceneCommand::CloseScene {
                scene_id: source.scene_id.clone(),
            })
            .unwrap();
        assert!(!finalize_if_current(
            &source,
            &capturing,
            &exit,
            || Some(store.snapshot()),
            || true,
            || focuses += 1
        ));
        assert_eq!(focuses, 1);
    }

    #[test]
    fn independent_pin_restore_is_not_discarded_when_the_source_scene_closed() {
        let (mut store, source) = source();
        store
            .apply(SceneCommand::CloseScene {
                scene_id: source.scene_id.clone(),
            })
            .unwrap();
        let capturing = Arc::new(AtomicBool::new(false));
        let exit = crate::lifecycle::ExitState::default();
        let mut pins = 0;
        assert_eq!(
            restore_groups(
                &source,
                &capturing,
                &exit,
                || Some(store.snapshot()),
                |scene_current| {
                    assert!(!scene_current);
                    pins += 1;
                }
            ),
            RestoreStatus::Applied
        );
        assert_eq!(pins, 1);
        let _explicit = crate::SpaceTransition::try_begin(&capturing).unwrap();
        assert_eq!(
            restore_groups(
                &source,
                &capturing,
                &exit,
                || panic!("explicit transition still owns snapshot"),
                |_| panic!("must defer independent pins without revealing during capture")
            ),
            RestoreStatus::Busy
        );
    }
}
