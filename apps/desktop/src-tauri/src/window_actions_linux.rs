// SPDX-License-Identifier: MPL-2.0
//! GTK-owned action identities. Resolve on the UI thread before taking Engine
//! locks; publication checks read only bounded scalar lifecycle state.
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

const UNAVAILABLE: &str = "图片窗口已关闭或收起";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    widget: usize,
    drawable: usize,
}
#[derive(Default)]
struct Owner {
    generation: u64,
    identity: Option<Identity>,
    visible: bool,
    token: Option<usize>,
}
#[derive(Default)]
struct Registry {
    owners: HashMap<String, Owner>,
    last_token: usize,
    failed: bool,
}
impl Registry {
    fn bind(
        &mut self,
        label: &str,
        identity: Identity,
        visible: bool,
    ) -> Result<(usize, u64, bool), String> {
        let owned_label = label == "space"
            || label.strip_prefix("pin-").is_some_and(|id| {
                uuid::Uuid::parse_str(id).is_ok_and(|value| value.hyphenated().to_string() == id)
            });
        // Bound tombstones too: destroyed unique labels must not grow forever.
        if self.failed
            || !owned_label
            || identity.widget == 0
            || identity.drawable == 0
            || (!self.owners.contains_key(label) && self.owners.len() >= 4096)
        {
            return Err(UNAVAILABLE.into());
        }
        if !visible {
            if let Some(owner) = self.owners.get_mut(label) {
                owner.visible = false;
                owner.token = None;
            }
            return Err(UNAVAILABLE.into());
        }
        let owner = self.owners.entry(label.into()).or_default();
        let subscribe = owner.identity != Some(identity);
        if subscribe {
            if owner.generation >= 4096 {
                self.failed = true;
                return Err(UNAVAILABLE.into());
            }
            owner.generation += 1;
            owner.identity = Some(identity);
            owner.token = None;
        }
        owner.visible = visible;
        if owner.token.is_none() {
            self.last_token = self.last_token.checked_add(1).ok_or(UNAVAILABLE)?;
            owner.token = Some(self.last_token);
        }
        Ok((owner.token.unwrap(), owner.generation, subscribe))
    }
    fn visible(&self, token: usize) -> bool {
        !self.failed
            && token != 0
            && self
                .owners
                .values()
                .any(|o| o.visible && o.identity.is_some() && o.token == Some(token))
    }
    fn update(&mut self, label: &str, generation: u64, visible: bool) {
        if let Some(owner) = self.owners.get_mut(label) {
            if owner.generation == generation {
                owner.visible = visible;
                if !visible {
                    owner.token = None;
                }
            }
        }
    }
    fn revoke(&mut self, label: &str, generation: Option<u64>) {
        if let Some(owner) = self.owners.get_mut(label) {
            if generation.is_none_or(|g| g == owner.generation) {
                owner.visible = false;
                owner.token = None;
                owner.identity = None;
            }
        }
    }
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
pub(crate) fn visible(token: usize) -> bool {
    registry().lock().is_ok_and(|r| r.visible(token))
}
pub(crate) fn revoke_owner(label: &str) {
    if let Ok(mut r) = registry().lock() {
        r.revoke(label, None);
    }
}

#[cfg(target_os = "linux")]
fn gtk_visible(widget: &gtk::ApplicationWindow) -> bool {
    use gtk::prelude::*;
    widget.is_mapped()
        && widget.is_visible()
        && widget.window().is_some_and(|w| {
            !w.state()
                .intersects(gtk::gdk::WindowState::ICONIFIED | gtk::gdk::WindowState::WITHDRAWN)
        })
}
#[cfg(target_os = "linux")]
fn bind(window: &tauri::WebviewWindow) -> Result<usize, String> {
    if !gtk::is_initialized_main_thread() {
        return Err(UNAVAILABLE.into());
    }
    let widget = window.gtk_window().map_err(|_| UNAVAILABLE)?;
    bind_widget(window.label(), &widget)
}
#[cfg(target_os = "linux")]
fn bind_widget(label: &str, widget: &gtk::ApplicationWindow) -> Result<usize, String> {
    use gtk::prelude::*;
    let drawable = widget.window().ok_or(UNAVAILABLE)?;
    let identity = Identity {
        widget: widget.as_ptr() as usize,
        drawable: drawable.as_ptr() as usize,
    };
    let (token, generation, subscribe) =
        registry()
            .lock()
            .map_err(|_| UNAVAILABLE)?
            .bind(label, identity, gtk_visible(&widget))?;
    if subscribe {
        let callback_label = label.to_owned();
        widget.connect_unrealize(move |_| {
            if let Ok(mut r) = registry().lock() {
                r.revoke(&callback_label, Some(generation));
            }
        });
        let callback_label = label.to_owned();
        widget.connect_unmap(move |_| {
            if let Ok(mut r) = registry().lock() {
                r.update(&callback_label, generation, false);
            }
        });
        let callback_label = label.to_owned();
        widget.connect_window_state_event(move |widget, _| {
            if let Ok(mut r) = registry().lock() {
                r.update(&callback_label, generation, gtk_visible(widget));
            }
            gtk::glib::Propagation::Proceed
        });
    }
    Ok(token)
}
#[cfg(target_os = "linux")]
pub(crate) fn window_handle(window: &tauri::WebviewWindow) -> Result<usize, String> {
    if gtk::is_initialized_main_thread() {
        return bind(window);
    }
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = send.send(bind(&target));
        })
        .map_err(|_| UNAVAILABLE)?;
    receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| UNAVAILABLE)?
}
/// UI only. Match the original token and GTK object again before presenting a
/// modal child. Never reinterpret a token as a native pointer.
#[cfg(target_os = "linux")]
pub(crate) fn parent(
    window: &tauri::WebviewWindow,
    token: usize,
) -> Result<gtk::ApplicationWindow, String> {
    if !visible(token) || bind(window)? != token {
        return Err(UNAVAILABLE.into());
    }
    window.gtk_window().map_err(|_| UNAVAILABLE.into())
}

/// GTK retains the image and serves future selection requests on its existing
/// main loop. A short-lived arboard owner would lose Linux clipboard contents.
#[cfg(target_os = "linux")]
pub(crate) fn publish_image(display: &gtk::gdk::Display, image: image::RgbaImage) -> Result<(), String> {
    if !gtk::is_initialized_main_thread() {
        return Err("只能在界面线程发布图片".into());
    }
    let width = image.width();
    let height = image.height();
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 33_554_432
    {
        return Err("复制图片尺寸无效".into());
    }
    let bytes = gtk::glib::Bytes::from_owned(image.into_raw());
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        gtk::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        width as i32,
        height as i32,
        width as i32 * 4,
    );
    gtk::Clipboard::for_display(display, &gtk::gdk::SELECTION_CLIPBOARD).set_image(&pixbuf);
    display.sync();
    Ok(())
}
#[cfg(target_os = "linux")]
pub(crate) fn copy_image(
    window: tauri::WebviewWindow,
    token: usize,
    image: image::RgbaImage,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    with_display(window, token, cancel, move |display| {
        publish_image(display, image)
    })
}

/// The blocking caller retains actual work until the original UI callback
/// finishes. The callback receives a GTK display only after its token matches.
#[cfg(target_os = "linux")]
pub(crate) fn with_display(
    window: tauri::WebviewWindow,
    token: usize,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    publish: impl FnOnce(&gtk::gdk::Display) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    use gtk::prelude::*;
    use std::sync::atomic::Ordering;
    if gtk::is_initialized_main_thread() {
        return Err("不能在界面线程等待复制操作".into());
    }
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                if cancel.load(Ordering::Acquire) {
                    return Err("复制请求已取消".into());
                }
                let parent = parent(&target, token)?;
                let display = parent.display();
                if cancel.load(Ordering::Acquire) {
                    return Err("复制请求已取消".into());
                }
                publish(&display)
            })();
            let _ = send.send(result);
        })
        .map_err(|_| "无法复制图片")?;
    // Keep the caller's actual Work alive through a queued UI callback. A
    // timeout must never release it while that callback can still publish.
    receive.recv().map_err(|_| "复制操作未完成")?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> Identity {
        Identity {
            widget: 1,
            drawable: 2,
        }
    }
    #[test]
    fn hide_remap_and_pointer_reuse_never_revive_an_action() {
        let mut r = Registry::default();
        let (old, generation, _) = r.bind("space", identity(), true).unwrap();
        assert!(r.visible(old));
        r.update("space", generation, false);
        let (remapped, _, subscribe) = r.bind("space", identity(), true).unwrap();
        assert!(!subscribe);
        assert!(!r.visible(old));
        assert!(r.visible(remapped));
        r.revoke("space", None);
        let (new, next, subscribe) = r.bind("space", identity(), true).unwrap();
        assert!(subscribe);
        assert!(next > generation);
        assert!(!r.visible(remapped));
        r.revoke("space", Some(generation));
        assert!(r.visible(new));
    }
    #[test]
    fn invalid_owner_and_exhausted_tokens_fail_closed() {
        let mut r = Registry::default();
        assert!(r.bind("renderer-window", identity(), true).is_err());
        r.last_token = usize::MAX;
        assert!(r.bind("space", identity(), true).is_err());
        assert!(!r.visible(0));
    }
    #[test]
    fn independent_pin_identity_never_reuses_space_or_destroyed_pin_tokens() {
        let mut r = Registry::default();
        let label = format!("pin-{}", uuid::Uuid::new_v4());
        let (space, _, _) = r.bind("space", identity(), true).unwrap();
        let (pin, generation, _) = r.bind(&label, identity(), true).unwrap();
        assert_ne!(space, pin);
        r.update(&label, generation, false);
        assert!(r.visible(space));
        assert!(!r.visible(pin));
        let (remapped, _, _) = r.bind(&label, identity(), true).unwrap();
        r.revoke(&label, None);
        assert!(!r.visible(remapped));
        assert!(r.bind("pin-arbitrary", identity(), true).is_err());
    }
    #[cfg(all(target_os = "linux", feature = "linux-x11-tests"))]
    #[test]
    fn x11_visibility_native_gtk_action_lifetime() {
        use gtk::prelude::*;
        gtk::init().expect("the Linux GTK action gate requires an actual X server");
        let widget = gtk::ApplicationWindow::builder()
            .title("Mewu synthetic GTK fixture")
            .build();
        assert!(bind_widget("space", &widget).is_err());
        widget.show_all();
        let old = bind_widget("space", &widget).unwrap();
        assert!(visible(old));
        widget.hide();
        assert!(!visible(old));
        widget.show_all();
        let fresh = bind_widget("space", &widget).unwrap();
        assert_ne!(old, fresh);
        assert!(visible(fresh));
        assert!(!visible(old));
        let display = widget.display();
        // SAFETY: a synthetic owned widget on the test's GTK thread.
        unsafe {
            widget.destroy();
        }
        assert!(!visible(fresh));
        let replacement = gtk::ApplicationWindow::builder().build();
        replacement.show_all();
        let replaced = bind_widget("space", &replacement).unwrap();
        assert_ne!(fresh, replaced);
        assert!(!visible(fresh));
        unsafe {
            replacement.destroy();
        }
        assert!(!visible(replaced));
        let pin = gtk::ApplicationWindow::builder().build();
        let label = format!("pin-{}", uuid::Uuid::new_v4());
        pin.show_all();
        let pin_token = bind_widget(&label, &pin).unwrap();
        pin.hide();
        assert!(!visible(pin_token));
        pin.show_all();
        let pin_remapped = bind_widget(&label, &pin).unwrap();
        assert_ne!(pin_token, pin_remapped);
        unsafe {
            pin.destroy();
        }
        assert!(!visible(pin_remapped));
        // Read through another X11 client after the publishing wrapper and
        // pixel buffer have been dropped; GTK must continue serving the data.
        let expected = vec![19, 41, 73, 255, 211, 101, 7, 255];
        publish_image(
            &display,
            image::RgbaImage::from_raw(2, 1, expected.clone()).unwrap(),
        )
        .unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_image());
            let _ = send.send(result);
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let actual = loop {
            if let Ok(result) = receive.try_recv() {
                break result.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "GTK clipboard did not answer the independent X11 client"
            );
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        worker.join().unwrap();
        assert_eq!((actual.width, actual.height), (2, 1));
        assert_eq!(actual.bytes.as_ref(), expected.as_slice());
    }
}
