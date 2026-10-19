// SPDX-License-Identifier: MPL-2.0
//! Keep synchronous WebView construction outside Windows event dispatch. Only
//! the owned HWND's message thread mutates capture leases and presents the HUD.
use super::{
    backend, edit_capture_session, exclude, Host, RecordingState, CONTROLS_HEIGHT, CONTROLS_WIDTH,
    LABEL,
};
use tauri::{AppHandle, Emitter, Manager};
use std::time::Duration;

/// Own the exact newly built window until the original capture session accepts
/// it. A late worker result, a dropped await or failed UI dispatch must not leave
/// a hidden label behind. Never look up a replacement by label for cleanup.
struct PendingControls<T> {
    window: Option<T>,
    cleanup: fn(T),
}
impl<T> PendingControls<T> {
    fn new(window: T, cleanup: fn(T)) -> Self {
        Self {
            window: Some(window),
            cleanup,
        }
    }
    fn window(&self) -> &T {
        self.window
            .as_ref()
            .expect("pending controls retain their window")
    }
    fn transferred(&mut self) {
        self.window.take();
    }
}
impl<T> Drop for PendingControls<T> {
    fn drop(&mut self) {
        if let Some(window) = self.window.take() {
            (self.cleanup)(window);
        }
    }
}

fn destroy_unattached(window: tauri::WebviewWindow) {
    // destroy targets this runtime window ID, not whichever window later has
    // LABEL. Its error only proves dispatch failed; do not report cleanup done.
    if window.destroy().is_err() {
        let _ =
            window
                .app_handle()
                .emit_to("space", "host-error", "无法关闭录屏控制条，请退出后重试");
    }
}

#[derive(Debug)]
pub(super) enum PrepareFailure {
    Canceled,
    Failed(String),
}
impl From<String> for PrepareFailure {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}
impl From<&str> for PrepareFailure {
    fn from(error: &str) -> Self {
        Self::Failed(error.into())
    }
}

fn current(app: &AppHandle, id: &str, require_session: bool) -> Result<(), PrepareFailure> {
    let state = app.state::<RecordingState>();
    let slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
    let active = slot.as_ref().ok_or("录屏已结束")?;
    current_policy(
        active,
        id,
        require_session,
        app.state::<Host>().exit.accepts_new_operations(),
    )
}

fn current_policy(
    active: &super::Active,
    id: &str,
    require_session: bool,
    accepts_new: bool,
) -> Result<(), PrepareFailure> {
    if active.status.id != id {
        return Err("录屏已结束".into());
    }
    if active.plugin_revoked {
        return Err("屏幕录制已停用或改变".into());
    }
    // begin_shutdown binds suppress_reveal under the same Active lock before
    // sending Stop. This exact bound operation may finish without any media.
    if active.commands.terminal_pending()? || (active.suppress_reveal && !accepts_new) {
        return Err(PrepareFailure::Canceled);
    }
    if !accepts_new {
        return Err("正在退出，请稍候".into());
    }
    if require_session && active.capture_session.is_none() {
        return Err("录屏窗口已改变".into());
    }
    Ok(())
}

fn presentation_ready(app: &AppHandle, id: &str, stage: &str) -> Result<bool, PrepareFailure> {
    let state = app.state::<RecordingState>();
    let slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
    let active = slot.as_ref().ok_or("录屏已结束")?;
    current_policy(active, id, true, app.state::<Host>().exit.accepts_new_operations())?;
    let hud = active.status.hud.as_ref().filter(|hud| hud.stage == stage)
        .ok_or("录屏界面阶段已改变")?;
    Ok(hud.ready)
}

fn accept_surface_ready(active: &mut super::Active, id: &str, stage: &str, owned: bool, accepts_new: bool) -> Result<bool, String> {
    if active.status.id != id || active.plugin_revoked || active.commands.terminal_pending()? || !accepts_new {
        return Ok(false);
    }
    if !owned { return Err("录屏界面不属于当前窗口".into()); }
    let Some(hud) = active.status.hud.as_mut().filter(|hud| hud.stage == stage) else { return Ok(false); };
    hud.ready = true;
    Ok(true)
}

pub(super) async fn surface_ready(
    app: &AppHandle, window: tauri::WebviewWindow, id: &str, stage: &str,
) -> Result<bool, String> {
    if !matches!(stage, "layout" | "mask") { return Err("录屏界面阶段无效".into()); }
    let target = app.clone(); let id = id.to_owned(); let stage = stage.to_owned();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<bool, String> {
            let state = target.state::<RecordingState>();
            let mut slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
            let Some(active) = slot.as_mut().filter(|active| active.status.id == id) else { return Ok(false); };
            let owned = active.capture_session.as_ref().is_some_and(|session| session.owns_hud(&window));
            accept_surface_ready(active, &id, &stage, owned, target.state::<Host>().exit.accepts_new_operations())
        })();
        let _ = send.send(result);
    }).map_err(|_| "无法确认录屏界面")?;
    receive.await.map_err(|_| "录屏界面确认中断")?
}

pub(super) async fn prepare(
    app: &AppHandle,
    id: &str,
    config: &backend::Config,
    scale: f64,
    f8: bool,
) -> Result<(), PrepareFailure> {
    if !scale.is_finite() || scale <= 0. { return Err("录屏显示比例无效".into()); }
    // Phase 1: save the original capture lease before mutating its HWND. Native
    // callbacks may reenter, so no RecordingState mutex spans window operations.
    let target = app.clone();
    let operation = id.to_owned();
    let config_for_capture = config.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(bool, bool, tauri::WebviewWindow), PrepareFailure> {
            current(&target, &operation, false)?;
            let main = target.get_webview_window("space").ok_or("窗口不存在")?;
            let shared = target
                .state::<Host>()
                .system_preferences
                .snapshot_for_operation()
                .map_err(|error| error.to_string())?
                .allow_screen_share;
            let roi = crate::shared_capture_windows::Rect::offset(
                config_for_capture.origin_x,
                config_for_capture.origin_y,
                config_for_capture.x,
                config_for_capture.y,
                config_for_capture.width,
                config_for_capture.height,
            )?;
            let session = crate::shared_capture_windows::Session::capture(&main, shared)?;
            let retain_main = session.can_retain_main_outside()?;
            {
                let state = target.state::<RecordingState>();
                let mut slot = state.0.lock().map_err(|_| "录屏状态不可用")?;
                let active = slot
                    .as_mut()
                    .filter(|active| active.status.id == operation)
                    .ok_or("录屏已结束")?;
                if active.capture_session.is_some() {
                    return Err("录屏窗口已准备".into());
                }
                active.capture_session = Some(session);
            }
            // Keep the original overlay visible through actual 3/2/1 rendering.
            // Its native capture lease is prepared only after the HUD mask ACK.
            let _ = roi;
            current(&target, &operation, true)?;
            Ok((shared, retain_main, main))
        })();
        let _ = send.send(result);
    })
    .map_err(|_| "无法准备录屏窗口")?;
    let (shared, retain_main, main) = receive.await.map_err(|_| "录屏窗口准备中断")??;

    // Phase 2: let Tauri dispatch creation from a blocking worker. Constructing
    // a WebView synchronously inside a Windows event callback can deadlock.
    let target = app.clone();
    let operation = id.to_owned();
    let config_for_window = config.clone();
    let (page_send, mut page_receive) = tokio::sync::oneshot::channel();
    let pending = tauri::async_runtime::spawn_blocking(move || {
        current(&target, &operation, true)?;
        let page_send = std::sync::Mutex::new(Some(page_send));
        let builder = tauri::WebviewWindowBuilder::new(
            &target,
            LABEL,
            tauri::WebviewUrl::App(if retain_main { "index.html?surface=recording" } else { "index.html?surface=recording&compound=1" }.into()),
        );
        // Owned HUDs stay above the retained main overlay. The cloak fallback
        // remains unowned so it cannot inherit its main window's cloak state.
        let builder = if retain_main {
            builder.owner(&main).map_err(|_| "无法绑定录屏控制条")?
        } else { builder };
        let window = builder
        .title("Mewu · 录屏")
        .inner_size(if retain_main { CONTROLS_WIDTH } else { config_for_window.monitor_width as f64 / scale },
            if retain_main { CONTROLS_HEIGHT } else { config_for_window.monitor_height as f64 / scale })
        .decorations(false)
        // One compound surface: transparent live ROI plus outside mask/controls.
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .on_page_load(move |_, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                if let Ok(mut pending) = page_send.lock() {
                    if let Some(send) = pending.take() { let _ = send.send(()); }
                }
            }
        })
        .disable_drag_drop_handler()
        .on_navigation(|url| {
            (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                || (matches!(url.scheme(), "http" | "https")
                    && url.host_str() == Some("tauri.localhost"))
                || (cfg!(debug_assertions)
                    && url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port() == Some(1420))
        })
        .build()
        .map_err(|error| {
            crate::window_creation_error::describe(
                crate::window_creation_error::Surface::RecordingControls,
                &error,
            )
        })?;
        Ok::<_, PrepareFailure>(PendingControls::new(window, destroy_unattached))
    })
    .await
    .map_err(|_| "录屏控制条创建中断")??;

    // Native navigation completion lets the compound page establish its initial
    // transparency before show. Hidden pages need not produce animation frames;
    // actual visible layout is still acknowledged separately after show below.
    let page_until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        current(app, id, true)?;
        match page_receive.try_recv() {
            Ok(()) => break,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => return Err("录屏页面加载中断".into()),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }
        if tokio::time::Instant::now() >= page_until { return Err("录屏页面加载超时".into()); }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Phase 3: only the original message thread may attach, verify, position and
    // show this HUD. If any fence fails before attachment the guard owns cleanup;
    // after attachment the existing finish() restores the exact capture session.
    let target = app.clone();
    let operation = id.to_owned();
    let config = config.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let mut pending = pending;
        let result = (|| -> Result<(), PrepareFailure> {
            current(&target, &operation, true)?;
            let toolbar = pending.window().clone();
            if !retain_main { toolbar.set_position(tauri::PhysicalPosition::new(config.origin_x, config.origin_y))
                .map_err(|_| "无法放置录屏界面")?; }
            toolbar.set_size(tauri::PhysicalSize::new(
                if retain_main { (CONTROLS_WIDTH * scale).round() as u32 } else { config.monitor_width },
                if retain_main { (CONTROLS_HEIGHT * scale).round() as u32 } else { config.monitor_height }))
                .map_err(|_| "无法调整录屏界面")?;
            if !shared { exclude(&toolbar, true)?; }
            edit_capture_session(&target.state::<RecordingState>(), &operation, |session| {
                session.attach_hud(&toolbar, retain_main)
            })?;
            // edit_capture_session has returned the lease to the same Active.
            pending.transferred();
            let display = crate::shared_capture_windows::Rect::new(
                    config.origin_x,
                    config.origin_y,
                    config.monitor_width,
                    config.monitor_height,
                )?;
            let roi = crate::shared_capture_windows::Rect::offset(
                    config.origin_x,
                    config.origin_y,
                    config.x,
                    config.y,
                    config.width,
                    config.height,
                )?;
            let work = crate::shared_capture_windows::work_area(&toolbar, display)?;
            if retain_main {
                let show = crate::shared_capture_windows::place_hud(&toolbar, roi, work, f8)?;
                if show {
                    edit_capture_session(&target.state::<RecordingState>(), &operation, |session| session.verify_hud_owner())?;
                    toolbar.show().map_err(|_| "无法显示录屏控制条")?;
                }
                return Ok(());
            }
            let position = crate::shared_capture_windows::hud_position(work, roi,
                ((CONTROLS_WIDTH * scale).round() as u32, (CONTROLS_HEIGHT * scale).round() as u32));
            if position.is_none() && !f8 { return Err("录屏区域外没有控制条空间，请使用较小选区".into()); }
            let rect = position.map(|(x,y)| super::Rect {
                x: (x as f64 - config.origin_x as f64) / config.monitor_width as f64,
                y: (y as f64 - config.origin_y as f64) / config.monitor_height as f64,
                width: CONTROLS_WIDTH * scale / config.monitor_width as f64,
                height: CONTROLS_HEIGHT * scale / config.monitor_height as f64,
            });
            current(&target, &operation, true)?;
            edit_capture_session(&target.state::<RecordingState>(), &operation, |session| {
                session.stage_scroll_hud(roi, &[display])
            })?;
            super::update(&target, &operation, |status| {
                status.hud = Some(super::Hud { rect, stage: "layout", ready: false });
            });
            edit_capture_session(&target.state::<RecordingState>(), &operation, |session| session.verify_hud_owner())?;
            toolbar.show().map_err(|_| "无法显示录屏界面")?;
            Ok(())
        })();
        let _ = send.send(result);
    })
    .map_err(|_| "无法准备录屏窗口")?;
    receive.await.map_err(|_| "录屏窗口准备中断")??;
    if !retain_main {
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        while !presentation_ready(app, id, "layout")? {
            if tokio::time::Instant::now() >= until { return Err("录屏界面未能完成布局".into()); }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    Ok(())
}

/// The worker is already prepared but cannot encode until the original message
/// thread has removed every owned HWND pixel from the ROI and flushed it.
pub(super) fn finish_countdown(app: &AppHandle, id: &str, config: &backend::Config) -> Result<bool, String> {
    let target = app.clone(); let operation = id.to_owned(); let config = config.clone();
    let transitioned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let completed = transitioned.clone();
    crate::shared_capture_windows::check(app, move || {
        match current(&target, &operation, true) {
            Ok(()) => {},
            Err(PrepareFailure::Canceled) => return Ok(()),
            Err(PrepareFailure::Failed(error)) => return Err(error),
        }
        let roi = crate::shared_capture_windows::Rect::offset(config.origin_x,config.origin_y,config.x,config.y,config.width,config.height)?;
        let display = crate::shared_capture_windows::Rect::new(config.origin_x,config.origin_y,config.monitor_width,config.monitor_height)?;
        let compound = target.state::<RecordingState>().0.lock().map_err(|_| "录屏状态不可用")?
            .as_ref().filter(|active| active.status.id == operation)
            .ok_or("录屏已结束")?.status.hud.is_some();
        let mut retain_main = false;
        edit_capture_session(&target.state::<RecordingState>(), &operation, |session| {
            if compound { session.stage_scroll_hud(roi, &[display])?; }
            session.main.prepare(roi, false)?;
            if compound { session.configure_scroll_hud(roi, &[display])?; }
            session.verify()?;
            retain_main = session.retains_main_outside()?;
            Ok(())
        })?;
        super::update(&target, &operation, |status| {
            status.phase = "starting";
            status.countdown = 0;
            status.retain_main = Some(retain_main);
            if let Some(hud) = &mut status.hud { hud.stage = "live"; }
        });
        completed.store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    })?;
    Ok(transitioned.load(std::sync::atomic::Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn recording_controls_caller_keeps_confirmed_cancel_distinct_from_creation_failure() {
        assert_eq!(super::super::preparation_result(Ok(())), Ok(true));
        assert_eq!(
            super::super::preparation_result(Err(PrepareFailure::Canceled)),
            Ok(false)
        );
        let commands = crate::recording_worker::ControlBus::default();
        commands.send(backend::Control::Stop).unwrap();
        let error = "无法打开录屏控制条（WebView2，0x80070005）".to_string();
        // Even a concurrent Stop cannot turn an already observed creation error
        // into successful cancellation or cause the media worker to be started.
        let outcome = super::super::preparation_result(Err(PrepareFailure::Failed(error.clone())));
        assert!(commands.terminal_pending().unwrap());
        assert_eq!(outcome, Err(error));
    }

    #[test]
    fn recording_controls_preparation_cancellation_is_exact_and_keeps_failures() {
        let commands = Arc::new(crate::recording_worker::ControlBus::default());
        let mut active = super::super::Active {
            capture_session: None,
            status: super::super::Status {
                id: "original".into(),
                scene_id: "scene".into(),
                phase: "countdown",
                countdown: 3,
                elapsed_ms: 0,
                rect: super::super::Rect {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                },
                stop_hotkey: "F8 / Esc".into(),
                audio: backend::AudioMode::System,
                hud: Some(super::super::Hud { rect: None, stage: "layout", ready: false }),
                retain_main: None,
            },
            commands: commands.clone(),
            grant: crate::plugins::RecordingGrant {
                plugin_id: crate::plugins::CORE_RECORDING.into(),
                revision: 1,
                contribution_id: "record".into(),
            },
            audio_grant: None,
            plugin_revoked: false,
            suppress_reveal: false,
        };
        assert!(current_policy(&active, "original", false, true).is_ok());
        assert!(!accept_surface_ready(&mut active, "replacement", "layout", true, true).unwrap());
        assert!(!accept_surface_ready(&mut active, "original", "mask", true, true).unwrap());
        assert!(accept_surface_ready(&mut active, "original", "layout", false, true).is_err());
        assert!(!active.status.hud.as_ref().unwrap().ready);
        assert!(!accept_surface_ready(&mut active, "original", "layout", true, false).unwrap());
        assert!(accept_surface_ready(&mut active, "original", "layout", true, true).unwrap());
        assert!(accept_surface_ready(&mut active, "original", "layout", true, true).unwrap());
        active.status.hud.as_mut().unwrap().stage = "mask";
        active.status.hud.as_mut().unwrap().ready = false;
        assert!(!accept_surface_ready(&mut active, "original", "layout", true, true).unwrap());
        assert!(matches!(
            current_policy(&active, "replacement", false, true),
            Err(PrepareFailure::Failed(_))
        ));
        assert!(matches!(
            current_policy(&active, "original", true, true),
            Err(PrepareFailure::Failed(_))
        ));
        assert!(matches!(
            current_policy(&active, "original", false, false),
            Err(PrepareFailure::Failed(_))
        ));
        active.suppress_reveal = true;
        assert!(matches!(
            current_policy(&active, "original", false, false),
            Err(PrepareFailure::Canceled)
        ));
        active.suppress_reveal = false;
        commands.send(backend::Control::Cancel).unwrap();
        assert!(!accept_surface_ready(&mut active, "original", "mask", true, true).unwrap());
        assert!(!active.status.hud.as_ref().unwrap().ready);
        assert!(matches!(
            current_policy(&active, "original", false, true),
            Err(PrepareFailure::Canceled)
        ));
        assert!(matches!(
            commands.take().unwrap(),
            Some(backend::Control::Cancel)
        ));
        active.plugin_revoked = true;
        assert!(!accept_surface_ready(&mut active, "original", "mask", true, true).unwrap());
        assert!(matches!(
            current_policy(&active, "original", false, true),
            Err(PrepareFailure::Failed(_))
        ));
    }

    #[test]
    fn recording_controls_pending_guard_closes_exact_unattached_and_transfers_only_once() {
        struct Window {
            id: u32,
            closed: Arc<Mutex<Vec<u32>>>,
        }
        fn close(window: Window) {
            window.closed.lock().unwrap().push(window.id);
        }
        let closed = Arc::new(Mutex::new(vec![]));
        let old = PendingControls::new(
            Window {
                id: 11,
                closed: closed.clone(),
            },
            close,
        );
        // A replacement is independent even if a platform label would be reused.
        let mut replacement = PendingControls::new(
            Window {
                id: 12,
                closed: closed.clone(),
            },
            close,
        );
        assert_eq!(old.window().id, 11);
        drop(old);
        assert_eq!(*closed.lock().unwrap(), vec![11]);
        replacement.transferred();
        replacement.transferred();
        drop(replacement);
        assert_eq!(*closed.lock().unwrap(), vec![11]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recording_controls_late_worker_result_is_owned_when_receiver_is_gone() {
        fn close(closed: Arc<Mutex<usize>>) {
            *closed.lock().unwrap() += 1;
        }
        let closed = Arc::new(Mutex::new(0));
        let observed = closed.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        drop(receive);
        tauri::async_runtime::spawn_blocking(move || {
            let pending = PendingControls::new(observed, close);
            // This is the same owned value returned by the real creation worker.
            drop(send.send(pending));
        })
        .await
        .unwrap();
        assert_eq!(*closed.lock().unwrap(), 1);
    }
}
