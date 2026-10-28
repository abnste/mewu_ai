// SPDX-License-Identifier: MPL-2.0
//! Image bounds from the captured applications' accessibility trees. This is
//! advisory selection metadata, never authority to click or query another frame.
use crate::window_snap::{FrameSnapMap, SnapRect, SnapshotWindow};
use std::time::{Duration, Instant};
use windows::{
    core::Interface,
    Win32::{
        Foundation::HWND,
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_MULTITHREADED,
            },
            Variant::VARIANT,
        },
        UI::{
            Accessibility::*,
            HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
        },
    },
};

pub fn discover(
    map: &FrameSnapMap,
    windows: &[(usize, SnapshotWindow)],
    origin: (i32, i32),
    active: impl Fn() -> bool,
) -> Result<Vec<SnapRect>, String> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|_| "图片区域识别不可用")?;
    }
    struct Runtime(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe {
                if !self.0 .0.is_null() {
                    SetThreadDpiAwarenessContext(self.0);
                }
                CoUninitialize();
            }
        }
    }
    let _runtime = Runtime(unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
    });
    if _runtime.0 .0.is_null() {
        return Err("无法确认图片的屏幕坐标".into());
    }
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| "图片区域识别不可用")?;
    let bounded = automation
        .cast::<IUIAutomation2>()
        .map_err(|_| "系统不支持有界图片区域识别")?;
    unsafe {
        bounded
            .SetConnectionTimeout(400)
            .map_err(|_| "无法限制图片查询时长")?;
        bounded
            .SetTransactionTimeout(400)
            .map_err(|_| "无法限制图片查询时长")?;
    }
    let condition = unsafe {
        automation.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_ImageControlTypeId.0),
        )
    }
    .map_err(|_| "图片查询条件无效")?;
    let began = Instant::now();
    let mut output = Vec::new();
    for (index, witness) in windows.iter().take(8) {
        if !active() || began.elapsed() > Duration::from_secs(3) {
            break;
        }
        if !witness.is_current() {
            continue;
        }
        let elements = unsafe { automation.ElementFromHandle(HWND(witness.handle as *mut _)) }
            .and_then(|root| unsafe { root.FindAll(TreeScope_Descendants, &condition) });
        let Ok(elements) = elements else { continue };
        let count = unsafe { elements.Length() }.unwrap_or(0).min(128);
        for i in 0..count {
            if !active() || began.elapsed() > Duration::from_secs(3) {
                break;
            }
            let Ok(element) = (unsafe { elements.GetElement(i) }) else {
                continue;
            };
            if unsafe { element.CurrentIsOffscreen() }.map_or(true, |value| value.as_bool()) {
                continue;
            }
            let Ok(rect) = (unsafe { element.CurrentBoundingRectangle() }) else {
                continue;
            };
            let root = map.roots[*index].bounds;
            let left = (i64::from(rect.left) - i64::from(origin.0)).max(i64::from(root.x));
            let top = (i64::from(rect.top) - i64::from(origin.1)).max(i64::from(root.y));
            let right = (i64::from(rect.right) - i64::from(origin.0))
                .min(i64::from(root.x) + i64::from(root.width));
            let bottom = (i64::from(rect.bottom) - i64::from(origin.1))
                .min(i64::from(root.y) + i64::from(root.height));
            if left < 0
                || top < 0
                || right - left < 24
                || bottom - top < 24
                || right > map.width as i64
                || bottom > map.height as i64
            {
                continue;
            }
            let center = ((left + right) / 2, (top + bottom) / 2);
            if map.roots[..*index].iter().any(|node| {
                let b = node.bounds;
                center.0 >= b.x as i64
                    && center.1 >= b.y as i64
                    && center.0 < (b.x + b.width) as i64
                    && center.1 < (b.y + b.height) as i64
            }) {
                continue;
            }
            let bounds = SnapRect {
                x: left as u32,
                y: top as u32,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            };
            if !output.contains(&bounds) {
                output.push(bounds);
            }
            if output.len() >= 128 {
                break;
            }
        }
        if !witness.is_current() {
            output.clear();
            return Err("图片来源窗口已改变，请重新截图".into());
        }
        if output.len() >= 128 {
            break;
        }
    }
    output.sort_by_key(|b| u64::from(b.width) * u64::from(b.height));
    Ok(output)
}
