// SPDX-License-Identifier: MPL-2.0
//! GNOME Wayland adapter. Portal owns input/capture authority; the compositor
//! witness attests target geometry; read-only AT-SPI attests keyboard focus.
use super::*;
use crate::{
    extension_region_a11y_linux::Accessibility, extension_region_desktop as desktop,
    extension_region_gnome::Witness, recording_pipewire_linux::Frames,
    recording_portal_linux::PortalSession,
};

pub(super) struct Target {
    state: Option<State>,
}
struct State {
    witness: Witness,
    bound: desktop::Target,
    portal: PortalSession,
    frames: Mutex<Frames>,
    accessibility: Accessibility,
    binding: RegionBinding,
    key_down: Mutex<Option<i32>>,
    pointer: Mutex<Option<[f64; 2]>>,
}
fn unavailable() -> RegionError {
    RegionError::new(
        "region_target_changed",
        "目标窗口已改变或失去焦点，区域操作已停止",
    )
}
fn map(error: desktop::Error) -> RegionError {
    match error {
        desktop::Error::Stopped => RegionError::new("region_authority_revoked", "区域操作已停止"),
        desktop::Error::DisplayChanged => {
            RegionError::new("region_display_changed", "显示器布局已改变")
        }
        desktop::Error::Boundary => {
            RegionError::new("region_window_boundary", "选区跨出了目标窗口")
        }
        desktop::Error::Occluded => {
            RegionError::new("region_target_obscured", "区域被其他窗口遮挡")
        }
        desktop::Error::FocusOutside => {
            RegionError::new("region_focus_outside", "输入焦点不在授权区域内")
        }
        _ => unavailable(),
    }
}
fn state(target: &Target) -> Result<&State> {
    target.state.as_ref().ok_or_else(unavailable)
}
fn snapshot(state: &State, points: &[[f64; 2]]) -> Result<desktop::Snapshot> {
    if !state.portal.live() {
        return Err(RegionError::new(
            "region_authority_revoked",
            "系统控制会话已结束",
        ));
    }
    state.witness.snapshot(points).map_err(|_| unavailable())
}
pub(super) fn dpi_scope() -> Result<()> {
    if std::env::var("XDG_SESSION_TYPE").ok().as_deref() != Some("wayland") {
        return Err(RegionError::new(
            "region_platform_unsupported",
            "区域操作需要受支持的 GNOME Wayland 桌面",
        ));
    }
    Ok(())
}
pub(super) fn bind(
    viewport: Viewport,
    binding: &RegionBinding,
    hooks: &Arc<dyn RegionOverlayHooks>,
) -> Result<Target> {
    if binding.coordinates.space != CaptureCoordinateSpace::PortalLogical {
        return Err(RegionError::new(
            "region_coordinates_unavailable",
            "请使用系统授权的桌面截图重新框选",
        ));
    }
    let witness =
        Witness::open().map_err(|e| RegionError::new("region_desktop_witness_unavailable", e))?;
    let r = viewport.native_rect(binding)?;
    let epsilon = 0.01;
    let points = [
        [r[0], r[1]],
        [r[2] - epsilon, r[1]],
        [r[0], r[3] - epsilon],
        [r[2] - epsilon, r[3] - epsilon],
        [(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.],
    ];
    let initial = witness.snapshot(&points).map_err(|_| unavailable())?;
    let bound = desktop::Target::bind(&initial, r).map_err(map)?;
    let mut heartbeat = Instant::now() - Duration::from_secs(1);
    let portal = PortalSession::open_control(Instant::now() + Duration::from_secs(90), &mut || {
        if hooks.validate_source(binding).is_err() {
            return false;
        }
        if heartbeat.elapsed() >= Duration::from_millis(500) {
            heartbeat = Instant::now();
            return witness.keep_alive() == Ok(true);
        }
        true
    })
    .map_err(|e| RegionError::new("region_authority_revoked", e))?;
    let c = binding.coordinates;
    if portal
        .logical_position
        .map(|(x, y)| (f64::from(x), f64::from(y)))
        != Some((c.origin_x, c.origin_y))
        || portal
            .logical_size
            .map(|(w, h)| (f64::from(w), f64::from(h)))
            != Some((c.width, c.height))
    {
        return Err(RegionError::new(
            "region_display_changed",
            "授权屏幕与原截图不一致",
        ));
    }
    let frames =
        Frames::connect(&portal).map_err(|e| RegionError::new("region_capture_failed", e))?;
    let accessibility =
        Accessibility::open().map_err(|e| RegionError::new("region_keyboard_focus_denied", e))?;
    let result = Target {
        state: Some(State {
            witness,
            bound,
            portal,
            frames: Mutex::new(frames),
            accessibility,
            binding: binding.clone(),
            key_down: Mutex::new(None),
            pointer: Mutex::new(None),
        }),
    };
    validate(&result, viewport)?;
    Ok(result)
}
pub(super) fn description(_: &Target) -> String {
    "GNOME Wayland 授权区域".into()
}
pub(super) fn topology(target: &Target) -> Result<Vec<crate::capture::MonitorInfo>> {
    let s = state(target)?;
    let snap = snapshot(s, &[])?;
    s.bound.validate(&snap, 0).map_err(map)?;
    let mut monitors = snap
        .monitors
        .into_iter()
        .map(|m| crate::capture::MonitorInfo {
            id: m.id,
            name: format!("GNOME {}", m.id),
            origin_x: m.frame.0[0],
            origin_y: m.frame.0[1],
            display_width: m.frame.0[2] as u32,
            display_height: m.frame.0[3] as u32,
            scale_factor: 1.,
            rotation: 0.,
            is_primary: m.id == 0,
            coordinate_space: crate::capture::CoordinateSpace::LogicalPoints,
        })
        .collect::<Vec<_>>();
    monitors.sort_by_key(|m| m.id);
    Ok(monitors)
}
pub(super) fn validate(target: &Target, _: Viewport) -> Result<()> {
    let s = state(target)?;
    s.bound.validate(&snapshot(s, &[])?, 0).map_err(map)
}
pub(super) fn neutral_input() -> Result<()> {
    Ok(())
} // snapshot validates real modifier state.
pub(super) fn stopped(target: &Target) -> bool {
    validate(
        target,
        Viewport {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    )
    .is_err()
}
fn check(session: &RegionSession, points: &[[f64; 2]]) -> Result<desktop::Snapshot> {
    session.ensure_live()?;
    session
        .hooks
        .validate_source(&session.info.binding)
        .map_err(|e| RegionError::new("region_authority_revoked", e))?;
    let s = state(&session.target)?;
    let snap = snapshot(s, points)?;
    s.bound.validate(&snap, points.len()).map_err(map)?;
    let allowed = 2
        | 16
        | if session.button_down.load(Ordering::SeqCst) {
            256
        } else {
            0
        };
    if snap.modifiers & !allowed != 0 {
        return Err(RegionError::new(
            "region_input_busy",
            "请松开鼠标和修饰键后重新发起",
        ));
    }
    Ok(snap)
}
fn pointer(session: &RegionSession, p: RegionPoint) -> Result<()> {
    let (x, y) = session.viewport.native_point(&session.info.binding, p)?;
    check(session, &[[x, y]])?;
    let s = state(&session.target)?;
    let c = s.binding.coordinates;
    s.portal
        .pointer(x - c.origin_x, y - c.origin_y, &mut || {
            !session.revoked.load(Ordering::SeqCst)
        })
        .map_err(|_| unavailable())?;
    let snap = check(session, &[[x, y]])?;
    verify_pointer(s, &snap, [x, y])?;
    *s.pointer.lock().map_err(|_| unavailable())? = Some([x, y]);
    Ok(())
}
fn verify_pointer(s: &State, snap: &desktop::Snapshot, expected: [f64; 2]) -> Result<()> {
    let p = snap.pointer.as_ref().ok_or_else(unavailable)?;
    if p.window != Some(s.bound.window.id)
        || (p.x - expected[0]).abs() > 1.
        || (p.y - expected[1]).abs() > 1.
    {
        return Err(RegionError::new(
            "region_cursor_clipped",
            "鼠标位置已被改变或受到系统限制",
        ));
    }
    Ok(())
}
fn button(session: &RegionSession, down: bool) -> Result<()> {
    let s = state(&session.target)?;
    if down {
        let point = s
            .pointer
            .lock()
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        let snap = check(session, &[point])?;
        verify_pointer(s, &snap, point)?;
        session.button_down.store(true, Ordering::SeqCst);
        s.portal
            .button(true, &mut || !session.revoked.load(Ordering::SeqCst))
            .map_err(|_| unavailable())
    } else {
        if session.button_down.swap(false, Ordering::SeqCst) {
            s.portal
                .button(false, &mut || true)
                .map_err(|_| unavailable())?
        }
        Ok(())
    }
}
pub(super) fn release(target: &Target, down: &AtomicBool) {
    if let Ok(s) = state(target) {
        if down.swap(false, Ordering::SeqCst) {
            let _ = s.portal.button(false, &mut || true);
        }
        if let Some(key) = s.key_down.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = s.portal.keysym(key, false, &mut || true);
        }
    }
}
pub(super) fn close(target: &Target) {
    if let Ok(s) = state(target) {
        s.portal.close();
        s.witness.close()
    }
}
fn keyboard(session: &RegionSession, editable: bool) -> Result<()> {
    check(session, &[])?;
    let s = state(&session.target)?;
    let focus = s
        .accessibility
        .focus(s.bound.window.pid as u32)
        .map_err(|e| RegionError::new("region_keyboard_focus_denied", e))?;
    if focus.secure {
        return Err(RegionError::new(
            "region_secure_input",
            "系统保护的输入框不能由区域操作填写",
        ));
    }
    if !focus.keyboard_supported || (editable && !focus.editable) {
        return Err(RegionError::new(
            "region_keyboard_focus_denied",
            "此应用无法提供可靠的输入框保护状态",
        ));
    }
    let snap = check(session, &[])?;
    s.bound
        .keyboard_identity(&snap, &focus.label_digest, focus.toplevel)
        .map_err(map)?;
    s.bound
        .focus_bounds(focus.toplevel, focus.bounds)
        .map_err(map)?;
    Ok(())
}
fn keysym(session: &RegionSession, key: i32, editable: bool) -> Result<()> {
    keyboard(session, editable)?;
    let s = state(&session.target)?;
    *s.key_down.lock().map_err(|_| unavailable())? = Some(key);
    let sent = s
        .portal
        .keysym(key, true, &mut || !session.revoked.load(Ordering::SeqCst))
        .map_err(|_| unavailable());
    // Always release a possibly-sent key even if the input acknowledgement was lost.
    let released = s
        .portal
        .keysym(key, false, &mut || true)
        .map_err(|_| unavailable());
    *s.key_down.lock().map_err(|_| unavailable())? = None;
    sent?;
    released?;
    Ok(())
}
pub(super) fn execute(session: &RegionSession, action: RegionAction) -> Result<()> {
    match action {
        RegionAction::Click { x, y } | RegionAction::DoubleClick { x, y } => {
            let count = if matches!(action, RegionAction::DoubleClick { .. }) {
                2
            } else {
                1
            };
            for i in 0..count {
                pointer(session, RegionPoint { x, y })?;
                button(session, true)?;
                button(session, false)?;
                if i + 1 < count {
                    std::thread::sleep(Duration::from_millis(45))
                }
            }
        }
        RegionAction::Drag { path } => {
            pointer(session, path[0])?;
            button(session, true)?;
            for segment in path.windows(2) {
                let dx = i64::from(segment[1].x) - i64::from(segment[0].x);
                let dy = i64::from(segment[1].y) - i64::from(segment[0].y);
                let n = ((dx.abs().max(dy.abs()) + 7) / 8).max(1);
                for i in 1..=n {
                    pointer(
                        session,
                        RegionPoint {
                            x: (i64::from(segment[0].x) + dx * i / n) as u32,
                            y: (i64::from(segment[0].y) + dy * i / n) as u32,
                        },
                    )?;
                    std::thread::sleep(Duration::from_millis(4));
                }
            }
            button(session, false)?;
        }
        RegionAction::Scroll { x, y, delta_y } => {
            pointer(session, RegionPoint { x, y })?;
            check(session, &[])?;
            state(&session.target)?
                .portal
                .scroll(
                    -delta_y.signum() * ((delta_y.abs() + 119) / 120),
                    &mut || !session.revoked.load(Ordering::SeqCst),
                )
                .map_err(|_| unavailable())?;
        }
        RegionAction::Type { text } => {
            for ch in text.chars() {
                let code = u32::from(ch);
                let key = if code <= 0xff {
                    code
                } else {
                    0x0100_0000 | code
                };
                keysym(session, key as i32, true)?;
            }
        }
        RegionAction::Key { key } => keysym(
            session,
            match key {
                RegionKey::Enter => 0xff0d,
                RegionKey::Backspace => 0xff08,
                RegionKey::Delete => 0xffff,
                RegionKey::ArrowLeft => 0xff51,
                RegionKey::ArrowRight => 0xff53,
                RegionKey::ArrowUp => 0xff52,
                RegionKey::ArrowDown => 0xff54,
                RegionKey::Home => 0xff50,
                RegionKey::End => 0xff57,
            },
            false,
        )?,
    }
    Ok(())
}
pub(super) fn capture(session: &RegionSession) -> Result<image::RgbaImage> {
    let s = state(&session.target)?;
    let frames = s.frames.lock().map_err(|_| unavailable())?;
    let expires = Instant::now() + Duration::from_secs(3);
    let barrier = loop {
        check(session, &[])?;
        if Instant::now() >= expires {
            eprintln!("[region-linux] capture_clock_not_ready");
            return Err(RegionError::new("region_capture_failed", "屏幕采集尚未建立时间轴"));
        }
        match frames.try_barrier().map_err(|e| {
            eprintln!("[region-linux] capture_clock_stream_ended");
            RegionError::new("region_capture_failed", e)
        })? {
            Some(barrier) => break barrier,
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    // Ask the compositor to repaint after this barrier. A static monitor may
    // otherwise stop producing frames; never relabel a cached buffer as fresh.
    s.witness.request_frame().map_err(|_| RegionError::new(
        "region_desktop_witness_unavailable", "请更新并启用 Mewu Desktop Witness 后重新发起",
    ))?;
    loop {
        check(session, &[])?;
        if Instant::now() >= expires {
            eprintln!("[region-linux] capture_fresh_frame_timeout");
            return Err(RegionError::new(
                "region_capture_failed",
                "未取得操作后的新屏幕帧",
            ));
        }
        let frame = frames
            .next_after(Duration::from_millis(40), Some(barrier))
            .map_err(|e| {
                eprintln!("[region-linux] capture_frame_stream_error");
                RegionError::new("region_capture_failed", e)
            })?;
        let Some(frame) = frame else { continue };
        if frame.width != s.binding.source_width || frame.height != s.binding.source_height {
            return Err(RegionError::new(
                "region_display_changed",
                "屏幕图像尺寸已改变",
            ));
        }
        let pixels = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .ok_or_else(unavailable)?;
        return Ok(image::imageops::crop_imm(
            &pixels,
            session.viewport.x as u32,
            session.viewport.y as u32,
            session.viewport.width,
            session.viewport.height,
        )
        .to_image());
    }
}
#[cfg(test)]
pub(super) fn fixture_target() -> Target {
    Target { state: None }
}
