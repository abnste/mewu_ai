// SPDX-License-Identifier: MPL-2.0
//! Native Hooks plugin. The rule engine has no desktop privileges; this module
//! grants effects only to current local user configuration, never event text.
mod actions;
mod input;
mod legacy_shortcuts;
mod shortcuts;
use mewu_hooks::{Action, Event, Rule, Store, Trigger};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::sync::{mpsc, Notify, Semaphore};

pub(super) type Result<T> = std::result::Result<T, String>;
pub(super) struct Shared {
    app: AppHandle,
    store: Mutex<Store>,
    enabled: AtomicBool,
    generation: AtomicU64,
    grant: Mutex<Option<(String, u64)>>,
    jobs: Mutex<HashMap<String, Job>>,
    sender: mpsc::Sender<QueuedEvent>,
    wake: Notify,
    stopped: Notify,
    stop: AtomicBool,
    slots: Arc<Semaphore>,
    prompt_slot: Arc<Semaphore>,
    pending: Mutex<HashSet<String>>,
    shortcuts: shortcuts::Registry,
    input_active: AtomicBool,
    quitting: AtomicBool,
    started: AtomicBool,
    run_ancestry: Mutex<HashMap<String, Lineage>>,
    prompt_origins: Mutex<HashMap<String, PromptOrigin>>,
    last_scene: Mutex<Option<String>>,
    startup_warning: Option<String>,
    operation_shortcuts: Mutex<HashMap<String, String>>,
}
#[derive(Clone)]
pub(crate) struct Runtime(Arc<Shared>);
struct Job {
    rule_id: String,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone)]
struct Lineage {
    ancestors: Vec<String>,
    depth: u8,
}
struct QueuedEvent {
    event: Event,
    generation: u64,
    key: Option<String>,
}
struct PromptOrigin {
    fence: actions::Fence,
    context: PromptContext,
    scene: Option<String>,
    text_sha256: [u8; 32],
    until: Instant,
}
struct JobGuard {
    shared: Arc<Shared>,
    id: String,
    cancel: Arc<AtomicBool>,
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Ok(mut store) = self.shared.store.lock() {
            let _ = store.finish(&self.id, "canceled", Some("执行已停止，未自动重试"));
        }
        if let Ok(mut jobs) = self.shared.jobs.lock() {
            jobs.remove(&self.id);
        }
        changed(&self.shared);
    }
}
pub(super) fn runtime(app: &AppHandle) -> Result<Arc<Shared>> {
    app.try_state::<Runtime>()
        .map(|r| r.0.clone())
        .ok_or("事件触发器未就绪".into())
}
pub(super) fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock().map_err(|_| "事件触发器状态不可用".into())
}
pub(crate) fn enabled(app: &AppHandle) -> bool {
    app.try_state::<Runtime>()
        .is_some_and(|r| r.0.enabled.load(Ordering::Acquire) && !r.0.stop.load(Ordering::Acquire))
}
fn require(r: &Shared) -> Result<()> {
    if r.enabled.load(Ordering::Acquire) && !r.stop.load(Ordering::Acquire) {
        r.app.state::<crate::Host>().exit.ensure_new_operation()
    } else {
        Err("事件触发器插件未启用".into())
    }
}
fn settings(app: &AppHandle, window: &WebviewWindow) -> Result<Arc<Shared>> {
    if window.label() != "settings" {
        return Err("请在设置中管理事件触发器".into());
    }
    let r = runtime(app)?;
    require(&r)?;
    Ok(r)
}
pub(super) fn changed(r: &Shared) {
    // Snapshot publication (including every pen/undo receipt) reads only this
    // cache. Never parse the automation store on the drawing hot path.
    let labels = (|| {
        let store = r.store.try_lock().ok()?;
        let rules = store.rules().ok()?;
        let states = r.shortcuts.statuses_for_cache()?;
        let mut values: HashMap<String, Vec<String>> = HashMap::new();
        for rule in rules.into_iter().filter(|rule| rule.enabled) {
            if let Some(label) = states
                .get(&rule.id)
                .and_then(|s| s.label.clone())
                .filter(|_| states[&rule.id].status == "active")
            {
                for action in rule.actions {
                    if let Action::App { method, .. } = action {
                        let entries = values.entry(method).or_default();
                        if !entries.contains(&label) {
                            entries.push(label.clone());
                        }
                    }
                }
            }
        }
        Some(
            values
                .into_iter()
                .map(|(method, labels)| (method, labels.join(" / ")))
                .collect(),
        )
    })();
    if let Ok(mut cache) = r.operation_shortcuts.lock() {
        *cache = labels.unwrap_or_default();
    }
    let _ = r.app.emit_to("settings", "hooks-changed", ());
}
pub(crate) fn shortcut_label(app: &AppHandle, method: &str) -> String {
    if !enabled(app) {
        return String::new();
    }
    runtime(app)
        .ok()
        .and_then(|r| {
            r.operation_shortcuts
                .lock()
                .ok()
                .and_then(|v| v.get(method).cloned())
        })
        .unwrap_or_default()
}
impl Runtime {
    pub(crate) fn initialize(
        app: &AppHandle,
        root: &Path,
        snapshot: &crate::plugins::PluginSnapshot,
    ) -> Result<()> {
        let mut store = Store::open(&root.join("hooks.db"))?;
        let startup_warning = legacy_shortcuts::import(app, root, &mut store).err();
        let (sender, receiver) = mpsc::channel(64);
        let last_scene = app
            .state::<crate::Host>()
            .lock()?
            .store
            .active_scene()
            .ok()
            .map(|scene| scene.id.clone());
        let shared = Arc::new(Shared {
            app: app.clone(),
            store: Mutex::new(store),
            enabled: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            grant: Mutex::new(None),
            jobs: Mutex::new(HashMap::new()),
            sender,
            wake: Notify::new(),
            stopped: Notify::new(),
            stop: AtomicBool::new(false),
            slots: Arc::new(Semaphore::new(2)),
            prompt_slot: Arc::new(Semaphore::new(1)),
            pending: Mutex::new(HashSet::new()),
            shortcuts: shortcuts::Registry::default(),
            input_active: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            started: AtomicBool::new(false),
            run_ancestry: Mutex::new(HashMap::new()),
            prompt_origins: Mutex::new(HashMap::new()),
            last_scene: Mutex::new(last_scene),
            startup_warning,
            operation_shortcuts: Mutex::new(HashMap::new()),
        });
        app.manage(Self(shared.clone()));
        reconcile(app, snapshot);
        tauri::async_runtime::spawn(event_worker(shared.clone(), receiver));
        tauri::async_runtime::spawn(scheduler(shared.clone()));
        Ok(())
    }
}
pub(crate) fn ready(app: &AppHandle) {
    if let Ok(r) = runtime(app) {
        if app.state::<crate::Host>().storage.is_ready()
            && crate::app_control_host::renderer_ready(app)
            && !r.started.swap(true, Ordering::AcqRel)
        {
            emit(app, Event::new("app.started"));
        }
    }
}
#[cfg(windows)]
pub(crate) fn system_shortcut_pressed(app: &AppHandle) {
    if let Ok(r) = runtime(app) {
        shortcuts::system_pressed(&r)
    }
}

/// Compatibility query for the former single-shortcut settings API. The rule
/// store is the sole source; no second shortcut preference or OS binding exists.
pub(crate) fn shortcut_state(
    app: &AppHandle,
    method: &str,
) -> Result<crate::capture_shortcut::CaptureShortcutState> {
    use crate::capture_shortcut::{CaptureShortcutState, ShortcutChord, ShortcutStatus};
    use tauri_plugin_global_shortcut::Modifiers;
    let r = runtime(app)?;
    let snapshot = lock(&r.store)?.snapshot()?;
    let rule = snapshot.rules.iter().find(|rule| {
        matches!(rule.trigger, Trigger::Shortcut { .. })
            && rule
                .actions
                .iter()
                .any(|a| matches!(a,Action::App{method:m,..}if m==method))
    });
    let configured = rule.and_then(|r| {
        if let Trigger::Shortcut { accelerator } = &r.trigger {
            accelerator
                .parse::<tauri_plugin_global_shortcut::Shortcut>()
                .ok()
        } else {
            None
        }
    });
    let chord = configured.as_ref().and_then(|k| {
        let chord = ShortcutChord {
            code: format!("{:?}", k.key),
            ctrl: k.mods.contains(Modifiers::CONTROL),
            alt: k.mods.contains(Modifiers::ALT),
            shift: k.mods.contains(Modifiers::SHIFT),
        };
        (!k.mods.contains(Modifiers::SUPER) && chord.validate().is_ok()).then_some(chord)
    });
    let statuses = shortcuts::statuses(&r);
    let state = rule.and_then(|v| statuses.get(&v.id));
    let wanted = rule.is_some_and(|v| v.enabled) && enabled(app);
    let active = wanted && state.is_some_and(|s| s.status == "active");
    let label = state.and_then(|s| s.label.clone());
    Ok(CaptureShortcutState {
        revision: snapshot.revision,
        sequence: snapshot.revision,
        configured: chord.clone(),
        active: active.then_some(chord).flatten(),
        status: if active {
            ShortcutStatus::Active
        } else if wanted {
            ShortcutStatus::Unavailable
        } else {
            ShortcutStatus::Disabled
        },
        editable: enabled(app),
        message: state.and_then(|s| s.message.clone()),
        backend: Some("native"),
        actual_trigger_description: active.then_some(label).flatten(),
        configuration_pending: None,
        system_settings_available: None,
        configuration_request_id: None,
        portal_enabled: None,
    })
}
pub(crate) fn change_action_shortcut(
    app: &AppHandle,
    expected: u64,
    method: &str,
    chord: Option<crate::host_preferences::ShortcutChord>,
) -> Result<crate::capture_shortcut::CaptureShortcutState> {
    let r = runtime(app)?;
    require(&r)?;
    let rule = lock(&r.store)?.rules()?.into_iter().find(|rule| {
        matches!(rule.trigger, Trigger::Shortcut { .. })
            && rule
                .actions
                .iter()
                .any(|a| matches!(a,Action::App{method:m,..}if m==method))
    });
    match (rule, chord) {
        (Some(rule), None) => shortcuts::remove(&r, expected, &rule.id, rule.revision)?,
        (None, None) => {
            if lock(&r.store)?.revision()? != expected {
                return Err("规则已变更，请重新载入".into());
            }
        }
        (previous, Some(chord)) => {
            chord.validate().map_err(|e| e.to_string())?;
            let accelerator = [
                chord.ctrl.then_some("Control"),
                chord.shift.then_some("Shift"),
                chord.alt.then_some("Alt"),
                Some(chord.code.as_str()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("+");
            let mut rule = previous.unwrap_or_else(|| Rule {
                id: uuid::Uuid::new_v4().to_string(),
                revision: 0,
                name: method.into(),
                enabled: true,
                trigger: Trigger::Shortcut {
                    accelerator: accelerator.clone(),
                },
                condition: String::new(),
                actions: vec![Action::App {
                    method: method.into(),
                    params: json!({}),
                }],
                cooldown_ms: 500,
                timeout_ms: 30000,
                stop_on_error: true,
            });
            rule.trigger = Trigger::Shortcut { accelerator };
            rule.enabled = true;
            shortcuts::save(&r, expected, rule)?;
        }
    }
    r.wake.notify_one();
    changed(&r);
    shortcut_state(app, method)
}
pub(crate) fn reconcile(app: &AppHandle, snapshot: &crate::plugins::PluginSnapshot) {
    let Some(r) = app.try_state::<Runtime>() else {
        return;
    };
    let r = r.0.clone();
    // Only a verified native installation carries this authority. Community
    // packages cannot turn a declaration into local user-code execution.
    let grant = snapshot
        .plugins
        .iter()
        .find(|p| {
            p.source == crate::plugins::PluginSource::Official
                && p.state == crate::plugins::PluginState::Enabled
                && p.error.is_none()
                && p.manifest
                    .contributions
                    .iter()
                    .any(|c| matches!(c, crate::plugins::PluginContribution::Hooks { .. }))
        })
        .map(|p| (p.manifest.id.clone(), p.revision));
    let Ok(mut old) = r.grant.lock() else { return };
    if *old == grant {
        return;
    }
    *old = grant.clone();
    drop(old);
    r.generation.fetch_add(1, Ordering::AcqRel);
    r.enabled.store(grant.is_some(), Ordering::Release);
    cancel_all(&r);
    r.wake.notify_one();
    let r2 = r.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = shortcuts::reconcile(&r2);
        changed(&r2);
    });
}
fn cancel_all(r: &Shared) {
    if let Ok(jobs) = r.jobs.lock() {
        for job in jobs.values() {
            job.cancel.store(true, Ordering::Release);
        }
    }
}
fn cancel_rule(r: &Shared, id: &str) {
    if let Ok(jobs) = r.jobs.lock() {
        for job in jobs.values().filter(|j| j.rule_id == id) {
            job.cancel.store(true, Ordering::Release);
        }
    }
}
pub(crate) fn shutdown(app: &AppHandle) {
    if let Some(r) = app.try_state::<Runtime>() {
        r.0.stop.store(true, Ordering::Release);
        r.0.enabled.store(false, Ordering::Release);
        r.0.generation.fetch_add(1, Ordering::AcqRel);
        cancel_all(&r.0);
        r.0.wake.notify_one();
        r.0.stopped.notify_waiters();
        shortcuts::clear(&r.0);
    }
}
pub(crate) fn pause(app: &AppHandle) {
    if let Ok(r) = runtime(app) {
        r.quitting.store(true, Ordering::Release);
        cancel_all(&r);
    }
}
pub(crate) fn resume(app: &AppHandle) {
    if let Ok(r) = runtime(app) {
        r.quitting.store(false, Ordering::Release);
        r.wake.notify_one();
    }
}
pub(crate) async fn drain(app: &AppHandle, timeout: Duration) -> Result<()> {
    let Ok(r) = runtime(app) else { return Ok(()) };
    pause(app);
    let until = Instant::now() + timeout;
    loop {
        if lock(&r.jobs)?.is_empty() && !r.input_active.load(Ordering::Acquire) {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err("事件触发器尚未停止，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
pub(crate) async fn before_quit(app: &AppHandle) {
    if !enabled(app) {
        return;
    }
    let Ok(r) = runtime(app) else { return };
    pause(app);
    let event = Event::new("app.beforeQuit");
    let generation = r.generation.load(Ordering::Acquire);
    let rules = lock(&r.store).and_then(|s| s.rules()).unwrap_or_default();
    // A user command cannot hold the host's exit/installer indefinitely.
    let until = Instant::now() + Duration::from_secs(3);
    for rule in rules {
        if rule.matches(&event).unwrap_or(false) {
            if tokio::time::timeout(
                until.saturating_duration_since(Instant::now()),
                execute(r.clone(), rule, event.clone(), false, generation),
            )
            .await
            .is_err()
            {
                break;
            }
        }
    }
    cancel_all(&r);
}
pub(crate) fn window_closed(app: &AppHandle, label: &str) {
    if label == "settings" {
        if let Some(r) = app.try_state::<Runtime>() {
            shortcuts::end_edit(&r.0, None);
        }
    }
}
pub(crate) fn emit(app: &AppHandle, event: Event) {
    if let Ok(r) = runtime(app) {
        if require(&r).is_ok() && !r.quitting.load(Ordering::Acquire) && event.validate().is_ok() {
            // A held/rapid key or overdue timer must not accumulate catch-up work.
            let key =
                matches!(event.name.as_str(), "schedule.due" | "shortcut.pressed").then(|| {
                    format!(
                        "{}:{}:{}",
                        event.name, event.payload["ruleId"], event.payload["ruleRevision"]
                    )
                });
            if let Some(key) = &key {
                if !r
                    .pending
                    .lock()
                    .is_ok_and(|mut pending| pending.insert(key.clone()))
                {
                    return;
                }
            }
            let item = QueuedEvent {
                event,
                generation: r.generation.load(Ordering::Acquire),
                key: key.clone(),
            };
            if r.sender.try_send(item).is_err() {
                if let Some(key) = key {
                    if let Ok(mut pending) = r.pending.lock() {
                        pending.remove(&key);
                    }
                }
                let _ = app.emit_to(
                    "settings",
                    "hooks-warning",
                    json!({"message":"事件队列已满"}),
                );
            }
        }
    }
}
pub(crate) fn lifecycle(app: &AppHandle, name: &str, scene: Option<&str>, payload: Value) {
    let mut e = Event::new(name);
    e.scene_id = scene.map(str::to_owned);
    e.payload = payload;
    // Feature events emitted while an authorized operation is claimed retain
    // its causal chain. Otherwise capture.open -> capture.opened -> capture.open
    // could lose ancestry and run forever. Queued/completed operations and
    // unrelated background Agent replies never lend their origin.
    if scene.is_some() {
        if let Some(origin) = crate::app_control_host::native_event_origin(app) {
            e.ancestors = origin.ancestors;
            e.depth = origin.depth;
        }
    }
    emit(app, e);
}
pub(crate) fn observe_scene(app: &AppHandle, id: &str) {
    if let Ok(r) = runtime(app) {
        let changed = r.last_scene.lock().is_ok_and(|mut previous| {
            let changed = previous.as_deref() != Some(id);
            *previous = Some(id.into());
            changed
        });
        if changed {
            lifecycle(app, "scene.activated", Some(id), Value::Null);
        }
    }
}
pub(crate) fn capture_opened(app: &AppHandle, snapshot: &mewu_core::Snapshot) {
    let id = &snapshot.active_scene_id;
    let payload = snapshot
        .scenes
        .iter()
        .find(|s| &s.id == id)
        .and_then(|s| s.background.as_ref())
        .map(|asset| json!({"assetId":asset.id,"width":asset.width,"height":asset.height}))
        .unwrap_or(Value::Null);
    lifecycle(app, "capture.opened", Some(id), payload);
}

#[derive(Clone)]
struct RuleGuard {
    shared: Arc<Shared>,
    id: String,
    revision: u64,
    generation: u64,
}
impl RuleGuard {
    fn check(&self) -> Result<()> {
        require(&self.shared)?;
        if self.shared.generation.load(Ordering::Acquire) != self.generation
            || !lock(&self.shared.store)?
                .rule(&self.id)?
                .is_some_and(|r| r.enabled && r.revision == self.revision)
        {
            return Err("提示词规则已更改或停用，请重新发送".into());
        }
        Ok(())
    }
}
#[derive(Clone, Default)]
pub(crate) struct PromptContext {
    pub instructions: Vec<String>,
    pub ancestors: Vec<String>,
    depth: u8,
    guards: Vec<RuleGuard>,
    origin_guards: Vec<actions::Fence>,
}
impl PromptContext {
    pub fn check(&self) -> Result<()> {
        for guard in &self.guards {
            guard.check()?;
        }
        for guard in &self.origin_guards {
            guard.check()?;
        }
        Ok(())
    }
}
pub(crate) async fn before_prompt(
    app: &AppHandle,
    scene: &str,
    text: &str,
    parent: Option<PromptContext>,
) -> Result<PromptContext> {
    let mut context = parent.unwrap_or_default();
    if !enabled(app) {
        return Ok(context);
    }
    if runtime(app)?.quitting.load(Ordering::Acquire) {
        return Err("软件正在退出".into());
    }
    let r = runtime(app)?;
    let generation = r.generation.load(Ordering::Acquire);
    let mut e = Event::new("prompt.beforeSend");
    e.scene_id = Some(scene.into());
    e.text = text.into();
    e.ancestors = context.ancestors.clone();
    e.depth = context.depth;
    e.validate()?;
    let rules = lock(&r.store)?.rules()?;
    let until = Instant::now() + Duration::from_secs(60);
    for rule in rules {
        if rule.matches(&e)? {
            let (injected, did_run) = tokio::time::timeout(
                until.saturating_duration_since(Instant::now()),
                execute(r.clone(), rule.clone(), e.clone(), false, generation),
            )
            .await
            .map_err(|_| "提示词触发超时，未自动重试")??;
            if did_run {
                context.ancestors.push(rule.id.clone());
                context.guards.push(RuleGuard {
                    shared: r.clone(),
                    id: rule.id,
                    revision: rule.revision,
                    generation,
                });
            }
            context.instructions.extend(injected);
        }
    }
    if context.instructions.iter().map(String::len).sum::<usize>() > 128 * 1024 {
        return Err("Hooks 注入内容过大".into());
    }
    Ok(context)
}
pub(crate) fn take_prompt_origin(
    app: &AppHandle,
    origin: Option<&str>,
    scene: &str,
    text: &str,
) -> Result<Option<PromptContext>> {
    let Some(origin) = origin else {
        return Ok(None);
    };
    use sha2::{Digest, Sha256};
    let r = runtime(app)?;
    let value = lock(&r.prompt_origins)?
        .remove(origin)
        .ok_or("自动化发送来源已结束或无效")?;
    value.fence.check()?;
    if Instant::now() >= value.until
        || value.scene.as_deref().is_some_and(|id| id != scene)
        || value.text_sha256 != <[u8; 32]>::from(Sha256::digest(text.as_bytes()))
    {
        return Err("自动化发送内容或会话已更改".into());
    }
    Ok(Some(value.context))
}
async fn send_prompt(work: &Execution, text: &str, scene: Option<&str>) -> Result<()> {
    use sha2::{Digest, Sha256};
    let id = uuid::Uuid::new_v4().to_string();
    let mut ancestors = work.event.ancestors.clone();
    ancestors.push(work.rule.id.clone());
    let context = PromptContext {
        instructions: vec![],
        ancestors,
        depth: work.event.depth + 1,
        guards: vec![],
        origin_guards: vec![actions::Fence::new(work)],
    };
    {
        let mut origins = lock(&work.shared.prompt_origins)?;
        origins.retain(|_, v| v.until > Instant::now());
        if origins.len() >= 64 {
            return Err("自动化发送请求过多".into());
        }
        origins.insert(
            id.clone(),
            PromptOrigin {
                fence: actions::Fence::new(work),
                context,
                scene: scene.map(str::to_owned),
                text_sha256: Sha256::digest(text.as_bytes()).into(),
                until: work.deadline,
            },
        );
    }
    let mut params = json!({"text":text,"automationOrigin":id});
    if let Some(scene) = scene {
        params["context"] = json!({"sceneId":scene});
    }
    let result = app_action(work, "chat.send", params).await;
    lock(&work.shared.prompt_origins)?.remove(&id);
    result
}
async fn app_action(work: &Execution, method: &str, params: Value) -> Result<()> {
    work.check()?;
    let fence = actions::Fence::new(work);
    let gate = Arc::new(move || fence.check());
    let mut ancestors = work.event.ancestors.clone();
    ancestors.push(work.rule.id.clone());
    let origin = crate::app_control_host::NativeEventOrigin {
        ancestors,
        depth: work.event.depth + 1,
    };
    crate::app_control_host::execute_native(
        &work.shared.app,
        method,
        params,
        gate,
        work.deadline,
        Some(origin),
    )
    .await?;
    Ok(())
}
pub(crate) fn begin_run(app: &AppHandle, scene: &str, run: &str, context: &PromptContext) {
    if let Ok(r) = runtime(app) {
        if let Ok(mut origins) = r.run_ancestry.lock() {
            if !context.ancestors.is_empty() {
                origins.insert(
                    run.into(),
                    Lineage {
                        ancestors: context.ancestors.clone(),
                        depth: context.depth,
                    },
                );
            }
        }
    }
    let mut e = Event::new("agent.started");
    e.scene_id = Some(scene.into());
    e.run_id = Some(run.into());
    e.ancestors = context.ancestors.clone();
    e.depth = context.depth;
    emit(app, e);
}
pub(crate) fn run_finished(
    app: &AppHandle,
    scene: &str,
    run: &str,
    snapshot: &mewu_core::Snapshot,
) {
    let Some(scene) = snapshot.scenes.iter().find(|s| s.id == scene) else {
        return;
    };
    let Some(current) = scene.run.as_ref().filter(|r| r.id == run) else {
        return;
    };
    let mut e = Event::new(if current.status == mewu_core::RunStatus::Completed {
        "agent.replyCompleted"
    } else {
        "agent.failed"
    });
    e.scene_id = Some(scene.id.clone());
    e.run_id = Some(run.into());
    if e.name == "agent.replyCompleted" {
        e.text = scene
            .messages
            .last()
            .map(|m| m.text.clone())
            .unwrap_or_default();
    }
    if let Ok(r) = runtime(app) {
        if let Ok(mut origins) = r.run_ancestry.lock() {
            if let Some(lineage) = origins.remove(run) {
                e.ancestors = lineage.ancestors;
                e.depth = lineage.depth;
            }
        }
    }
    emit(app, e);
}
pub(crate) fn plugin_event(
    app: &AppHandle,
    plugin: &str,
    revision: u64,
    name: &str,
    scene: Option<&str>,
    run: &str,
    data: Value,
) {
    let mut event = Event::new("plugin.event");
    event.scene_id = scene.map(str::to_owned);
    event.run_id = Some(run.into());
    event.payload = json!({"pluginId":plugin,"pluginRevision":revision,"name":name,"data":data});
    if let Ok(r) = runtime(app) {
        if let Ok(runs) = r.run_ancestry.lock() {
            if let Some(lineage) = runs.get(run) {
                event.ancestors = lineage.ancestors.clone();
                event.depth = lineage.depth;
            }
        }
    }
    emit(app, event);
}
pub(crate) fn forget_run(app: &AppHandle, run: &str) {
    if let Ok(r) = runtime(app) {
        if let Ok(mut runs) = r.run_ancestry.lock() {
            runs.remove(run);
        }
    }
}
pub(crate) fn tool_settled(
    app: &AppHandle,
    scene: &str,
    run: &str,
    tool: &str,
    result: &std::result::Result<crate::journal_runtime::CommittedToolOutput, String>,
) {
    let failed = match result {
        Err(_) => true,
        Ok(output) => serde_json::from_str::<Value>(&output.model_json)
            .ok()
            .is_some_and(|value| value.get("isError").and_then(Value::as_bool) == Some(true)),
    };
    let mut event = Event::new(if failed {
        "tool.failed"
    } else {
        "tool.completed"
    });
    event.scene_id = Some(scene.into());
    event.run_id = Some(run.into());
    event.payload = json!({"tool":tool});
    if let Ok(r) = runtime(app) {
        if let Ok(runs) = r.run_ancestry.lock() {
            if let Some(lineage) = runs.get(run) {
                event.ancestors = lineage.ancestors.clone();
                event.depth = lineage.depth;
            }
        }
    }
    emit(app, event);
}

pub(super) struct Execution {
    pub shared: Arc<Shared>,
    pub rule: Rule,
    pub event: Event,
    pub cancel: Arc<AtomicBool>,
    generation: u64,
    pub deadline: Instant,
}
impl Execution {
    pub fn check(&self) -> Result<()> {
        require(&self.shared)?;
        if self.cancel.load(Ordering::Acquire)
            || self.shared.generation.load(Ordering::Acquire) != self.generation
            || (self.shared.quitting.load(Ordering::Acquire) && self.event.name != "app.beforeQuit")
        {
            return Err("规则已停用、更改或停止".into());
        }
        if Instant::now() >= self.deadline {
            return Err("规则执行超时，未自动重试".into());
        }
        let current = lock(&self.shared.store)?.rule(&self.rule.id)?;
        if !current.is_some_and(|r| r.enabled && r.revision == self.rule.revision) {
            return Err("规则已更改或删除".into());
        }
        Ok(())
    }
    pub fn child_event(&self, name: &str, payload: Value) -> Event {
        let mut e = Event::new(name);
        e.scene_id = self.event.scene_id.clone();
        e.run_id = self.event.run_id.clone();
        e.ancestors = self.event.ancestors.clone();
        e.ancestors.push(self.rule.id.clone());
        e.depth = self.event.depth + 1;
        e.payload = payload;
        e
    }
}
async fn execute(
    r: Arc<Shared>,
    rule: Rule,
    event: Event,
    manual: bool,
    generation: u64,
) -> Result<(Vec<String>, bool)> {
    require(&r)?;
    check_admission(&r, &event, generation)?;
    let pool = if event.name == "prompt.beforeSend" && !manual {
        r.prompt_slot.clone()
    } else {
        r.slots.clone()
    };
    let slot = tokio::time::timeout(Duration::from_millis(rule.timeout_ms), pool.acquire_owned())
        .await
        .map_err(|_| "事件触发器等待超时")?
        .map_err(|_| "事件触发器已停止")?;
    check_admission(&r, &event, generation)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let run =
        { lock(&r.store)?.claim(&rule, &event, chrono::Utc::now().timestamp_millis(), manual)? };
    let Some(run) = run else {
        return Ok((vec![], false));
    };
    lock(&r.jobs)?.insert(
        run.id.clone(),
        Job {
            rule_id: rule.id.clone(),
            cancel: cancel.clone(),
        },
    );
    let _job = JobGuard {
        shared: r.clone(),
        id: run.id.clone(),
        cancel: cancel.clone(),
    };
    changed(&r);
    let work = Execution {
        shared: r.clone(),
        deadline: Instant::now() + Duration::from_millis(rule.timeout_ms),
        rule,
        event,
        cancel: cancel.clone(),
        generation,
    };
    let outcome = async {
        work.check()?;
        if !work.rule.condition.trim().is_empty() {
            let script = work.rule.condition.clone();
            let event = work.event.clone();
            let c = cancel.clone();
            let value = tauri::async_runtime::spawn_blocking(move || {
                mewu_hooks::evaluate(&script, &event, c)
            })
            .await
            .map_err(|_| "条件执行中断")??;
            if value == Value::Bool(false) {
                return Ok((vec![], "skipped"));
            }
            if value != Value::Bool(true) {
                return Err("附加条件必须返回 true 或 false".into());
            }
        }
        let mut prompts = vec![];
        let mut failure = None;
        let mut completed = 0;
        for action in &work.rule.actions {
            work.check()?;
            match actions::execute(&work, action).await {
                Ok(extra) => {
                    prompts.extend(extra);
                    completed += 1;
                    lock(&r.store)?.progress(&run.id, completed)?;
                    changed(&r);
                }
                Err(e) => {
                    failure = Some(e);
                    if work.rule.stop_on_error {
                        break;
                    }
                }
            }
        }
        if let Some(e) = failure {
            return Err(e);
        }
        Ok((prompts, "completed"))
    }
    .await;
    let result = match outcome {
        Ok((prompts, status)) => {
            lock(&r.store)?.finish(&run.id, status, None)?;
            Ok((prompts, status != "skipped"))
        }
        Err(error) => {
            let status = if cancel.load(Ordering::Acquire)
                || r.generation.load(Ordering::Acquire) != generation
            {
                "canceled"
            } else {
                "failed"
            };
            lock(&r.store)?.finish(&run.id, status, Some(&error))?;
            if work.rule.stop_on_error {
                Err(error)
            } else {
                Ok((vec![], true))
            }
        }
    };
    drop(slot);
    changed(&r);
    result
}
fn check_admission(r: &Shared, event: &Event, generation: u64) -> Result<()> {
    require(r)?;
    admit_generation(
        generation,
        r.generation.load(Ordering::Acquire),
        r.quitting.load(Ordering::Acquire),
        &event.name,
    )
}
fn admit_generation(expected: u64, current: u64, quitting: bool, event: &str) -> Result<()> {
    if expected != current || (quitting && event != "app.beforeQuit") {
        Err("事件授权已撤销或软件正在退出".into())
    } else {
        Ok(())
    }
}
async fn event_worker(r: Arc<Shared>, mut receiver: mpsc::Receiver<QueuedEvent>) {
    loop {
        if r.stop.load(Ordering::Acquire) {
            break;
        }
        let item = tokio::select! {e=receiver.recv()=>match e{Some(e)=>e,None=>break},_=r.stopped.notified()=>break};
        if require(&r).is_ok() && item.generation == r.generation.load(Ordering::Acquire) {
            let rules = lock(&r.store).and_then(|s| s.rules()).unwrap_or_default();
            for rule in rules {
                if check_admission(&r, &item.event, item.generation).is_err() {
                    break;
                }
                if rule.matches(&item.event).unwrap_or(false) {
                    let _ =
                        execute(r.clone(), rule, item.event.clone(), false, item.generation).await;
                }
            }
        }
        if let Some(key) = item.key {
            if let Ok(mut pending) = r.pending.lock() {
                pending.remove(&key);
            }
        }
    }
}
async fn scheduler(r: Arc<Shared>) {
    let mut next: HashMap<String, (u64, chrono::DateTime<chrono::Utc>)> = HashMap::new();
    loop {
        if r.stop.load(Ordering::Acquire) {
            break;
        }
        let now = chrono::Utc::now();
        let mut delay = Duration::from_secs(60);
        if r.enabled.load(Ordering::Acquire) {
            let rules = lock(&r.store).and_then(|s| s.rules()).unwrap_or_default();
            next.retain(|id, _| rules.iter().any(|v| &v.id == id && v.enabled));
            for rule in rules.into_iter().filter(|r| r.enabled) {
                if let Trigger::Cron {
                    expression,
                    timezone,
                } = &rule.trigger
                {
                    if next
                        .get(&rule.id)
                        .is_none_or(|(revision, _)| *revision != rule.revision)
                    {
                        if let Ok(at) = mewu_hooks::next_occurrence(expression, timezone, now) {
                            next.insert(rule.id.clone(), (rule.revision, at));
                        }
                    }
                    if let Some((_, at)) = next.get(&rule.id).cloned() {
                        if at <= now {
                            let mut event = Event::new("schedule.due");
                            event.payload = json!({"ruleId":rule.id,"ruleRevision":rule.revision});
                            emit(&r.app, event);
                            if let Ok(at) = mewu_hooks::next_occurrence(expression, timezone, now) {
                                next.insert(rule.id.clone(), (rule.revision, at));
                            } else {
                                next.remove(&rule.id);
                            }
                        }
                    }
                }
            }
            for (_, at) in next.values() {
                if let Ok(wait) = (*at - now).to_std() {
                    delay = delay.min(wait);
                }
            }
        } else {
            next.clear();
        }
        tokio::select! {_=tokio::time::sleep(delay.max(Duration::from_millis(100)))=>{},_=r.wake.notified()=>{}}
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    #[serde(flatten)]
    snapshot: mewu_hooks::Snapshot,
    enabled: bool,
    shortcuts: HashMap<String, shortcuts::Status>,
    next_runs: HashMap<String, i64>,
    events: Vec<Value>,
    mcp_servers: Vec<Value>,
    platform: &'static str,
    warning: Option<String>,
}
fn view(r: &Shared) -> Result<View> {
    let snapshot = lock(&r.store)?.snapshot()?;
    let next_runs = snapshot
        .rules
        .iter()
        .filter_map(|rule| {
            if let Trigger::Cron {
                expression,
                timezone,
            } = &rule.trigger
            {
                if rule.enabled {
                    return mewu_hooks::next_occurrence(expression, timezone, chrono::Utc::now())
                        .ok()
                        .map(|d| (rule.id.clone(), d.timestamp_millis()));
                }
            }
            None
        })
        .collect();
    let host = r.app.state::<crate::Host>();
    let servers=host.lock()?.store.mcp_servers().iter().filter(|s|s.enabled).map(|s|json!({"id":s.id,"name":s.label,"tools":s.tools.iter().map(|t|json!({"name":t.name})).collect::<Vec<_>>()})).collect();
    Ok(View {
        snapshot,
        enabled: r.enabled.load(Ordering::Acquire),
        shortcuts: shortcuts::statuses(r),
        next_runs,
        events: EVENTS
            .iter()
            .map(|(name, title)| json!({"name":name,"title":title}))
            .collect(),
        mcp_servers: servers,
        platform: std::env::consts::OS,
        warning: r.startup_warning.clone(),
    })
}
const EVENTS: &[(&str, &str)] = &[
    ("app.started", "软件启动"),
    ("app.beforeQuit", "退出前"),
    ("capture.opened", "截图就绪"),
    ("scene.created", "创建会话"),
    ("scene.activated", "切换会话"),
    ("scene.closed", "关闭会话"),
    ("agent.started", "Agent 开始"),
    ("agent.replyCompleted", "AI 回复完成"),
    ("agent.failed", "Agent 失败"),
    ("tool.completed", "工具执行完成"),
    ("tool.failed", "工具执行失败"),
    ("plugin.changed", "插件变更"),
    ("plugin.event", "插件事件"),
];
#[tauri::command]
pub(crate) async fn get_hooks(app: AppHandle, window: WebviewWindow) -> Result<View> {
    let r = settings(&app, &window)?;
    tauri::async_runtime::spawn_blocking(move || view(&r))
        .await
        .map_err(|_| "读取规则中断")?
}
#[tauri::command]
pub(crate) async fn save_hook(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: u64,
    mut rule: Rule,
) -> Result<View> {
    let r = settings(&app, &window)?;
    tauri::async_runtime::spawn_blocking(move || {
        require(&r)?;
        shortcuts::canonicalize(&mut rule)?;
        let snapshot = shortcuts::save(&r, expected_revision, rule)?;
        drop(snapshot);
        r.wake.notify_one();
        changed(&r);
        view(&r)
    })
    .await
    .map_err(|_| "保存规则中断")?
}
#[tauri::command]
pub(crate) async fn remove_hook(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: u64,
    id: String,
    rule_revision: u64,
) -> Result<View> {
    let r = settings(&app, &window)?;
    tauri::async_runtime::spawn_blocking(move || {
        require(&r)?;
        shortcuts::remove(&r, expected_revision, &id, rule_revision)?;
        r.wake.notify_one();
        changed(&r);
        view(&r)
    })
    .await
    .map_err(|_| "删除规则中断")?
}
#[tauri::command]
pub(crate) async fn check_hook(
    app: AppHandle,
    window: WebviewWindow,
    mut rule: Rule,
) -> Result<()> {
    let _ = settings(&app, &window)?;
    tauri::async_runtime::spawn_blocking(move || shortcuts::canonicalize(&mut rule))
        .await
        .map_err(|_| "校验规则中断")?
}
#[tauri::command]
pub(crate) async fn run_hook(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    rule_revision: u64,
    text: String,
) -> Result<String> {
    let r = settings(&app, &window)?;
    let rule = lock(&r.store)?
        .rule(&id)?
        .filter(|r| r.enabled && r.revision == rule_revision)
        .ok_or("规则已更改或停用")?;
    let mut event = Event::new(match &rule.trigger {
        Trigger::Prompt { .. } => "prompt.beforeSend",
        Trigger::Reply { .. } => "agent.replyCompleted",
        Trigger::Event { name } => name,
        Trigger::Cron { .. } => "schedule.due",
        Trigger::Shortcut { .. } => "shortcut.pressed",
    });
    event.text = text;
    event.payload = json!({"ruleId":rule.id,"ruleRevision":rule.revision});
    event.validate()?;
    if matches!(rule.trigger, Trigger::Prompt { .. } | Trigger::Reply { .. })
        && !rule.matches(&event)?
    {
        return Err("测试内容未匹配规则".into());
    }
    let id = event.id.clone();
    let generation = r.generation.load(Ordering::Acquire);
    tauri::async_runtime::spawn(async move {
        let _ = execute(r, rule, event, true, generation).await;
    });
    Ok(id)
}
#[tauri::command]
pub(crate) fn cancel_hook(app: AppHandle, window: WebviewWindow, run_id: String) -> Result<()> {
    let r = settings(&app, &window)?;
    if let Some(job) = lock(&r.jobs)?.get(&run_id) {
        job.cancel.store(true, Ordering::Release);
    }
    Ok(())
}
#[tauri::command]
pub(crate) fn edit_hook_shortcut(
    app: AppHandle,
    window: WebviewWindow,
    lease_id: String,
    active: bool,
) -> Result<()> {
    let r = settings(&app, &window)?;
    shortcuts::edit(&r, lease_id, active)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_queued_event_cannot_inherit_authority_after_disable_and_reenable() {
        assert!(admit_generation(4, 4, false, "app.started").is_ok());
        assert!(admit_generation(4, 6, false, "app.started").is_err());
        assert!(admit_generation(4, 4, true, "shortcut.pressed").is_err());
        assert!(admit_generation(4, 4, true, "app.beforeQuit").is_ok());
        assert!(admit_generation(4, 6, true, "app.beforeQuit").is_err());
    }
}
pub(crate) fn route_shortcut(
    app: &AppHandle,
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
    pressed: bool,
) -> bool {
    runtime(app).is_ok_and(|r| shortcuts::route(&r, shortcut, pressed))
}
