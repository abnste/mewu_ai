// SPDX-License-Identifier: MPL-2.0
//! A typed contribution, not CSS or a plugin-specific host branch. Image data
//! is inert raster data; neither URLs nor executable markup enter host styles.
use crate::{ExtensionError, ServiceKind, ServiceRegistration};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use base64::Engine;

pub const APPEARANCE_CONTRACT: &str = include_str!("appearance-contract.json");
pub const APPEARANCE_SURFACE: &str = "application-theme";
pub const APPEARANCE_PERMISSION: &str = "ui.theme";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppearanceTheme {
    pub version: u32,
    pub base: String,
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    #[serde(default)]
    pub backgrounds: BTreeMap<String, AppearanceBackground>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppearanceBackground {
    /// Only a bounded, canonical base64 PNG. No URL, SVG, font or script.
    pub png: String,
    pub fit: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeMetadata {
    surface: String,
    theme: AppearanceTheme,
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

pub fn appearance_contract() -> serde_json::Value {
    serde_json::from_str(APPEARANCE_CONTRACT).expect("bundled appearance contract")
}
pub fn appearance_theme(service: &ServiceRegistration) -> Result<Option<AppearanceTheme>, ExtensionError> {
    if service.kind != ServiceKind::Ui || service.metadata.as_ref().and_then(|v|v.get("surface")).and_then(|v|v.as_str()) != Some(APPEARANCE_SURFACE) { return Ok(None); }
    let invalid = || ExtensionError::InvalidPackage("invalid appearance contribution");
    let metadata: ThemeMetadata = serde_json::from_value(service.metadata.clone().ok_or_else(invalid)?).map_err(|_|invalid())?;
    if metadata.surface != APPEARANCE_SURFACE || metadata.platforms.as_ref().is_some_and(|v| !crate::valid_plugin_platforms(v)) { return Err(invalid()); }
    let theme = metadata.theme;
    let contract = appearance_contract();
    let colors = contract["colors"].as_object().ok_or_else(invalid)?;
    if theme.version != 1 || !matches!(theme.base.as_str(), "light"|"dark")
        || theme.colors.iter().any(|(key,value)|!colors.contains_key(key) || value.len()!=7 || !value.starts_with('#') || !value[1..].bytes().all(|b|b.is_ascii_hexdigit()))
        || theme.backgrounds.len()>3 { return Err(invalid()); }
    for (surface, background) in &theme.backgrounds {
        if !matches!(surface.as_str(), "workspace"|"settings"|"composer") || !matches!(background.fit.as_str(),"cover"|"contain"|"repeat")
            || !valid_png(&background.png) { return Err(invalid()); }
    }
    Ok(Some(theme))
}

/// Canonical base64 and a bounded PNG header. The host renderer decodes the
/// raster; dimensions and encoded size bound that work before it reaches CSS.
fn valid_png(value:&str)->bool {
    if value.len()<44 || value.len()>256*1024 || value.len()%4!=0 { return false; }
    let Ok(header)=base64::engine::general_purpose::STANDARD.decode(value) else{return false};
    if header.len()<33{return false;}
    if header[..8]!=[137,80,78,71,13,10,26,10] || header[8..16]!=[0,0,0,13,73,72,68,82] {return false;}
    let w=u32::from_be_bytes(header[16..20].try_into().unwrap()); let h=u32::from_be_bytes(header[20..24].try_into().unwrap());
    w>0 && h>0 && w<=4096 && h<=4096 && u64::from(w)*u64::from(h)<=8_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn service(theme:serde_json::Value)->ServiceRegistration{serde_json::from_value(json!({"kind":"ui","id":"independent.theme","title":"Theme","metadata":{"surface":APPEARANCE_SURFACE,"theme":theme}})).unwrap()}
    #[test] fn appearance_closed_contract_rejects_css_urls_unknown_fields_and_tokens(){
        let base=json!({"version":1,"base":"dark","colors":{"accent":"#aabbcc"}});
        assert!(appearance_theme(&service(base.clone())).unwrap().is_some());
        for bad in [json!({"colors":{"accent":"red;display:none"}}),json!({"colors":{"arbitrary":"#aabbcc"}}),json!({"version":2}),json!({"base":"automatic"}),json!({"css":"*{display:none}"}),json!({"backgrounds":{"settings":{"png":"https://example.com/x.png","fit":"cover"}}})] {
            let mut value=base.clone();for (k,v) in bad.as_object().unwrap(){value[k]=v.clone();} assert!(appearance_theme(&service(value)).is_err());
        }
    }
    #[test] fn appearance_png_accepts_inert_bounded_pixels_and_rejects_bombs(){
        let png="iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==";
        assert!(valid_png(png));
        let mut bytes=base64::engine::general_purpose::STANDARD.decode(png).unwrap();
        bytes[16..20].copy_from_slice(&10000u32.to_be_bytes());
        assert!(!valid_png(&base64::engine::general_purpose::STANDARD.encode(bytes)));
        assert!(!valid_png("data:image/svg+xml;base64,AAAA"));
        assert!(!valid_png(&(png.to_owned()+"\n")));
    }
}
