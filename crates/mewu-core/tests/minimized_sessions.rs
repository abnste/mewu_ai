// SPDX-License-Identifier: MPL-2.0
use mewu_core::{SceneCommand, Store};
#[test]
fn only_explicit_minimize_marks_a_scene_and_restore_preserves_that_choice() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store.snapshot().active_scene_id;
    let legacy = serde_json::to_value(store.snapshot()).unwrap();
    assert!(legacy["scenes"][0].get("minimized").is_none());
    store.apply(SceneCommand::NewScene).unwrap();
    assert!(
        !store
            .snapshot()
            .scenes
            .iter()
            .find(|s| s.id == a)
            .unwrap()
            .minimized
    );
    let b = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::FreezeScene {
            scene_id: b.clone(),
        })
        .unwrap();
    assert!(
        store
            .snapshot()
            .scenes
            .iter()
            .find(|s| s.id == b)
            .unwrap()
            .minimized
    );
    store
        .apply(SceneCommand::ActivateScene {
            scene_id: b.clone(),
        })
        .unwrap();
    let state = store.snapshot();
    let scene = state.scenes.iter().find(|s| s.id == b).unwrap();
    assert!(scene.minimized && !scene.frozen && !scene.closed);
    store
        .apply(SceneCommand::CloseScene {
            scene_id: b.clone(),
        })
        .unwrap();
    assert!(
        store
            .snapshot()
            .scenes
            .iter()
            .find(|s| s.id == b)
            .unwrap()
            .closed
    );
}

#[test]
fn failed_minimize_retains_late_answer_and_restores_only_its_previous_choice() {
    for previously_minimized in [false, true] {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.snapshot().active_scene_id;
        if previously_minimized {
            store
                .apply(SceneCommand::FreezeScene {
                    scene_id: id.clone(),
                })
                .unwrap();
            store
                .apply(SceneCommand::ActivateScene {
                    scene_id: id.clone(),
                })
                .unwrap();
        }
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "合成问题".into(),
            })
            .unwrap();
        let run = store.begin_run(&id).unwrap();
        store
            .apply(SceneCommand::FreezeScene {
                scene_id: id.clone(),
            })
            .unwrap();
        store
            .finish_run(&id, &run.run_id, "窗口回退前到达的回答")
            .unwrap();
        store
            .compensate_failed_minimize(&id, previously_minimized)
            .unwrap();
        let state = store.snapshot();
        let scene = state.scenes.iter().find(|s| s.id == id).unwrap();
        assert_eq!(scene.minimized, previously_minimized);
        assert!(!scene.frozen && !scene.closed);
        assert_eq!(scene.messages.last().unwrap().text, "窗口回退前到达的回答");
    }
}
