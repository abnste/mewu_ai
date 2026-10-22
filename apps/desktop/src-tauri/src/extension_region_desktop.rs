// SPDX-License-Identifier: MPL-2.0
//! Typed compositor metadata policy. The native caller must additionally pin
//! the trusted bus owner/process lifetime and hold a separate Portal input
//! grant. Neither a plugin nor a JSON snapshot can create that authority.
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Rect(pub [i32; 4]);
impl Rect {
    fn valid(self) -> bool {
        self.0[2] > 0 && self.0[3] > 0 && self.0.iter().all(|v| v.unsigned_abs() <= 1_000_000)
    }
    pub(crate) fn bounds(self) -> [f64; 4] {
        let [x, y, w, h] = self.0;
        [
            f64::from(x),
            f64::from(y),
            f64::from(x) + f64::from(w),
            f64::from(y) + f64::from(h),
        ]
    }
    fn contains(self, r: [f64; 4]) -> bool {
        let b = self.bounds();
        r.iter().all(|v| v.is_finite())
            && r[0] < r[2]
            && r[1] < r[3]
            && r[0] >= b[0]
            && r[1] >= b[1]
            && r[2] <= b[2]
            && r[3] <= b[3]
    }
    fn intersects(self, r: [f64; 4]) -> bool {
        let b = self.bounds();
        b[0] < r[2] && b[2] > r[0] && b[1] < r[3] && b[3] > r[1]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Monitor {
    pub(crate) id: u32,
    pub(crate) frame: Rect,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Window {
    pub(crate) id: u32,
    pub(crate) pid: i32,
    pub(crate) frame: Rect,
    pub(crate) buffer: Rect,
    pub(crate) override_redirect: bool,
    pub(crate) label_digest: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub(crate) protocol: u32,
    pub(crate) epoch: String,
    pub(crate) stopped: bool,
    pub(crate) blocked: bool,
    pub(crate) modifiers: u32,
    pub(crate) pointer: Option<Pointer>,
    pub(crate) workspace: u32,
    pub(crate) focus: Option<u32>,
    pub(crate) monitors: Vec<Monitor>,
    pub(crate) windows: Vec<Window>,
    pub(crate) hits: Vec<Option<u32>>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pointer {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) window: Option<u32>,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Invalid,
    Stopped,
    DisplayChanged,
    TargetChanged,
    Boundary,
    Occluded,
    FocusOutside,
}
impl Snapshot {
    pub(crate) fn valid(&self, hit_count: usize) -> Result<(), Error> {
        let unique = |ids: Vec<u32>| {
            ids.iter().all(|id| *id > 0) && ids.iter().collect::<HashSet<_>>().len() == ids.len()
        };
        if self.protocol != 1
            || uuid::Uuid::parse_str(&self.epoch).is_err()
            || self.monitors.is_empty()
            || self.monitors.len() > 32
            || self.windows.len() > 256
            || self.hits.len() != hit_count
            || hit_count > 16
            || self.pointer.as_ref().is_some_and(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || p.x.abs() > 1_000_000.
                    || p.y.abs() > 1_000_000.
            })
            || !self.monitors.iter().all(|m| m.frame.valid())
            || self
                .monitors
                .iter()
                .map(|m| m.id)
                .collect::<HashSet<_>>()
                .len()
                != self.monitors.len()
            || !unique(self.windows.iter().map(|w| w.id).collect())
            || !self.windows.iter().all(|w| {
                w.pid >= 0
                    && w.frame.valid()
                    && w.buffer.valid()
                    && w.label_digest
                        .as_ref()
                        .is_none_or(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            })
        {
            return Err(Error::Invalid);
        }
        if self.stopped {
            return Err(Error::Stopped);
        }
        if self.blocked {
            return Err(Error::TargetChanged);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Target {
    epoch: String,
    workspace: u32,
    pub(crate) window: Window,
    pub(crate) monitors: Vec<Monitor>,
    pub(crate) region: [f64; 4],
}
impl Target {
    pub(crate) fn bind(snapshot: &Snapshot, region: [f64; 4]) -> Result<Self, Error> {
        snapshot.valid(5)?;
        if !snapshot.monitors.iter().any(|m| m.frame.contains(region)) {
            return Err(Error::DisplayChanged);
        }
        let window = snapshot
            .windows
            .iter()
            .find(|w| Some(w.id) == snapshot.focus)
            .ok_or(Error::TargetChanged)?;
        if window.pid <= 0 || window.override_redirect || !window.frame.contains(region) {
            return Err(Error::Boundary);
        }
        let target = Self {
            epoch: snapshot.epoch.clone(),
            workspace: snapshot.workspace,
            window: window.clone(),
            monitors: snapshot.monitors.clone(),
            region,
        };
        target.validate(snapshot, 5)?;
        Ok(target)
    }
    pub(crate) fn validate(&self, snapshot: &Snapshot, hit_count: usize) -> Result<(), Error> {
        snapshot.valid(hit_count)?;
        if snapshot.epoch != self.epoch || snapshot.workspace != self.workspace {
            return Err(Error::TargetChanged);
        }
        if snapshot.monitors != self.monitors {
            return Err(Error::DisplayChanged);
        }
        let index = snapshot
            .windows
            .iter()
            .position(|w| w.id == self.window.id)
            .ok_or(Error::TargetChanged)?;
        let current = &snapshot.windows[index];
        if snapshot.focus != Some(self.window.id)
            || current.id != self.window.id
            || current.pid != self.window.pid
            || current.frame != self.window.frame
            || current.buffer != self.window.buffer
            || current.override_redirect != self.window.override_redirect
        {
            return Err(Error::TargetChanged);
        }
        // X11 override-redirect stacking is explicitly not guaranteed by
        // Mutter's sorting API. Never discard a possible occluder as 'below'.
        if snapshot.windows.iter().enumerate().any(|(i, w)| {
            w.id != self.window.id
                && (i > index || w.override_redirect)
                && (w.frame.intersects(self.region) || w.buffer.intersects(self.region))
        }) {
            return Err(Error::Occluded);
        }
        if snapshot.hits.iter().any(|id| *id != Some(self.window.id)) {
            return Err(Error::Occluded);
        }
        Ok(())
    }
    /// AT-SPI window-relative geometry is accepted only when its toplevel size
    /// exactly identifies the compositor frame or buffer. Never guess a title
    /// bar offset or treat unknown Wayland screen coordinates as global.
    pub(crate) fn focus_bounds(&self, toplevel: Rect, focused: Rect) -> Result<[f64; 4], Error> {
        if !toplevel.valid()
            || toplevel.0[0..2] != [0, 0]
            || !focused.valid()
            || !toplevel.contains(focused.bounds())
        {
            return Err(Error::FocusOutside);
        }
        let mut origins = [self.window.frame, self.window.buffer]
            .into_iter()
            .filter(|r| r.0[2..4] == toplevel.0[2..4])
            .map(|r| [r.0[0], r.0[1]])
            .collect::<Vec<_>>();
        origins.sort();
        origins.dedup();
        if origins.len() != 1 {
            return Err(Error::FocusOutside);
        }
        let [x, y] = origins[0];
        let b = focused.bounds();
        let result = [
            b[0] + f64::from(x),
            b[1] + f64::from(y),
            b[2] + f64::from(x),
            b[3] + f64::from(y),
        ];
        if result[0] < self.region[0]
            || result[1] < self.region[1]
            || result[2] > self.region[2]
            || result[3] > self.region[3]
        {
            return Err(Error::FocusOutside);
        }
        Ok(result)
    }
    pub(crate) fn keyboard_identity(
        &self,
        snapshot: &Snapshot,
        label_digest: &str,
        toplevel: Rect,
    ) -> Result<(), Error> {
        self.validate(snapshot, 0)?;
        let current = snapshot
            .windows
            .iter()
            .find(|w| w.id == self.window.id)
            .ok_or(Error::FocusOutside)?;
        if current.label_digest.as_deref() != Some(label_digest) {
            return Err(Error::FocusOutside);
        }
        let candidates = snapshot
            .windows
            .iter()
            .filter(|w| {
                w.pid == current.pid
                    && w.label_digest == current.label_digest
                    && [w.frame, w.buffer]
                        .iter()
                        .any(|r| r.0[2..4] == toplevel.0[2..4])
            })
            .count();
        if candidates != 1 {
            return Err(Error::FocusOutside);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        serde_json::from_value(serde_json::json!({
        "protocol":1,"epoch":"771def2d-bf01-4c27-8402-32b3c8db2a76","stopped":false,"blocked":false,"modifiers":0,"workspace":0,"focus":42,
        "monitors":[{"id":0,"frame":[-1280,0,1280,800]}],
        "windows":[{"id":42,"pid":1234,"frame":[-1200,100,800,600],"buffer":[-1210,90,820,620],"overrideRedirect":false}],"hits":[42,42,42,42,42]
    })).unwrap()
    }
    fn region() -> [f64; 4] {
        [-1150., 150., -800., 500.]
    }
    #[test]
    fn focused_window_binding_rejects_cross_window_unknown_hit_and_opaque_shell() {
        let s = snapshot();
        assert!(Target::bind(&s, region()).is_ok());
        assert_eq!(
            Target::bind(&s, [-1250., 150., -800., 500.]).unwrap_err(),
            Error::Boundary
        );
        let mut changed = s.clone();
        changed.hits[2] = None;
        assert_eq!(
            Target::bind(&changed, region()).unwrap_err(),
            Error::Occluded
        );
        changed = s.clone();
        changed.blocked = true;
        assert_eq!(
            Target::bind(&changed, region()).unwrap_err(),
            Error::TargetChanged
        );
    }
    #[test]
    fn revoked_or_replaced_desktop_window_process_geometry_and_monitor_never_rebind() {
        let s = snapshot();
        let t = Target::bind(&s, region()).unwrap();
        for field in 0..8 {
            let mut c = s.clone();
            match field {
                0 => c.stopped = true,
                1 => c.epoch = uuid::Uuid::new_v4().to_string(),
                2 => c.workspace += 1,
                3 => c.monitors[0].frame.0[2] += 1,
                4 => c.windows[0].pid += 1,
                5 => c.windows[0].frame.0[0] += 1,
                6 => c.windows[0].buffer.0[0] += 1,
                _ => c.focus = None,
            };
            assert!(t.validate(&c, 5).is_err(), "change {field}");
        }
    }
    #[test]
    fn lower_normal_windows_are_allowed_but_uncertain_redirect_popups_are_never_ignored() {
        let s = snapshot();
        let t = Target::bind(&s, region()).unwrap();
        let mut c = s.clone();
        let mut other = c.windows[0].clone();
        other.id = 77;
        other.pid = 2345;
        c.windows.insert(0, other);
        assert!(t.validate(&c, 5).is_ok());
        c.windows[0].override_redirect = true;
        assert_eq!(t.validate(&c, 5), Err(Error::Occluded));
        c.windows[0].override_redirect = false;
        c.windows.swap(0, 1);
        assert_eq!(t.validate(&c, 5), Err(Error::Occluded));
    }
    #[test]
    fn keyboard_focus_needs_exact_window_relative_mapping_and_complete_region_containment() {
        let t = Target::bind(&snapshot(), region()).unwrap();
        assert_eq!(
            t.focus_bounds(Rect([0, 0, 800, 600]), Rect([60, 60, 100, 30]))
                .unwrap(),
            [-1140., 160., -1040., 190.]
        );
        assert_eq!(
            t.focus_bounds(Rect([0, 0, 820, 620]), Rect([70, 70, 100, 30]))
                .unwrap(),
            [-1140., 160., -1040., 190.]
        );
        for (top, node) in [
            (Rect([0, 0, 801, 600]), Rect([60, 60, 100, 30])),
            (Rect([1, 0, 800, 600]), Rect([60, 60, 100, 30])),
            (Rect([0, 0, 800, 600]), Rect([0, 0, 100, 30])),
        ] {
            assert_eq!(t.focus_bounds(top, node), Err(Error::FocusOutside));
        }
        let mut ambiguous = snapshot();
        ambiguous.windows[0].buffer = Rect([-1210, 90, 800, 600]);
        let ambiguous = Target::bind(&ambiguous, region()).unwrap();
        assert_eq!(
            ambiguous.focus_bounds(Rect([0, 0, 800, 600]), Rect([60, 60, 100, 30])),
            Err(Error::FocusOutside)
        );
    }
    #[test]
    fn keyboard_window_matching_rejects_stale_or_ambiguous_accessibility_identity_without_stopping_legitimate_title_changes(
    ) {
        let mut s = snapshot();
        let first = "a".repeat(64);
        let second = "b".repeat(64);
        s.windows[0].label_digest = Some(first.clone());
        let t = Target::bind(&s, region()).unwrap();
        s.hits.clear();
        assert!(t
            .keyboard_identity(&s, &first, Rect([0, 0, 800, 600]))
            .is_ok());
        let mut other = s.windows[0].clone();
        other.id = 77;
        other.frame = Rect([100, 0, 800, 600]);
        other.buffer = Rect([90, -10, 820, 620]);
        s.windows.insert(0, other);
        assert_eq!(
            t.keyboard_identity(&s, &first, Rect([0, 0, 800, 600])),
            Err(Error::FocusOutside)
        );
        s.windows[1].label_digest = Some(second.clone());
        assert!(t.validate(&s, 0).is_ok());
        assert_eq!(
            t.keyboard_identity(&s, &first, Rect([0, 0, 800, 600])),
            Err(Error::FocusOutside)
        );
        assert!(t
            .keyboard_identity(&s, &second, Rect([0, 0, 800, 600]))
            .is_ok());
    }
}
