// SPDX-License-Identifier: MPL-2.0
use mewu_core::*;

fn origin() -> ExternalRunOrigin {
    ExternalRunOrigin {
        plugin_id: "community.custom-agent".into(),
        service_id: "agent".into(),
        revision: 3,
        sha256: "ab".repeat(32),
    }
}
fn input(store: &mut Store) -> String {
    let scene = store.snapshot().active_scene_id;
    store
        .apply(SceneCommand::SetSceneConnection {
            scene_id: scene.clone(),
            connection_id: None,
        })
        .unwrap();
    store
        .apply(SceneCommand::SetDraft {
            scene_id: scene.clone(),
            draft: "独立服务处理问题".into(),
        })
        .unwrap();
    scene
}
#[test]
fn independent_agent_needs_no_model_connection_and_preserves_real_origin_and_history() {
    let path = std::env::temp_dir().join(format!("mewu-external-{}.sqlite", uuid::Uuid::new_v4()));
    {
        let mut store = Store::open(&path).unwrap();
        let scene = input(&mut store);
        assert!(matches!(
            store.begin_run(&scene),
            Err(CoreError::ConnectionUnselected)
        ));
        let context = store.begin_external_run(&scene, origin()).unwrap();
        assert_eq!(context.messages.last().unwrap().text, "独立服务处理问题");
        let snapshot = store.snapshot();
        let run = snapshot.scenes[0].run.as_ref().unwrap();
        assert_eq!(run.external_runtime.as_ref(), Some(&origin()));
        assert!(run.connection_id.is_none() && run.connection_revision.is_none());
        assert!(matches!(
            store.begin_external_run(&scene, origin()),
            Err(CoreError::RunInProgress)
        ));
        store
            .finish_run(&scene, &context.run_id, "独立插件真实回答")
            .unwrap();
        let summary = store
            .run_journal_summary(&scene, &context.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(summary.status, RecordedRunStatus::Completed);
        assert!(!summary.can_continue);
    }
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.snapshot().scenes[0].messages.last().unwrap().text,
            "独立插件真实回答"
        );
        assert_eq!(
            store.snapshot().scenes[0]
                .run
                .as_ref()
                .unwrap()
                .external_runtime
                .as_ref(),
            Some(&origin())
        );
    }
    let _ = std::fs::remove_file(path);
}
#[test]
fn invalid_origin_does_not_consume_draft_and_cancel_rejects_late_answer() {
    let mut store = Store::open_in_memory().unwrap();
    let scene = input(&mut store);
    let before = store.snapshot();
    let mut invalid = origin();
    invalid.sha256 = "bad".into();
    assert!(store.begin_external_run(&scene, invalid).is_err());
    assert_eq!(store.snapshot(), before);
    let context = store.begin_external_run(&scene, origin()).unwrap();
    store.cancel_run(&scene).unwrap();
    assert!(matches!(
        store.finish_run(&scene, &context.run_id, "late"),
        Err(CoreError::StaleRun)
    ));
    assert_eq!(
        store
            .run_journal_summary(&scene, &context.run_id)
            .unwrap()
            .unwrap()
            .status,
        RecordedRunStatus::Canceled
    );
}
