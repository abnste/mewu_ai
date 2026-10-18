// SPDX-License-Identifier: MPL-2.0
//! One on-demand, first-party window for minimized scenes. Run completion only emits
//! summaries; showing or focusing windows is restricted to explicit user actions.
use mewu_core::{MessageRole, RunStatus, Scene, Snapshot};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

pub const LABEL: &str = "frozen-widget";

/// Serialize creation, layout and retirement without blocking the window thread.
/// The native Destroyed acknowledgement releases the label before any recreation.
#[derive(Default)]
pub struct Lifecycle {
    operations: tokio::sync::Mutex<()>,
    retiring: AtomicBool,
    retired: tokio::sync::Notify,
    cleanup_pending: AtomicBool,
}

fn is_minimized(scene: &Scene) -> bool {
    scene.frozen && scene.minimized && !scene.closed && has_content(scene)
}

pub fn destroyed(app: &AppHandle) {
    if let Some(lifecycle) = app.try_state::<Lifecycle>() {
        lifecycle.retiring.store(false, Ordering::Release);
        lifecycle.retired.notify_one();
    }
}

async fn wait_retired(lifecycle: &Lifecycle) -> Result<(), String> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while lifecycle.retiring.load(Ordering::Acquire) {
            lifecycle.retired.notified().await;
        }
    })
    .await
    .map_err(|_| "冻结会话窗口尚未关闭".to_string())
}

/// A background answer may update an existing widget, but cannot create or show
/// one. Once no minimized scene remains, retire its renderer as well as its HWND.
pub fn reconcile(app: &AppHandle, snapshot: &Snapshot) {
    if snapshot.scenes.iter().any(is_minimized) || app.get_webview_window(LABEL).is_none() {
        return;
    }
    let lifecycle = app.state::<Lifecycle>();
    if lifecycle.cleanup_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let lifecycle = app.state::<Lifecycle>();
        struct Pending<'a>(&'a AtomicBool);
        impl Drop for Pending<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _pending = Pending(&lifecycle.cleanup_pending);
        let host = app.state::<crate::Host>();
        loop {
            if !host.exit.accepts_new_operations() {
                return;
            }
            let _operation = lifecycle.operations.lock().await;
            // Capture witnesses must not lose a window during pixel acquisition.
            // Do not hold the lifecycle lock while a user transition owns it.
            let Some(_transition) = crate::SpaceTransition::try_begin(&host.capturing) else {
                drop(_operation);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            };
            let empty = host
                .lock()
                .is_ok_and(|engine| !engine.store.scenes().iter().any(is_minimized));
            if !empty || !host.exit.accepts_new_operations() {
                return;
            }
            if let Some(window) = app.get_webview_window(LABEL) {
                lifecycle.retiring.store(true, Ordering::Release);
                if window.destroy().is_err() {
                    lifecycle.retiring.store(false, Ordering::Release);
                    return;
                }
                // Do not reuse a label merely because destroy() was queued.
                let _ = wait_retired(&lifecycle).await;
            }
            return;
        }
    });
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenScene {
    id: String,
    title: String,
    preview: String,
    status: &'static str,
    updated_at: u64,
    run_id: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct FrozenSnapshot {
    revision: u64,
    scenes: Vec<FrozenScene>,
}

pub fn has_content(scene: &Scene) -> bool {
    scene.background.is_some()
        || !scene.messages.is_empty()
        || !scene.items.is_empty()
        || !scene.regions.is_empty()
        || !scene.refs.is_empty()
        || !scene.draft.trim().is_empty()
}

// Presentation only: do not rename persisted scenes or send their message history
// to the widget. Keep this fallback order aligned with SessionDock.tsx.
fn display_title(scene: &Scene) -> String {
    let title = scene.title.trim();
    if !matches!(title, "" | "新场景" | "新会话") {
        return title.into();
    }
    let question = scene
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User && !message.text.trim().is_empty())
        .map(|message| message.text.as_str());
    let first_asset = scene.items.first().map(|item| item.asset.name.as_str());
    [question, Some(scene.draft.as_str()), first_asset]
        .into_iter()
        .flatten()
        .find(|text| !text.trim().is_empty())
        .map(|text| {
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(40)
                .collect()
        })
        .unwrap_or_else(|| {
            if scene.background.is_some() {
                "截图会话".into()
            } else {
                "新会话".into()
            }
        })
}

pub fn snapshot(state: &Snapshot, revision: u64) -> FrozenSnapshot {
    snapshot_scenes(&state.scenes, revision)
}

pub(crate) fn snapshot_scenes(source: &[Scene], revision: u64) -> FrozenSnapshot {
    let mut scenes: Vec<_> = source
        .iter()
        .filter(|scene| is_minimized(scene))
        .map(|scene| {
            let preview = scene
                .messages
                .iter()
                .rev()
                .find(|message| {
                    message.role == MessageRole::Assistant
                        && scene
                            .run
                            .as_ref()
                            .is_none_or(|run| message.run_id.as_ref() == Some(&run.id))
                })
                .map(|message| message.text.as_str())
                .unwrap_or("")
                .trim()
                .chars()
                .take(91)
                .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
                .collect::<String>();
            let preview = if preview.chars().count() > 90 {
                format!("{}...", preview.chars().take(90).collect::<String>())
            } else {
                preview
            };
            FrozenScene {
                id: scene.id.clone(),
                title: display_title(scene),
                preview,
                status: match scene.run.as_ref().map(|run| &run.status) {
                    Some(RunStatus::Running) => "running",
                    Some(RunStatus::Completed) => "completed",
                    Some(RunStatus::Failed) => "failed",
                    Some(RunStatus::Canceled) => "canceled",
                    None => "idle",
                },
                updated_at: scene.updated_at,
                run_id: scene.run.as_ref().map(|run| run.id.clone()),
            }
        })
        .collect();
    scenes.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    FrozenSnapshot { revision, scenes }
}

fn build(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        tauri::WebviewUrl::App("index.html?surface=frozen".into()),
    )
    .title("Mewu · 冻结会话")
    .inner_size(226., 74.)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .background_color(tauri::window::Color(0, 0, 0, 0))
    // Windows adds a rectangular 1px border to undecorated windows with native
    // shadows. The rounded pill draws its own bounded shadow in frozen.css.
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .disable_drag_drop_handler()
    .on_navigation(|url| {
        let local_app = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (matches!(url.scheme(), "http" | "https")
                && url.host_str() == Some("tauri.localhost"));
        let local_dev = cfg!(debug_assertions)
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == Some(1420);
        local_app || local_dev
    })
    .build()
}

pub fn hide(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.hide().map_err(|_| "无法收起冻结会话")?;
    }
    Ok(())
}

/// Restore a saved scene on its original monitor when it is still available.
/// A disconnected monitor falls back to the current screen; scene content is retained.
pub async fn prepare_restore(
    app: &AppHandle,
    background: Option<&mewu_core::Asset>,
) -> Result<(), String> {
    let window = app.get_webview_window("space").ok_or("空间窗口不可用")?;
    let original_origin = background.and_then(|asset| Some((asset.origin_x?, asset.origin_y?)));
    let target = window.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                target
                    .app_handle()
                    .state::<crate::Host>()
                    .exit
                    .ensure_new_operation()?;
                target.set_decorations(false).map_err(|_| "无法恢复空间")?;
                target.set_resizable(false).map_err(|_| "无法恢复空间")?;
                #[cfg(windows)]
                {
                    let monitors = target.available_monitors().map_err(|_| "无法定位显示器")?;
                    let monitor = monitors
                        .into_iter()
                        .find(|monitor| {
                            original_origin == Some((monitor.position().x, monitor.position().y))
                        })
                        .or(target.current_monitor().map_err(|_| "无法定位空间")?)
                        .or(target.primary_monitor().map_err(|_| "无法定位显示器")?)
                        .ok_or("显示器不可用")?;
                    target
                        .set_simple_fullscreen(false)
                        .map_err(|_| "无法恢复空间")?;
                    target
                        .set_position(*monitor.position())
                        .map_err(|_| "无法定位空间")?;
                    target
                        .set_size(*monitor.size())
                        .map_err(|_| "无法调整空间")?;
                    target
                        .set_fullscreen_on_monitor(tauri::PhysicalPosition::new(
                            f64::from(monitor.position().x) + f64::from(monitor.size().width) / 2.,
                            f64::from(monitor.position().y) + f64::from(monitor.size().height) / 2.,
                        ))
                        .map_err(|_| "无法全屏恢复空间")?;
                }
                #[cfg(not(windows))]
                {
                    let _ = original_origin;
                    target
                        .set_simple_fullscreen(true)
                        .map_err(|_| "无法全屏恢复空间")?;
                }
                target
                    .set_always_on_top(true)
                    .map_err(|_| "无法恢复空间置顶")?;
                Ok(())
            })();
            let _ = sender.send(result);
        })
        .map_err(|_| "无法恢复空间")?;
    receiver.await.map_err(|_| "空间恢复中断")?
}

/// The work area already excludes taskbars and docks. Only window size and
/// margins are scaled; multiplying the monitor's global origin is incorrect.
fn placement(
    origin: (i32, i32),
    size: (u32, u32),
    scale: f64,
    expanded: bool,
) -> (i32, i32, u32, u32) {
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    let (width, height) = if expanded { (280., 380.) } else { (226., 74.) };
    let margin = (8. * scale).round() as i64;
    let width = ((width * scale).round() as u32)
        .min(size.0.saturating_sub((margin * 2) as u32))
        .max(1);
    let height = ((height * scale).round() as u32)
        .min(size.1.saturating_sub((margin * 2) as u32))
        .max(1);
    let x = i64::from(origin.0) + i64::from(size.0) - i64::from(width) - margin;
    let y = i64::from(origin.1) + i64::from(size.1) - i64::from(height) - margin;
    (
        x.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        y.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        width,
        height,
    )
}

fn resize_anchor(
    current: (i32, i32, u32, u32),
    desired: (u32, u32),
    origin: (i32, i32),
    area: (u32, u32),
) -> (i32, i32) {
    let min_x = i64::from(origin.0);
    let min_y = i64::from(origin.1);
    let max_x = min_x + i64::from(area.0.saturating_sub(desired.0));
    let max_y = min_y + i64::from(area.1.saturating_sub(desired.1));
    let x =
        (i64::from(current.0) + i64::from(current.2) - i64::from(desired.0)).clamp(min_x, max_x);
    let y =
        (i64::from(current.1) + i64::from(current.3) - i64::from(desired.1)).clamp(min_y, max_y);
    (
        x.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        y.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
    )
}

pub async fn layout(app: &AppHandle, expanded: bool, reveal: bool) -> Result<(), String> {
    let lifecycle = app.state::<Lifecycle>();
    let _operation = lifecycle.operations.lock().await;
    wait_retired(&lifecycle).await?;
    app.state::<crate::Host>().exit.ensure_new_operation()?;
    if !app
        .state::<crate::Host>()
        .lock()?
        .store
        .scenes()
        .iter()
        .any(is_minimized)
    {
        return Err("没有最小化的会话".into());
    }
    let window = if let Some(window) = app.get_webview_window(LABEL) {
        window
    } else if reveal {
        // WebView2 creation in a synchronous event/main-thread closure can
        // deadlock. Let Tauri dispatch native work from this blocking worker.
        let target = app.clone();
        tauri::async_runtime::spawn_blocking(move || build(&target))
            .await
            .map_err(|_| "冻结会话窗口创建中断")?
            .map_err(|_| "无法创建冻结会话窗口")?
    } else {
        return Err("冻结会话窗口不可用".into());
    };
    let handle = app.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                handle.state::<crate::Host>().exit.ensure_new_operation()?;
                let widget = handle
                    .get_webview_window(LABEL)
                    .ok_or("冻结会话窗口不可用")?;
                let space = handle.get_webview_window("space").ok_or("空间窗口不可用")?;
                let monitor = if reveal {
                    space.current_monitor()
                } else {
                    widget.current_monitor()
                }
                .map_err(|_| "无法定位冻结会话")?
                .or(widget.primary_monitor().map_err(|_| "无法定位显示器")?)
                .ok_or("显示器不可用")?;
                let work = monitor.work_area();
                let (mut x, mut y, width, height) = placement(
                    (work.position.x, work.position.y),
                    (work.size.width, work.size.height),
                    monitor.scale_factor(),
                    expanded,
                );
                if !reveal {
                    let position = widget.outer_position().map_err(|_| "无法读取浮窗位置")?;
                    let size = widget.inner_size().map_err(|_| "无法读取浮窗大小")?;
                    (x, y) = resize_anchor(
                        (position.x, position.y, size.width, size.height),
                        (width, height),
                        (work.position.x, work.position.y),
                        (work.size.width, work.size.height),
                    );
                }
                widget
                    .set_position(tauri::PhysicalPosition::new(x, y))
                    .map_err(|_| "无法定位冻结会话")?;
                widget
                    .set_size(tauri::PhysicalSize::new(width, height))
                    .map_err(|_| "无法调整冻结会话")?;
                if reveal {
                    // Both calls happen on the window thread before acknowledging the command.
                    space.hide().map_err(|_| "无法隐藏空间")?;
                    crate::capture_visibility::apply(&widget)?;
                    widget.show().map_err(|_| "无法显示冻结会话")?;
                }
                let _ = handle.emit_to(LABEL, "frozen-widget-layout", expanded);
                Ok(())
            })();
            let _ = sender.send(result);
        })
        .map_err(|_| "无法切换冻结会话")?;
    receiver.await.map_err(|_| "冻结会话切换中断")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{SceneCommand, Store};

    #[tokio::test]
    async fn stale_destroy_notification_cannot_release_a_new_retirement() {
        let lifecycle = Lifecycle::default();
        lifecycle.retired.notify_one();
        lifecycle.retiring.store(true, Ordering::Release);
        let waiting = wait_retired(&lifecycle);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting)
                .await
                .is_err()
        );
        lifecycle.retiring.store(false, Ordering::Release);
        lifecycle.retired.notify_one();
        assert!(waiting.await.is_ok());
        // An acknowledgement before wait_retired starts is also safe.
        assert!(wait_retired(&lifecycle).await.is_ok());
    }

    #[test]
    fn summary_title_identifies_the_first_question_without_overwriting_explicit_names() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: format!("  看看\n{}\t后续", "图".repeat(45)),
            })
            .unwrap();
        let run = store.begin_run(&id).unwrap();
        store
            .finish_run(&id, &run.run_id, "回答内容不应变成标题")
            .unwrap();
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "后来的草稿".into(),
            })
            .unwrap();
        store
            .apply(SceneCommand::FreezeScene {
                scene_id: id.clone(),
            })
            .unwrap();
        let state = store.snapshot();
        assert_eq!(
            snapshot(&state, 1).scenes[0].title,
            format!("看看 {}", "图".repeat(37))
        );
        assert!(matches!(
            state
                .scenes
                .iter()
                .find(|scene| scene.id == id)
                .unwrap()
                .title
                .as_str(),
            "新场景" | "新会话"
        ));

        store
            .apply(SceneCommand::RenameScene {
                scene_id: id,
                title: "订单分析".into(),
            })
            .unwrap();
        assert_eq!(snapshot(&store.snapshot(), 2).scenes[0].title, "订单分析");
    }

    #[test]
    fn frozen_summary_keeps_scene_identity_and_status_without_connection_or_asset_paths() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: a.clone(),
                draft: "问题 A".into(),
            })
            .unwrap();
        let run = store.begin_run(&a).unwrap();
        store
            .apply(SceneCommand::FreezeScene {
                scene_id: a.clone(),
            })
            .unwrap();
        let active = store.snapshot().active_scene_id;
        let before = snapshot(&store.snapshot(), 1);
        assert_eq!(before.scenes.len(), 1);
        assert_eq!(before.scenes[0].id, a);
        assert_eq!(before.scenes[0].status, "running");
        store.finish_run(&a, &run.run_id, "回答 A").unwrap();
        let after = snapshot(&store.snapshot(), 2);
        assert_eq!(after.scenes[0].id, a);
        assert_eq!(after.scenes[0].status, "completed");
        assert_eq!(after.scenes[0].preview, "回答 A");
        assert_eq!(store.snapshot().active_scene_id, active);
        let wire = serde_json::to_value(after).unwrap();
        assert!(wire.get("connection").is_none());
        assert!(wire["scenes"][0].get("background").is_none());
        assert!(wire["scenes"][0].get("messages").is_none());
        store
            .apply(SceneCommand::ActivateScene {
                scene_id: a.clone(),
            })
            .unwrap();
        assert!(
            snapshot(&store.snapshot(), 3).scenes.is_empty(),
            "automatic blank scenes are not floating conversations"
        );
    }

    #[test]
    fn widget_counts_only_explicitly_minimized_open_scenes_and_truncates_final_answer_from_start() {
        use mewu_core::{SceneCommand, Store};
        let mut store = Store::open_in_memory().unwrap();
        let history = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: history.clone(),
                draft: "只在历史中".into(),
            })
            .unwrap();
        store.apply(SceneCommand::NewScene).unwrap();
        let minimized = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: minimized.clone(),
                draft: "会话".into(),
            })
            .unwrap();
        let run = store.begin_run(&minimized).unwrap();
        store
            .apply(SceneCommand::FreezeScene {
                scene_id: minimized.clone(),
            })
            .unwrap();
        store
            .finish_run(
                &minimized,
                &run.run_id,
                &format!("开头{}", "🙂".repeat(100)),
            )
            .unwrap();
        let state = snapshot(&store.snapshot(), 2);
        assert_eq!(state.scenes.len(), 1);
        assert_eq!(state.scenes[0].id, minimized);
        assert_eq!(
            state.scenes[0].preview,
            format!("开头{}...", "🙂".repeat(88))
        );
        store
            .apply(SceneCommand::CloseScene {
                scene_id: minimized,
            })
            .unwrap();
        assert!(snapshot(&store.snapshot(), 3).scenes.is_empty());
    }

    #[test]
    fn borrowed_scene_summaries_preserve_full_snapshot_output_and_scene_selection() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.snapshot().active_scene_id;
        store.apply(SceneCommand::SetDraft {
            scene_id: id.clone(), draft: "借用摘要".into(),
        }).unwrap();
        let run = store.begin_run(&id).unwrap();
        store.finish_run(&id, &run.run_id, &format!("开头{}", "🙂".repeat(100))).unwrap();
        store.apply(SceneCommand::FreezeScene { scene_id: id.clone() }).unwrap();
        let state = store.snapshot();
        let active = state.active_scene_id.clone();
        let original = serde_json::to_value(&state).unwrap();
        let borrowed = snapshot_scenes(store.scenes(), 17);
        assert_eq!(serde_json::to_value(&borrowed).unwrap(), serde_json::to_value(snapshot(&state, 17)).unwrap());
        assert_eq!(borrowed.revision, 17);
        assert_eq!(borrowed.scenes.len(), 1);
        assert_eq!(borrowed.scenes[0].id, id);
        assert_eq!(borrowed.scenes[0].status, "completed");
        assert_eq!(borrowed.scenes[0].preview, format!("开头{}...", "🙂".repeat(88)));
        assert_eq!(store.snapshot().active_scene_id, active);
        assert_eq!(serde_json::to_value(store.snapshot()).unwrap(), original);
        store.apply(SceneCommand::CloseScene { scene_id: id }).unwrap();
        assert!(snapshot_scenes(store.scenes(), 18).scenes.is_empty());
        assert_eq!(serde_json::to_value(snapshot_scenes(store.scenes(), 18)).unwrap(),
            serde_json::to_value(snapshot(&store.snapshot(), 18)).unwrap());
    }

    #[test]
    fn widget_placement_respects_negative_monitor_origin_dpi_and_work_area() {
        assert_eq!(
            placement((-2560, -200), (2560, 1400), 1.5, false),
            (-351, 1077, 339, 111)
        );
        let (x, y, width, height) = placement((1920, 0), (1920, 1040), 2., true);
        assert_eq!((x, y, width, height), (3264, 264, 560, 760));
        let (x, y, width, height) = placement((0, 0), (320, 240), 1., true);
        assert_eq!((x, y, width, height), (32, 8, 280, 224));
        assert_eq!(
            resize_anchor((1000, 600, 226, 74), (280, 380), (0, 0), (1920, 1040)),
            (946, 294)
        );
        assert_eq!(
            resize_anchor((-1910, 200, 226, 74), (280, 380), (-1920, 0), (1920, 1040)),
            (-1920, 0)
        );
    }
}
