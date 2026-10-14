// SPDX-License-Identifier: MPL-2.0
//! X11 hiding proof on the capture connection, not a hide API acknowledgement.
//! StructureNotify catches a map/unmap pair that a final map_state misses.

use std::time::{Duration, Instant};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{ChangeWindowAttributesAux, ConnectionExt as _, EventMask, MapState},
        Event,
    },
};

pub(super) struct HiddenWindows<'a, C: Connection> {
    connection: &'a C,
    windows: Vec<u32>,
    armed: bool,
}

pub(super) struct HiddenReceipt {
    windows: Vec<u32>,
}

impl HiddenReceipt {
    pub(super) fn check_on(
        &self,
        connection: &impl Connection,
        allowed: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        HiddenWindows {
            connection,
            windows: self.windows.clone(),
            armed: true,
        }
        .check(allowed)
    }
}

impl<'a, C: Connection> HiddenWindows<'a, C> {
    pub(super) fn subscribe(connection: &'a C, windows: &[u32]) -> Result<Self, String> {
        if windows.is_empty()
            || windows.len() > 2
            || windows.contains(&0)
            || (windows.len() == 2 && windows[0] == windows[1])
        {
            return Err("截图窗口集合无效".into());
        }
        for &window in windows {
            connection
                .change_window_attributes(
                    window,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
                )
                .map_err(|_| "无法订阅截图窗口状态")?
                .check()
                .map_err(|_| "截图窗口已不可用")?;
        }
        Ok(Self {
            connection,
            windows: windows.to_vec(),
            armed: false,
        })
    }

    // Independent pins have an eight-window quota. Keep the ordinary capture
    // and recording subscribe() admission above limited to its original two.
    pub(super) fn subscribe_pins(connection: &'a C, windows: &[u32]) -> Result<Self, String> {
        let own = Self::resume_subscribed_pins(connection, windows)?;
        for &window in windows {
            connection
                .change_window_attributes(
                    window,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
                )
                .map_err(|_| "无法订阅贴图窗口状态")?
                .check()
                .map_err(|_| "贴图窗口已不可用")?;
        }
        Ok(own)
    }

    // Called exactly once after subscribe_pins() on this same connection and
    // synchronous UI hide. Do not resubscribe or drain its notification queue.
    // wait_hidden() arms it; all later checks use its existing HiddenReceipt.
    pub(super) fn resume_subscribed_pins(
        connection: &'a C,
        windows: &[u32],
    ) -> Result<Self, String> {
        if !pin_windows_valid(windows) {
            return Err("贴图窗口集合无效".into());
        }
        Ok(Self {
            connection,
            windows: windows.to_vec(),
            armed: false,
        })
    }

    fn events(&self) -> Result<(), String> {
        // A hostile or busy display must not create an unbounded drain loop.
        for _ in 0..4096 {
            let Some(event) = self
                .connection
                .poll_for_event()
                .map_err(|_| "无法读取截图窗口状态")?
            else {
                return Ok(());
            };
            match event {
                Event::DestroyNotify(event) if self.windows.contains(&event.window) => {
                    return Err("截图窗口已销毁，请重新截图".into())
                }
                Event::MapNotify(event) if self.armed && self.windows.contains(&event.window) => {
                    return Err("截图窗口在捕获期间重新显示，请重新截图".into())
                }
                // Reparenting can change the drawable's ownership/visibility
                // contract. Reject rather than infer it from title or PID.
                Event::ReparentNotify(event) if self.windows.contains(&event.window) => {
                    return Err("截图窗口结构已改变，请重新截图".into())
                }
                _ => {}
            }
        }
        Err("截图窗口事件过多，请重新截图".into())
    }

    fn unmapped(&self) -> Result<bool, String> {
        let mut all_hidden = true;
        for &window in &self.windows {
            let attributes = self
                .connection
                .get_window_attributes(window)
                .map_err(|_| "无法检查截图窗口")?
                .reply()
                .map_err(|_| "截图窗口已不可用")?;
            // UNVIEWABLE means mapped under an unmapped ancestor. It does not
            // prove that the application's own drawable has been unmapped.
            all_hidden &= attributes.map_state == MapState::UNMAPPED;
        }
        // Each attributes reply is a server round trip; drain preceding events
        // after the last reply, including a transient map followed by unmap.
        self.events()?;
        Ok(all_hidden)
    }

    pub(super) fn wait_hidden(
        &mut self,
        allowed: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        let until = Instant::now() + Duration::from_millis(250);
        loop {
            allowed()?;
            let hidden = self.unmapped()?;
            allowed()?;
            if hidden {
                self.armed = true;
                return self.check(allowed);
            }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("截图窗口尚未隐藏，请重新截图".into());
            }
            // Only rate-limits requests. The GetWindowAttributes reply is the
            // evidence; elapsed sleeping time is never accepted as proof.
            std::thread::sleep(remaining.min(Duration::from_millis(5)));
        }
    }

    pub(super) fn check(&self, allowed: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
        allowed()?;
        if !self.armed || !self.unmapped()? {
            return Err("截图窗口在捕获期间重新显示，请重新截图".into());
        }
        allowed()
    }

    pub(super) fn receipt(&self) -> Result<HiddenReceipt, String> {
        if !self.armed {
            return Err("截图窗口未完成隐藏检查".into());
        }
        Ok(HiddenReceipt {
            windows: self.windows.clone(),
        })
    }
}

fn pin_windows_valid(windows: &[u32]) -> bool {
    !windows.is_empty()
        && windows.len() <= 8
        && !windows.contains(&0)
        && windows.iter().enumerate().all(|(i, window)| !windows[..i].contains(window))
}

#[cfg(test)]
mod pin_admission_tests {
    use super::*;

    #[test]
    fn independent_pin_quota_rejects_empty_zero_duplicate_and_ninth_drawable() {
        assert!(pin_windows_valid(&[7, 14, 21, 28, 35, 42, 49, 56]));
        assert!(!pin_windows_valid(&[]));
        assert!(!pin_windows_valid(&[7, 14, 0, 28]));
        assert!(!pin_windows_valid(&[7, 14, 21, 7]));
        assert!(!pin_windows_valid(&[7, 14, 21, 28, 35, 42, 49, 56, 63]));
    }
}

// Run separately under a real Xvfb server, after the ordinary source gate:
// xvfb-run -a cargo test --locked --features linux-x11-tests x11_visibility_native
// The feature has no production behavior. Absence of a server fails these tests.
#[cfg(all(test, target_os = "linux", feature = "linux-x11-tests"))]
mod x11_visibility_native {
    use super::*;
    use x11rb::{
        protocol::xproto::{CreateWindowAux, WindowClass},
        COPY_DEPTH_FROM_PARENT,
    };
    fn window(connection: &impl Connection, parent: u32) -> u32 {
        let window = connection.generate_id().unwrap();
        connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                parent,
                0,
                0,
                32,
                32,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        window
    }
    #[test]
    fn mapped_window_is_rejected_until_server_reports_unmapped() {
        let (connection, screen) =
            x11rb::connect(None).expect("a real X11 test display is required");
        let window = window(&connection, connection.setup().roots[screen].root);
        connection.map_window(window).unwrap().check().unwrap();
        let mut guard = HiddenWindows::subscribe(&connection, &[window]).unwrap();
        assert!(guard.wait_hidden(&|| Ok(())).is_err());
        connection.unmap_window(window).unwrap().check().unwrap();
        guard.wait_hidden(&|| Ok(())).unwrap();
        guard.check(&|| Ok(())).unwrap();
        assert!(guard.check(&|| Err("canceled".into())).is_err());
    }
    #[test]
    fn transient_remap_then_unmap_is_rejected_even_with_hidden_final_state() {
        let (connection, screen) =
            x11rb::connect(None).expect("a real X11 test display is required");
        let window = window(&connection, connection.setup().roots[screen].root);
        let mut guard = HiddenWindows::subscribe(&connection, &[window]).unwrap();
        guard.wait_hidden(&|| Ok(())).unwrap();
        connection.map_window(window).unwrap().check().unwrap();
        connection.unmap_window(window).unwrap().check().unwrap();
        assert_eq!(
            connection
                .get_window_attributes(window)
                .unwrap()
                .reply()
                .unwrap()
                .map_state,
            MapState::UNMAPPED
        );
        assert!(guard.check(&|| Ok(())).is_err());
    }

    #[test]
    fn one_pin_subscription_preserves_all_eight_windows_remap_events() {
        let (connection, screen) =
            x11rb::connect(None).expect("a real X11 test display is required");
        let windows: Vec<_> = (0..8)
            .map(|_| window(&connection, connection.setup().roots[screen].root))
            .collect();
        // The original capture/recording admission cannot accept this group.
        assert!(HiddenWindows::subscribe(&connection, &windows).is_err());
        let initial = HiddenWindows::subscribe_pins(&connection, &windows).unwrap();
        drop(initial);
        let mut group = HiddenWindows::resume_subscribed_pins(&connection, &windows).unwrap();
        group.wait_hidden(&|| Ok(())).unwrap();
        let receipt = group.receipt().unwrap();
        drop(group);
        // A notification for another group member cannot be swallowed by a
        // per-pin observer before the final server state returns to UNMAPPED.
        connection.map_window(windows[7]).unwrap().check().unwrap();
        connection.unmap_window(windows[7]).unwrap().check().unwrap();
        assert_eq!(connection.get_window_attributes(windows[7]).unwrap().reply().unwrap().map_state, MapState::UNMAPPED);
        assert!(receipt.check_on(&connection, &|| Ok(())).is_err());
        for window in windows {
            connection.destroy_window(window).unwrap().check().unwrap();
        }
    }
    #[test]
    fn destroyed_window_and_mapped_child_of_hidden_parent_are_rejected() {
        let (connection, screen) =
            x11rb::connect(None).expect("a real X11 test display is required");
        let parent = window(&connection, connection.setup().roots[screen].root);
        let child = window(&connection, parent);
        connection.map_window(child).unwrap().check().unwrap();
        let mut child_guard = HiddenWindows::subscribe(&connection, &[child]).unwrap();
        assert!(child_guard.wait_hidden(&|| Ok(())).is_err());
        let mut parent_guard = HiddenWindows::subscribe(&connection, &[parent]).unwrap();
        parent_guard.wait_hidden(&|| Ok(())).unwrap();
        connection.destroy_window(parent).unwrap().check().unwrap();
        // Reusing the exact same resource ID must not revive an earlier proof.
        connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                parent,
                connection.setup().roots[screen].root,
                0,
                0,
                32,
                32,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            connection
                .get_window_attributes(parent)
                .unwrap()
                .reply()
                .unwrap()
                .map_state,
            MapState::UNMAPPED
        );
        assert!(parent_guard.check(&|| Ok(())).is_err());
    }
}
