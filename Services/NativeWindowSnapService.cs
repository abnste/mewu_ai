using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Automation;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Finds the smallest useful native/UIA object under the pointer.
///
/// The capture overlay is itself the top-most window, so a plain
/// WindowFromPoint call is not sufficient. We first use it when possible,
/// then fall back to the top-to-bottom EnumWindows list and stop at the first
/// real window under the point. UI Automation is only queried for that one
/// window; the old implementation walked every descendant of every hit
/// window, which made the preview lag behind the pointer.
/// </summary>
internal sealed class NativeWindowSnapService
{
    private const int MinTargetWidth = 16;
    private const int MinTargetHeight = 16;
    private const int MaxUsefulControlWidth = 1200;
    private const int MaxUsefulControlHeight = 240;
    private const int MaxUsefulControlArea = 360_000;
    // Modern WebView/Electron shells (including Codex) expose actionable
    // controls below several generic Region/Group/Document layers. Eight
    // levels stopped before the actual buttons. This remains a single branch
    // walk, with a strict time budget, rather than a whole-window scan.
    private const int MaxAutomationDepth = 24;
    private const int MaxAutomationSiblingsPerLevel = 512;
    private const int AutomationTraversalBudgetMs = 120;
    private const int MaxWindowCacheAgeMs = 250;
    private const int MaxPreciseCacheAgeMs = 450;
    private const uint ChildWindowSkipInvisible = 0x0001;
    private const uint ChildWindowSkipDisabled = 0x0002;
    private const uint ChildWindowSkipTransparent = 0x0004;
    private const uint ChildWindowFlags = ChildWindowSkipInvisible | ChildWindowSkipDisabled | ChildWindowSkipTransparent;
    private const uint GetAncestorRoot = 2;
    private const int DwmWindowAttributeCloaked = 14;

    private readonly object _cacheGate = new();
    private WindowHitCache? _windowCache;
    private PreciseTargetCache? _preciseCache;

    internal ScreenRect? FindTopmostWindowAt(int screenX, int screenY, IntPtr excludedWindow)
        => FindTopmostTargetAt(screenX, screenY, excludedWindow)?.Bounds;

    /// <summary>
    /// Cheap native-only hit test used by the UI thread for the immediate
    /// preview. It never touches UI Automation and is safe to call for every
    /// pointer move.
    /// </summary>
    internal WindowSnapTarget? FindFastTargetAt(int screenX, int screenY, IntPtr excludedWindow)
    {
        var root = FindRootWindowAt(screenX, screenY, excludedWindow);
        if (root == IntPtr.Zero)
            return null;

        if (!TryGetBounds(root, out var rootBounds) || !Contains(rootBounds, screenX, screenY))
            return null;

        var child = DeepestChildAt(root, screenX, screenY);
        if (child != IntPtr.Zero && child != root &&
            TryGetBounds(child, out var childBounds) && Contains(childBounds, screenX, screenY) &&
            IsUsefulNativeChild(childBounds, rootBounds))
        {
            var target = new WindowSnapTarget(child, childBounds);
            CacheWindow(root, rootBounds);
            return target;
        }

        // A maximized application is still a valid Snipaste-style automatic
        // selection.  Its semantic controls can replace this rectangle in the
        // refinement path, but suppressing the root entirely made a maximized
        // Codex window look as though smart selection was unavailable.
        CacheWindow(root, rootBounds);
        return new WindowSnapTarget(root, rootBounds);
    }

    /// <summary>
    /// Refines the fast native hit with one UIA point lookup and, only when
    /// necessary, a spatially-following UIA branch. No whole-window tree
    /// traversal is performed.
    /// </summary>
    internal WindowSnapTarget? FindTopmostTargetAt(int screenX, int screenY, IntPtr excludedWindow)
    {
        if (TryGetPreciseCache(screenX, screenY, out var cached))
            return cached;

        var fast = FindFastTargetAt(screenX, screenY, excludedWindow);
        var root = fast is null ? FindRootWindowAt(screenX, screenY, excludedWindow) : GetRootWindow(fast.Handle);
        if (root == IntPtr.Zero || root == excludedWindow)
            return null;

        // `AutomationElement.FromHandle` is not interchangeable for modern
        // WinUI/WebView applications.  The top-level frame often exposes only
        // a generic window, while the HWND directly under the pointer owns the
        // Document/Region branch that contains buttons and editors.  Resolve
        // that bounded native path first, then use the root only as a fallback.
        // This mirrors an accessibility-first computer-use hit test without
        // ever recursively scanning a complete desktop or window tree.
        var pointedHost = DeepestChildAt(root, screenX, screenY);

        // A maximized app is deliberately not a snap rectangle, but it can
        // still contain a precise button, menu item, edit box, etc.
        if (fast is null)
        {
            var maximizedAutomation = FindAutomationElementAtPoint(screenX, screenY, excludedWindow, root)
                ?? FindAutomationBranchAt(pointedHost, screenX, screenY, excludedWindow)
                ?? (pointedHost != root ? FindAutomationBranchAt(root, screenX, screenY, excludedWindow) : null);
            if (maximizedAutomation is not null)
            {
                CachePrecise(maximizedAutomation);
                return maximizedAutomation;
            }
            return null;
        }

        var automation = FindAutomationElementAtPoint(screenX, screenY, excludedWindow, root);
        if (automation is not null)
        {
            CachePrecise(automation);
            return automation;
        }

        // Chromium/Electron and some WPF controls expose the actual button or
        // edit box only from a branch rooted at the top-level automation
        // element. A native render-host HWND can be slightly smaller than its
        // root, so restricting this fallback to fast.Handle == root made the
        // snapper stop at that host instead of reaching the real control.
        // Always do this one bounded branch walk before accepting the coarse
        // native rectangle.
        automation = FindAutomationBranchAt(pointedHost, screenX, screenY, excludedWindow)
            ?? (pointedHost != root ? FindAutomationBranchAt(root, screenX, screenY, excludedWindow) : null);
        if (automation is not null)
        {
            CachePrecise(automation);
            return automation;
        }

        return fast;
    }

    private IntPtr FindRootWindowAt(int screenX, int screenY, IntPtr excludedWindow)
    {
        var point = new NativePoint { X = screenX, Y = screenY };
        var direct = WindowFromPoint(point);
        var directRoot = GetRootWindow(direct);
        if (IsSelectableWindow(directRoot, excludedWindow))
        {
            GetWindowThreadProcessId(directRoot, out var directProcess);
            if (directProcess != (uint)Environment.ProcessId)
            {
                if (TryGetBounds(directRoot, out var directBounds))
                    CacheWindow(directRoot, directBounds);
                return directRoot;
            }
        }

        lock (_cacheGate)
        {
            var cache = _windowCache;
            if (cache is not null && Stopwatch.GetTimestamp() <= cache.ExpiresAt && Contains(cache.Bounds, screenX, screenY) && IsSelectableWindow(cache.Handle, excludedWindow))
                return cache.Handle;
            if (cache is not null && Stopwatch.GetTimestamp() > cache.ExpiresAt)
                _windowCache = null;
        }

        // The overlay normally wins WindowFromPoint. EnumWindows is ordered
        // from front to back, so the first containing window is the one the
        // user sees beneath the overlay. This is bounded by the OS window
        // list and never walks a UIA tree.
        var currentProcess = (uint)Environment.ProcessId;
        IntPtr match = IntPtr.Zero;
        EnumWindows((handle, _) =>
        {
            if (!IsSelectableWindow(handle, excludedWindow) || handle == IntPtr.Zero)
                return true;
            GetWindowThreadProcessId(handle, out var processId);
            if (processId == currentProcess)
                return true;
            if (!TryGetBounds(handle, out var bounds) || !Contains(bounds, screenX, screenY))
                return true;
            match = handle;
            return false;
        }, IntPtr.Zero);
        return match;
    }

    private static bool IsSelectableWindow(IntPtr handle, IntPtr excludedWindow)
    {
        if (handle == IntPtr.Zero || handle == excludedWindow || !IsWindowVisible(handle) || IsIconic(handle) || IsWindowCloaked(handle) || IsDesktopShellWindow(handle))
            return false;
        return true;
    }

    private static bool IsWindowCloaked(IntPtr handle)
    {
        try
        {
            return DwmGetWindowAttribute(handle,DwmWindowAttributeCloaked,out int value,sizeof(int))>=0&&value!=0;
        }
        catch(DllNotFoundException){return false;}
        catch(EntryPointNotFoundException){return false;}
    }

    private static IntPtr GetRootWindow(IntPtr handle)
        => handle == IntPtr.Zero ? IntPtr.Zero : GetAncestor(handle, GetAncestorRoot);

    private static WindowSnapTarget? FindAutomationElementAtPoint(int screenX, int screenY, IntPtr excludedWindow, IntPtr rootWindow)
    {
        try
        {
            var element = AutomationElement.FromPoint(new System.Windows.Point(screenX, screenY));
            var walker = TreeWalker.RawViewWalker;
            var currentProcess = (int)Environment.ProcessId;
            for (var depth = 0; element is not null && depth < MaxAutomationDepth; depth++)
            {
                var info = element.Current;
                var rectangle = info.BoundingRectangle;
                if (info.ProcessId != currentProcess &&
                    info.NativeWindowHandle != excludedWindow.ToInt32() &&
                    IsUsefulElement(element, info, rectangle, screenX, screenY) &&
                    IsInsideRoot(rectangle, rootWindow))
                {
                    return ToTarget(info, rectangle, rootWindow);
                }
                element = walker.GetParent(element);
            }
        }
        catch (COMException) { }
        catch (InvalidOperationException) { }
        catch (ArgumentException) { }
        return null;
    }

    private static WindowSnapTarget? FindAutomationBranchAt(IntPtr windowHandle, int screenX, int screenY, IntPtr excludedWindow)
        // Computer-use resolves Codex's semantic Control View (Button/Edit/
        // MenuItem), while Raw View may expose only its WebView render hosts.
        // Prefer the semantic tree and only fall back to Raw View for apps
        // whose providers omit controls from the former.
        => FindAutomationBranchAt(windowHandle,screenX,screenY,excludedWindow,TreeWalker.ControlViewWalker)
           ??FindAutomationBranchAt(windowHandle,screenX,screenY,excludedWindow,TreeWalker.RawViewWalker);

    private static WindowSnapTarget? FindAutomationBranchAt(IntPtr windowHandle, int screenX, int screenY, IntPtr excludedWindow,TreeWalker walker)
    {
        try
        {
            // When the capture overlay is topmost, FromPoint correctly sees
            // the overlay, not the application below it.  Resolve the actual
            // top-level desktop child by HWND before falling back to
            // FromHandle: packaged WinUI apps can give those two entry points
            // different provider roots even for the same native window.
            var current = FindDesktopWindowAutomationRoot(windowHandle) ?? AutomationElement.FromHandle(windowHandle);
            var currentProcess = (int)Environment.ProcessId;
            WindowSnapTarget? best = null;
            var deadline=Stopwatch.GetTimestamp()+Stopwatch.Frequency*AutomationTraversalBudgetMs/1000;

            for (var depth = 0; current is not null && depth < MaxAutomationDepth && Stopwatch.GetTimestamp()<=deadline; depth++)
            {
                var info = current.Current;
                var rectangle = info.BoundingRectangle;
                if (info.ProcessId != currentProcess && info.NativeWindowHandle != excludedWindow.ToInt32() && IsUsefulElement(current, info, rectangle, screenX, screenY))
                    best = PreferSmaller(best, ToTarget(info, rectangle, windowHandle));

                AutomationElement? next = null;
                var nextArea = double.PositiveInfinity;
                var nextEnabled = false;
                var child = walker.GetFirstChild(current);
                for (var siblings = 0; child is not null && siblings < MaxAutomationSiblingsPerLevel && Stopwatch.GetTimestamp()<=deadline; siblings++)
                {
                    var childInfo = child.Current;
                    var childRectangle = childInfo.BoundingRectangle;
                    if (childInfo.ProcessId != currentProcess && Contains(childRectangle, screenX, screenY))
                    {
                        var area = Math.Max(1, childRectangle.Width * childRectangle.Height);
                        // WinUI/WebView roots can expose an equal-sized,
                        // disabled placeholder Pane before the real enabled
                        // Region.  Choosing only by smallest area permanently
                        // entered that dead branch and made Codex controls
                        // unreachable.  On equal geometry prefer the enabled
                        // provider branch while retaining the bounded spatial
                        // single-branch traversal.
                        if (area < nextArea || (Math.Abs(area-nextArea)<1 && childInfo.IsEnabled && !nextEnabled))
                        {
                            next = child;
                            nextArea = area;
                            nextEnabled = childInfo.IsEnabled;
                        }
                    }
                    child = walker.GetNextSibling(child);
                }
                current = next;
            }
            return best;
        }
        catch (COMException) { return null; }
        catch (InvalidOperationException) { return null; }
        catch (ArgumentException) { return null; }
    }

    private static AutomationElement? FindDesktopWindowAutomationRoot(IntPtr windowHandle)
    {
        if (windowHandle == IntPtr.Zero)
            return null;
        try
        {
            var handle = unchecked((int)windowHandle.ToInt64());
            return AutomationElement.RootElement.FindFirst(
                TreeScope.Children,
                new PropertyCondition(AutomationElement.NativeWindowHandleProperty, handle));
        }
        catch (COMException) { return null; }
        catch (InvalidOperationException) { return null; }
        catch (ArgumentException) { return null; }
    }

    private static WindowSnapTarget PreferSmaller(WindowSnapTarget? current, WindowSnapTarget candidate)
    {
        if (current is null)
            return candidate;
        var currentArea = (long)current.Bounds.Width * current.Bounds.Height;
        var candidateArea = (long)candidate.Bounds.Width * candidate.Bounds.Height;
        return candidateArea < currentArea ? candidate : current;
    }

    private static WindowSnapTarget ToTarget(AutomationElement.AutomationElementInformation info, Rect rectangle, IntPtr fallbackHandle)
    {
        var handle = info.NativeWindowHandle == 0 ? fallbackHandle : new IntPtr(info.NativeWindowHandle);
        return new WindowSnapTarget(handle, ToScreenRect(rectangle));
    }

    private static ScreenRect ToScreenRect(Rect rectangle)
        => new((int)Math.Round(rectangle.Left), (int)Math.Round(rectangle.Top), (int)Math.Round(rectangle.Width), (int)Math.Round(rectangle.Height));

    private static bool IsUsefulElement(AutomationElement element, AutomationElement.AutomationElementInformation info, Rect rectangle, int screenX, int screenY)
    {
        if (info.IsOffscreen || rectangle.Width < MinTargetWidth || rectangle.Height < MinTargetHeight ||
            rectangle.Width > 4096 || rectangle.Height > 4096 || !rectangle.Contains(screenX, screenY) ||
            IsNearMonitorFullScreen(rectangle, screenX, screenY))
            return false;

        var type = info.ControlType;
        if (type is not null && (type == ControlType.Button || type == ControlType.CheckBox || type == ControlType.ComboBox ||
            type == ControlType.Hyperlink || type == ControlType.Image || type == ControlType.ListItem ||
            type == ControlType.MenuItem || type == ControlType.RadioButton || type == ControlType.TabItem ||
            type == ControlType.Edit || type == ControlType.Slider || type == ControlType.SplitButton))
            return true;

        // Custom providers (notably Chromium/Electron) often report Custom
        // for actionable controls. Keep the fallback compact and actionable
        // so a Document/Pane/window cannot become the target.
        if (rectangle.Width <= MaxUsefulControlWidth && rectangle.Height <= MaxUsefulControlHeight &&
            rectangle.Width * rectangle.Height <= MaxUsefulControlArea && info.IsKeyboardFocusable)
            return true;
        try
        {
            return rectangle.Width <= MaxUsefulControlWidth && rectangle.Height <= MaxUsefulControlHeight &&
                rectangle.Width * rectangle.Height <= MaxUsefulControlArea &&
                (element.TryGetCurrentPattern(InvokePattern.Pattern, out _) ||
                 element.TryGetCurrentPattern(TogglePattern.Pattern, out _) ||
                 element.TryGetCurrentPattern(SelectionItemPattern.Pattern, out _) ||
                 element.TryGetCurrentPattern(ValuePattern.Pattern, out _));
        }
        catch (InvalidOperationException) { return false; }
    }

    private static bool IsInsideRoot(Rect rectangle, IntPtr rootWindow)
    {
        if (!TryGetBounds(rootWindow, out var rootBounds))
            return false;
        var root = new Rect(rootBounds.X, rootBounds.Y, rootBounds.Width, rootBounds.Height);
        return root.Contains(rectangle.TopLeft) || root.IntersectsWith(rectangle);
    }

    private static bool IsUsefulWindow(IntPtr handle, ScreenRect bounds, int screenX, int screenY)
    {
        if (!Contains(bounds, screenX, screenY) || bounds.Width < 24 || bounds.Height < 24)
            return false;
        return !IsNearMonitorFullScreen(bounds);
    }

    private static bool IsUsefulNativeChild(ScreenRect child, ScreenRect parent)
    {
        if (child.Width < MinTargetWidth || child.Height < MinTargetHeight)
            return false;
        var childArea = (long)child.Width * child.Height;
        var parentArea = Math.Max(1L, (long)parent.Width * parent.Height);
        // A render-host HWND that fills almost the whole browser is not a
        // button or image. Keep only a materially smaller child.
        return childArea * 100 < parentArea * 92;
    }

    private static bool Contains(ScreenRect bounds, int x, int y)
        => x >= bounds.X && x < bounds.Right && y >= bounds.Y && y < bounds.Bottom;

    private static bool Contains(Rect bounds, int x, int y)
        => x >= bounds.Left && x < bounds.Right && y >= bounds.Top && y < bounds.Bottom;

    private static bool IsNearMonitorFullScreen(ScreenRect bounds)
    {
        try
        {
            var monitor = System.Windows.Forms.Screen.FromPoint(new System.Drawing.Point(bounds.X + bounds.Width / 2, bounds.Y + bounds.Height / 2)).Bounds;
            var monitorArea = Math.Max(1L, (long)monitor.Width * monitor.Height);
            var area = (long)bounds.Width * bounds.Height;
            return area * 100 >= monitorArea * 97;
        }
        catch { return false; }
    }

    private static bool IsNearMonitorFullScreen(Rect bounds, int screenX, int screenY)
    {
        try
        {
            var monitor = System.Windows.Forms.Screen.FromPoint(new System.Drawing.Point(screenX, screenY)).Bounds;
            var monitorArea = Math.Max(1L, (long)monitor.Width * monitor.Height);
            var area = Math.Max(0, bounds.Width) * Math.Max(0, bounds.Height);
            return area * 100 >= monitorArea * 97;
        }
        catch { return false; }
    }

    private static bool TryGetBounds(IntPtr handle, out ScreenRect bounds)
    {
        bounds = default;
        if (handle == IntPtr.Zero || !IsWindow(handle))
            return false;
        NativeRect rectangle;
        try
        {
            if (DwmGetWindowAttribute(handle, 9, out rectangle, Marshal.SizeOf<NativeRect>()) < 0 && !GetWindowRect(handle, out rectangle))
                return false;
        }
        catch (DllNotFoundException)
        {
            if (!GetWindowRect(handle, out rectangle))
                return false;
        }
        var width = rectangle.Right - rectangle.Left;
        var height = rectangle.Bottom - rectangle.Top;
        if (width <= 0 || height <= 0)
            return false;
        bounds = new ScreenRect(rectangle.Left, rectangle.Top, width, height);
        return true;
    }

    private void CacheWindow(IntPtr handle, ScreenRect bounds)
    {
        lock (_cacheGate)
            _windowCache = new WindowHitCache(handle, bounds, Stopwatch.GetTimestamp() + Stopwatch.Frequency * MaxWindowCacheAgeMs / 1000);
    }

    private void CachePrecise(WindowSnapTarget target)
    {
        lock (_cacheGate)
            _preciseCache = new PreciseTargetCache(target, Stopwatch.GetTimestamp() + Stopwatch.Frequency * MaxPreciseCacheAgeMs / 1000);
    }

    private bool TryGetPreciseCache(int screenX, int screenY, out WindowSnapTarget? target)
    {
        lock (_cacheGate)
        {
            var cache = _preciseCache;
            if (cache is not null && Stopwatch.GetTimestamp() <= cache.ExpiresAt && Contains(cache.Target.Bounds, screenX, screenY) && IsWindow(cache.Target.Handle))
            {
                target = cache.Target;
                return true;
            }
            if (cache is not null && Stopwatch.GetTimestamp() > cache.ExpiresAt)
                _preciseCache = null;
        }
        target = null;
        return false;
    }

    private static bool IsDesktopShellWindow(IntPtr handle)
    {
        Span<char> buffer = stackalloc char[128];
        var length = GetClassName(handle, ref MemoryMarshal.GetReference(buffer), buffer.Length);
        if (length <= 0)
            return false;
        var name = new string(buffer[..length]);
        return name is "Progman" or "WorkerW" or "Shell_TrayWnd" or "Shell_SecondaryTrayWnd" or "Windows.UI.Core.CoreWindow";
    }

    private static IntPtr DeepestChildAt(IntPtr root, int screenX, int screenY)
    {
        var current = root;
        for (var depth = 0; depth < 12; depth++)
        {
            var point = new NativePoint { X = screenX, Y = screenY };
            if (!ScreenToClient(current, ref point))
                break;
            var child = ChildWindowFromPointEx(current, point, ChildWindowFlags);
            if (child == IntPtr.Zero || child == current)
                break;
            current = child;
        }
        return current;
    }

    private sealed record WindowHitCache(IntPtr Handle, ScreenRect Bounds, long ExpiresAt);
    private sealed record PreciseTargetCache(WindowSnapTarget Target, long ExpiresAt);
    private delegate bool EnumWindowsCallback(IntPtr handle, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)] private struct NativeRect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct NativePoint { public int X, Y; }
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool IsWindow(IntPtr handle);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr handle);
    [DllImport("user32.dll")] private static extern bool IsIconic(IntPtr handle);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(NativePoint point);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr handle, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr handle, ref char className, int maxCount);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr handle, out NativeRect rectangle);
    [DllImport("user32.dll")] private static extern bool ScreenToClient(IntPtr handle, ref NativePoint point);
    [DllImport("user32.dll")] private static extern IntPtr ChildWindowFromPointEx(IntPtr parent, NativePoint point, uint flags);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr handle, int attribute, out NativeRect value, int valueSize);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr handle, int attribute, out int value, int valueSize);
}

internal sealed record WindowSnapTarget(IntPtr Handle, ScreenRect Bounds);

internal static class SelectionSnapPolicy
{
    internal static Rect SnapResize(Rect value, string directions, Rect target, double threshold)
    {
        if (value.IsEmpty || target.IsEmpty || threshold < 0)
            return value;
        var left = value.Left;
        var top = value.Top;
        var right = value.Right;
        var bottom = value.Bottom;
        if (directions.Contains('W')) left = Nearest(left, target.Left, target.Right, threshold);
        if (directions.Contains('E')) right = Nearest(right, target.Left, target.Right, threshold);
        if (directions.Contains('N')) top = Nearest(top, target.Top, target.Bottom, threshold);
        if (directions.Contains('S')) bottom = Nearest(bottom, target.Top, target.Bottom, threshold);
        return right - left >= 12 && bottom - top >= 12 ? new Rect(new Point(left, top), new Point(right, bottom)) : value;
    }

    private static double Nearest(double value, double first, double second, double threshold)
    {
        var firstDistance = Math.Abs(value - first);
        var secondDistance = Math.Abs(value - second);
        if (firstDistance <= threshold && firstDistance <= secondDistance) return first;
        if (secondDistance <= threshold) return second;
        return value;
    }
}
