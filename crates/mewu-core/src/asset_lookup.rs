// SPDX-License-Identifier: MPL-2.0
use crate::{Asset, Snapshot};

impl Snapshot {
    /// Only live typed references grant access; never look in undo history or
    /// trust a path supplied by a view. Borrowing avoids copying conversations.
    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.scenes
            .iter()
            .flat_map(|scene| {
                scene
                    .background
                    .iter()
                    .chain(
                        scene
                            .regions
                            .iter()
                            .filter_map(|region| region.image_override.as_ref()),
                    )
                    .chain(scene.regions.iter().filter_map(|region| {
                        region.translation.as_ref().map(|saved| &saved.overlay)
                    }))
                    .chain(scene.items.iter().map(|item| &item.asset))
                    .chain(
                        scene
                            .messages
                            .iter()
                            .flat_map(|message| message.attachments.iter().flatten()),
                    )
            })
            .find(|asset| asset.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AssetKind, OcrDocument, Region, SavedTranslation, SceneCommand, Store, TranslationDocument,
    };

    fn image(name: &str) -> Asset {
        Asset {
            capture_coordinates: None,
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            kind: AssetKind::Image,
            path: format!("assets/{name}.png"),
            width: Some(20),
            height: Some(20),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        }
    }

    #[test]
    fn lookup_borrows_each_registered_source_and_revokes_removed_layers() {
        let store = Store::open_in_memory().unwrap();
        let mut snapshot = store.snapshot();
        let background = image("background");
        let replacement = image("replacement");
        let overlay = image("translation");
        let scene = &mut snapshot.scenes[0];
        scene.closed = true;
        scene.frozen = true;
        scene.background = Some(background.clone());
        scene.regions.push(Region {
            id: "region".into(),
            width: 20.,
            height: 20.,
            image_override: Some(replacement.clone()),
            translation: Some(SavedTranslation {
                background_id: background.id.clone(),
                drawing_revision: 0,
                source_id: replacement.id.clone(),
                overlay: overlay.clone(),
                document: TranslationDocument {
                    version: 1,
                    target_language: "zh-CN".into(),
                    width: 20,
                    height: 20,
                    text_angle: None,
                    lines: vec![],
                },
                selection: OcrDocument {
                    engine: "test".into(),
                    language: "en".into(),
                    width: 20,
                    height: 20,
                    text_angle: None,
                    lines: vec![],
                },
            }),
            ..Default::default()
        });
        for asset in [&background, &replacement, &overlay] {
            assert_eq!(snapshot.asset(&asset.id), Some(asset));
        }
        assert!(std::ptr::eq(
            snapshot.asset(&background.id).unwrap(),
            snapshot.scenes[0].background.as_ref().unwrap()
        ));
        snapshot.scenes[0].regions.clear();
        assert!(snapshot.asset(&replacement.id).is_none());
        assert!(snapshot.asset(&overlay.id).is_none());
        assert_eq!(snapshot.asset(&background.id), Some(&background));
        assert!(snapshot.asset("../../unregistered-file").is_none());
    }

    #[test]
    fn store_lookup_preserves_closed_history_attachments_without_a_snapshot_copy() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.snapshot().active_scene_id;
        let background = image("background");
        let historical = image("history");
        store.set_background(&id, background.clone()).unwrap();
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "test".into(),
            })
            .unwrap();
        let run = store.begin_run(&id).unwrap();
        store
            .attach_run_assets(&id, &run.run_id, vec![historical.clone()])
            .unwrap();
        store.finish_run(&id, &run.run_id, "answer").unwrap();
        store
            .apply(SceneCommand::CloseScene { scene_id: id })
            .unwrap();
        assert_eq!(store.asset(&background.id), Some(&background));
        assert_eq!(store.asset(&historical.id), Some(&historical));
        assert!(store.asset("unregistered").is_none());
    }
}
