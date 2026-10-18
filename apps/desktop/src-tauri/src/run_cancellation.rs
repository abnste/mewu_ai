// SPDX-License-Identifier: MPL-2.0
//! A queued cancellation may affect only the run named by that user action.
use mewu_core::{CoreError, RunStatus, Snapshot, Store};

pub(crate) fn cancel_current(
    store: &mut Store,
    scene_id: &str,
    expected_run_id: &str,
    before_cancel: impl FnOnce(),
) -> Result<Snapshot, CoreError> {
    let scene = store.scene(scene_id)?;
    if !scene.run.as_ref().is_some_and(|run| run.id == expected_run_id && run.status == RunStatus::Running) {
        return Err(CoreError::StaleRun);
    }
    before_cancel();
    store.cancel_run(scene_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::SceneCommand;

    fn running() -> (Store, String, String) {
        let mut store = Store::open_in_memory().unwrap();
        let scene = store.snapshot().active_scene_id;
        store.apply(SceneCommand::SetDraft { scene_id: scene.clone(), draft: "synthetic cancellation".into() }).unwrap();
        let run = store.begin_run(&scene).unwrap();
        (store, scene, run.run_id)
    }

    #[test]
    fn an_old_queued_cancellation_cannot_cancel_the_next_run_or_its_side_effects() {
        let (mut store, scene, old) = running();
        store.finish_run(&scene, &old, "old run completed").unwrap();
        store.apply(SceneCommand::SetDraft { scene_id: scene.clone(), draft: "new explicit task".into() }).unwrap();
        let next = store.begin_run(&scene).unwrap().run_id;
        assert_ne!(next, old);
        let before = store.snapshot();
        let mut callback_ran = false;
        assert!(matches!(cancel_current(&mut store, &scene, &old, || callback_ran = true), Err(CoreError::StaleRun)));
        assert!(!callback_ran);
        assert_eq!(store.snapshot(), before);
        assert!(store.run_is_active(&scene, &next));
        assert!(!store.run_is_active(&scene, &old));
    }

    #[test]
    fn the_named_current_run_is_actually_canceled_and_cannot_be_canceled_twice() {
        let (mut store, scene, run) = running();
        let mut callback_count = 0;
        let saved = cancel_current(&mut store, &scene, &run, || callback_count += 1).unwrap();
        assert_eq!(callback_count, 1);
        assert_eq!(store.snapshot(), saved);
        assert_eq!(store.scene(&scene).unwrap().run.as_ref().unwrap().status, RunStatus::Canceled);
        assert!(!store.run_is_active(&scene, &run));
        assert!(matches!(cancel_current(&mut store, &scene, &run, || callback_count += 1), Err(CoreError::StaleRun)));
        assert_eq!(callback_count, 1);
        assert_eq!(store.snapshot(), saved);
    }

    #[test]
    fn terminal_empty_and_missing_scenes_keep_domain_errors_without_cancel_side_effects() {
        let (mut store, scene, run) = running();
        store.finish_run(&scene, &run, "finished").unwrap();
        let before = store.snapshot();
        assert!(matches!(cancel_current(&mut store, &scene, &run, || panic!("terminal cancellation")), Err(CoreError::StaleRun)));
        assert!(matches!(cancel_current(&mut store, "missing-scene", &run, || panic!("missing scene cancellation")), Err(CoreError::NotFound { .. })));
        assert_eq!(store.snapshot(), before);
        store.apply(SceneCommand::NewScene).unwrap();
        let blank = store.snapshot().active_scene_id;
        let before = store.snapshot();
        assert!(matches!(cancel_current(&mut store, &blank, &run, || panic!("blank scene cancellation")), Err(CoreError::StaleRun)));
        assert_eq!(store.snapshot(), before);
    }
}
