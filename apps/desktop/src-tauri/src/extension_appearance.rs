// SPDX-License-Identifier: MPL-2.0
//! Read-only appearance projections for first-party windows. The projection
//! never includes source paths, credentials, tools or delegated authority.
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewWindow};
use crate::extension_host::ExtensionHost;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct AppearanceChoice { pub service_id:String, pub title:String, pub theme:mewu_extension::AppearanceTheme }
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct AppearanceRecord { pub plugin_id:String, pub revision:u64, pub themes:Vec<AppearanceChoice> }
pub(crate) fn projection(records:&[crate::extension_store::Record])->Vec<AppearanceRecord>{
    records.iter().map(|record|{
        let enabled=record.state=="enabled" && record.platform_error.is_none() && record.manifest.permissions.iter().any(|p|p==mewu_extension::APPEARANCE_PERMISSION);
        let themes=if enabled {record.services.iter().filter(|service|!record.unsupported_services.contains_key(&service.id)).filter_map(|service|{
            mewu_extension::appearance_theme(service).ok().flatten().map(|theme|AppearanceChoice{service_id:service.id.clone(),title:service.title.clone(),theme})
        }).collect()}else{vec![]};
        AppearanceRecord{plugin_id:record.manifest.id.clone(),revision:record.revision,themes}
    }).collect()
}
fn trusted(label:&str)->bool{
    matches!(label,"space"|"settings"|"frozen-widget"|"recording-controls"|"scroll-controls")
        || crate::pin_host::is_pin_label(label) || crate::extension_surface_window::id(label).is_some()
}
#[tauri::command]
pub(crate) fn get_extension_appearance(window:WebviewWindow,app:AppHandle)->Result<Vec<AppearanceRecord>,String>{
    if !trusted(window.label()){return Err("此窗口不能读取外观".into());}
    Ok(projection(&app.state::<ExtensionHost>().store()?.list()?))
}
