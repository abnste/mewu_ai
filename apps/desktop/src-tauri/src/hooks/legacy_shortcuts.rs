// SPDX-License-Identifier: MPL-2.0
//! One-time import boundary only. No reserved rule ID, protected default, or
//! alternate execution route survives import; all effects use the action bus.
use super::*;
use crate::host_preferences::PreferencesFile;

pub(super) fn import(app: &AppHandle, root: &Path, store: &mut Store) -> Result<()> {
    if store.imported("legacy.global-shortcuts.v1")? {
        return Ok(());
    }
    let loaded = PreferencesFile::open(root)
        .map_err(|e| e.to_string())?
        .load()
        .map_err(|e| e.to_string())?;
    let mut rules = Vec::new();
    if let Some(chord) = loaded.preferences.capture_shortcut {
        let mut rule = binding(
            "唤起截图",
            [
                chord.ctrl.then_some("Control"),
                chord.shift.then_some("Shift"),
                chord.alt.then_some("Alt"),
                Some(chord.code.as_str()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("+"),
        );
        #[cfg(target_os = "linux")]
        if crate::capture_shortcut_wayland_host::is_wayland()
            && loaded.stamp != crate::host_preferences::FileStamp::Missing
        {
            rule.enabled = loaded.preferences.wayland_shortcut_enabled;
        }
        shortcuts::canonicalize(&mut rule)?;
        rules.push(rule);
    }
    #[cfg(windows)]
    {
        let mut rule = binding("系统截图", "Super+Shift+KeyS".into());
        rule.enabled = app
            .state::<crate::Host>()
            .system_preferences
            .view()
            .map_err(|e| e.to_string())?
            .replace_system_capture_shortcut;
        shortcuts::canonicalize(&mut rule)?;
        rules.push(rule);
    }
    let _ = app;
    let _ = &loaded.stamp;
    store.import_once("legacy.global-shortcuts.v1", rules)?;
    Ok(())
}
pub(super) fn binding(name: &str, accelerator: String) -> Rule {
    Rule {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 0,
        name: name.into(),
        enabled: true,
        trigger: Trigger::Shortcut { accelerator },
        condition: String::new(),
        actions: vec![Action::App {
            method: "capture.open".into(),
            params: json!({}),
        }],
        cooldown_ms: 500,
        timeout_ms: 30000,
        stop_on_error: true,
    }
}
