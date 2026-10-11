// SPDX-License-Identifier: MPL-2.0
//! Host-owned GTK/X11 identities. Native objects stay on the UI thread;
//! workers receive only bounded scalar identities and lifecycle revisions.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    widget: usize,
    drawable: usize,
    xid: u32,
    display: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CaptureWitness {
    label: String,
    generation: u64,
    map_revision: u64,
    identity: Identity,
}

impl CaptureWitness {
    pub(crate) fn native_id(&self) -> u32 {
        self.identity.xid
    }
    pub(crate) fn display(&self) -> &str {
        &self.identity.display
    }
}

#[derive(Default)]
struct Owner {
    generation: u64,
    map_revision: u64,
    identity: Option<Identity>,
}

#[derive(Default)]
struct Registry {
    owners: HashMap<String, Owner>,
    failed: bool,
}

impl Registry {
    fn bind(&mut self, label: &str, identity: Identity) -> Result<(CaptureWitness, bool), String> {
        if self.failed
            || !matches!(label, "space" | "frozen-widget")
            || identity.xid == 0
            || identity.widget == 0
            || identity.drawable == 0
            || identity.display.is_empty()
            || identity.display.len() > 1024
        {
            return Err("截图窗口身份不可用".into());
        }
        let owner = self.owners.entry(label.into()).or_default();
        let changed = owner.identity.as_ref() != Some(&identity);
        if changed {
            // Signal handlers belong to the GTK widget and disappear with it.
            // Bound re-realizations of a surviving widget too, so repeatedly
            // replacing its drawable cannot accumulate unlimited callbacks.
            if owner.generation >= 4096 {
                self.failed = true;
                return Err("截图窗口服务需要重新启动".into());
            }
            owner.generation = owner
                .generation
                .checked_add(1)
                .ok_or("截图窗口世代已耗尽")?;
            owner.map_revision = 0;
            owner.identity = Some(identity.clone());
        }
        Ok((
            CaptureWitness {
                label: label.into(),
                generation: owner.generation,
                map_revision: owner.map_revision,
                identity,
            },
            changed,
        ))
    }

    fn current(&self, witness: &CaptureWitness) -> Result<(), String> {
        if !self.failed
            && self.owners.get(&witness.label).is_some_and(|owner| {
                owner.generation == witness.generation
                    && owner.map_revision == witness.map_revision
                    && owner.identity.as_ref() == Some(&witness.identity)
            })
        {
            Ok(())
        } else {
            Err("截图窗口已改变，请重新截图".into())
        }
    }

    fn revoke(&mut self, label: &str, generation: Option<u64>) {
        if let Some(owner) = self.owners.get_mut(label) {
            if generation.is_none_or(|generation| generation == owner.generation) {
                // Keep the generation counter: reusing an XID or object address
                // after destruction must never revive an earlier receipt.
                owner.identity = None;
            }
        }
    }

    fn mapped(&mut self, label: &str, generation: u64) {
        if let Some(owner) = self.owners.get_mut(label) {
            if owner.generation == generation && owner.identity.is_some() {
                if let Some(revision) = owner.map_revision.checked_add(1) {
                    owner.map_revision = revision;
                } else {
                    self.failed = true;
                }
            }
        }
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

pub(crate) fn current(witness: &CaptureWitness) -> Result<(), String> {
    registry()
        .lock()
        .map_err(|_| "截图窗口服务不可用")?
        .current(witness)
}

pub(crate) fn revoke_owner(label: &str) {
    if let Ok(mut registry) = registry().lock() {
        registry.revoke(label, None);
    }
}

/// Must be invoked inside run_on_main_thread. Do not use Tauri's raw display
/// getter: Tao 0.37.1 opens another XOpenDisplay(NULL) there, which need not
/// identify this GTK window's actual display. Use its own GdkWindow instead.
#[cfg(target_os = "linux")]
pub(crate) fn capture_witness(window: &tauri::WebviewWindow) -> Result<CaptureWitness, String> {
    use gtk::{
        glib::{translate::from_glib, types::Type},
        prelude::*,
    };
    let widget = window.gtk_window().map_err(|_| "无法读取截图窗口")?;
    // Realizing a hidden GTK window creates its owned drawable without showing
    // it. The frozen widget is constructed hidden during application setup.
    if widget.window().is_none() {
        widget.realize();
    }
    let drawable = widget.window().ok_or("截图窗口尚未创建")?;
    let display = drawable.display();
    let x11_window_type: Type = unsafe { from_glib(gdkx11_sys::gdk_x11_window_get_type()) };
    let x11_display_type: Type = unsafe { from_glib(gdkx11_sys::gdk_x11_display_get_type()) };
    if !drawable.type_().is_a(x11_window_type) || !display.type_().is_a(x11_display_type) {
        return Err("当前桌面尚未接入屏幕共享授权，请使用 X11 会话".into());
    }
    // GType was checked while both referenced GTK objects are alive on the UI
    // thread. No borrowed pointers or GTK objects leave this callback.
    let xid = unsafe { gdkx11_sys::gdk_x11_window_get_xid(drawable.as_ptr().cast()) };
    let identity = Identity {
        widget: widget.as_ptr() as usize,
        drawable: drawable.as_ptr() as usize,
        xid: u32::try_from(xid).map_err(|_| "截图窗口标识无效")?,
        display: display.name().into(),
    };
    let (witness, subscribe) = registry()
        .lock()
        .map_err(|_| "截图窗口服务不可用")?
        .bind(window.label(), identity)?;
    if subscribe {
        let label = witness.label.clone();
        let generation = witness.generation;
        widget.connect_unrealize(move |_| {
            if let Ok(mut registry) = registry().lock() {
                registry.revoke(&label, Some(generation));
            }
        });
        let label = witness.label.clone();
        widget.connect_map(move |_| {
            if let Ok(mut registry) = registry().lock() {
                registry.mapped(&label, generation);
            }
        });
    }
    Ok(witness)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(xid: u32) -> Identity {
        Identity {
            widget: 1,
            drawable: 2,
            xid,
            display: ":43.0".into(),
        }
    }
    #[test]
    fn destroyed_and_reused_native_identity_never_revives_a_receipt() {
        let mut registry = Registry::default();
        let (old, _) = registry.bind("space", identity(19)).unwrap();
        registry.revoke("space", None);
        let (new, subscribe) = registry.bind("space", identity(19)).unwrap();
        assert!(subscribe);
        assert!(registry.current(&old).is_err());
        registry.current(&new).unwrap();
        // A delayed unrealize from the former owner cannot revoke the new one.
        registry.revoke("space", Some(old.generation));
        registry.current(&new).unwrap();
    }
    #[test]
    fn remap_revokes_capture_even_when_native_identity_is_unchanged() {
        let mut registry = Registry::default();
        let (old, _) = registry.bind("space", identity(19)).unwrap();
        registry.mapped("space", old.generation);
        assert!(registry.current(&old).is_err());
        let (new, subscribe) = registry.bind("space", identity(19)).unwrap();
        assert!(!subscribe);
        registry.current(&new).unwrap();
        registry.mapped("space", old.generation + 1);
        registry.current(&new).unwrap();
    }
    #[test]
    fn replaced_display_or_drawable_and_counter_overflow_fail_closed() {
        let mut registry = Registry::default();
        let (old, _) = registry.bind("space", identity(19)).unwrap();
        let mut changed = identity(19);
        changed.display = ":44.0".into();
        let (new, _) = registry.bind("space", changed).unwrap();
        assert!(registry.current(&old).is_err());
        registry.owners.get_mut("space").unwrap().map_revision = u64::MAX;
        registry.mapped("space", new.generation);
        assert!(registry.current(&new).is_err());
        assert!(registry.bind("space", identity(19)).is_err());
        assert!(Registry::default()
            .bind("frontend-window", identity(19))
            .is_err());
    }
}
