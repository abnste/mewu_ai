// SPDX-License-Identifier: MPL-2.0
//! One-shot, scene-bound action tickets. The renderer prepares existing UI state;
//! native code owns the effect, deadline and originating run's cancellation.
use super::*;

pub(super) const HOST_ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PENDING_ACTIONS: usize = 64;
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActionRequest {
    action: String,
    region_id: String,
    scene_id: String,
    variant: Option<String>,
}
impl ActionRequest {
    pub(super) fn parse(params: &Value, scene: Option<&str>) -> Result<Self, String> {
        let scene_id = scene.ok_or("宿主动作没有会话授权")?;
        let fields = params.as_object().ok_or("宿主动作请求无效")?;
        if fields.keys().any(|key| !matches!(key.as_str(), "action" | "regionId" | "variant")) {
            return Err("宿主动作请求无效".into());
        }
        let action = string(params, "action")?;
        let region = string(params, "regionId")?;
        if !matches!(action, "copy" | "save" | "pin" | "reference")
            || region.is_empty() || region.len() > 256 || region.chars().any(char::is_control) {
            return Err("宿主动作请求无效".into());
        }
        let variant = match params.get("variant") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if matches!(value.as_str(), "original" | "annotated") => Some(value.clone()),
            _ => return Err("图片导出版本无效".into()),
        };
        Ok(Self { action: action.into(), region_id: region.into(), scene_id: scene_id.into(), variant })
    }
    pub(super) fn event(&self, id: &str) -> Value {
        json!({"id":id,"sceneId":self.scene_id,"regionId":self.region_id,"action":self.action,"variant":self.variant})
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase { Pending, Canceled, Published }
#[derive(Clone)]
pub(crate) struct ActionLease(Arc<LeaseInner>);
struct LeaseInner {
    phase: Mutex<Phase>,
    parent: Arc<AtomicBool>,
    created: Instant,
}
impl ActionLease {
    pub(crate) fn new(parent: Arc<AtomicBool>, created: Instant) -> Self {
        Self(Arc::new(LeaseInner { phase: Mutex::new(Phase::Pending), parent, created }))
    }
    fn check_phase(&self, phase: Phase) -> Result<(), String> {
        // Accepted pins are independent objects; ending their creator must not
        // invalidate their subsequent drag/zoom/export operations.
        if phase == Phase::Published { return Ok(()); }
        if phase == Phase::Canceled || self.0.parent.load(Ordering::Acquire) { return Err("已取消".into()); }
        if self.age() >= HOST_ACTION_TIMEOUT { return Err("动作执行超时".into()); }
        Ok(())
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        self.check_phase(*self.0.phase.lock().map_err(|_| "宿主动作状态不可用")?)
    }
    pub(crate) fn accept(&self) -> Result<(), String> {
        let mut phase = self.0.phase.lock().map_err(|_| "宿主动作状态不可用")?;
        self.check_phase(*phase)?;
        *phase = Phase::Published;
        Ok(())
    }
    pub(super) fn age(&self) -> Duration { self.0.created.elapsed() }
    pub(crate) fn cancel(&self) {
        if let Ok(mut phase) = self.0.phase.lock() {
            if *phase == Phase::Pending { *phase = Phase::Canceled; }
        }
    }
}

struct PendingAction {
    sender: mpsc::Sender<Result<(), String>>,
    request: ActionRequest,
    lease: ActionLease,
    claimed: bool,
}
#[derive(Default)]
pub(crate) struct PendingActions { pending: Mutex<HashMap<String, PendingAction>> }
impl PendingActions {
    pub(super) fn begin(&self, id: &str, sender: mpsc::Sender<Result<(), String>>, request: ActionRequest,
        parent: Arc<AtomicBool>, now: Instant) -> Result<ActionLease, String> {
        let mut pending = self.pending.lock().map_err(|_| "宿主动作状态不可用")?;
        if pending.len() >= MAX_PENDING_ACTIONS || pending.contains_key(id) { return Err("宿主动作请求过多，请稍后重试".into()); }
        let lease = ActionLease::new(parent, now);
        lease.check()?;
        pending.insert(id.into(), PendingAction { sender, request, lease: lease.clone(), claimed: false });
        Ok(lease)
    }
    fn claim(&self, id: &str) -> Result<(ActionRequest, ActionLease), String> {
        let mut pending = self.pending.lock().map_err(|_| "宿主动作状态不可用")?;
        let action = pending.get_mut(id).ok_or("宿主动作已结束")?;
        action.lease.check()?;
        if action.claimed { return Err("宿主动作已执行".into()); }
        action.claimed = true;
        Ok((action.request.clone(), action.lease.clone()))
    }
    fn resolve(&self, id: &str, ok: bool, reason: Option<&str>) {
        let Ok(mut pending) = self.pending.lock() else { return; };
        if let Some(action) = pending.remove(id) {
            let result = if ok { action.lease.check() } else { Err(reason.filter(|s| !s.is_empty()).unwrap_or("宿主动作执行失败").into()) };
            action.lease.cancel();
            let _ = action.sender.send(result);
        }
    }
    fn retire(&self, id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(action) = pending.remove(id) { action.lease.cancel(); }
        }
    }
}
pub(super) struct PendingGuard<'a> { pub registry: &'a PendingActions, pub id: &'a str }
impl Drop for PendingGuard<'_> { fn drop(&mut self) { self.registry.retire(self.id); } }

#[tauri::command]
pub fn resolve_extension_action(app: AppHandle, window: tauri::WebviewWindow, id: String, ok: bool, reason: Option<String>) -> Result<(), String> {
    if window.label() != "space" { return Err("此窗口不能回复宿主动作".into()); }
    app.state::<ExtensionHost>().actions.resolve(&id, ok, reason.as_deref());
    Ok(())
}

#[tauri::command]
pub async fn execute_extension_action(app: AppHandle, window: tauri::WebviewWindow, id: String) -> Result<bool, String> {
    if window.label() != "space" { return Err("此窗口不能执行宿主动作".into()); }
    let (request, lease) = app.state::<ExtensionHost>().actions.claim(&id)?;
    let (target, translation) = {
        let host = app.state::<Host>();
        host.exit.ensure_running()?;
        let engine = host.lock()?;
        let scene = engine.store.active_scene().map_err(|e| e.to_string())?;
        if scene.id != request.scene_id || scene.closed || scene.frozen { return Err("选区不存在或已关闭".into()); }
        let region = scene.regions.iter().find(|r| r.id == request.region_id).ok_or("选区不存在或已关闭")?;
        let background = scene.background.as_ref().ok_or("尚未截图")?;
        (mewu_core::OcrTarget { scene_id: scene.id.clone(), region_id: region.id.clone(), background_id: background.id.clone(),
            drawing_revision: region.drawing_revision, x: region.x, y: region.y, width: region.width, height: region.height },
            region.translation.as_ref().map(|t| t.overlay.id.clone()))
    };
    match request.action.as_str() {
        "reference" => { lease.accept()?; Ok(true) }
        "copy" | "save" => {
            let variant = if request.variant.as_deref() == Some("original") { crate::image_host::ImageExportVariant::Original } else { crate::image_host::ImageExportVariant::Annotated };
            crate::image_host::export_extension_region(app, window, request.scene_id, request.region_id, request.action == "copy", variant, lease).await
        }
        "pin" => {
            let entry = {
                let host = app.state::<Host>();
                let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?.store.snapshot()?;
                plugins.plugins.iter().filter(|p| p.state == crate::plugins::PluginState::Enabled && p.error.is_none())
                    .find_map(|p| p.manifest.contributions.iter().find_map(|c| match c {
                        crate::plugins::PluginContribution::Pin { id, .. } => Some((p.manifest.id.clone(), p.revision, id.clone())), _ => None,
                    })).ok_or("没有可用的贴图功能")?
            };
            crate::pin_host::run_extension_pin(app, window, id, entry.0, entry.1, entry.2, target, translation, Some(lease)).await.map(|_| true)
        }
        _ => Err("宿主动作无效".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> ActionRequest { ActionRequest::parse(&json!({"action":"save","regionId":"r"}), Some("scene-A")).unwrap() }
    #[test]
    fn tickets_are_scene_bound_single_use_and_late_replies_do_not_replay() {
        let registry = PendingActions::default();
        let (tx, rx) = mpsc::channel();
        registry.begin("one", tx, request(), Arc::new(AtomicBool::new(false)), Instant::now()).unwrap();
        let (request, lease) = registry.claim("one").unwrap();
        assert_eq!(request.scene_id, "scene-A");
        assert!(registry.claim("one").is_err());
        lease.accept().unwrap();
        registry.resolve("one", true, None);
        assert_eq!(rx.recv().unwrap(), Ok(()));
        registry.resolve("one", true, None);
        assert!(registry.claim("one").is_err());
    }
    #[test]
    fn cancel_and_deadline_prevent_late_publication_after_dialog_or_flush() {
        for expired in [false, true] {
            let parent = Arc::new(AtomicBool::new(false));
            let created = if expired { Instant::now() - HOST_ACTION_TIMEOUT } else { Instant::now() };
            let lease = ActionLease::new(parent.clone(), created);
            if !expired { parent.store(true, Ordering::Release); }
            let mut published = false;
            if lease.accept().is_ok() { published = true; }
            assert!(!published);
            assert!(lease.check().is_err());
        }
    }
    #[test]
    fn retiring_requests_releases_capacity_and_revokes_already_claimed_effects() {
        let registry = PendingActions::default();
        for n in 0..MAX_PENDING_ACTIONS {
            let (tx, _) = mpsc::channel();
            registry.begin(&n.to_string(), tx, request(), Arc::new(AtomicBool::new(false)), Instant::now()).unwrap();
        }
        let (tx, _) = mpsc::channel();
        assert!(registry.begin("overflow", tx, request(), Arc::new(AtomicBool::new(false)), Instant::now()).is_err());
        let (_, lease) = registry.claim("0").unwrap();
        { let _guard = PendingGuard { registry: &registry, id: "0" }; }
        assert!(lease.accept().is_err());
        assert!(registry.claim("0").is_err());
        let (tx, _) = mpsc::channel();
        registry.begin("new", tx, request(), Arc::new(AtomicBool::new(false)), Instant::now()).unwrap();
    }
    #[test]
    fn accepted_pins_survive_creator_exit_but_unaccepted_work_does_not() {
        let parent = Arc::new(AtomicBool::new(false));
        let accepted = ActionLease::new(parent.clone(), Instant::now());
        let pending = ActionLease::new(parent.clone(), Instant::now());
        accepted.accept().unwrap();
        parent.store(true, Ordering::Release);
        accepted.cancel(); pending.cancel();
        assert!(accepted.check().is_ok());
        assert!(pending.accept().is_err());
    }
    #[test]
    fn invalid_or_unscoped_requests_cannot_acquire_authority() {
        assert!(ActionRequest::parse(&json!({"action":"copy","regionId":"r"}), None).is_err());
        for params in [json!({"action":"copy","regionId":"r","sceneId":"other"}), json!({"action":"copy","regionId":""}), json!({"action":"shell","regionId":"r"}), json!({"action":"save","regionId":"r","variant":"other"})] {
            assert!(ActionRequest::parse(&params, Some("a")).is_err());
        }
    }
}
