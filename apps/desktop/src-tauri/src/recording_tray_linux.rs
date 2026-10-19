// SPDX-License-Identifier: MPL-2.0
//! Wayland cannot exclude an application window from a shared monitor. Keep
//! recording controls in the compositor's existing tray instead of the video.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Mutex,
};
use tauri::{
    menu::{Menu, MenuItem},
    AppHandle, Emitter, Manager,
};

#[derive(Clone)]
struct Attached {
    id: String,
    pause: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
}
pub(crate) struct Controls {
    menu: Menu<tauri::Wry>,
    attached: Mutex<Option<Attached>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    pause_enabled: bool,
    resume: bool,
    stop_enabled: bool,
}
fn view(phase: &str) -> View {
    View {
        pause_enabled: matches!(phase, "recording" | "paused"),
        resume: phase == "paused",
        stop_enabled: phase != "stopping",
    }
}
fn event_id(action: &str, id: &str) -> String {
    format!("recording-{action}:{id}")
}
fn action(value: &str) -> Option<(&str, &str)> {
    let (action, id) = value.split_once(':')?;
    if !matches!(action, "recording-pause" | "recording-stop")
        || !uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
    {
        return None;
    }
    Some((action, id))
}
impl Controls {
    pub(crate) fn new(menu: Menu<tauri::Wry>) -> Self {
        Self {
            menu,
            attached: Mutex::new(None),
        }
    }
    pub(crate) fn update_language(&self) -> Result<(), String> {
        let attached = self
            .attached
            .lock()
            .map_err(|_| "录屏托盘控件不可用")?
            .clone();
        if let Some(attached) = attached {
            let resume = crate::recording_wayland_host::status()?
                .is_some_and(|s| s.id == attached.id && s.phase == "paused");
            attached
                .pause
                .set_text(pause_text(resume))
                .map_err(|_| "无法更新托盘语言")?;
            attached
                .stop
                .set_text(crate::ui_language::text("停止录屏", "Stop Recording"))
                .map_err(|_| "无法更新托盘语言")?;
        }
        Ok(())
    }
    /// Only called on the native UI thread. No recording/Engine mutex is held
    /// while GTK menu operations run; each status is read before this method.
    fn apply(
        &self,
        app: &AppHandle,
        status: Option<crate::recording::Status>,
    ) -> Result<(), String> {
        let previous = self
            .attached
            .lock()
            .map_err(|_| "录屏托盘控件不可用")?
            .clone();
        if previous
            .as_ref()
            .is_some_and(|old| status.as_ref().is_none_or(|next| next.id != old.id))
        {
            let old = previous.as_ref().unwrap();
            self.remove_if_present(&old.pause)?;
            self.remove_if_present(&old.stop)?;
            *self.attached.lock().map_err(|_| "录屏托盘控件不可用")? = None;
        }
        let Some(status) = status else {
            return Ok(());
        };
        let current = self
            .attached
            .lock()
            .map_err(|_| "录屏托盘控件不可用")?
            .clone();
        let attached = match current {
            Some(current) => current,
            None => {
                let pause = MenuItem::with_id(
                    app,
                    event_id("pause", &status.id),
                    pause_text(false),
                    false,
                    None::<&str>,
                )
                .map_err(|_| "无法创建录屏托盘控件")?;
                let stop = MenuItem::with_id(
                    app,
                    event_id("stop", &status.id),
                    crate::ui_language::text("停止录屏", "Stop Recording"),
                    true,
                    None::<&str>,
                )
                .map_err(|_| "无法创建录屏托盘控件")?;
                let attached = Attached {
                    id: status.id.clone(),
                    pause,
                    stop,
                };
                // Retain both handles before the first fallible insertion.
                // Cancellation can then clean up even a partially built menu.
                *self.attached.lock().map_err(|_| "录屏托盘控件不可用")? = Some(attached.clone());
                attached
            }
        };
        if !self.contains(&attached.pause)? {
            self.menu
                .insert(&attached.pause, 1)
                .map_err(|_| "无法显示录屏托盘控件")?;
        }
        if !self.contains(&attached.stop)? {
            self.menu
                .insert(&attached.stop, 2)
                .map_err(|_| "无法显示录屏托盘控件")?;
        }
        let value = view(status.phase);
        attached
            .pause
            .set_text(pause_text(value.resume))
            .map_err(|_| "无法更新录屏托盘控件")?;
        attached
            .pause
            .set_enabled(value.pause_enabled)
            .map_err(|_| "无法更新录屏托盘控件")?;
        attached
            .stop
            .set_enabled(value.stop_enabled)
            .map_err(|_| "无法更新录屏托盘控件")?;
        Ok(())
    }
    fn contains(&self, item: &MenuItem<tauri::Wry>) -> Result<bool, String> {
        Ok(self
            .menu
            .items()
            .map_err(|_| "无法读取录屏托盘控件")?
            .iter()
            .any(|value| value.id() == item.id()))
    }
    fn remove_if_present(&self, item: &MenuItem<tauri::Wry>) -> Result<(), String> {
        if self.contains(item)? {
            self.menu.remove(item).map_err(|_| "无法收起录屏托盘控件")?;
        }
        Ok(())
    }
}
fn pause_text(resume: bool) -> &'static str {
    if resume {
        crate::ui_language::text("继续录屏", "Resume Recording")
    } else {
        crate::ui_language::text("暂停录屏", "Pause Recording")
    }
}
fn apply_current(app: &AppHandle) -> Result<(), String> {
    let status = crate::recording_wayland_host::status()?;
    let labels = app
        .try_state::<crate::ui_language::TrayLabels>()
        .ok_or("录屏托盘不可用")?;
    labels.recording_controls.apply(app, status)
}
/// Progress changes only the timer, so it must not enqueue GTK work per frame.
pub(crate) fn refresh(app: &AppHandle, phase: Option<&str>) {
    static PHASE: AtomicU8 = AtomicU8::new(0);
    let next = match phase {
        None => 0,
        Some("countdown") => 1,
        Some("starting") => 2,
        Some("recording") => 3,
        Some("paused") => 4,
        Some(_) => 5,
    };
    // An admission failure can leave a partially attached pair before the
    // first countdown event. Always reconcile None, even when the last public
    // phase was also idle, so cancellation still removes those owned handles.
    if PHASE.swap(next, Ordering::AcqRel) == next && next != 0 {
        return;
    }
    let target = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(error) = apply_current(&target) {
            let _ = target.emit_to("space", "host-error", error);
        }
    });
}
/// The worker countdown cannot start until the actual Stop entry exists.
pub(crate) async fn present(app: &AppHandle, id: &str) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let target = app.clone();
    let id = id.to_owned();
    app.run_on_main_thread(move || {
        let result = (|| {
            if send.is_closed() {
                return Err("录屏托盘初始化已取消".into());
            }
            crate::recording_wayland_host::tray_admission(&target, &id)?;
            apply_current(&target)
        })();
        let _ = send.send(result);
    })
    .map_err(|_| "无法初始化录屏托盘控件")?;
    tokio::time::timeout(std::time::Duration::from_secs(2), receive)
        .await
        .map_err(|_| "录屏托盘初始化超时")?
        .map_err(|_| "录屏托盘初始化中断")?
}
pub(crate) fn handle(app: &AppHandle, value: &str) -> bool {
    let Some((kind, id)) = action(value) else {
        return false;
    };
    let Ok(Some(status)) = crate::recording_wayland_host::status() else {
        return true;
    };
    // A queued click from a retired recording must never control its successor.
    if status.id != id {
        return true;
    }
    let action = if kind == "recording-stop" {
        "stop"
    } else if status.phase == "paused" {
        "resume"
    } else if status.phase == "recording" {
        "pause"
    } else {
        return true;
    };
    if let Err(error) = crate::recording::control_recording(app.clone(), id.into(), action.into()) {
        let _ = app.emit_to("space", "host-error", error);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_and_stopping_keep_stop_semantics_without_pause() {
        assert_eq!(
            view("countdown"),
            View {
                pause_enabled: false,
                resume: false,
                stop_enabled: true
            }
        );
        assert_eq!(view("starting"), view("countdown"));
        assert_eq!(
            view("stopping"),
            View {
                pause_enabled: false,
                resume: false,
                stop_enabled: false
            }
        );
    }
    #[test]
    fn active_pause_and_resume_are_distinct_states() {
        assert_eq!(
            view("recording"),
            View {
                pause_enabled: true,
                resume: false,
                stop_enabled: true
            }
        );
        assert_eq!(
            view("paused"),
            View {
                pause_enabled: true,
                resume: true,
                stop_enabled: true
            }
        );
    }
    #[test]
    fn menu_events_bind_exact_recording_identity() {
        let id = "b741fa14-303c-482e-8eef-9e5487cc2a2c";
        assert_eq!(
            action(&event_id("pause", id)),
            Some(("recording-pause", id))
        );
        assert_eq!(action(&event_id("stop", id)), Some(("recording-stop", id)));
        for bad in [
            "recording-stop",
            "recording-stop:",
            "recording-stop:old",
            "quit:b741fa14-303c-482e-8eef-9e5487cc2a2c",
        ] {
            assert!(action(bad).is_none());
        }
    }
}
