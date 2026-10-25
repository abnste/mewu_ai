// SPDX-License-Identifier: MPL-2.0
use super::*;
use serde::Serialize;
use std::str::FromStr;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut};
#[cfg(target_os = "linux")]
mod wayland;

#[derive(Clone)]
struct Entry {
    rule_id: String,
    revision: u64,
    key: Shortcut,
}
#[derive(Clone, Serialize)]
pub(super) struct Status {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}
#[derive(Default)]
pub(super) struct Registry {
    update: Mutex<()>,
    keys: Mutex<HashMap<u32, Entry>>,
    pressed: Mutex<std::collections::HashSet<u32>>,
    status: Mutex<HashMap<String, Status>>,
    editor: Mutex<Option<(String, Instant)>>,
    #[cfg(target_os = "linux")]
    portal_epoch: AtomicU64,
    #[cfg(target_os = "linux")]
    portal_fault: AtomicBool,
    #[cfg(target_os = "linux")]
    portal_serial: tokio::sync::Mutex<()>,
}
fn parse(value: &str) -> Result<Shortcut> {
    let key = Shortcut::from_str(value).map_err(|_| "快捷键格式无效")?;
    let function = format!("{:?}", key.key)
        .strip_prefix('F')
        .is_some_and(|s| s.parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)));
    if !key
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
        && !function
        && format!("{:?}", key.key) != "PrintScreen"
    {
        return Err("全局快捷键需要 Control、Alt、Super，或使用功能键".into());
    }
    Ok(key)
}
pub(super) fn canonicalize(rule: &mut Rule) -> Result<()> {
    if let Trigger::Shortcut { accelerator } = &mut rule.trigger {
        *accelerator = parse(accelerator)?.to_string();
    }
    rule.validate()?;
    if !rule.condition.trim().is_empty() {
        mewu_hooks::validate_script(&rule.condition)?;
    }
    for action in &rule.actions {
        if let Action::Script { code } = action {
            mewu_hooks::validate_script(code)?;
        }
    }
    Ok(())
}
pub(super) fn save(r: &Shared, expected: u64, mut rule: Rule) -> Result<mewu_hooks::Snapshot> {
    let _update = lock(&r.shortcuts.update)?;
    canonicalize(&mut rule)?;
    let previous;
    {
        let store = lock(&r.store)?;
        if store.revision() != Ok(expected) {
            return Err("规则已变更，请重新载入".into());
        }
        previous = store.rule(&rule.id)?;
        if rule.enabled {
            if let Trigger::Shortcut { accelerator } = &rule.trigger {
                let wanted = parse(accelerator)?.id();
                for other in store.rules()? {
                    if other.enabled && other.id != rule.id {
                        if let Trigger::Shortcut { accelerator } = &other.trigger {
                            if parse(accelerator)?.id() == wanted {
                                return Err("此快捷键已被其他规则使用".into());
                            }
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    if crate::capture_shortcut_wayland_host::is_wayland() {
        let saved = lock(&r.store)?.save(expected, rule.clone())?;
        super::cancel_rule(r, &rule.id);
        if shortcut_binding_changed(previous.as_ref(), Some(&rule)) {
            clear(r);
            wayland::restart(runtime(&r.app)?);
        }
        return Ok(saved);
    }
    let wanted = if rule.enabled {
        if let Trigger::Shortcut { accelerator } = &rule.trigger {
            Some(parse(accelerator)?)
        } else {
            None
        }
    } else {
        None
    };
    let old = lock(&r.shortcuts.keys)?
        .values()
        .find(|entry| entry.rule_id == rule.id)
        .cloned();
    let mut reserved = None;
    if let Some(key) = wanted {
        if old.as_ref().is_none_or(|e| e.key.id() != key.id()) {
            if r.app.global_shortcut().is_registered(key) {
                return Err("此快捷键已被 Mewu 的其他功能使用".into());
            }
            r.app
                .global_shortcut()
                .register(key)
                .map_err(|_| "全局快捷键注册失败，可能已被系统或其他软件占用")?;
            reserved = Some(key);
        }
    }
    let saved = match lock(&r.store)?.save(expected, rule.clone()) {
        Ok(v) => v,
        Err(e) => {
            if let Some(key) = reserved {
                let _ = r.app.global_shortcut().unregister(key);
            }
            return Err(e);
        }
    };
    let revision = saved
        .rules
        .iter()
        .find(|v| v.id == rule.id)
        .ok_or("规则保存后不可用")?
        .revision;
    {
        let mut keys = lock(&r.shortcuts.keys)?;
        if let Some(old) = &old {
            keys.remove(&old.key.id());
        }
        if let Some(key) = wanted {
            keys.insert(
                key.id(),
                Entry {
                    rule_id: rule.id.clone(),
                    revision,
                    key,
                },
            );
        }
    }
    if let Some(old) = old {
        if wanted.is_none_or(|key| key.id() != old.key.id()) {
            let _ = r.app.global_shortcut().unregister(old.key);
        }
    }
    let mut statuses = lock(&r.shortcuts.status)?;
    statuses.remove(&rule.id);
    if let Some(key) = wanted {
        statuses.insert(
            rule.id.clone(),
            Status {
                status: "active",
                message: None,
                label: Some(key.to_string()),
            },
        );
    }
    drop(statuses);
    super::cancel_rule(r, &rule.id);
    Ok(saved)
}
// A changed rule revision also changes the event authority captured by the
// Portal session. Non-shortcut edits must not interrupt unrelated OS bindings.
#[cfg(any(target_os = "linux", test))]
fn shortcut_binding_changed(old: Option<&Rule>, new: Option<&Rule>) -> bool {
    [old, new]
        .into_iter()
        .flatten()
        .any(|r| r.enabled && matches!(r.trigger, Trigger::Shortcut { .. }))
}
pub(super) fn remove(r: &Shared, expected: u64, id: &str, revision: u64) -> Result<()> {
    let _update = lock(&r.shortcuts.update)?;
    let previous = lock(&r.store)?.rule(id)?;
    lock(&r.store)?.remove(expected, id, revision)?;
    super::cancel_rule(r, id);
    #[cfg(target_os = "linux")]
    if crate::capture_shortcut_wayland_host::is_wayland() {
        if shortcut_binding_changed(previous.as_ref(), None) {
            clear(r);
            wayland::restart(runtime(&r.app)?);
        }
        return Ok(());
    }
    let _ = previous;
    let removed = {
        let mut keys = lock(&r.shortcuts.keys)?;
        let key = keys
            .iter()
            .find_map(|(key, entry)| (entry.rule_id == id).then_some(*key));
        key.and_then(|key| keys.remove(&key))
    };
    if let Some(entry) = removed {
        r.app
            .global_shortcut()
            .unregister(entry.key)
            .map_err(|_| "快捷键注销失败")?;
        lock(&r.shortcuts.pressed)?.remove(&entry.key.id());
    }
    lock(&r.shortcuts.status)?.remove(id);
    Ok(())
}
pub(super) fn clear(r: &Shared) {
    #[cfg(target_os = "linux")]
    r.shortcuts.portal_epoch.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut pressed) = r.shortcuts.pressed.lock() {
        pressed.clear();
    }
    let old = r
        .shortcuts
        .keys
        .lock()
        .map(|mut keys| keys.drain().map(|(_, e)| e.key).collect::<Vec<_>>())
        .unwrap_or_default();
    for key in old {
        let _ = r.app.global_shortcut().unregister(key);
    }
    if let Ok(mut states) = r.shortcuts.status.lock() {
        states.clear();
    }
    end_edit(r, None);
}
pub(super) fn reconcile(r: &Shared) -> Result<()> {
    let _update = lock(&r.shortcuts.update)?;
    clear(r);
    if !r.enabled.load(Ordering::Acquire) {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if crate::capture_shortcut_wayland_host::is_wayland() {
        wayland::restart(runtime(&r.app)?);
        return Ok(());
    }
    let rules = lock(&r.store)?.rules()?;
    for rule in rules.into_iter().filter(|r| r.enabled) {
        if let Trigger::Shortcut { accelerator } = &rule.trigger {
            let key = parse(accelerator)?;
            match r.app.global_shortcut().register(key) {
                Ok(()) => {
                    lock(&r.shortcuts.keys)?.insert(
                        key.id(),
                        Entry {
                            rule_id: rule.id.clone(),
                            revision: rule.revision,
                            key,
                        },
                    );
                    lock(&r.shortcuts.status)?.insert(
                        rule.id,
                        Status {
                            status: "active",
                            message: None,
                            label: Some(key.to_string()),
                        },
                    );
                }
                Err(_) => {
                    lock(&r.shortcuts.status)?.insert(
                        rule.id,
                        Status {
                            status: "unavailable",
                            message: Some("全局快捷键被占用或不可用".into()),
                            label: None,
                        },
                    );
                }
            }
        }
    }
    Ok(())
}
pub(super) fn statuses(r: &Shared) -> HashMap<String, Status> {
    r.shortcuts
        .status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}
pub(super) fn awaiting_release(r: &Shared) -> Result<bool> {
    Ok(!lock(&r.shortcuts.pressed)?.is_empty())
}
pub(super) fn edit(r: &Shared, id: String, active: bool) -> Result<()> {
    mewu_hooks::valid_uuid(&id)?;
    if active {
        *lock(&r.shortcuts.editor)? = Some((id, Instant::now() + Duration::from_secs(30)));
    } else {
        end_edit(r, Some(&id));
    }
    Ok(())
}
pub(super) fn end_edit(r: &Shared, id: Option<&str>) {
    let ended = r.shortcuts.editor.lock().ok().and_then(|mut edit| {
        if id.is_none()
            || edit
                .as_ref()
                .is_some_and(|(current, _)| Some(current.as_str()) == id)
        {
            edit.take().map(|(id, _)| id)
        } else {
            None
        }
    });
    if let Some(id) = ended {
        let _ = crate::capture_shortcut_host::end_editor(&r.app, &id);
    }
}
pub(super) fn recording(r: &Shared, accelerator: Option<&str>) -> bool {
    let own = r.shortcuts.editor.lock().ok().and_then(|mut edit| {
        if edit
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() < *until)
        {
            edit.as_ref().map(|(id, _)| id.clone())
        } else {
            *edit = None;
            None
        }
    });
    if let Some(lease_id) = own {
        if let Some(accelerator) = accelerator {
            let _ = r.app.emit_to(
                "settings",
                "hooks-shortcut-recorded",
                json!({"leaseId":lease_id,"accelerator":accelerator}),
            );
        }
        return true;
    }
    crate::capture_shortcut_host::editor_active(&r.app)
}
pub(super) fn route(r: &Shared, key: &Shortcut, pressed: bool) -> bool {
    let entry = r
        .shortcuts
        .keys
        .lock()
        .ok()
        .and_then(|keys| keys.get(&key.id()).cloned());
    let Some(entry) = entry else { return false };
    if let Ok(mut keys) = r.shortcuts.pressed.lock() {
        if !pressed {
            keys.remove(&key.id());
            return true;
        }
        if !keys.insert(key.id()) {
            return true;
        }
    }
    if require(r).is_err() || r.input_active.load(Ordering::Acquire) {
        return true;
    }
    if recording(r, Some(&key.to_string())) {
        return true;
    }
    let mut event = Event::new("shortcut.pressed");
    event.payload = json!({"ruleId":entry.rule_id,"ruleRevision":entry.revision});
    emit(&r.app, event);
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rule(trigger: serde_json::Value) -> Rule {
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"revision":0,"name":"Synthetic","enabled":true,"trigger":trigger,"condition":"","actions":[{"kind":"delay","milliseconds":0}],"cooldownMs":500,"timeoutMs":1000,"stopOnError":true})).unwrap()
    }
    #[test]
    fn typing_shortcuts_and_modifier_only_keys_are_not_global_bindings() {
        for key in ["KeyA", "Shift+KeyA", "Control", "Alt", "Space", "Enter"] {
            assert!(parse(key).is_err(), "{key}");
        }
        for key in [
            "Control+Alt+KeyK",
            "Super+Shift+Digit1",
            "F1",
            "F24",
            "PrintScreen",
        ] {
            let canonical = parse(key).unwrap().to_string();
            assert_eq!(parse(&canonical).unwrap().id(), parse(key).unwrap().id());
        }
    }
    #[test]
    fn non_shortcut_edits_do_not_replace_os_sessions_but_authority_changes_do() {
        let ordinary = rule(json!({"kind":"event","name":"app.started"}));
        let hotkey = rule(json!({"kind":"shortcut","accelerator":"Control+Alt+KeyK"}));
        assert!(!shortcut_binding_changed(None, Some(&ordinary)));
        assert!(!shortcut_binding_changed(Some(&ordinary), None));
        assert!(!shortcut_binding_changed(Some(&ordinary), Some(&ordinary)));
        assert!(shortcut_binding_changed(Some(&ordinary), Some(&hotkey)));
        assert!(shortcut_binding_changed(Some(&hotkey), Some(&ordinary)));
        assert!(shortcut_binding_changed(Some(&hotkey), None));
        let mut disabled = hotkey.clone();
        disabled.enabled = false;
        assert!(!shortcut_binding_changed(Some(&disabled), None));
    }
}
