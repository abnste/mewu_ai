// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;

#[test]
fn only_completed_assistant_answers_can_be_read_from_frozen_or_closed_history() {
    let mut store = Store::open_in_memory().unwrap();
    let scene = store.snapshot().active_scene_id;
    store.apply(SceneCommand::SetDraft { scene_id: scene.clone(), draft: "识别表格".into() }).unwrap();
    let run = store.begin_run(&scene).unwrap();
    let user_id = run.messages.last().unwrap().id.clone();
    assert!(store.completed_message_text(&scene, &user_id).is_err());
    assert!(store.completed_message_text(&scene, "missing").is_err());
    store.apply(SceneCommand::FreezeScene { scene_id: scene.clone() }).unwrap();
    let active = store.snapshot().active_scene_id;
    let answer = "|姓名|编号|\n|---|---|\n|中文|0012|";
    store.finish_run(&scene, &run.run_id, answer).unwrap();
    let id = store.snapshot().scenes.iter().find(|s| s.id == scene).unwrap().messages.last().unwrap().id.clone();
    assert_eq!(store.completed_message_text(&scene, &id).unwrap(), answer);
    assert_eq!(store.snapshot().active_scene_id, active);
    store.apply(SceneCommand::CloseScene { scene_id: scene.clone() }).unwrap();
    assert_eq!(store.completed_message_text(&scene, &id).unwrap(), answer);
    assert!(store.completed_message_text(&active, &id).is_err());
}

#[test]
fn canceled_run_cannot_publish_a_table_result() {
    let mut store = Store::open_in_memory().unwrap();
    let scene = store.snapshot().active_scene_id;
    store.apply(SceneCommand::SetDraft { scene_id: scene.clone(), draft: "识别表格".into() }).unwrap();
    let run = store.begin_run(&scene).unwrap();
    store.cancel_run(&scene).unwrap();
    assert!(store.finish_run(&scene, &run.run_id, "|A|\n|---|\n|B|").is_err());
    assert!(store.snapshot().scenes[0].messages.iter().all(|message| store.completed_message_text(&scene, &message.id).is_err()));
}
