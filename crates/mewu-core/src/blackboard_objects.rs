// SPDX-License-Identifier: MPL-2.0
use crate::{Asset, Scene, Snapshot, SpaceItem};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
type Result<T> = std::result::Result<T, crate::CoreError>;

pub(crate) const SOURCE_KEY: &str = "__mewuBoardSource";
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Source {
    pub scene_id: String,
    pub item_id: String,
}
pub(crate) fn source(item: &SpaceItem) -> Result<Option<Source>> {
    item.state
        .as_ref()
        .and_then(|s| s.get(SOURCE_KEY))
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|_| crate::CoreError::Invalid("黑板素材来源无效".into()))
        })
        .transpose()
}
pub(crate) fn valid_link(state: &Snapshot, scene: &Scene, item: &SpaceItem) -> Result<()> {
    if let Some(source) = source(item)? {
        if source.item_id.is_empty()
            || !scene.blackboard_link.as_ref().is_some_and(|link| {
                link.parent_scene_id == source.scene_id
                    && state.scenes.iter().any(|s| s.id == source.scene_id)
            })
        {
            return Err(crate::CoreError::Invalid("黑板素材关联无效".into()));
        }
    }
    Ok(())
}
pub(crate) fn carried(parent: &Scene, background: &Asset) -> Result<Vec<SpaceItem>> {
    let bw = f64::from(background.width.unwrap());
    let bh = f64::from(background.height.unwrap());
    parent
        .items
        .iter()
        .map(|original| {
            let mut item = original.clone();
            item.id = Uuid::new_v4().to_string();
            let state = item.state.get_or_insert_with(Default::default);
            if state.get("coordinateSpace").and_then(Value::as_str) == Some("background")
                && parent.background.as_ref().is_some_and(|asset| {
                    state.get("backgroundId").and_then(Value::as_str) == Some(asset.id.as_str())
                })
            {
                let asset = parent.background.as_ref().unwrap();
                if let (Some(width), Some(height)) = (asset.width, asset.height) {
                    let w = f64::from(width);
                    let h = f64::from(height);
                    let scale = (bw / w).min(bh / h);
                    item.width = (item.width * w * scale / bw).min(1.);
                    item.height = (item.height * h * scale / bh).min(1.);
                    item.x = (((bw - w * scale) / 2. + item.x * w * scale) / bw)
                        .clamp(0., 1. - item.width);
                    item.y = (((bh - h * scale) / 2. + item.y * h * scale) / bh)
                        .clamp(0., 1. - item.height);
                }
                state.remove("coordinateSpace");
                state.remove("backgroundId");
            }
            state.remove("blackboardSceneId");
            state.insert(
                SOURCE_KEY.into(),
                serde_json::to_value(Source {
                    scene_id: parent.id.clone(),
                    item_id: original.id.clone(),
                })?,
            );
            Ok(item)
        })
        .collect()
}
