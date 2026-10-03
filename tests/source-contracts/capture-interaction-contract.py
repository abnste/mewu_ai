# SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
# SPDX-License-Identifier: MPL-2.0
"""Source wiring regression, NOT a Windows event/runtime test."""
from pathlib import Path
r=Path(__file__).resolve().parents[2]
s=(r/'Views/CaptureOverlayWindow.xaml.cs').read_text(encoding='utf-8')
def part(start,end): return s[s.index(start):s.index(end,s.index(start))]
mouse=part('    private void OnMouseDown(', '    private void OnMouseMove(')
assert 'FindTopmostTargetAt' not in mouse and 'ProbeSnapRect' not in mouse
assert 'ProbeNativeSnapRect(p,out var immediateRoot)' in mouse
assert mouse.index('var cachedSnap=')<mouse.index('ResetSnapPreview();')
assert 'cachedSnapRoot==immediateRoot' in mouse
native=part('    private Rect ProbeNativeSnapRect(Point point,out', '    private Rect MonitorBounds(')
assert 'forceFresh:true' in native and 'FindTopmostTargetAt' not in native
resize=part('    private void ResizeDelta(', '    private void ResizeCompleted(')
assert 'InvalidateImageDerivedLayers' not in resize and 'CaptureSelectionGeometry.Resize' in resize
complete=part('    private void ResizeCompleted(', '    private void AddRegion(')
assert 'ApplyOverlaySnapshot' not in complete
assert 'if(e.Canceled){SetSelectionBoundsPreservingManualContent(target,original.Bounds);UpdateSelection(target);}' in complete
assert 'else if(CaptureOverlayPolicy.HasContentGeometryChanged' in complete
keys=part('    private void OnPreviewKeyDown(', '        if(e.Key==Key.F8')
assert '_resizeTarget is not null' in keys and 'resizeHandle.CancelDrag()' in keys and 'e.Handled=true;return;' in keys
refresh=part('    private CaptureFrame CaptureCleanDesktopForRefresh()', '    private void RefreshDesktopFrameIncludingPinnedWindows()')
assert refresh.index('IsExcludedFromCapture')<refresh.index('.CaptureDesktop(')
assert 'WithWindowCloaked' in refresh
hover=part('    private async void UpdateSnapPreview(', '    private void CancelSnapProbe(')
assert 'await Task.Run(()=>_windowSnap.FindTopmostTargetAt' in hover
assert 'probeFrameVersion!=_desktopFrameVersion' in hover
assert '_stableSnapProbePoint=probePoint' in hover
assert 'Interlocked.CompareExchange' in hover
print('PASS capture event source contracts; actual Windows focus/cancel/mixed-DPI/blocked-provider tests still required')
