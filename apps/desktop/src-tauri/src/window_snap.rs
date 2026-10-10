// SPDX-License-Identifier: MPL-2.0
//! Read-only, capture-time native window bounds. No live queries of old images.
use mewu_core::{Asset, AssetKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
    sync::Mutex,
};

const MAX_NODES: usize = 2048;
const MAX_ROOTS: usize = 256;
const MAX_DEPTH: usize = 12;
const MAX_WIRE_BYTES: usize = 256 * 1024;
const MAX_FRAMES: usize = 8;
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl SnapRect {
    fn contains(self, other: Self) -> bool {
        other.width > 0
            && other.height > 0
            && other.x >= self.x
            && other.y >= self.y
            && u64::from(other.x) + u64::from(other.width)
                <= u64::from(self.x) + u64::from(self.width)
            && u64::from(other.y) + u64::from(other.height)
                <= u64::from(self.y) + u64::from(self.height)
    }
}

/// Every sibling list is front-to-back. A nonselectable root blocks everything
/// below it; a nonselectable child may contain useful grandchildren.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapNode {
    pub id: u32,
    pub bounds: SnapRect,
    pub selectable: bool,
    pub children: Vec<SnapNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameSnapMap {
    pub version: u8,
    pub background_id: String,
    pub width: u32,
    pub height: u32,
    pub roots: Vec<SnapNode>,
}

fn valid_dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= 16_384
        && height <= 16_384
        && u64::from(width) * u64::from(height) <= 32 * 1024 * 1024
}
fn validate(map: &FrameSnapMap) -> Result<(), String> {
    if map.version != 1
        || !valid_dimensions(map.width, map.height)
        || uuid::Uuid::parse_str(&map.background_id).is_err()
        || map.background_id.len() != 36
        || map.roots.len() > MAX_ROOTS
    {
        return Err("窗口地图无效".into());
    }
    fn visit(
        nodes: &[SnapNode],
        parent: SnapRect,
        depth: usize,
        ids: &mut HashSet<u32>,
    ) -> Result<(), String> {
        if depth > MAX_DEPTH && !nodes.is_empty() {
            return Err("窗口地图层级过多".into());
        }
        for node in nodes {
            if node.id == 0
                || !ids.insert(node.id)
                || ids.len() > MAX_NODES
                || !parent.contains(node.bounds)
                || (depth == 1 && !node.selectable && !node.children.is_empty())
            {
                return Err("窗口地图边界无效".into());
            }
            visit(&node.children, node.bounds, depth + 1, ids)?;
        }
        Ok(())
    }
    visit(
        &map.roots,
        SnapRect {
            x: 0,
            y: 0,
            width: map.width,
            height: map.height,
        },
        1,
        &mut HashSet::new(),
    )?;
    if serde_json::to_vec(map)
        .map_err(|_| "窗口地图无法编码")?
        .len()
        > MAX_WIRE_BYTES
    {
        return Err("窗口地图过大".into());
    }
    Ok(())
}

struct CachedFrame {
    source: Asset,
    map: FrameSnapMap,
    heap_bytes: usize,
}
struct Cache {
    frames: VecDeque<CachedFrame>,
    heap_bytes: usize,
}
/// Cache accounting includes owned capacities and fixed container overhead.
/// Temporary capture data and cloned IPC replies have separate lifetimes.
pub struct WindowSnapRegistry {
    cache: Mutex<Cache>,
}
impl Default for WindowSnapRegistry {
    fn default() -> Self {
        Self {
            cache: Mutex::new(Cache {
                frames: VecDeque::with_capacity(MAX_FRAMES),
                heap_bytes: 0,
            }),
        }
    }
}
fn node_heap(nodes: &Vec<SnapNode>) -> usize {
    nodes.capacity() * size_of::<SnapNode>()
        + nodes.iter().map(|n| node_heap(&n.children)).sum::<usize>()
}
fn frame_heap(source: &Asset, map: &FrameSnapMap) -> usize {
    source.id.capacity()
        + source.name.capacity()
        + source.path.capacity()
        + map.background_id.capacity()
        + node_heap(&map.roots)
}
fn retained_bytes(cache: &Cache) -> usize {
    size_of::<WindowSnapRegistry>()
        + cache.frames.capacity() * size_of::<CachedFrame>()
        + cache.heap_bytes
}
impl WindowSnapRegistry {
    pub fn insert(&self, source: &Asset, map: FrameSnapMap) -> Result<(), String> {
        validate(&map)?;
        if source.kind != AssetKind::Image
            || source.id != map.background_id
            || source.width != Some(map.width)
            || source.height != Some(map.height)
            || source.origin_x.is_none()
            || source.origin_y.is_none()
        {
            return Err("窗口地图与截图不一致".into());
        }
        let source = source.clone();
        let heap_bytes = frame_heap(&source, &map);
        let mut cache = self.cache.lock().map_err(|_| "窗口地图缓存不可用")?;
        if heap_bytes
            + size_of::<WindowSnapRegistry>()
            + cache.frames.capacity() * size_of::<CachedFrame>()
            > MAX_CACHE_BYTES
        {
            return Err("窗口地图缓存过大".into());
        }
        if let Some(index) = cache.frames.iter().position(|f| f.source.id == source.id) {
            let old = cache.frames.remove(index).expect("found frame");
            cache.heap_bytes -= old.heap_bytes;
        }
        // Evict before pushing so VecDeque never grows beyond eight slots.
        while cache.frames.len() >= MAX_FRAMES
            || retained_bytes(&cache) + heap_bytes > MAX_CACHE_BYTES
        {
            let old = cache
                .frames
                .pop_front()
                .expect("capacity checked for one frame");
            cache.heap_bytes -= old.heap_bytes;
        }
        cache.heap_bytes += heap_bytes;
        cache.frames.push_back(CachedFrame {
            source,
            map,
            heap_bytes,
        });
        Ok(())
    }
    pub fn get(&self, source: &Asset) -> Option<FrameSnapMap> {
        let mut cache = self.cache.lock().ok()?;
        let index = cache.frames.iter().position(|f| f.source.id == source.id)?;
        let frame = cache.frames.remove(index)?;
        if &frame.source != source {
            cache.heap_bytes -= frame.heap_bytes;
            return None;
        }
        let map = frame.map.clone();
        cache.frames.push_back(frame);
        Some(map)
    }
    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.frames.clear();
            cache.heap_bytes = 0;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DesktopRect {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}
impl DesktopRect {
    fn intersection(self, other: Self) -> Option<Self> {
        let result = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (result.left < result.right && result.top < result.bottom).then_some(result)
    }
    fn local(self, frame: Self) -> SnapRect {
        SnapRect {
            x: (self.left - frame.left) as u32,
            y: (self.top - frame.top) as u32,
            width: (self.right - self.left) as u32,
            height: (self.bottom - self.top) as u32,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Identity {
    handle: usize,
    process: u32,
    thread: u32,
    class: String,
}
#[derive(Debug, Clone)]
struct RawNode {
    identity: Identity,
    bounds: DesktopRect,
    selectable: bool,
    children: Vec<RawNode>,
}
impl RawNode {
    fn same_frame(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.bounds == other.bounds
            && self.selectable == other.selectable
    }
}
pub(crate) struct CaptureProbe {
    frame: DesktopRect,
    roots: Vec<RawNode>,
}

/// Must run within the capture worker's physical-DPI scope, never on the UI
/// thread. Unsupported platforms deliberately have no automatic candidates.
pub(crate) fn begin_capture(x: i32, y: i32, width: u32, height: u32) -> Option<CaptureProbe> {
    if !valid_dimensions(width, height) {
        return None;
    }
    let frame = DesktopRect {
        left: i64::from(x),
        top: i64::from(y),
        right: i64::from(x) + i64::from(width),
        bottom: i64::from(y) + i64::from(height),
    };
    Some(CaptureProbe {
        frame,
        roots: platform::snapshot(frame)?,
    })
}
pub(crate) fn finish_capture(
    probe: Option<CaptureProbe>,
    background_id: &str,
) -> Option<FrameSnapMap> {
    let probe = probe?;
    let after = platform::snapshot(probe.frame)?;
    build_map(probe.frame, background_id, &probe.roots, &after)
}

fn subtract(rect: DesktopRect, cover: DesktopRect) -> Vec<DesktopRect> {
    let Some(cut) = rect.intersection(cover) else {
        return vec![rect];
    };
    let candidates = [
        DesktopRect {
            bottom: cut.top,
            ..rect
        },
        DesktopRect {
            top: cut.bottom,
            ..rect
        },
        DesktopRect {
            left: rect.left,
            top: cut.top,
            right: cut.left,
            bottom: cut.bottom,
        },
        DesktopRect {
            left: cut.right,
            top: cut.top,
            right: rect.right,
            bottom: cut.bottom,
        },
    ];
    candidates
        .into_iter()
        .filter(|r| r.left < r.right && r.top < r.bottom)
        .collect()
}

/// Retain stable siblings. Moving/appearing/disappearing/reordered windows are
/// blockers in their old AND new footprints; never click through them. Stable
/// foreground windows covering those footprints remain usable.
fn reconcile(
    before: &[RawNode],
    after: &[RawNode],
    clip: DesktopRect,
    frame: DesktopRect,
    depth: usize,
    count: &mut u32,
) -> Option<Vec<SnapNode>> {
    let before_index: HashMap<&Identity, usize> = before
        .iter()
        .enumerate()
        .map(|(i, n)| (&n.identity, i))
        .collect();
    let after_index: HashMap<&Identity, usize> = after
        .iter()
        .enumerate()
        .map(|(i, n)| (&n.identity, i))
        .collect();
    if before_index.len() != before.len() || after_index.len() != after.len() {
        return None;
    }
    let mut stable: Vec<bool> = before
        .iter()
        .map(|n| {
            after_index
                .get(&n.identity)
                .is_some_and(|&i| n.same_frame(&after[i]))
        })
        .collect();
    for i in 0..before.len() {
        let Some(&ai) = after_index.get(&before[i].identity) else {
            continue;
        };
        for j in i + 1..before.len() {
            let Some(&aj) = after_index.get(&before[j].identity) else {
                continue;
            };
            if ai > aj
                && (before[i].bounds.intersection(before[j].bounds).is_some()
                    || after[ai].bounds.intersection(after[aj].bounds).is_some())
            {
                stable[i] = false;
                stable[j] = false;
            }
        }
    }
    let mut output = Vec::new();
    let mut unstable = HashSet::new();
    for node in before.iter().chain(after) {
        if before_index.get(&node.identity).is_some_and(|&i| stable[i])
            || !unstable.insert(&node.identity)
        {
            continue;
        }
        let bi = before_index.get(&node.identity).copied();
        let ai = after_index.get(&node.identity).copied();
        let mut footprints = Vec::new();
        if let Some(i) = bi {
            if let Some(r) = before[i].bounds.intersection(clip) {
                footprints.push(r);
            }
        }
        if let Some(i) = ai {
            if let Some(r) = after[i].bounds.intersection(clip) {
                if !footprints.contains(&r) {
                    footprints.push(r);
                }
            }
        }
        for (i, front) in before.iter().enumerate().filter(|(i, _)| stable[*i]) {
            let front_after = after_index[&front.identity];
            if bi.is_none_or(|b| i < b) && ai.is_none_or(|a| front_after < a) {
                footprints = footprints
                    .into_iter()
                    .flat_map(|r| subtract(r, front.bounds))
                    .collect();
                if footprints.len() > MAX_NODES {
                    return None;
                }
            }
        }
        for bounds in footprints {
            *count += 1;
            if *count as usize > MAX_NODES {
                return None;
            }
            output.push(SnapNode {
                id: *count,
                bounds: bounds.local(frame),
                selectable: false,
                children: vec![],
            });
        }
    }
    for (i, node) in before.iter().enumerate().filter(|(i, _)| stable[*i]) {
        let Some(bounds) = node.bounds.intersection(clip) else {
            continue;
        };
        *count += 1;
        if *count as usize > MAX_NODES {
            return None;
        }
        let id = *count;
        let children = if depth < MAX_DEPTH && (depth > 1 || node.selectable) {
            reconcile(
                &node.children,
                &after[after_index[&node.identity]].children,
                bounds,
                frame,
                depth + 1,
                count,
            )?
        } else {
            vec![]
        };
        output.push(SnapNode {
            id,
            bounds: bounds.local(frame),
            selectable: node.selectable,
            children,
        });
        let _ = i;
    }
    Some(output)
}
fn build_map(
    frame: DesktopRect,
    background_id: &str,
    before: &[RawNode],
    after: &[RawNode],
) -> Option<FrameSnapMap> {
    let roots = reconcile(before, after, frame, frame, 1, &mut 0)?;
    let map = FrameSnapMap {
        version: 1,
        background_id: background_id.into(),
        width: (frame.right - frame.left) as u32,
        height: (frame.bottom - frame.top) as u32,
        roots,
    };
    validate(&map).ok()?;
    Some(map)
}

#[cfg(any(target_os = "windows", test))]
fn append_root(
    roots: &mut Vec<RawNode>,
    enumerated: bool,
    observed: Result<Option<RawNode>, ()>,
    frame: DesktopRect,
) -> Option<()> {
    let Some(node) = observed.ok()? else {
        return Some(());
    };
    if node.bounds.intersection(frame).is_none() {
        return Some(());
    }
    // A root created during the pass has no certified position in the original
    // enumeration. An unknown footprint must not become a transparent hole.
    if !enumerated || roots.len() >= MAX_ROOTS {
        return None;
    }
    roots.push(node);
    Some(())
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::*;
    pub(super) fn snapshot(_: DesktopRect) -> Option<Vec<RawNode>> {
        None
    }
}
#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::{
        ptr::null_mut,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
        System::Threading::GetCurrentProcessId,
        UI::{Input::KeyboardAndMouse::IsWindowEnabled, WindowsAndMessaging::*},
    };
    const MAX_HANDLES: usize = 4096;
    const MAX_SIBLINGS: usize = 512;
    const PASS_BUDGET: Duration = Duration::from_millis(120);
    struct Enumeration {
        handles: Vec<usize>,
        deadline: Instant,
        overflow: bool,
    }
    unsafe extern "system" fn collect(handle: HWND, parameter: LPARAM) -> i32 {
        // Synchronous EnumWindows owns this stack context; no callback escapes.
        let state = unsafe { &mut *(parameter as *mut Enumeration) };
        if state.handles.len() >= MAX_HANDLES || Instant::now() >= state.deadline {
            state.overflow = true;
            return 0;
        }
        state.handles.push(handle as usize);
        1
    }
    fn rect(value: RECT) -> Option<DesktopRect> {
        (value.left < value.right && value.top < value.bottom).then_some(DesktopRect {
            left: i64::from(value.left),
            top: i64::from(value.top),
            right: i64::from(value.right),
            bottom: i64::from(value.bottom),
        })
    }
    // None is positively absent/hidden. Err means visible but without reliable
    // bounds: discard this sibling pass instead of exposing a window behind it.
    fn read(handle: HWND, root: bool, own_process: u32) -> Result<Option<RawNode>, ()> {
        if unsafe { IsWindowVisible(handle) } == 0 || unsafe { IsIconic(handle) } != 0 {
            return Ok(None);
        }
        let mut cloaked = 0u32;
        let cloak_known = !root
            || unsafe {
                DwmGetWindowAttribute(
                    handle,
                    DWMWA_CLOAKED as u32,
                    (&mut cloaked as *mut u32).cast(),
                    size_of::<u32>() as u32,
                )
            } >= 0;
        if cloak_known && cloaked != 0 {
            return Ok(None);
        }
        let mut native = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let dwm_ok = root
            && unsafe {
                DwmGetWindowAttribute(
                    handle,
                    DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                    (&mut native as *mut RECT).cast(),
                    size_of::<RECT>() as u32,
                )
            } >= 0;
        if !dwm_ok && unsafe { GetWindowRect(handle, &mut native) } == 0 {
            return Err(());
        }
        let bounds = rect(native).ok_or(())?;
        let mut process = 0u32;
        let thread = unsafe { GetWindowThreadProcessId(handle, &mut process) };
        let mut name = [0u16; 128];
        let length = unsafe { GetClassNameW(handle, name.as_mut_ptr(), name.len() as i32) };
        let class_known = length > 0 && (length as usize) < name.len() - 1;
        let class = String::from_utf16_lossy(&name[..(length.max(0) as usize).min(name.len())]);
        if unsafe { IsWindow(handle) } == 0 {
            return Ok(None);
        }
        let transparent =
            unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32 & WS_EX_TRANSPARENT != 0;
        let shell = matches!(
            class.as_str(),
            "Progman"
                | "WorkerW"
                | "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "Windows.UI.Core.CoreWindow"
        );
        Ok(Some(RawNode {
            identity: Identity {
                handle: handle as usize,
                process,
                thread,
                class,
            },
            bounds,
            // Keep uncertain metadata as an opaque, nonselectable footprint.
            // A long/unreadable class or failed DWM query is not transparency.
            selectable: cloak_known
                && class_known
                && process != 0
                && thread != 0
                && process != own_process
                && !shell
                && !transparent
                && (root || unsafe { IsWindowEnabled(handle) } != 0),
            children: vec![],
        }))
    }
    fn children(
        parent: &RawNode,
        root_bounds: DesktopRect,
        frame: DesktopRect,
        depth: usize,
        remaining: &mut usize,
        deadline: Instant,
    ) -> Option<Vec<RawNode>> {
        if depth > MAX_DEPTH || *remaining == 0 {
            return Some(vec![]);
        }
        let mut handle = unsafe { GetWindow(parent.identity.handle as HWND, GW_CHILD) };
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        while !handle.is_null() {
            if seen.len() >= MAX_SIBLINGS
                || !seen.insert(handle as usize)
                || Instant::now() >= deadline
            {
                return None;
            }
            if let Some(mut node) = read(handle, false, unsafe { GetCurrentProcessId() }).ok()? {
                if node
                    .bounds
                    .intersection(parent.bounds)
                    .and_then(|r| r.intersection(frame))
                    .is_some()
                {
                    if *remaining == 0 {
                        return None;
                    }
                    *remaining -= 1;
                    let width = node.bounds.right - node.bounds.left;
                    let height = node.bounds.bottom - node.bounds.top;
                    let child_area = i128::from(width) * i128::from(height);
                    let root_area = i128::from(root_bounds.right - root_bounds.left)
                        * i128::from(root_bounds.bottom - root_bounds.top);
                    let traverse = node.selectable;
                    node.selectable &=
                        width >= 16 && height >= 16 && child_area * 100 < root_area * 92;
                    if traverse {
                        // An incomplete child list must not expose siblings behind
                        // omitted occluders. Safely fall back to this ancestor.
                        node.children =
                            children(&node, root_bounds, frame, depth + 1, remaining, deadline)
                                .unwrap_or_default();
                    }
                    result.push(node);
                }
            }
            handle = unsafe { GetWindow(handle, GW_HWNDNEXT) };
        }
        Some(result)
    }
    pub(super) fn snapshot(frame: DesktopRect) -> Option<Vec<RawNode>> {
        let deadline = Instant::now() + PASS_BUDGET;
        let mut enumeration = Enumeration {
            handles: vec![],
            deadline,
            overflow: false,
        };
        let complete = unsafe {
            EnumWindows(
                Some(collect),
                (&mut enumeration as *mut Enumeration) as LPARAM,
            )
        };
        if complete == 0 || enumeration.overflow {
            return None;
        }
        let handles: HashSet<usize> = enumeration.handles.into_iter().collect();
        // EnumWindows is reliable for the handle set; use the documented Z
        // relationship explicitly, with count/cycle guards for concurrent changes.
        let mut seen = HashSet::new();
        let mut handle = unsafe { GetTopWindow(null_mut()) };
        let mut roots = Vec::new();
        while !handle.is_null() {
            if seen.len() >= MAX_HANDLES
                || !seen.insert(handle as usize)
                || Instant::now() >= deadline
            {
                return None;
            }
            append_root(
                &mut roots,
                handles.contains(&(handle as usize)),
                read(handle, true, unsafe { GetCurrentProcessId() }),
                frame,
            )?;
            handle = unsafe { GetWindow(handle, GW_HWNDNEXT) };
        }
        // A captured root that vanished while Z order was read cannot be silently
        // treated as a transparent hole. Decline the map on topology uncertainty.
        for missing in handles.difference(&seen) {
            if read(*missing as HWND, true, unsafe { GetCurrentProcessId() })
                .ok()?
                .is_some_and(|n| n.bounds.intersection(frame).is_some())
            {
                return None;
            }
        }
        let mut remaining = MAX_NODES - roots.len();
        for root in &mut roots {
            if Instant::now() >= deadline {
                return None;
            }
            if root.selectable {
                root.children = children(root, root.bounds, frame, 2, &mut remaining, deadline)
                    .unwrap_or_default();
            }
        }
        (Instant::now() < deadline).then_some(roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds(x: i64, y: i64, w: i64, h: i64) -> DesktopRect {
        DesktopRect {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        }
    }
    fn raw(id: usize, r: DesktopRect, selectable: bool) -> RawNode {
        RawNode {
            identity: Identity {
                handle: id,
                process: 7,
                thread: 8,
                class: "synthetic".into(),
            },
            bounds: r,
            selectable,
            children: vec![],
        }
    }
    fn map(before: &[RawNode], after: &[RawNode]) -> FrameSnapMap {
        build_map(
            bounds(-200, 100, 800, 600),
            &uuid::Uuid::new_v4().to_string(),
            before,
            after,
        )
        .unwrap()
    }
    fn hit(nodes: &[SnapNode], x: u32, y: u32, root: bool) -> Option<SnapRect> {
        let node = nodes.iter().find(|n| {
            x >= n.bounds.x
                && y >= n.bounds.y
                && u64::from(x) < u64::from(n.bounds.x) + u64::from(n.bounds.width)
                && u64::from(y) < u64::from(n.bounds.y) + u64::from(n.bounds.height)
        })?;
        if root && !node.selectable {
            return None;
        }
        hit(&node.children, x, y, false).or_else(|| node.selectable.then_some(node.bounds))
    }
    fn asset(map: &FrameSnapMap) -> Asset {
        Asset {
            id: map.background_id.clone(),
            name: "synthetic.png".into(),
            path: format!("{}.png", map.background_id),
            kind: AssetKind::Image,
            width: Some(map.width),
            height: Some(map.height),
            origin_x: Some(-200),
            origin_y: Some(100),
            scale_factor: Some(1.5),
        }
    }
    #[test]
    fn maps_clip_negative_screen_coordinates_and_keep_front_root_occlusion() {
        let mut front = raw(1, bounds(-180, 120, 200, 180), true);
        let mut host = raw(2, bounds(-175, 125, 190, 170), false);
        host.children.push(raw(3, bounds(-160, 140, 50, 30), true));
        front.children.push(host);
        let behind = raw(4, bounds(-300, 50, 900, 700), true);
        let m = map(&[front.clone(), behind.clone()], &[front, behind]);
        assert_eq!(
            hit(&m.roots, 40, 40, true),
            Some(SnapRect {
                x: 40,
                y: 40,
                width: 50,
                height: 30
            })
        );
        assert_eq!(
            hit(&m.roots, 100, 100, true),
            Some(SnapRect {
                x: 20,
                y: 20,
                width: 200,
                height: 180
            })
        );
        assert_eq!(
            hit(&m.roots, 220, 200, true),
            Some(SnapRect {
                x: 0,
                y: 0,
                width: 800,
                height: 600
            })
        );
        assert_eq!(hit(&m.roots, 800, 600, true), None);
    }
    #[test]
    fn moving_front_window_blocks_both_footprints_without_losing_unrelated_targets() {
        let old = raw(1, bounds(-180, 120, 100, 100), true);
        let mut new = old.clone();
        new.bounds = bounds(-60, 120, 100, 100);
        let back = raw(2, bounds(-200, 100, 800, 600), true);
        let m = map(&[old, back.clone()], &[new, back]);
        assert_eq!(hit(&m.roots, 30, 30, true), None);
        assert_eq!(hit(&m.roots, 150, 30, true), None);
        assert!(hit(&m.roots, 400, 400, true).is_some());
    }
    #[test]
    fn changes_behind_stable_foreground_do_not_block_the_foreground() {
        let front = raw(1, bounds(-190, 110, 300, 300), true);
        let old = raw(2, bounds(-200, 100, 800, 600), true);
        let mut new = old.clone();
        new.bounds = bounds(-199, 100, 799, 600);
        let m = map(&[front.clone(), old], &[front, new]);
        assert_eq!(
            hit(&m.roots, 30, 30, true),
            Some(SnapRect {
                x: 10,
                y: 10,
                width: 300,
                height: 300
            })
        );
        assert_eq!(hit(&m.roots, 600, 400, true), None);
    }
    #[test]
    fn appearing_disappearing_and_reordered_foreground_never_expose_hidden_window() {
        let front = raw(1, bounds(-180, 120, 100, 100), true);
        let back = raw(2, bounds(-200, 100, 800, 600), true);
        for (before, after) in [
            (vec![back.clone()], vec![front.clone(), back.clone()]),
            (vec![front.clone(), back.clone()], vec![back.clone()]),
            (
                vec![front.clone(), back.clone()],
                vec![back.clone(), front.clone()],
            ),
        ] {
            assert_eq!(hit(&map(&before, &after).roots, 30, 30, true), None);
        }
    }
    #[test]
    fn nonselectable_root_and_changed_child_cannot_punch_through_occlusion() {
        let blocker = raw(1, bounds(-180, 120, 100, 100), false);
        let back = raw(2, bounds(-200, 100, 800, 600), true);
        assert_eq!(
            hit(
                &map(&[blocker.clone(), back.clone()], &[blocker, back]).roots,
                30,
                30,
                true
            ),
            None
        );
        let mut parent = raw(3, bounds(-190, 110, 300, 300), true);
        parent.children = vec![
            raw(4, bounds(-180, 120, 60, 60), true),
            raw(5, bounds(-185, 115, 120, 120), true),
        ];
        let mut after = parent.clone();
        after.children[0].bounds = bounds(-110, 120, 60, 60);
        let m = map(&[parent], &[after]);
        assert_eq!(
            hit(&m.roots, 30, 30, true),
            Some(SnapRect {
                x: 10,
                y: 10,
                width: 300,
                height: 300
            })
        );
    }
    #[test]
    fn registry_lru_is_source_exact_and_retains_no_more_than_eight_frames() {
        let registry = WindowSnapRegistry::default();
        let mut sources = Vec::new();
        for _ in 0..8 {
            let m = map(&[], &[]);
            let a = asset(&m);
            registry.insert(&a, m).unwrap();
            sources.push(a);
        }
        assert!(registry.get(&sources[0]).is_some());
        let m = map(&[], &[]);
        let a = asset(&m);
        registry.insert(&a, m).unwrap();
        assert!(registry.get(&sources[1]).is_none());
        assert!(registry.get(&sources[0]).is_some());
        let mut changed = sources[0].clone();
        changed.origin_x = Some(-201);
        assert!(registry.get(&changed).is_none());
        assert!(registry.get(&sources[0]).is_none());
        let cache = registry.cache.lock().unwrap();
        assert!(cache.frames.len() <= 8 && retained_bytes(&cache) <= MAX_CACHE_BYTES);
        drop(cache);
        registry.clear();
        assert!(registry.get(&a).is_none());
    }
    #[test]
    fn malformed_map_or_source_does_not_replace_an_existing_cache_entry() {
        let registry = WindowSnapRegistry::default();
        let m = map(
            &[raw(1, bounds(-180, 120, 100, 100), true)],
            &[raw(1, bounds(-180, 120, 100, 100), true)],
        );
        let a = asset(&m);
        registry.insert(&a, m.clone()).unwrap();
        for mode in ["id", "bounds", "dimensions", "root-child", "wire"] {
            let mut invalid = m.clone();
            match mode {
                "id" => invalid.roots.push(invalid.roots[0].clone()),
                "bounds" => invalid.roots[0].bounds.width = u32::MAX,
                "dimensions" => invalid.width = 16_385,
                "root-child" => {
                    invalid.roots[0].selectable = false;
                    invalid.roots[0].children = vec![SnapNode {
                        id: 999,
                        bounds: invalid.roots[0].bounds,
                        selectable: true,
                        children: vec![],
                    }];
                }
                "wire" => invalid.version = 2,
                _ => unreachable!(),
            }
            assert!(registry.insert(&a, invalid).is_err(), "{mode}");
            assert_eq!(registry.get(&a), Some(m.clone()));
        }
        let mut wrong = a.clone();
        wrong.width = Some(1);
        assert!(registry.insert(&wrong, m.clone()).is_err());
        assert_eq!(registry.get(&a), Some(m));
    }
    #[test]
    fn retained_capacity_budget_evicts_before_an_oversized_incoming_allocation() {
        let registry = WindowSnapRegistry::default();
        let mut first = None;
        for _ in 0..8 {
            let mut m = map(&[], &[]);
            // Allocator capacity is real retained memory even when JSON is tiny.
            m.roots = Vec::with_capacity(6000);
            let a = asset(&m);
            if first.is_none() {
                first = Some(a.clone());
            }
            registry.insert(&a, m).unwrap();
            let cache = registry.cache.lock().unwrap();
            assert!(retained_bytes(&cache) <= MAX_CACHE_BYTES);
        }
        assert!(registry.get(&first.unwrap()).is_none());
        let mut huge = map(&[], &[]);
        huge.roots = Vec::with_capacity(MAX_CACHE_BYTES / size_of::<SnapNode>() + 1);
        let a = asset(&huge);
        assert!(registry.insert(&a, huge).is_err());
    }

    #[test]
    fn uncertain_or_new_root_is_never_discarded_as_transparent() {
        let frame = bounds(-200, 100, 800, 600);
        let mut roots = vec![];
        assert!(append_root(&mut roots, true, Ok(None), frame).is_some());
        assert!(roots.is_empty());
        // A known footprint with incomplete metadata remains an occluder.
        let incomplete = raw(1, bounds(-180, 120, 100, 100), false);
        assert!(append_root(&mut roots, true, Ok(Some(incomplete)), frame).is_some());
        let behind = raw(2, frame, true);
        assert!(append_root(&mut roots, true, Ok(Some(behind)), frame).is_some());
        assert_eq!(hit(&map(&roots, &roots).roots, 30, 30, true), None);
        assert!(append_root(&mut roots, true, Err(()), frame).is_none());
        let appearing = raw(3, bounds(-150, 140, 100, 100), true);
        assert!(append_root(&mut roots, false, Ok(Some(appearing)), frame).is_none());
        let elsewhere = raw(4, bounds(2000, 2000, 100, 100), true);
        assert!(append_root(&mut roots, false, Ok(Some(elsewhere)), frame).is_some());
    }
}
