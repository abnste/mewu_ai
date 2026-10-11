// SPDX-License-Identifier: MPL-2.0
//! Non-nested GTK dialogs. Source policy stays on the caller's blocking worker,
//! which retains its operation/source leases until GTK destruction is acknowledged.
use gtk::{glib, prelude::*};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

type Collect<T> = Box<dyn Fn(&gtk::Dialog, gtk::ResponseType) -> Result<T, String>>;
thread_local! {
    static SESSIONS: std::cell::RefCell<std::collections::HashMap<uuid::Uuid, gtk::Dialog>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

pub(crate) fn show<T: Send + 'static>(
    window: tauri::WebviewWindow,
    token: usize,
    cancel: &AtomicBool,
    allowed: &dyn Fn() -> Result<(), String>,
    build: impl FnOnce(&gtk::ApplicationWindow) -> Result<(gtk::Dialog, Collect<T>), String>
        + Send
        + 'static,
) -> Result<Option<T>, String> {
    if gtk::is_initialized_main_thread() {
        return Err("不能在界面线程等待导出窗口".into());
    }
    let native_cancel = Arc::new(AtomicBool::new(false));
    let ui_cancel = native_cancel.clone();
    let (send, receive) = mpsc::sync_channel(1);
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let prepare = (|| {
                if SESSIONS.with(|sessions| sessions.borrow().len() >= 8) {
                    return Err("请先关闭其他导出窗口".into());
                }
                if ui_cancel.load(Ordering::Acquire) {
                    return Err("导出请求已取消".into());
                }
                let parent = crate::window_actions_linux::parent(&target, token)?;
                let (dialog, collect) = build(&parent)?;
                dialog.set_transient_for(Some(&parent));
                dialog.set_modal(true);
                dialog.set_destroy_with_parent(true);
                Ok((dialog, collect))
            })();
            let (dialog, collect) = match prepare {
                Ok(value) => value,
                Err(e) => {
                    let _ = send.send(Err(e));
                    return;
                }
            };
            let sender = std::rc::Rc::new(std::cell::RefCell::new(Some(send)));
            let id = uuid::Uuid::new_v4();
            SESSIONS.with(|sessions| {
                sessions.borrow_mut().insert(id, dialog.clone());
            });
            let response_sender = sender.clone();
            dialog.connect_response(move |dialog, response| {
                let Some(send) = response_sender.borrow_mut().take() else {
                    return;
                };
                let result = collect(dialog, response);
                let dialog = dialog.clone();
                let display = dialog.display();
                // Keep an owned reference and unwind the response signal before
                // destroying the widget. Only its scalar result leaves the UI.
                glib::idle_add_local_once(move || {
                    // SAFETY: exclusively owned on this GTK UI thread.
                    unsafe {
                        dialog.destroy();
                    }
                    display.sync();
                    SESSIONS.with(|sessions| {
                        sessions.borrow_mut().remove(&id);
                    });
                    drop(dialog);
                    let _ = send.send(result);
                });
            });
            dialog.connect_destroy(move |dialog| {
                if let Some(send) = sender.borrow_mut().take() {
                    let display = dialog.display();
                    // Destruction from a parent is still inside GTK's signal
                    // stack. Acknowledge only after that stack has unwound.
                    glib::idle_add_local_once(move || {
                        display.sync();
                        SESSIONS.with(|sessions| {
                            sessions.borrow_mut().remove(&id);
                        });
                        let _ = send.send(Err("导出窗口已关闭".into()));
                    });
                }
            });
            let weak = dialog.downgrade();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let Some(dialog) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if ui_cancel.load(Ordering::Acquire) || !crate::window_actions_linux::visible(token)
                {
                    dialog.response(gtk::ResponseType::Cancel);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
            dialog.show_all();
        })
        .map_err(|_| "无法打开导出窗口")?;
    // A source rejection is terminal. Even a panic must cancel and wait for
    // UI cleanup rather than drop Work while its modal dialog survives.
    let mut rejection = None;
    let mut canceled = false;
    loop {
        if rejection.is_none() && !canceled {
            if cancel.load(Ordering::Acquire) {
                canceled = true;
            } else {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(allowed)) {
                    Ok(Ok(())) => (),
                    Ok(Err(e)) => rejection = Some(e),
                    Err(_) => rejection = Some("无法核对导出来源".into()),
                }
            }
            canceled |= cancel.load(Ordering::Acquire);
            if rejection.is_some() || canceled {
                native_cancel.store(true, Ordering::Release);
            }
        }
        match receive.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => {
                if let Some(e) = rejection {
                    return Err(e);
                }
                if canceled || cancel.load(Ordering::Acquire) {
                    return Ok(None);
                }
                allowed()?;
                return result.map(Some);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("导出窗口未完成".into()),
        }
    }
}
