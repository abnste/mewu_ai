// SPDX-License-Identifier: MPL-2.0
//! Independent GTK pins on an existing local X11/XWayland screen. The capture
//! Space keeps its original display and Wayland portal; no process-wide backend
//! or display default is changed. All GTK objects stay on the UI thread.
use super::{PinRegistry, Rect};
use gtk::prelude::*;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Manager, WebviewWindow};

struct PendingRestore {
    topmost: bool,
    cancel: Arc<AtomicBool>,
}

thread_local! {
    static SPACE_BINDINGS: RefCell<HashSet<usize>> = RefCell::default();
    static HIDDEN_FOR_SPACE: RefCell<HashMap<String, Rect>> = RefCell::default();
    static RESTORING: RefCell<HashMap<String, PendingRestore>> = RefCell::default();
    static CAPTURE_HIDDEN: RefCell<HashSet<String>> = RefCell::default();
}

fn local_display_name(value: &str) -> bool {
    let Some(value) = value.strip_prefix(':') else {
        return false;
    };
    let mut parts = value.split('.');
    let valid = |part: &str| {
        !part.is_empty() && part.len() <= 5 && part.bytes().all(|b| b.is_ascii_digit())
    };
    valid(parts.next().unwrap_or_default())
        && parts.next().is_none_or(valid)
        && parts.next().is_none()
}

pub(super) fn prepare(window: &WebviewWindow) -> Result<(), String> {
    prepare_floating(window)?;
    bind_space(window.app_handle())?;
    Ok(())
}
/// The application registers a GtkWindow before Tao attaches its WebKit child.
/// The temporary handler affects only synchronous construction on the UI thread
/// and is removed on every return; the default display and Space stay Wayland.
pub(super) fn build_floating<T>(
    app:&AppHandle,
    build:impl FnOnce(&AppHandle)->Result<T,String>,
) -> Result<T,String> {
    if !gtk::is_initialized_main_thread(){return Err("浮动对象只能在界面线程创建".into());}
    let space=app.get_webview_window("space").ok_or("截图窗口不可用")?;
    let widget=space.gtk_window().map_err(|_|"截图窗口不可用")?;
    if widget.display().type_().name()=="GdkX11Display" {return build(app);}
    let name=std::env::var("DISPLAY").map_err(|_|"当前桌面没有可用的原位贴图服务")?;
    if !local_display_name(&name){return Err("原位贴图需要本机显示服务".into());}
    let manager=gtk::gdk::DisplayManager::get();
    let display=manager.list_displays().into_iter()
        .find(|d|!d.is_closed()&&d.type_().name()=="GdkX11Display"&&d.name()==name)
        .or_else(||gtk::gdk::Display::open(&name)).ok_or("无法连接原位贴图服务")?;
    if display.is_closed()||display.type_().name()!="GdkX11Display"{return Err("原位贴图服务不可用".into());}
    let application=widget.application().ok_or("界面应用不可用")?;
    let screen=display.default_screen();
    let handler=application.connect_window_added(move |_,window| {
        if !window.is_realized()&&!window.is_mapped(){window.set_screen(&screen);}
    });
    struct ScopedHandler(gtk::Application,Option<gtk::glib::SignalHandlerId>);
    impl Drop for ScopedHandler {fn drop(&mut self){if let Some(id)=self.1.take(){self.0.disconnect(id);}}}
    let _handler=ScopedHandler(application,Some(handler));
    build(app)
}
pub(crate) fn prepare_floating(window: &WebviewWindow) -> Result<(), String> {
    if !gtk::is_initialized_main_thread() {
        return Err("贴图窗口只能在界面线程操作".into());
    }
    let widget = window.gtk_window().map_err(|_| "贴图窗口不可用")?;
    if widget.is_mapped() {
        return Err("贴图初始化状态无效".into());
    }
    let display = widget.display();
    if display.is_closed()||display.type_().name()!="GdkX11Display" {
        return Err("浮动对象必须先在本机显示服务创建".into());
    }
    if !widget.is_realized(){widget.set_visual(display.default_screen().rgba_visual().as_ref());}
    widget.set_focus_on_map(false);
    widget.set_accept_focus(true);
    // These builder hints were first applied to the former Wayland drawable.
    // Reapply them to the pin's actual X11 screen before its first native map.
    widget.set_keep_above(true);
    widget.set_skip_taskbar_hint(true);
    widget.set_skip_pager_hint(true);
    Ok(())
}

fn drawable(window: &WebviewWindow) -> Result<gtk::gdk::Window, String> {
    if !gtk::is_initialized_main_thread() {
        return Err("贴图窗口只能在界面线程操作".into());
    }
    let widget = window.gtk_window().map_err(|_| "贴图窗口不可用")?;
    let drawable = widget.window().ok_or("贴图窗口尚未创建")?;
    if drawable.is_destroyed()
        || drawable.type_().name() != "GdkX11Window"
        || drawable.display().type_().name() != "GdkX11Display"
        || drawable.display().is_closed()
    {
        return Err("原位贴图服务不可用".into());
    }
    Ok(drawable)
}

pub(super) fn current_rect(window: &WebviewWindow) -> Result<Rect, String> {
    // While Space renders the independent object, its native duplicate is
    // intentionally withdrawn. WM withdrawal can reparent/reposition it; use
    // the last real rectangle saved before hide, or a subsequent object move.
    if let Some(rect) = HIDDEN_FOR_SPACE.with(|hidden| hidden.borrow().get(window.label()).copied())
    {
        return Ok(rect);
    }
    native_rect(window)
}

pub(super) fn remember_rect(window: &WebviewWindow, rect: Rect) -> Result<(), String> {
    let own = window.clone();
    let update = move || {
        HIDDEN_FOR_SPACE.with(|hidden| {
            if let Some(saved) = hidden.borrow_mut().get_mut(own.label()) {
                if *saved != rect {
                    RESTORING.with(|pending| {
                        if let Some(restore) = pending.borrow().get(own.label()) {
                            restore.cancel.store(true, Ordering::Release);
                        }
                    });
                }
                *saved = rect;
            }
        });
    };
    if gtk::is_initialized_main_thread() {
        update();
    } else {
        window
            .run_on_main_thread(update)
            .map_err(|_| "无法定位贴图")?;
    }
    Ok(())
}

fn native_rect(window: &WebviewWindow) -> Result<Rect, String> {
    let drawable = drawable(window)?;
    // GTK's native X11 frame_extents reads XGetGeometry/XTranslateCoordinates.
    // Tao outer_position/outer_size are last ConfigureNotify caches and can
    // acknowledge geometry before the window manager has actually mapped it.
    let frame = drawable.frame_extents();
    let scale = drawable.scale_factor();
    if scale <= 0 || frame.width() <= 0 || frame.height() <= 0 {
        return Err("贴图原位尺寸不可用".into());
    }
    Ok(Rect {
        x: frame.x().checked_mul(scale).ok_or("贴图位置超出范围")?,
        y: frame.y().checked_mul(scale).ok_or("贴图位置超出范围")?,
        width: u32::try_from(frame.width())
            .ok()
            .and_then(|v| v.checked_mul(scale as u32))
            .ok_or("贴图尺寸超出范围")?,
        height: u32::try_from(frame.height())
            .ok()
            .and_then(|v| v.checked_mul(scale as u32))
            .ok_or("贴图尺寸超出范围")?,
    })
}

fn atom_words(data: &[u8]) -> Result<Vec<usize>, String> {
    const WORD: usize = std::mem::size_of::<usize>();
    if data.len() > 32 * WORD || data.len() % WORD != 0 {
        return Err("贴图窗口状态格式无效".into());
    }
    Ok(data
        .chunks_exact(WORD)
        .map(|word| usize::from_ne_bytes(word.try_into().unwrap()))
        .collect())
}

pub(super) fn apply_rect(window: &WebviewWindow, rect: Rect) -> Result<(), String> {
    if gtk::is_initialized_main_thread() {
        remember_rect(window, rect)?;
        return place_native(window, rect);
    }
    let own = window.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let result = remember_rect(&own, rect).and_then(|()| place_native(&own, rect));
            let _ = tx.send(result);
        })
        .map_err(|_| "无法调整贴图")?;
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| "贴图尺寸调整中断")?
}

fn request_logical_size(window: &WebviewWindow, rect: Rect) -> Result<(), String> {
    if !gtk::is_initialized_main_thread() {
        return Err("贴图窗口只能在界面线程操作".into());
    }
    let widget = window.gtk_window().map_err(|_| "贴图窗口不可用")?;
    let scale = widget.scale_factor();
    if scale <= 0 {
        return Err("贴图原位尺寸不可用".into());
    }
    let logical =
        tauri::PhysicalSize::new(rect.width, rect.height).to_logical::<i32>(f64::from(scale));
    if logical.width <= 0 || logical.height <= 0 {
        return Err("贴图原位尺寸不可用".into());
    }
    // GTK otherwise promotes the previous WebKit natural size to a fixed
    // minimum. Keep programmatic resizing enabled internally, but lock both
    // native bounds to this target so users cannot freely resize the object.
    widget.set_resizable(true);
    widget.set_geometry_hints(
        None::<&gtk::Window>,
        Some(&gtk::gdk::Geometry::new(
            logical.width,
            logical.height,
            logical.width,
            logical.height,
            0,
            0,
            0,
            0,
            0.,
            0.,
            gtk::gdk::Gravity::Center,
        )),
        gtk::gdk::WindowHints::MIN_SIZE | gtk::gdk::WindowHints::MAX_SIZE,
    );
    // Update the real child requisition before every resize and restore.
    window
        .with_webview(move |platform| {
            platform
                .inner()
                .set_size_request(logical.width, logical.height);
        })
        .map_err(|_| "无法调整贴图内容".into())
}

fn place_native(window: &WebviewWindow, rect: Rect) -> Result<(), String> {
    request_logical_size(window, rect)?;
    // A restore job's target is immutable. Do not call the object-edit wrapper:
    // it would overwrite a newer desired rectangle while this job is pending.
    window
        .set_position(tauri::PhysicalPosition::new(rect.x, rect.y))
        .map_err(|_| "无法定位贴图")?;
    window
        .set_size(tauri::PhysicalSize::new(rect.width, rect.height))
        .map_err(|_| "无法调整贴图".into())
}

fn native_ready(window: &WebviewWindow, rect: Rect, topmost: bool) -> Result<bool, String> {
    let widget = window.gtk_window().map_err(|_| "贴图窗口不可用")?;
    // Screen migration can allocate children before the new native drawable
    // has been realized/mapped. That state is pending, not a failed identity.
    // Once created, preserve every strict native type/geometry/state check.
    if !widget.is_realized() || !widget.is_mapped() || widget.window().is_none() {
        return Ok(false);
    }
    let drawable = drawable(window)?;
    if !widget.is_mapped() || !drawable.is_viewable() || native_rect(window)? != rect {
        return Ok(false);
    }
    let scale = drawable.scale_factor();
    if widget.allocated_width().checked_mul(scale) != i32::try_from(rect.width).ok()
        || widget.allocated_height().checked_mul(scale) != i32::try_from(rect.height).ok()
    {
        // A native X rectangle may be accepted before GTK has laid out the
        // former 80px hidden WebView. Do not admit that smaller content surface.
        return Ok(false);
    }
    let state = gtk::gdk::Atom::intern("_NET_WM_STATE");
    let Some((kind, format, data)) =
        gtk::gdk::property_get(&drawable, &state, &gtk::gdk::SELECTION_TYPE_ATOM, 0, 128, 0)
    else {
        return Ok(false);
    };
    if kind != gtk::gdk::SELECTION_TYPE_ATOM || format != 32 {
        return Err("贴图窗口状态格式无效".into());
    }
    // GTK converts X11 ATOM properties into native-sized GdkAtom words.
    // Never interpret their 32-bit wire format as a packed native Vec<Atom>.
    let atoms = atom_words(&data)?;
    let above = atoms.contains(&gtk::gdk::Atom::intern("_NET_WM_STATE_ABOVE").value());
    Ok(above == topmost
        && atoms.contains(&gtk::gdk::Atom::intern("_NET_WM_STATE_SKIP_TASKBAR").value()))
}

fn bind_space(app: &AppHandle) -> Result<(), String> {
    let space = app.get_webview_window("space").ok_or("空间窗口不可用")?;
    let widget = space.gtk_window().map_err(|_| "空间窗口不可用")?;
    let identity = widget.as_ptr() as usize;
    let added = SPACE_BINDINGS.with(|set| set.borrow_mut().insert(identity));
    if !added {
        return Ok(());
    }
    for mapped in [true, false] {
        let app = app.clone();
        if mapped {
            widget.connect_map(move |_| super::arrange_for_space(&app));
        } else {
            widget.connect_unmap(move |_| super::arrange_for_space(&app));
        }
    }
    widget.connect_destroy(move |_| {
        SPACE_BINDINGS.with(|set| {
            set.borrow_mut().remove(&identity);
        });
    });
    Ok(())
}

pub(super) async fn show(
    window: &WebviewWindow,
    original: Rect,
    canceled: Arc<AtomicBool>,
) -> Result<(), String> {
    show_with_state(window, original, true, canceled).await
}

async fn show_with_state(
    window: &WebviewWindow,
    original: Rect,
    topmost: bool,
    canceled: Arc<AtomicBool>,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let signal = Arc::new(Mutex::new(Vec::<gtk::glib::SignalHandlerId>::new()));
    let callback = sender.clone();
    let callback_signal = signal.clone();
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                if canceled.load(Ordering::Acquire) {
                    return Err("已取消贴图".into());
                }
                let widget = target.gtk_window().map_err(|_| "贴图窗口不可用")?;
                if widget.display().type_().name() != "GdkX11Display" {
                    return Err("原位贴图服务不可用".into());
                }
                // Client mapped flags and old WM properties can outlive a
                // withdrawal. Admit this show only after its real MapNotify;
                // otherwise cleanup could remove the post-map hint handler
                // while the manager is still deleting the previous state.
                let observed_map = Arc::new(AtomicBool::new(false));
                let acknowledge = Arc::new({
                    let own = target.clone();
                    let ack = callback.clone();
                    let cancel = canceled.clone();
                    let observed_map = observed_map.clone();
                    move || {
                        let result = if cancel.load(Ordering::Acquire) {
                            Some(Err("已取消贴图".into()))
                        } else if !observed_map.load(Ordering::Acquire) {
                            None
                        } else {
                            match native_ready(&own, original, topmost) {
                                Ok(true) => Some(Ok(())),
                                Ok(false) => None,
                                Err(error) => Some(Err(error)),
                            }
                        };
                        if let Some(result) = result {
                            if let Ok(mut sender) = ack.lock() {
                                if let Some(tx) = sender.take() {
                                    let _ = tx.send(result);
                                }
                            }
                        }
                    }
                });
                let ack = acknowledge.clone();
                let configured = widget.connect_configure_event(move |_, _| {
                    ack();
                    false
                });
                let ack = acknowledge.clone();
                let mapped_window = target.clone();
                let mapped_cancel = canceled.clone();
                let mapped_observed = observed_map.clone();
                let mapped = widget.connect_map_event(move |_, event| {
                    if !mapped_cancel.load(Ordering::Acquire) {
                        if let Ok(native) = drawable(&mapped_window) {
                            if event.window().as_ref() != Some(&native) {
                                // A child's map or a former display's queued
                                // event is not this top-level's mapping epoch.
                                ack();
                                return gtk::glib::Propagation::Proceed;
                            }
                            mapped_observed.store(true, Ordering::Release);
                            // GTK caches skip-taskbar/pager booleans; repeated
                            // setters can be no-ops after WM withdrawal deletes
                            // the old _NET_WM_STATE. On real MapNotify, request
                            // hints on this newly mapped native GdkWindow. Its
                            // X11 backend sends normal EWMH client messages.
                            native.set_keep_above(topmost);
                            native.set_skip_taskbar_hint(true);
                            native.set_skip_pager_hint(true);
                        }
                    }
                    ack();
                    gtk::glib::Propagation::Proceed
                });
                let ack = acknowledge.clone();
                let changed = widget.connect_window_state_event(move |_, _| {
                    ack();
                    gtk::glib::Propagation::Proceed
                });
                let ack = acknowledge.clone();
                let allocated = widget.connect_size_allocate(move |_, _| ack());
                let ack = acknowledge.clone();
                let unmapped_observed = observed_map.clone();
                let unmapped_window = target.clone();
                let unmapped = widget.connect_unmap_event(move |_, event| {
                    if let Ok(native) = drawable(&unmapped_window) {
                        if event.window().as_ref() == Some(&native) {
                            unmapped_observed.store(false, Ordering::Release);
                        }
                    }
                    ack();
                    gtk::glib::Propagation::Proceed
                });
                *callback_signal.lock().map_err(|_| "贴图状态不可用")? =
                    vec![configured, mapped, changed, allocated, unmapped];
                widget.set_keep_above(topmost);
                widget.set_skip_taskbar_hint(true);
                widget.set_skip_pager_hint(true);
                target.show().map_err(|_| "无法显示贴图")?;
                // X11 managers may ignore initial geometry before map. Reassert the
                // original rectangle after showing; native map/configure/state
                // and actual GTK allocation admit it, never an optimistic cache.
                place_native(&target, original)?;
                acknowledge();
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                if let Ok(mut sender) = callback.lock() {
                    if let Some(tx) = sender.take() {
                        let _ = tx.send(Err(error));
                    }
                }
            }
        })
        .map_err(|_| "无法显示贴图")?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(8), rx)
        .await
        .map_err(|_| "贴图原位尺寸未能确认，请重试".to_string())
        .and_then(|reply| reply.map_err(|_| "贴图显示中断".to_string()))
        .and_then(|reply| reply);
    let cleanup = window.clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(mut signal) = signal.lock() {
            if let Ok(widget) = cleanup.gtk_window() {
                for id in signal.drain(..) {
                    widget.disconnect(id);
                }
            }
        }
    });
    result
}

pub(super) fn hide_for_capture(window: &WebviewWindow) -> Result<(), String> {
    let native = drawable(window)?;
    let widget = window.gtk_window().map_err(|_| "贴图窗口不可用")?;
    // current_rect prefers HIDDEN_FOR_SPACE's saved object rectangle, including
    // a pin already withdrawn while Space renders it. Never overwrite it with
    // the window manager's unmapped/reparented position.
    let rect = current_rect(window)?;
    RESTORING.with(|pending| {
        if let Some(restore) = pending.borrow().get(window.label()) {
            restore.cancel.store(true, Ordering::Release);
        }
    });
    // Preserve the last true object position before GTK withdraws it. The
    // normal Space-unmap path, not an unchecked raw show, restores this target.
    HIDDEN_FOR_SPACE.with(|hidden| {
        hidden.borrow_mut().insert(window.label().into(), rect);
    });
    CAPTURE_HIDDEN.with(|hidden| {
        hidden.borrow_mut().insert(window.label().into());
    });
    widget.hide();
    // This flush alone is not evidence. The subscribed bounded X11 guard reads
    // actual UNMAPPED and all StructureNotify events before portal admission.
    // https://docs.gtk.org/gdk3/method.Display.flush.html
    native.display().flush();
    if widget.is_mapped() {
        Err("贴图窗口尚未取消映射".into())
    } else {
        Ok(())
    }
}

pub(super) fn finish_capture_hides(labels: &[String]) {
    CAPTURE_HIDDEN.with(|hidden| {
        let mut hidden = hidden.borrow_mut();
        for label in labels {
            hidden.remove(label);
        }
    });
}

pub(super) fn restack(app: &AppHandle) {
    let Some(space) = app.get_webview_window("space") else {
        return;
    };
    let Ok(space_widget) = space.gtk_window() else {
        return;
    };
    let Some(registry) = app.try_state::<PinRegistry>() else {
        return;
    };
    let labels = match registry.state.lock() {
        Ok(state) => state
            .entries
            .iter()
            .filter(|(_, entry)| entry.live)
            .map(|(label, entry)| (label.clone(), entry.topmost))
            .collect::<Vec<_>>(),
        Err(_) => return,
    };
    let visible = space_widget.is_visible() && space_widget.is_mapped();
    let cross_display = space_widget.display().type_().name() != "GdkX11Display";
    HIDDEN_FOR_SPACE.with(|hidden| {
        let mut hidden = hidden.borrow_mut();
        hidden.retain(|label, _| labels.iter().any(|(live, _)| live == label));
        CAPTURE_HIDDEN.with(|capture| {
            capture
                .borrow_mut()
                .retain(|label| labels.iter().any(|(live, _)| live == label))
        });
        RESTORING.with(|pending| {
            pending.borrow_mut().retain(|label, restore| {
                let live = labels.iter().any(|(live, _)| live == label);
                if !live {
                    restore.cancel.store(true, Ordering::Release);
                }
                live
            });
        });
        for (label, topmost) in labels {
            // Hiding the Wayland Space emits unmap before permission is shown.
            // That ordinary callback must not remap a pin while this capture's
            // subscribed native guard owns its hidden lifetime.
            if CAPTURE_HIDDEN.with(|hidden| hidden.borrow().contains(&label)) {
                continue;
            }
            let Some(pin) = app.get_webview_window(&label) else {
                continue;
            };
            RESTORING.with(|pending| {
                if let Some(restore) = pending.borrow().get(&label) {
                    if restore.topmost != topmost {
                        restore.cancel.store(true, Ordering::Release);
                    }
                }
            });
            if visible && cross_display {
                // Wayland does not allow a sibling ordering request against an
                // XWayland window. Space already draws the exact same objects;
                // hide only native duplicates, retaining their live pixel lease.
                if pin.is_visible().unwrap_or(false) {
                    RESTORING.with(|pending| {
                        if let Some(restore) = pending.borrow().get(&label) {
                            restore.cancel.store(true, Ordering::Release);
                        }
                    });
                    let rect = hidden
                        .get(&label)
                        .copied()
                        .map(Ok)
                        .unwrap_or_else(|| native_rect(&pin));
                    if let Ok(rect) = rect {
                        if pin.hide().is_ok() {
                            hidden.insert(label, rect);
                        }
                    }
                }
            } else if !visible {
                if let Some(rect) = hidden.get(&label).copied() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    let admitted = RESTORING.with(|pending| {
                        let mut pending = pending.borrow_mut();
                        if pending.contains_key(&label) {
                            false
                        } else {
                            pending.insert(
                                label.clone(),
                                PendingRestore {
                                    topmost,
                                    cancel: cancel.clone(),
                                },
                            );
                            true
                        }
                    });
                    if !admitted {
                        continue;
                    }
                    tauri::async_runtime::spawn(async move {
                        let result = show_with_state(&pin, rect, topmost, cancel.clone()).await;
                        let finished = pin.clone();
                        let _ = pin.run_on_main_thread(move || {
                            let current = RESTORING.with(|pending| {
                                let mut pending = pending.borrow_mut();
                                if pending
                                    .get(&label)
                                    .is_some_and(|own| Arc::ptr_eq(&own.cancel, &cancel))
                                {
                                    pending.remove(&label);
                                    true
                                } else {
                                    false
                                }
                            });
                            if !current {
                                return;
                            }
                            let app = finished.app_handle();
                            let space_visible = app
                                .get_webview_window("space")
                                .is_some_and(|space| space.is_visible().unwrap_or(true));
                            let canceled = cancel.load(Ordering::Acquire);
                            if result.is_ok() && !canceled && !space_visible {
                                // Object queries use desired geometry while ACK
                                // is pending; only now resume live server reads.
                                HIDDEN_FOR_SPACE.with(|hidden| {
                                    hidden.borrow_mut().remove(&label);
                                });
                            } else {
                                // Preserve latest object edits, never replace
                                // them with an intermediate WM remap position.
                                let _ = finished.hide();
                            }
                            // A canceled remap follows a new object move or
                            // Space transition. Retry that latest desired state;
                            // ordinary native failure waits for a user action.
                            if result.is_ok() || canceled {
                                super::arrange_for_space(app);
                            }
                        });
                    });
                }
            } else if let (Ok(pin_widget), Some(parent)) = (pin.gtk_window(), space_widget.window())
            {
                if let Some(drawable) = pin_widget.window() {
                    drawable.restack(Some(&parent), false);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atom_property_uses_native_gdk_words_and_rejects_truncation_or_excess() {
        let data = [17usize.to_ne_bytes(), 314usize.to_ne_bytes()].concat();
        assert_eq!(atom_words(&data).unwrap(), vec![17, 314]);
        assert!(atom_words(&data[..data.len() - 1]).is_err());
        assert!(atom_words(&vec![0; 33 * std::mem::size_of::<usize>()]).is_err());
        assert_eq!(atom_words(&[]).unwrap(), Vec::<usize>::new());
    }
    #[test]
    fn pin_display_only_admits_bounded_existing_local_x11_names() {
        for name in [":0", ":1.0", ":65535.65535"] {
            assert!(local_display_name(name));
        }
        for name in [
            "",
            "wayland-0",
            "host:0",
            "127.0.0.1:0",
            ":",
            ":-1",
            ":0.",
            ":0.1.2",
            ":000000",
            ":0\n",
        ] {
            assert!(!local_display_name(name));
        }
    }
}
