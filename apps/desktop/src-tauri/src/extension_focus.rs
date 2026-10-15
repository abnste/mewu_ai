// SPDX-License-Identifier: MPL-2.0
//! One explicit foreground region invocation may return focus at final drain.
//! Per-step observations stay nonactivating; no foreground identity is guessed.
use crate::{
    extension_overlay_policy::{self, RestoreSource},
    extension_region, Host,
};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, SetForegroundWindow, ShowWindow, GA_ROOT, SW_HIDE,
};

enum Handoff {
    Unseen,
    Waiting,
    Unowned,
    Target(extension_region::ForegroundIdentity),
}
pub(crate) struct FocusTicket {
    app: AppHandle,
    run_id: String,
    source: RestoreSource,
    space: WebviewWindow,
    handle: usize,
    handoff: Mutex<Handoff>,
}
impl FocusTicket {
    pub(crate) fn new(app: &AppHandle, run_id: String, scene_id: &str) -> Option<Arc<Self>> {
        let source = {
            let host = app.state::<Host>();
            let engine = host.lock().ok()?;
            let scene = engine
                .store
                .scenes()
                .iter()
                .find(|scene| scene.id == scene_id && !scene.closed && !scene.frozen)?;
            RestoreSource {
                scene_id: scene.id.clone(),
                background_id: scene.background.as_ref()?.id.clone(),
            }
        };
        let space = app.get_webview_window("space")?;
        let handle = space.hwnd().ok()?.0 as usize;
        let ticket = Arc::new(Self {
            app: app.clone(),
            run_id: run_id.clone(),
            source,
            space,
            handle,
            handoff: Mutex::new(Handoff::Unseen),
        });
        *app.state::<crate::extension_host::ExtensionHost>()
            .region_focus_owner
            .lock()
            .ok()? = Some(run_id);
        Some(ticket)
    }
    /// Called on the UI thread immediately before the first actual overlay hide.
    pub(crate) fn before_hide(&self) {
        let Ok(mut handoff) = self.handoff.lock() else {
            return;
        };
        if !matches!(*handoff, Handoff::Unseen) {
            return;
        }
        let foreground = unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) } as usize;
        *handoff = if foreground == self.handle
            && self.space.hwnd().is_ok_and(|h| h.0 as usize == self.handle)
        {
            Handoff::Waiting
        } else {
            Handoff::Unowned
        };
    }
    /// Capture only the first handoff, including failed RegionSession::begin.
    pub(crate) fn after_hide(&self) {
        let Ok(mut handoff) = self.handoff.lock() else {
            return;
        };
        if matches!(*handoff, Handoff::Waiting) {
            *handoff = extension_region::foreground_identity()
                .map(Handoff::Target)
                .unwrap_or(Handoff::Unowned);
        }
    }
    pub(crate) fn finalize(self: Arc<Self>) {
        let app = self.app.clone();
        let dispatcher = self.app.clone();
        let _ = dispatcher.run_on_main_thread(move || {
            let extensions = app.state::<crate::extension_host::ExtensionHost>();
            let Ok(mut owner) = extensions.region_focus_owner.lock() else {
                return;
            };
            if owner.as_deref() != Some(&self.run_id) {
                return;
            }
            let target = self
                .handoff
                .lock()
                .ok()
                .and_then(|handoff| match &*handoff {
                    Handoff::Target(target) => Some(target.clone()),
                    _ => None,
                });
            if let Some(target) = target {
                let host = app.state::<Host>();
                extension_overlay_policy::finalize_if_current(
                    &self.source,
                    &host.capturing,
                    &host.exit,
                    || host.lock().ok().map(|engine| engine.store.snapshot()),
                    || {
                        self.space
                            .hwnd()
                            .is_ok_and(|handle| handle.0 as usize == self.handle)
                            && extension_region::foreground_matches(&target)
                    },
                    || unsafe {
                        // Windows may reject foreground activation. Never leave
                        // a fullscreen capture layer covering another app while
                        // that app still owns the keyboard. Keep the scene saved.
                        if SetForegroundWindow(self.handle as _) == 0
                            || GetAncestor(GetForegroundWindow(), GA_ROOT) as usize != self.handle
                        {
                            ShowWindow(self.handle as _, SW_HIDE);
                        }
                    },
                );
            }
            *owner = None;
        });
    }
}
