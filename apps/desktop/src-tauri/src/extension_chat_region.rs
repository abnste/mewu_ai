// SPDX-License-Identifier: MPL-2.0
//! One explicit composer reference grants one immutable capture to one chat run.
//! The session, Escape owner and focus ticket survive individual plugin tools.
use crate::{extension_region, Engine, Host};
use mewu_core::{Asset, AssetKind, ReferenceKind, Region, RegionGeometry, Scene};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Manager};

#[derive(Clone)]
pub(crate) struct PreparedChatRegion {
    scene_id: String,
    agent_id: String,
    background: Asset,
    selection: Region,
}
impl PreparedChatRegion {
    pub(crate) fn capture(
        scene: &Scene,
        active: &str,
        requested: Option<&str>,
    ) -> Result<Option<Self>, String> {
        // Native region input is currently implemented on Windows. Images
        // still participate in ordinary Q&A on every other platform.
        Ok(Self::capture_source(scene, active, requested)?.filter(|_| cfg!(windows)))
    }
    fn capture_source(
        scene: &Scene,
        active: &str,
        requested: Option<&str>,
    ) -> Result<Option<Self>, String> {
        let Some(id) = requested else { return Ok(None) };
        if scene.id != active
            || scene.closed
            || scene.frozen
            || !scene
                .refs
                .iter()
                .any(|r| r.kind == ReferenceKind::Region && r.id == id)
        {
            return Err("请引用当前截图中的操作选区".into());
        }
        let selection = scene
            .regions
            .iter()
            .find(|r| r.id == id)
            .ok_or("选区不存在")?;
        let Some(background) = scene.background.as_ref() else {
            return Ok(None);
        };
        let (Some(width), Some(height)) = (background.width, background.height) else {
            return Ok(None);
        };
        // A derived image or imported media is useful context, not OS input
        // authority. Never synthesize a screen transform for it.
        if scene.blackboard_link.is_some()
            || background.kind != AssetKind::Image
            || selection.image_override.is_some()
            || !background
                .capture_coordinates
                .is_some_and(|c| c.valid_for(width, height))
        {
            return Ok(None);
        }
        if ![selection.x, selection.y, selection.width, selection.height]
            .iter()
            .all(|v| v.is_finite())
            || selection.x < 0.
            || selection.y < 0.
            || selection.width < 1.
            || selection.height < 1.
            || selection.x + selection.width > f64::from(width)
            || selection.y + selection.height > f64::from(height)
        {
            return Err("操作选区坐标无效，请重新框选".into());
        }
        Ok(Some(Self {
            scene_id: scene.id.clone(),
            agent_id: scene.agent_id.clone(),
            background: background.clone(),
            selection: selection.clone(),
        }))
    }
    pub(crate) fn check_source(&self, scene: &Scene, active: &str) -> Result<(), String> {
        if scene.id != self.scene_id
            || active != self.scene_id
            || scene.agent_id != self.agent_id
            || scene.closed
            || scene.frozen
            || scene.blackboard_link.is_some()
            || scene.background.as_ref() != Some(&self.background)
            || !scene
                .refs
                .iter()
                .any(|r| r.kind == ReferenceKind::Region && r.id == self.selection.id)
            || !scene.regions.iter().any(|r| {
                r.id == self.selection.id
                    && RegionGeometry::from(r) == RegionGeometry::from(&self.selection)
                    && r.drawing_revision == self.selection.drawing_revision
                    && r.image_override.is_none()
            })
        {
            return Err("本次会话的操作选区或截图来源已变化".into());
        }
        Ok(())
    }
    pub(crate) fn bind(self, run_id: String, canceled: Arc<AtomicBool>) -> Arc<ChatRegionScope> {
        Arc::new(ChatRegionScope {
            source: self,
            run_id,
            canceled,
            region: Mutex::new(None),
            #[cfg(windows)]
            escape: crate::extension_escape::RunEscapeLease::default(),
            #[cfg(windows)]
            focus: Mutex::new(None),
        })
    }
}

pub(crate) struct ChatRegionScope {
    source: PreparedChatRegion,
    run_id: String,
    canceled: Arc<AtomicBool>,
    region: Mutex<Option<Arc<extension_region::RegionSession>>>,
    #[cfg(windows)]
    escape: crate::extension_escape::RunEscapeLease,
    #[cfg(windows)]
    focus: Mutex<Option<Arc<crate::extension_focus::FocusTicket>>>,
}
impl ChatRegionScope {
    pub(crate) fn cancel_token(&self) -> Arc<AtomicBool> {
        self.canceled.clone()
    }
    pub(crate) fn token(&self) -> Arc<AtomicBool> {
        self.cancel_token()
    }
    pub(crate) fn region_id(&self) -> &str {
        &self.source.selection.id
    }
    pub(crate) fn scene_id(&self) -> &str {
        &self.source.scene_id
    }
    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }
    pub(crate) fn check_engine(&self, engine: &Engine) -> Result<(), String> {
        if self.canceled.load(Ordering::SeqCst)
            || !crate::run_is_authorized(engine, self.scene_id(), self.run_id(), None)
        {
            return Err("本次区域运行已停止".into());
        }
        let scene = engine
            .store
            .scene(self.scene_id())
            .map_err(|e| e.to_string())?;
        let active = engine.store.active_scene().map_err(|e| e.to_string())?;
        self.source.check_source(scene, &active.id)
    }
    pub(crate) fn check(&self, app: &AppHandle) -> Result<(), String> {
        let host = app.state::<Host>();
        host.exit.ensure_background()?;
        let engine = host.lock()?;
        self.check_engine(&engine)
    }
    pub(crate) fn source(&self) -> (Asset, Region) {
        (
            self.source.background.clone(),
            self.source.selection.clone(),
        )
    }
    // The initializer runs on a blocking worker with no Engine/Store guard.
    // Its hooks hold only Weak<Self>, preventing a session/owner Arc cycle.
    pub(crate) fn session(
        &self,
        begin: impl FnOnce() -> Result<Arc<extension_region::RegionSession>, String>,
    ) -> Result<Arc<extension_region::RegionSession>, String> {
        let mut session = self.region.lock().map_err(|_| "区域运行状态不可用")?;
        if session.is_none() {
            *session = Some(begin()?);
        }
        Ok(session
            .as_ref()
            .expect("initialized region session")
            .clone())
    }
    #[cfg(windows)]
    pub(crate) fn escape(&self) -> Result<Arc<crate::extension_escape::EscapeLease>, &'static str> {
        self.escape.acquire(self.canceled.clone())
    }
    #[cfg(windows)]
    pub(crate) fn focus(
        &self,
        app: &AppHandle,
    ) -> Result<Option<Arc<crate::extension_focus::FocusTicket>>, String> {
        let mut ticket = self.focus.lock().map_err(|_| "区域焦点状态不可用")?;
        if ticket.is_none() {
            *ticket =
                crate::extension_focus::FocusTicket::new(app, self.run_id.clone(), self.scene_id());
        }
        Ok(ticket.clone())
    }
}
impl Drop for ChatRegionScope {
    fn drop(&mut self) {
        if let Ok(session) = self.region.get_mut() {
            if let Some(session) = session.take() {
                session.revoke();
                drop(session);
            }
        }
        #[cfg(windows)]
        self.escape.release();
        #[cfg(windows)]
        if let Ok(ticket) = self.focus.get_mut() {
            if let Some(ticket) = ticket.take() {
                ticket.finalize();
            }
        }
    }
}
/// Poll only the task's cancellation/source fences, never physical keys. The
/// short-lived native Escape lease sets the same signal while a model waits.
pub(crate) async fn canceled(app: &AppHandle, scope: Option<&Arc<ChatRegionScope>>) {
    let Some(scope) = scope else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        if scope.check(app).is_err() {
            scope.canceled.store(true, Ordering::SeqCst);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene() -> Scene {
        let store = mewu_core::Store::open_in_memory().unwrap();
        let mut scene = store.snapshot().scenes.remove(0);
        scene.background = Some(Asset {
            id: "capture".into(),
            name: "capture.png".into(),
            kind: AssetKind::Image,
            path: "host-owned/capture.png".into(),
            width: Some(800),
            height: Some(600),
            origin_x: Some(-800),
            origin_y: Some(0),
            scale_factor: Some(1.),
            capture_coordinates: Some(mewu_core::CaptureCoordinates {
                space: mewu_core::CaptureCoordinateSpace::PhysicalPixels,
                origin_x: -800.,
                origin_y: 0.,
                width: 800.,
                height: 600.,
            }),
        });
        scene.regions = vec![Region {
            id: "roi".into(),
            x: 20.,
            y: 30.,
            width: 400.,
            height: 300.,
            ..Default::default()
        }];
        scene.refs = vec![mewu_core::Reference {
            kind: ReferenceKind::Region,
            id: "roi".into(),
        }];
        scene
    }
    #[test]
    fn region_is_never_inferred_from_a_reference_or_first_selection() {
        let scene = scene();
        assert!(PreparedChatRegion::capture_source(&scene, &scene.id, None)
            .unwrap()
            .is_none());
        assert!(
            PreparedChatRegion::capture_source(&scene, &scene.id, Some("roi"))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            PreparedChatRegion::capture(&scene, &scene.id, Some("roi"))
                .unwrap()
                .is_some(),
            cfg!(windows)
        );
        assert!(PreparedChatRegion::capture_source(&scene, &scene.id, Some("missing")).is_err());
        let mut missing = scene.clone();
        missing.refs.clear();
        assert!(PreparedChatRegion::capture_source(&missing, &missing.id, Some("roi")).is_err());
    }
    #[test]
    fn imported_derived_and_outside_regions_cannot_mint_a_capture_grant() {
        let original = scene();
        for mutation in 0..5 {
            let mut scene = original.clone();
            match mutation {
                0 => scene.background.as_mut().unwrap().capture_coordinates = None,
                1 => scene.regions[0].image_override = scene.background.clone(),
                2 => scene.regions[0].x = f64::NAN,
                3 => scene.regions[0].width = 801.,
                _ => {
                    scene
                        .background
                        .as_mut()
                        .unwrap()
                        .capture_coordinates
                        .as_mut()
                        .unwrap()
                        .width = 799.
                }
            }
            let grant = PreparedChatRegion::capture_source(&scene, &scene.id, Some("roi"));
            if matches!(mutation, 0 | 1 | 4) {
                assert!(grant.unwrap().is_none());
            } else {
                assert!(grant.is_err());
            }
        }
    }
    #[test]
    fn an_existing_grant_rejects_retarget_close_freeze_source_and_revision_changes() {
        let original = scene();
        let prepared = PreparedChatRegion::capture_source(&original, &original.id, Some("roi"))
            .unwrap()
            .unwrap();
        assert!(prepared.check_source(&original, &original.id).is_ok());
        assert!(prepared.check_source(&original, "another-scene").is_err());
        for mutation in 0..8 {
            let mut scene = original.clone();
            match mutation {
                0 => scene.closed = true,
                1 => scene.frozen = true,
                2 => scene.agent_id = "another-agent".into(),
                3 => scene.background.as_mut().unwrap().id = "new-capture".into(),
                4 => scene.background.as_mut().unwrap().path = "replacement.png".into(),
                5 => scene.regions[0].x += 1.,
                6 => scene.regions[0].drawing_revision += 1,
                _ => scene.refs.clear(),
            }
            assert!(prepared.check_source(&scene, &scene.id).is_err());
        }
    }
    #[test]
    fn actual_core_run_requires_current_admission_sender_and_uncanceled_scope() {
        use std::collections::HashMap;
        let mut engine = Engine {
            store: mewu_core::Store::open_in_memory().unwrap(),
            optional: crate::optional_capabilities::Authorizations::test_defaults(),
            agent_runs: HashMap::new(),
            runs: HashMap::new(),
            plugin_runs: HashMap::new(),
            visual_runs: HashMap::new(),
            ocr_jobs: crate::ocr_host::OcrRegistry::default(),
            translation_jobs: crate::translation_host::TranslationRegistry::default(),
        };
        let id = engine.store.snapshot().active_scene_id;
        let template = scene();
        engine
            .store
            .set_background(&id, template.background.unwrap())
            .unwrap();
        engine
            .store
            .apply(mewu_core::SceneCommand::AddRegion {
                scene_id: id.clone(),
                region: template.regions[0].clone(),
            })
            .unwrap();
        engine
            .store
            .apply(mewu_core::SceneCommand::SetRefs {
                scene_id: id.clone(),
                refs: template.refs,
            })
            .unwrap();
        engine
            .store
            .apply(mewu_core::SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "operate selected region".into(),
            })
            .unwrap();
        let prepared =
            PreparedChatRegion::capture_source(engine.store.scene(&id).unwrap(), &id, Some("roi"))
                .unwrap()
                .unwrap();
        let run = engine.store.begin_run(&id).unwrap();
        let token = Arc::new(AtomicBool::new(false));
        let scope = prepared.bind(run.run_id.clone(), token.clone());
        assert!(scope.check_engine(&engine).is_err());
        let (sender, _receiver) = tokio::sync::oneshot::channel();
        engine.runs.insert(run.run_id.clone(), sender);
        let grant = engine.optional.require_agent().unwrap();
        engine
            .agent_runs
            .insert(run.run_id.clone(), (id.clone(), grant.into()));
        assert!(scope.check_engine(&engine).is_ok());
        token.store(true, Ordering::SeqCst);
        assert!(scope.check_engine(&engine).is_err());
        token.store(false, Ordering::SeqCst);
        let admission = engine.agent_runs.remove(&run.run_id).unwrap();
        assert!(scope.check_engine(&engine).is_err());
        engine.agent_runs.insert(run.run_id.clone(), admission);
        engine.store.cancel_run(&id).unwrap();
        assert!(scope.check_engine(&engine).is_err());
        assert!(!scope.cancel_token().load(Ordering::SeqCst));
    }
}
