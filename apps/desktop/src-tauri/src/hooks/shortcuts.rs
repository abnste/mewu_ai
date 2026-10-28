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
    hold: Option<u32>,
}
#[derive(Clone, Serialize)]
pub(super) struct Status {
    pub(super) status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) label: Option<String>,
}
#[derive(Default)]
pub(super) struct Registry {
    update: Mutex<()>,
    keys: Mutex<HashMap<u32, Entry>>,
    pressed: Mutex<std::collections::HashSet<u32>>,
    hold_tokens: Mutex<HashMap<u32, u64>>,
    hold_epoch: AtomicU64,
    status: Mutex<HashMap<String, Status>>,
    editor: Mutex<Option<(String, Instant)>>,
    #[cfg(target_os = "linux")]
    portal_epoch: AtomicU64,
    #[cfg(target_os = "linux")]
    portal_fault: AtomicBool,
    #[cfg(target_os = "linux")]
    portal_serial: tokio::sync::Mutex<()>,
}
impl Registry {
    pub(super) fn statuses_for_cache(&self) -> Option<HashMap<String, Status>> {
        self.status.try_lock().ok().map(|s| s.clone())
    }
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
fn binding(trigger: &Trigger) -> Result<Option<Shortcut>> {
    match trigger {
        Trigger::Hold { accelerator, .. } if accelerator == "Super" => {
            #[cfg(windows)]
            {
                return Ok(Some(Shortcut::new(
                    None,
                    tauri_plugin_global_shortcut::Code::MetaLeft,
                )));
            }
            #[cfg(not(windows))]
            {
                return Err("此系统不支持单独长按系统修饰键；请选择功能键或组合键".into());
            }
        }
        Trigger::Shortcut { accelerator } | Trigger::Hold { accelerator, .. } => {
            parse(accelerator).map(Some)
        }
        _ => Ok(None),
    }
}
fn hold(rule: &Rule) -> Option<u32> {
    if let Trigger::Hold { milliseconds, .. } = &rule.trigger {
        Some(*milliseconds)
    } else {
        None
    }
}
#[cfg(windows)]
fn modifier_hold(key: &Shortcut) -> bool {
    key.mods.is_empty() && key.key == tauri_plugin_global_shortcut::Code::MetaLeft
}
// Windows reserves this physical chord. Its native transport feeds the same
// generic rule registry; it has no capture action or special rule identity.
#[cfg(windows)]
fn reserved_transport(key: &Shortcut) -> bool {
    key.mods == (Modifiers::SUPER | Modifiers::SHIFT) && format!("{:?}", key.key) == "KeyS"
}
fn register(r: &Shared, key: Shortcut, hold: Option<u32>) -> Result<()> {
    #[cfg(windows)]
    if modifier_hold(&key) {
        return crate::hold_key_windows::configure(&r.app, hold.ok_or("长按快捷键缺少时长")?);
    }
    #[cfg(not(windows))]
    let _ = hold;
    #[cfg(windows)]
    if reserved_transport(&key) {
        crate::system_capture_shortcut::Change::apply(&r.app, true)?.commit();
        return Ok(());
    }
    r.app
        .global_shortcut()
        .register(key)
        .map_err(|_| "全局快捷键注册失败，可能已被系统或其他软件占用".into())
}
fn unregister(r: &Shared, key: Shortcut) -> Result<()> {
    #[cfg(windows)]
    if modifier_hold(&key) {
        crate::hold_key_windows::disable();
        return Ok(());
    }
    #[cfg(windows)]
    if reserved_transport(&key) {
        crate::system_capture_shortcut::Change::apply(&r.app, false)?.commit();
        return Ok(());
    }
    r.app
        .global_shortcut()
        .unregister(key)
        .map_err(|_| "快捷键注销失败".into())
}
#[cfg(windows)]
pub(super) fn system_pressed(r: &Shared) {
    if let Ok(key) = parse("Super+Shift+KeyS") {
        route(r, &key, true);
        route(r, &key, false);
    }
}
pub(super) fn canonicalize(rule: &mut Rule) -> Result<()> {
    if let Trigger::Shortcut { accelerator } = &mut rule.trigger {
        *accelerator = parse(accelerator)?.to_string();
    }
    if let Trigger::Hold { accelerator, .. } = &mut rule.trigger {
        if matches!(accelerator.as_str(), "Win" | "Meta" | "Super") {
            *accelerator = "Super".into();
        } else {
            *accelerator = parse(accelerator)?.to_string();
        }
    }
    let _ = binding(&rule.trigger)?;
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
    #[cfg(target_os = "linux")]
    let previous;
    {
        let store = lock(&r.store)?;
        if store.revision() != Ok(expected) {
            return Err("规则已变更，请重新载入".into());
        }
        #[cfg(target_os = "linux")]
        {
            previous = store.rule(&rule.id)?;
        }
        if rule.enabled {
            if let Some(wanted) = binding(&rule.trigger)? {
                let wanted = wanted.id();
                for other in store.rules()? {
                    if other.enabled && other.id != rule.id {
                        if let Some(other_key) = binding(&other.trigger)? {
                            if other_key.id() == wanted {
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
        binding(&rule.trigger)?
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
            register(r, key, hold(&rule))?;
            reserved = Some(key);
        }
    }
    #[cfg(windows)]
    let changed_hold = if let (Some(old), Some(key)) = (&old, wanted) {
        modifier_hold(&key) && old.hold != hold(&rule)
    } else {
        false
    };
    #[cfg(windows)]
    if changed_hold {
        register(r, wanted.expect("checked key"), hold(&rule))?;
    }
    let saved = match lock(&r.store)?.save(expected, rule.clone()) {
        Ok(v) => v,
        Err(e) => {
            #[cfg(windows)]
            if changed_hold {
                if let Some(old) = &old {
                    let _ = register(r, old.key, old.hold);
                }
            }
            if let Some(key) = reserved {
                let _ = unregister(r, key);
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
                    hold: hold(&rule),
                },
            );
        }
    }
    if let Some(old) = old {
        if wanted.is_none_or(|key| key.id() != old.key.id()) {
            let _ = unregister(r, old.key);
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
    crate::quick_capture::withdraw(&r.app, &rule.id);
    Ok(saved)
}
// A changed rule revision also changes the event authority captured by the
// Portal session. Non-shortcut edits must not interrupt unrelated OS bindings.
#[cfg(any(target_os = "linux", test))]
fn shortcut_binding_changed(old: Option<&Rule>, new: Option<&Rule>) -> bool {
    [old, new]
        .into_iter()
        .flatten()
        .any(|r| r.enabled && r.trigger.accelerator().is_some())
}
pub(super) fn remove(r: &Shared, expected: u64, id: &str, revision: u64) -> Result<()> {
    let _update = lock(&r.shortcuts.update)?;
    let previous = lock(&r.store)?.rule(id)?;
    lock(&r.store)?.remove(expected, id, revision)?;
    crate::quick_capture::withdraw(&r.app, id);
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
        unregister(r, entry.key)?;
        lock(&r.shortcuts.pressed)?.remove(&entry.key.id());
    }
    lock(&r.shortcuts.status)?.remove(id);
    Ok(())
}
pub(super) fn clear(r: &Shared) {
    if let Ok(mut held) = r.shortcuts.hold_tokens.lock() {
        held.clear();
    }
    crate::quick_capture::detach(&r.app);
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
        let _ = unregister(r, key);
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
        if let Some(key) = binding(&rule.trigger)? {
            match register(r, key, hold(&rule)) {
                Ok(()) => {
                    lock(&r.shortcuts.keys)?.insert(
                        key.id(),
                        Entry {
                            rule_id: rule.id.clone(),
                            revision: rule.revision,
                            key,
                            hold: hold(&rule),
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
    let values = r
        .shortcuts
        .status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    #[cfg(windows)]
    {
        let mut values = values;
        if let Some(message) = crate::hold_key_windows::warning() {
            if let Ok(keys) = r.shortcuts.keys.lock() {
                for entry in keys.values().filter(|entry| modifier_hold(&entry.key)) {
                    if let Some(value) = values.get_mut(&entry.rule_id) {
                        value.status = "unavailable";
                        value.message = Some(message.into());
                    }
                }
            }
        }
        return values;
    }
    #[cfg(not(windows))]
    values
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
            if let Ok(mut tokens) = r.shortcuts.hold_tokens.lock() {
                tokens.remove(&key.id());
            }
            if entry.hold.is_some() {
                crate::quick_capture::released(&r.app, &entry.rule_id);
            }
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
    if entry.hold.is_some() {
        schedule_hold(r, entry, key.id());
        return true;
    }
    let mut event = Event::new("shortcut.pressed");
    event.payload = json!({"ruleId":entry.rule_id,"ruleRevision":entry.revision});
    emit(&r.app, event);
    true
}
fn schedule_hold(r: &Shared, entry: Entry, key_id: u32) {
    let Some(milliseconds) = entry.hold else {
        return;
    };
    let Some(shared) = runtime(&r.app).ok() else {
        return;
    };
    let token = r.shortcuts.hold_epoch.fetch_add(1, Ordering::AcqRel) + 1;
    if let Ok(mut tokens) = r.shortcuts.hold_tokens.lock() {
        tokens.insert(key_id, token);
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(milliseconds as u64)).await;
        if require(&shared).is_err()
            || !shared
                .shortcuts
                .pressed
                .lock()
                .is_ok_and(|keys| keys.contains(&key_id))
        {
            return;
        }
        if !shared
            .shortcuts
            .hold_tokens
            .lock()
            .is_ok_and(|keys| keys.get(&key_id) == Some(&token))
        {
            return;
        }
        if !shared.store.lock().is_ok_and(|store| {
            store.rule(&entry.rule_id).is_ok_and(|rule| {
                rule.is_some_and(|rule| {
                    rule.enabled
                        && rule.revision == entry.revision
                        && hold(&rule) == Some(milliseconds)
                })
            })
        }) {
            return;
        }
        crate::quick_capture::held(&shared.app, &entry.rule_id);
        let mut event = Event::new("shortcut.held");
        event.payload = json!({"ruleId":entry.rule_id,"ruleRevision":entry.revision});
        emit(&shared.app, event);
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn reserved_windows_transport_has_no_feature_action_or_rule_identity() {
        assert!(reserved_transport(&parse("Super+Shift+KeyS").unwrap()));
        for key in ["Control+Shift+KeyS", "Super+KeyS", "Super+Shift+KeyA", "F8"] {
            assert!(!reserved_transport(&parse(key).unwrap()));
        }
    }
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
        let held = rule(json!({"kind":"hold","accelerator":"F8","milliseconds":400}));
        assert!(shortcut_binding_changed(None, Some(&held)));
        assert!(shortcut_binding_changed(Some(&held), None));
        let mut disabled = hotkey.clone();
        disabled.enabled = false;
        assert!(!shortcut_binding_changed(Some(&disabled), None));
    }
}
