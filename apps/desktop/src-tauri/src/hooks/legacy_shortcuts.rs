// SPDX-License-Identifier: MPL-2.0
//! One-time import boundary only. No reserved rule ID, protected default, or
//! alternate execution route survives import; all effects use the action bus.
use super::*;
use crate::host_preferences::PreferencesFile;

pub(super) fn import(app: &AppHandle, root: &Path, store: &mut Store) -> Result<()> {
    if store.imported("legacy.global-shortcuts.v1")? {
        retire_extra_system_binding(store)?;
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
    let _ = app;
    let _ = &loaded.stamp;
    store.import_once("legacy.global-shortcuts.v1", rules)?;
    // New imports never manufacture a second binding from the old OS toggle.
    store.retire_import_once("legacy.extra-system-shortcut.v1", &[])?;
    Ok(())
}

fn retire_extra_system_binding(store: &mut Store) -> Result<()> {
    const SOURCE: &str = "legacy.extra-system-shortcut.v1";
    if store.imported(SOURCE)? {
        return Ok(());
    }
    let rules = store.rules()?;
    let mut expected = binding("系统截图", "Super+Shift+KeyS".into());
    shortcuts::canonicalize(&mut expected)?;
    let candidates: Vec<Rule> = rules
        .iter()
        .filter(|rule| {
            // Only the exact unedited 1.0.8 import and only when another usable
            // screenshot binding remains. Never drop a user's sole shortcut.
            if rule.revision != 1 {
                return false;
            }
            let mut baseline = expected.clone();
            baseline.id = rule.id.clone();
            baseline.revision = 1;
            baseline.enabled = rule.enabled;
            serde_json::to_value(rule).ok() == serde_json::to_value(&baseline).ok()
                && rules.iter().any(|other| {
                    other.id != rule.id
                        && other.enabled
                        && matches!(other.trigger, Trigger::Shortcut { .. })
                        && other
                            .actions
                            .iter()
                            .any(|a| matches!(a,Action::App {method,..} if method=="capture.open"))
                })
        })
        .cloned()
        .collect();
    store.retire_import_once(SOURCE, &candidates)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    fn imported() -> Store {
        let mut s = Store::memory().unwrap();
        let mut primary = binding("唤起截图", "Alt+Shift+KeyS".into());
        let mut extra = binding("系统截图", "Super+Shift+KeyS".into());
        shortcuts::canonicalize(&mut primary).unwrap();
        shortcuts::canonicalize(&mut extra).unwrap();
        s.import_once("legacy.global-shortcuts.v1", vec![primary, extra])
            .unwrap();
        s
    }
    #[test]
    fn retires_only_the_unedited_extra_and_does_not_recreate_it() {
        let mut s = imported();
        let primary = s.rules().unwrap()[0].clone();
        retire_extra_system_binding(&mut s).unwrap();
        assert_eq!(s.rules().unwrap(), vec![primary]);
        let revision = s.revision().unwrap();
        retire_extra_system_binding(&mut s).unwrap();
        assert_eq!(s.revision().unwrap(), revision);
    }
    #[test]
    fn edited_or_sole_shortcut_is_preserved() {
        let mut s = imported();
        let mut extra = s.rules().unwrap()[1].clone();
        extra.name = "我的截图".into();
        let saved = s.save(s.revision().unwrap(), extra).unwrap();
        retire_extra_system_binding(&mut s).unwrap();
        assert_eq!(s.rules().unwrap(), saved.rules);
        let mut s = imported();
        let primary = s.rules().unwrap()[0].clone();
        s.remove(s.revision().unwrap(), &primary.id, primary.revision)
            .unwrap();
        let before = s.rules().unwrap();
        retire_extra_system_binding(&mut s).unwrap();
        assert_eq!(s.rules().unwrap(), before);
    }
}
