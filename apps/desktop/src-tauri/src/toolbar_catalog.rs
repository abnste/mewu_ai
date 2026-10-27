// SPDX-License-Identifier: MPL-2.0
//! A host-owned, read-only UI contract; no plugin business rules or preferences.
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::{Mutex, OnceLock}};
use tauri::WebviewWindow;
const SOURCE: &str = include_str!("../../ui/toolbar-catalog.json");
static CURRENT: OnceLock<Mutex<BTreeMap<String, Value>>> = OnceLock::new();
pub(crate) fn definitions() -> Value {
    let mut value: Value = serde_json::from_str(SOURCE).expect("validated toolbar catalog");
    for surface in value["surfaces"].as_array_mut().unwrap() {
        surface["title"] = crate::ui_language::translated(surface["title"].as_str().unwrap()).into();
        for item in surface["items"].as_array_mut().unwrap() {
            item["title"] = crate::ui_language::translated(item["title"].as_str().unwrap()).into();
        }
    }
    value
}
pub(crate) fn describe(surface: &str) -> Result<Value, String> {
    let catalog = definitions();
    let definition = catalog["surfaces"].as_array().unwrap().iter().find(|s| s["id"] == surface).ok_or("未知工具条")?;
    let locale=crate::ui_language::text("zh-CN","en-US");
    let current = CURRENT.get_or_init(Default::default).lock().map_err(|_| "工具条目录不可用")?.get(surface).filter(|v|v["locale"]==locale).map(|v|v["items"].clone());
    Ok(json!({"version":1,"surface":surface,"locale":locale,"items":definition["items"],"current":current,
        "currentScope":"Most recent rendered context only; null before first use. Definitions include contextual controls, not an availability promise."}))
}
fn validate_current(surface: &str, items: &Value) -> Result<(), String> {
    let known = definitions();
    if !known["surfaces"].as_array().unwrap().iter().any(|s| s["id"] == surface) {return Err("未知工具条".into());}
    let values = items.as_array().filter(|v| v.len() <= 128).ok_or("工具条目录无效")?;
    let mut ids = std::collections::BTreeSet::new();
    for item in values {
        let object = item.as_object().ok_or("工具条项目无效")?;
        if object.keys().any(|k| !matches!(k.as_str(),"id"|"title"|"kind"|"contextual")) {return Err("工具条项目字段无效".into());}
        if object.get("contextual").is_some_and(|v|!v.is_boolean()){return Err("工具条项目字段无效".into());}
        let id = item["id"].as_str().filter(|v| !v.is_empty() && v.len() <= 128).ok_or("工具条项目 ID 无效")?;
        if !ids.insert(id) || !matches!(item["kind"].as_str(),Some("button"|"separator")) || item["title"].as_str().is_none_or(|v| v.len()>512) {return Err("工具条项目无效".into());}
    }
    Ok(())
}
#[tauri::command]
pub fn publish_toolbar_catalog(window: WebviewWindow, surface: String, locale:String, items: Value) -> Result<(), String> {
    if window.label() != "space" {return Err("此窗口不能发布工具条目录".into());}
    if !matches!(locale.as_str(),"zh-CN"|"en-US"){return Err("工具条语言无效".into());}
    validate_current(&surface,&items)?;
    CURRENT.get_or_init(Default::default).lock().map_err(|_|"工具条目录不可用")?.insert(surface,json!({"locale":locale,"items":items}));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn catalog_keeps_explicit_separators_and_rejects_untrusted_shape() {
        let catalog=definitions(); let items=&catalog["surfaces"][0]["items"];
        validate_current("capture.selection",items).unwrap();
        assert_eq!(items.as_array().unwrap().iter().filter(|i| i["kind"]=="separator").count(),3);
        assert!(validate_current("capture.selection",&json!([{"id":"a","title":"A","kind":"button","html":"bad"}])).is_err());
        assert!(validate_current("unknown",&json!([])).is_err());
    }
}
