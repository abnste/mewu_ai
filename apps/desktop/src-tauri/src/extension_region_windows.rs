// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0

//! Windows native region backend. SendInput respects UIPI; failures are errors,
//! never a reason to request elevation. The input mutex serializes cancellation
//! with every injection and cleanup, including mouse-down ownership.
//! Microsoft Learn: SendInput, GetGUIThreadInfo, GetAncestor, GetProcessTimes,
//! SetThreadDpiAwarenessContext, MapWindowPoints and GetPhysicalCursorPos.

use super::*;
use std::{ffi::c_void, marker::PhantomData, rc::Rc};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, SetLastError, FILETIME, HWND, LPARAM, POINT, RECT},
    Graphics::{
        Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
        Gdi::ClientToScreen,
    },
    System::Threading::{
        GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

static INPUT_LOCK: Mutex<()> = Mutex::new(());

#[link(name = "user32")]
extern "system" {
    fn SetThreadDpiAwarenessContext(context: *mut c_void) -> *mut c_void;
}

pub(super) struct DpiScope {
    previous: *mut c_void,
    _thread: PhantomData<Rc<()>>,
}
pub(super) fn dpi_scope() -> Result<DpiScope> {
    let previous = unsafe { SetThreadDpiAwarenessContext(-4isize as *mut c_void) };
    if previous.is_null() {
        return Err(error(
            "region_coordinates_unavailable",
            "无法确认物理屏幕坐标",
        ));
    }
    Ok(DpiScope {
        previous,
        _thread: PhantomData,
    })
}
impl Drop for DpiScope {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.previous);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    handle: usize,
    process: u32,
    thread: u32,
    created: u64,
    class: String,
    client: [i32; 4],
    bounds: [i32; 4],
}

#[cfg(test)]
pub(super) fn fixture_target() -> Target {
    Target {
        handle: 0,
        process: 0,
        thread: 0,
        created: 0,
        class: String::new(),
        client: [0; 4],
        bounds: [0; 4],
    }
}

fn error(code: &'static str, message: &str) -> RegionError {
    RegionError::new(code, message)
}
fn unavailable() -> RegionError {
    error(
        "region_target_changed",
        "目标窗口已改变或失去焦点，区域操作已停止",
    )
}

fn process_creation(process: u32) -> Result<u64> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process);
        if handle.is_null() {
            return Err(unavailable());
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        let created = u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime);
        if ok && created != 0 {
            Ok(created)
        } else {
            Err(unavailable())
        }
    }
}

fn root(handle: HWND) -> HWND {
    unsafe { GetAncestor(handle, GA_ROOT) }
}

fn client_rect(handle: HWND) -> Result<[i32; 4]> {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetClientRect(handle, &mut rect) } == 0 {
        return Err(unavailable());
    }
    let mut left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let mut right = POINT {
        x: rect.right,
        y: rect.bottom,
    };
    if unsafe { ClientToScreen(handle, &mut left) } == 0
        || unsafe { ClientToScreen(handle, &mut right) } == 0
        || left.x >= right.x
        || left.y >= right.y
    {
        return Err(unavailable());
    }
    Ok([left.x, left.y, right.x, right.y])
}

fn window_rect(handle: HWND) -> Result<[i32; 4]> {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(handle, &mut rect) } == 0
        || rect.left >= rect.right || rect.top >= rect.bottom {
        return Err(unavailable());
    }
    Ok([rect.left, rect.top, rect.right, rect.bottom])
}

fn read_target(handle: HWND) -> Result<Target> {
    if handle.is_null()
        || unsafe { IsWindow(handle) } == 0
        || unsafe { IsWindowVisible(handle) } == 0
        || unsafe { IsIconic(handle) } != 0
        || unsafe { IsWindowEnabled(handle) } == 0
        || root(handle) != handle
    {
        return Err(unavailable());
    }
    let mut cloaked = 0u32;
    if unsafe {
        DwmGetWindowAttribute(
            handle,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast(),
            4,
        )
    } < 0
        || cloaked != 0
    {
        return Err(unavailable());
    }
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(handle, &mut process) };
    if process == 0 || thread == 0 || process == unsafe { GetCurrentProcessId() } {
        return Err(unavailable());
    }
    let mut class = [0u16; 256];
    let length = unsafe { GetClassNameW(handle, class.as_mut_ptr(), class.len() as i32) };
    if length <= 0 || length as usize >= class.len() - 1 {
        return Err(unavailable());
    }
    let class = String::from_utf16_lossy(&class[..length as usize]);
    if matches!(
        class.as_str(),
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "DV2ControlHost"
            | "XamlExplorerHostIslandWindow"
    ) {
        return Err(error("region_system_target_denied", "请选择应用内容区域"));
    }
    let target = Target {
        handle: handle as usize,
        process,
        thread,
        created: process_creation(process)?,
        class,
        client: client_rect(handle)?,
        bounds: window_rect(handle)?,
    };
    // Handle reuse/exit while collecting must not produce a mixed identity.
    let mut final_process = 0;
    if unsafe { GetWindowThreadProcessId(handle, &mut final_process) } != target.thread
        || final_process != target.process
        || unsafe { IsWindow(handle) } == 0
    {
        return Err(unavailable());
    }
    Ok(target)
}

pub(super) fn foreground_identity() -> Option<Target> {
    read_target(root(unsafe { GetForegroundWindow() })).ok()
}

pub(super) fn foreground_matches(expected: &Target) -> bool {
    foreground_identity().as_ref() == Some(expected)
}

fn contains(client: [i32; 4], viewport: Viewport) -> bool {
    viewport.x >= client[0]
        && viewport.y >= client[1]
        && i64::from(viewport.x) + i64::from(viewport.width) <= i64::from(client[2])
        && i64::from(viewport.y) + i64::from(viewport.height) <= i64::from(client[3])
}

fn mouse_passthrough_overlay(handle: HWND) -> bool {
    // Zero is also a valid extended style, so clear the thread-local error before
    // distinguishing an unreadable window from a successful zero-style read.
    let style = unsafe {
        SetLastError(0);
        let style = GetWindowLongPtrW(handle, GWL_EXSTYLE);
        if style == 0 && GetLastError() != 0 {
            return false;
        }
        style as u32
    };
    // Windows defines this combination as passing mouse events to windows
    // underneath. WS_EX_TRANSPARENT alone describes paint ordering, not input.
    // No process/class whitelist or assumption about the overlay's pixels.
    // https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#layered-windows
    style & (WS_EX_LAYERED | WS_EX_TRANSPARENT) == (WS_EX_LAYERED | WS_EX_TRANSPARENT)
}

/// Reject foreign input-obstructing top-level windows above the target that
/// overlap the grant. System-defined layered mouse-pass-through overlays do not
/// obstruct input, but their visible pixels may remain in the observed ROI.
/// Unknown bounds/styles stay blocked; every action also checks its real hit target.
fn input_unobstructed(target: &Target, viewport: Viewport) -> Result<()> {
    struct Pass {
        target: usize,
        viewport: Viewport,
        found: bool,
        blocked: bool,
        count: u32,
    }
    unsafe extern "system" fn visit(handle: HWND, param: LPARAM) -> i32 {
        let pass = unsafe { &mut *(param as *mut Pass) };
        pass.count += 1;
        if pass.count > 4096 {
            pass.blocked = true;
            return 0;
        }
        if handle as usize == pass.target {
            pass.found = true;
            return 0;
        }
        if unsafe { IsWindowVisible(handle) } == 0 || unsafe { IsIconic(handle) } != 0 {
            return 1;
        }
        let mut cloaked = 0u32;
        if unsafe {
            DwmGetWindowAttribute(
                handle,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast(),
                4,
            )
        } >= 0
            && cloaked != 0
        {
            return 1;
        }
        let mut r: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetWindowRect(handle, &mut r) } == 0 {
            pass.blocked = true;
            return 0;
        }
        let v = pass.viewport;
        if r.left < v.x.saturating_add_unsigned(v.width)
            && r.right > v.x
            && r.top < v.y.saturating_add_unsigned(v.height)
            && r.bottom > v.y
        {
            if mouse_passthrough_overlay(handle) {
                return 1;
            }
            pass.blocked = true;
            return 0;
        }
        1
    }
    let mut pass = Pass {
        target: target.handle,
        viewport,
        found: false,
        blocked: false,
        count: 0,
    };
    unsafe {
        EnumWindows(Some(visit), (&mut pass as *mut Pass) as LPARAM);
    }
    if !pass.found || pass.blocked {
        return Err(error("region_target_obscured", "操作区域被其他窗口遮挡"));
    }
    Ok(())
}

pub(super) fn bind(viewport: Viewport) -> Result<Target> {
    let center = POINT {
        x: viewport.x + (viewport.width / 2) as i32,
        y: viewport.y + (viewport.height / 2) as i32,
    };
    let target = read_target(root(unsafe { WindowFromPoint(center) }))?;
    // Window snapping includes the title bar and frame. Those are part of the
    // same authorized HWND; do not reject them as if they were another app.
    // Individual input hit tests and keyboard focus containment still apply.
    if !contains(target.bounds, viewport) {
        return Err(error(
            "region_window_boundary",
            "选区必须完整位于同一个应用窗口内",
        ));
    }
    validate(&target, viewport)?;
    Ok(target)
}

pub(super) fn description(target: &Target) -> String {
    target.class.clone()
}

pub(super) fn validate(target: &Target, viewport: Viewport) -> Result<()> {
    let handle = target.handle as HWND;
    if read_target(handle)? != *target
        || !contains(target.bounds, viewport)
        || root(unsafe { GetForegroundWindow() }) != handle
    {
        return Err(unavailable());
    }
    // Re-observation must still hit the same actual application, including
    // after a pass-through overlay changed between previous operations.
    let center = POINT {
        x: viewport.x + (viewport.width / 2) as i32,
        y: viewport.y + (viewport.height / 2) as i32,
    };
    if root(unsafe { WindowFromPoint(center) }) != handle {
        return Err(unavailable());
    }
    input_unobstructed(target, viewport)
}

fn modifiers_clear() -> Result<()> {
    for key in [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN] {
        if unsafe { GetAsyncKeyState(key as i32) } < 0 {
            return Err(error("region_input_busy", "请先松开键盘修饰键"));
        }
    }
    Ok(())
}

pub(super) fn neutral_input() -> Result<()> {
    modifiers_clear()?;
    for key in [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2] {
        if unsafe { GetAsyncKeyState(key as i32) } < 0 {
            return Err(error("region_input_busy", "请先松开鼠标按钮"));
        }
    }
    Ok(())
}

pub(super) fn stop_requested() -> bool {
    unsafe { GetAsyncKeyState(VK_ESCAPE as i32) < 0 }
}

fn validate_keyboard(target: &Target, viewport: Viewport) -> Result<()> {
    validate(target, viewport)?;
    modifiers_clear()?;
    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(target.thread, &mut info) } == 0
        || info.hwndFocus.is_null()
        || root(info.hwndFocus) as usize != target.handle
        || root(info.hwndActive) as usize != target.handle
        || !info.hwndMenuOwner.is_null()
        || !info.hwndMoveSize.is_null()
    {
        return Err(error(
            "region_keyboard_focus_denied",
            "无法确认选区内的输入焦点",
        ));
    }
    // A focused native control wholly inside the grant is strong spatial proof.
    let focus = client_rect(info.hwndFocus)?;
    if viewport.contains_rect(focus[0], focus[1], focus[2], focus[3]) {
        return Ok(());
    }
    // Do NOT infer keyboard authority from a previous mouse click. Caret bounds
    // use the caret window's DPI, so conservatively reject the ambiguous mixed-
    // DPI conversion instead of treating logical rcCaret as physical pixels.
    // Native controls with larger bounds therefore need a larger user selection.
    Err(error(
        "region_keyboard_focus_denied",
        "请将输入框完整包含在操作选区内",
    ))
}

fn mouse(flags: u32, data: u32, dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
fn keyboard(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
fn send(inputs: &[INPUT]) -> Result<()> {
    if unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    } != inputs.len() as u32
    {
        return Err(error("region_input_failed", "系统拒绝了区域输入"));
    }
    Ok(())
}

fn check_live(session: &RegionSession) -> Result<()> {
    // Native identity and focus checked immediately before input. Slow source
    // hashes/display enumeration run outside INPUT_LOCK at action boundaries.
    // INPUT_LOCK is held here. Mark revoked directly; execute's outer cleanup
    // releases a held mouse after this guard drops, avoiding recursive locking.
    if stop_requested() {
        session.revoked.store(true, Ordering::SeqCst);
    }
    if session.revoked.load(Ordering::SeqCst) || session.created.elapsed() > MAX_SESSION {
        return Err(error("region_authority_revoked", "区域操作已停止"));
    }
    session
        .hooks
        .validate_source(&session.info.binding)
        .map_err(|e| RegionError::new("region_authority_revoked", e))?;
    validate(&session.target, session.viewport)?;
    modifiers_clear()
}

fn move_pointer(session: &RegionSession, point: RegionPoint) -> Result<()> {
    let (x, y) = session.viewport.point(point)?;
    let _lock = INPUT_LOCK.lock().map_err(|_| unavailable())?;
    check_live(session)?;
    if root(unsafe { WindowFromPoint(POINT { x, y }) }) as usize != session.target.handle {
        return Err(unavailable());
    }
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let (dx, dy) = absolute_point(x, y, left, top, width, height)?;
    send(&[mouse(
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        0,
        dx,
        dy,
    )])?;
    validate_pointer(session)
}

fn validate_pointer(session: &RegionSession) -> Result<()> {
    let mut actual = POINT { x: 0, y: 0 };
    if unsafe { GetPhysicalCursorPos(&mut actual) } == 0
        || !session.viewport.contains_rect(
            actual.x,
            actual.y,
            actual.x.saturating_add(1),
            actual.y.saturating_add(1),
        )
        || root(unsafe { WindowFromPoint(actual) }) as usize != session.target.handle
    {
        return Err(error("region_cursor_clipped", "鼠标位置受到系统限制"));
    }
    Ok(())
}

fn absolute_point(
    x: i32,
    y: i32,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
) -> Result<(i32, i32)> {
    let px = i64::from(x) - i64::from(left);
    let py = i64::from(y) - i64::from(top);
    if width <= 1
        || height <= 1
        || px < 0
        || py < 0
        || px >= i64::from(width)
        || py >= i64::from(height)
    {
        return Err(error("region_display_changed", "桌面坐标范围已改变"));
    }
    let dx = ((px * 65535 + i64::from(width - 1) / 2) / i64::from(width - 1)) as i32;
    let dy = ((py * 65535 + i64::from(height - 1) / 2) / i64::from(height - 1)) as i32;
    Ok((dx, dy))
}

fn button(session: &RegionSession, down: bool) -> Result<()> {
    let _lock = INPUT_LOCK.lock().map_err(|_| unavailable())?;
    if down {
        check_live(session)?;
        validate_pointer(session)?;
        // Record ownership before calling SendInput, so partial/error cleanup
        // always attempts to release the button that might have been inserted.
        session.button_down.store(true, Ordering::SeqCst);
        send(&[mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0)])
    } else {
        if session.button_down.swap(false, Ordering::SeqCst) {
            send(&[mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0)])?;
        }
        Ok(())
    }
}
pub(super) fn release_button(down: &AtomicBool) {
    // Recover poison only for release; never enable further operations.
    let _lock = INPUT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if down.swap(false, Ordering::SeqCst) {
        let _ = send(&[mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0)]);
    }
}

pub(super) fn execute(session: &RegionSession, action: RegionAction) -> Result<()> {
    match action {
        RegionAction::Click { x, y } | RegionAction::DoubleClick { x, y } => {
            let times = if matches!(action, RegionAction::DoubleClick { .. }) {
                2
            } else {
                1
            };
            for i in 0..times {
                move_pointer(session, RegionPoint { x, y })?;
                button(session, true)?;
                button(session, false)?;
                if i + 1 < times {
                    std::thread::sleep(Duration::from_millis(45));
                }
            }
        }
        RegionAction::Drag { path } => {
            move_pointer(session, path[0])?;
            button(session, true)?;
            for segment in path.windows(2) {
                // Limit distance per native step to avoid jumping across an
                // unvalidated intermediate foreign window on a long segment.
                let dx = i64::from(segment[1].x) - i64::from(segment[0].x);
                let dy = i64::from(segment[1].y) - i64::from(segment[0].y);
                let steps = ((dx.abs().max(dy.abs()) + 7) / 8).max(1);
                for n in 1..=steps {
                    move_pointer(
                        session,
                        RegionPoint {
                            x: (i64::from(segment[0].x) + dx * n / steps) as u32,
                            y: (i64::from(segment[0].y) + dy * n / steps) as u32,
                        },
                    )?;
                    std::thread::sleep(Duration::from_millis(4));
                }
            }
            button(session, false)?;
        }
        RegionAction::Scroll { x, y, delta_y } => {
            move_pointer(session, RegionPoint { x, y })?;
            let _lock = INPUT_LOCK.lock().map_err(|_| unavailable())?;
            check_live(session)?;
            validate_pointer(session)?;
            send(&[mouse(MOUSEEVENTF_WHEEL, delta_y as u32, 0, 0)])?;
        }
        RegionAction::Type { text } => {
            for ch in text.chars() {
                let _lock = INPUT_LOCK.lock().map_err(|_| unavailable())?;
                check_live(session)?;
                validate_keyboard(&session.target, session.viewport)?;
                let mut units = [0u16; 2];
                let units = ch.encode_utf16(&mut units);
                let mut inputs = Vec::with_capacity(units.len() * 2);
                for &unit in units.iter() {
                    inputs.push(keyboard(0, unit, KEYEVENTF_UNICODE));
                    inputs.push(keyboard(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
                if let Err(e) = send(&inputs) {
                    for &unit in units.iter() {
                        let _ = send(&[keyboard(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]);
                    }
                    return Err(e);
                }
            }
        }
        RegionAction::Key { key } => {
            let vk = match key {
                RegionKey::Enter => VK_RETURN,
                RegionKey::Backspace => VK_BACK,
                RegionKey::Delete => VK_DELETE,
                RegionKey::ArrowLeft => VK_LEFT,
                RegionKey::ArrowRight => VK_RIGHT,
                RegionKey::ArrowUp => VK_UP,
                RegionKey::ArrowDown => VK_DOWN,
                RegionKey::Home => VK_HOME,
                RegionKey::End => VK_END,
            };
            let _lock = INPUT_LOCK.lock().map_err(|_| unavailable())?;
            check_live(session)?;
            validate_keyboard(&session.target, session.viewport)?;
            if let Err(e) = send(&[keyboard(vk, 0, 0), keyboard(vk, 0, KEYEVENTF_KEYUP)]) {
                let _ = send(&[keyboard(vk, 0, KEYEVENTF_KEYUP)]);
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OwnedWindow(HWND);
    impl Drop for OwnedWindow {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.0) };
        }
    }

    #[test]
    fn snapped_window_bounds_include_title_bar_without_granting_adjacent_pixels() {
        let bounds = [-800, 100, 200, 800];
        let client = [-792, 132, 192, 792];
        let snapped = Viewport { x: -800, y: 100, width: 1000, height: 700 };
        assert!(contains(bounds, snapped));
        assert!(!contains(client, snapped));
        for outside in [
            Viewport { x: -801, ..snapped },
            Viewport { y: 99, ..snapped },
            Viewport { width: 1001, ..snapped },
            Viewport { height: 701, ..snapped },
        ] { assert!(!contains(bounds, outside)); }
    }

    #[test]
    fn only_native_layered_and_transparent_combination_passes_mouse_input() {
        // Real owned, hidden top-level HWNDs, like the production overlays. Never
        // set WS_VISIBLE or send input. Layered child windows require a Windows
        // 8-aware manifest, which Rust test executables do not necessarily have:
        // https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features
        // Exercise the production reader, including a successful EXSTYLE=0.
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let parent = OwnedWindow(unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        });
        assert!(!parent.0.is_null());
        for (style, expected) in [
            (0, false),
            (WS_EX_TRANSPARENT, false),
            (WS_EX_LAYERED, false),
            (WS_EX_NOACTIVATE, false),
            (WS_EX_TOOLWINDOW, false),
            (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, false),
            (WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, false),
            (
                WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                false,
            ),
            (WS_EX_LAYERED | WS_EX_TRANSPARENT, true),
            (
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                true,
            ),
        ] {
            let child = OwnedWindow(unsafe {
                CreateWindowExW(
                    style,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    parent.0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            });
            assert!(
                !child.0.is_null(),
                "native fixture rejected style {style:x}: {}",
                unsafe { GetLastError() }
            );
            assert_eq!(unsafe { GetWindow(child.0, GW_OWNER) }, parent.0);
            assert_eq!(unsafe { IsWindowVisible(parent.0) }, 0);
            assert_eq!(unsafe { IsWindowVisible(child.0) }, 0);
            assert_eq!(
                mouse_passthrough_overlay(child.0),
                expected,
                "style {style:x}"
            );
        }
        assert_eq!(unsafe { IsWindowVisible(parent.0) }, 0);
    }

    #[test]
    fn unreadable_native_window_style_cannot_authorize_passthrough() {
        assert!(!mouse_passthrough_overlay(std::ptr::null_mut()));
        assert_ne!(unsafe { GetLastError() }, 0);
    }

    #[test]
    fn absolute_coordinates_handle_negative_monitors_and_desktop_edges() {
        assert_eq!(
            absolute_point(-1920, -120, -1920, -120, 3840, 1200).unwrap(),
            (0, 0)
        );
        assert_eq!(
            absolute_point(1919, 1079, -1920, -120, 3840, 1200).unwrap(),
            (65535, 65535)
        );
        assert!(absolute_point(-1921, 0, -1920, -120, 3840, 1200).is_err());
        assert!(absolute_point(1920, 0, -1920, -120, 3840, 1200).is_err());
        assert!(absolute_point(0, 0, 0, 0, 1, 1080).is_err());
    }

    #[test]
    fn system_input_records_are_paired_and_do_not_include_modifiers() {
        let down = keyboard(VK_RETURN, 0, 0);
        let up = keyboard(VK_RETURN, 0, KEYEVENTF_KEYUP);
        unsafe {
            assert_eq!(down.Anonymous.ki.wVk, VK_RETURN);
            assert_eq!(up.Anonymous.ki.dwFlags, KEYEVENTF_KEYUP);
        }
        let down = mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0);
        unsafe {
            assert_eq!(down.Anonymous.mi.dwFlags, MOUSEEVENTF_LEFTDOWN);
        }
    }

    #[test]
    fn real_process_creation_identity_is_stable_and_invalid_pid_is_rejected() {
        // Read-only OS metadata; no desktop access, window activation or input.
        let pid = unsafe { GetCurrentProcessId() };
        let first = process_creation(pid).unwrap();
        assert_ne!(first, 0);
        assert_eq!(process_creation(pid).unwrap(), first);
        assert!(process_creation(0).is_err());
        assert!(read_target(std::ptr::null_mut()).is_err());
    }
}
