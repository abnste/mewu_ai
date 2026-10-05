// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Automation;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal sealed record MemoryInputCandidate(
    AutomationElement? Element,
    Rect Bounds,
    bool IsPassword,
    bool IsReadOnly,
    string ControlTypeName);

internal static class MemoryFillService
{
    internal static IReadOnlyList<MemoryInputCandidate> FindInputs(ApplicationSnapshotTarget target, ScreenRect region, CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if (!target.IsCurrent()) return [];
        var timer = System.Diagnostics.Stopwatch.StartNew();
        var root = AutomationElement.FromHandle(new IntPtr(target.Handle));
        var result = new List<MemoryInputCandidate>();
        var walker = TreeWalker.ControlViewWalker;
        var queue = new Queue<AutomationElement>();
        queue.Enqueue(root);
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var visited = 0;
        while (queue.Count > 0 && result.Count < 128 && visited++ < 10_000)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (timer.Elapsed > TimeSpan.FromSeconds(5)) break;
            var element = queue.Dequeue();
            AutomationElement.AutomationElementInformation info;
            try { info = element.Current; } catch { continue; }
            var bounds = info.BoundingRectangle;
            var controlType = info.ControlType;
            var isInput = controlType == ControlType.Edit || controlType == ControlType.ComboBox || controlType == ControlType.Document;
            if (!info.IsOffscreen && info.IsEnabled && !bounds.IsEmpty && isInput &&
                bounds.IntersectsWith(new Rect(region.X, region.Y, region.Width, region.Height)))
            {
                var intersection = Rect.Intersect(bounds, new Rect(region.X, region.Y, region.Width, region.Height));
                var area = Math.Max(1d, bounds.Width * bounds.Height);
                var overlap = intersection.IsEmpty ? 0d : intersection.Width * intersection.Height / area;
                var tooLarge = bounds.Width > Math.Max(1200d, region.Width * 0.98) || bounds.Height > Math.Max(260d, region.Height * 0.8);
                if (overlap >= .25 && !tooLarge && element.TryGetCurrentPattern(ValuePattern.Pattern, out var rawPattern) && rawPattern is ValuePattern pattern)
                {
                    var identity = $"{bounds.Left:F1}:{bounds.Top:F1}:{bounds.Width:F1}:{bounds.Height:F1}:{info.AutomationId}";
                    if (seen.Add(identity))
                    {
                        var hint = $"{info.Name} {info.AutomationId} {info.HelpText}";
                        var isPassword = info.IsPassword || hint.Contains("password", StringComparison.OrdinalIgnoreCase) || hint.Contains("passwd", StringComparison.OrdinalIgnoreCase) || hint.Contains("密码", StringComparison.OrdinalIgnoreCase);
                        result.Add(new MemoryInputCandidate(element, bounds, isPassword, pattern.Current.IsReadOnly, controlType.ProgrammaticName));
                    }
                }
            }
            try
            {
                for (var child = walker.GetFirstChild(element); child is not null && queue.Count + visited < 10_000; child = walker.GetNextSibling(child))
                {
                    cancellationToken.ThrowIfCancellationRequested();
                    if (timer.Elapsed > TimeSpan.FromSeconds(5)) break;
                    queue.Enqueue(child);
                }
            }
            catch (OperationCanceledException) { throw; }
            catch { }
        }
        return result;
    }

    internal static bool TryFill(MemoryInputCandidate candidate, string value, IntPtr targetWindow)
    {
        if (candidate.IsReadOnly || candidate.Element is null || !BelongsToWindow(candidate.Element, targetWindow)) return false;
        try
        {
            if (candidate.Element.TryGetCurrentPattern(ValuePattern.Pattern, out var pattern) && pattern is ValuePattern valuePattern && !valuePattern.Current.IsReadOnly)
            {
                valuePattern.SetValue(value);
                // Password providers deliberately forbid reading Current.Value.
                return candidate.IsPassword || VerifyValue(candidate.Element, value);
            }
        }
        catch (ElementNotEnabledException) { }
        catch (InvalidOperationException) { }
        catch (System.Runtime.InteropServices.COMException) { }
        try
        {
            // Never append to an existing value, or guess a password field's contents.
            if (candidate.IsPassword || !VerifyValue(candidate.Element, string.Empty)) return false;
            if (!NativeMethods.SetForegroundWindow(targetWindow)) return false;
            candidate.Element.SetFocus();
            if (!candidate.Element.Equals(AutomationElement.FocusedElement)) return false;
            return NativeMethods.TrySendUnicodeText(targetWindow,value) && VerifyValue(candidate.Element, value);
        }
        catch(InvalidOperationException){return false;}
        catch(System.Runtime.InteropServices.COMException){return false;}
    }

    private static bool BelongsToWindow(AutomationElement element, IntPtr targetWindow)
    {
        if (targetWindow == IntPtr.Zero || !NativeMethods.IsWindow(targetWindow)) return false;
        try
        {
            var current = element;
            for (var depth = 0; current is not null && depth < 64; current = TreeWalker.ControlViewWalker.GetParent(current), depth++)
                if (new IntPtr(current.Current.NativeWindowHandle) == targetWindow) return true;
        }
        catch (Exception ex) when (ex is InvalidOperationException or System.Runtime.InteropServices.COMException) { }
        return false;
    }

    private static bool VerifyValue(AutomationElement element, string expected)
    {
        try
        {
            return element.TryGetCurrentPattern(ValuePattern.Pattern, out var raw) && raw is ValuePattern pattern &&
                   string.Equals(pattern.Current.Value, expected, StringComparison.Ordinal);
        }
        catch { return false; }
    }

    internal static double Score(MemoryEntry entry, MemoryInputCandidate candidate, Point anchor, ScreenRect region)
    {
        var maxDistance = Math.Max(1d, Math.Max(region.Width, region.Height));
        var x = Math.Clamp(anchor.X, candidate.Bounds.Left, candidate.Bounds.Right);
        var y = Math.Clamp(anchor.Y, candidate.Bounds.Top, candidate.Bounds.Bottom);
        var distance = Math.Sqrt(Math.Pow(anchor.X - x, 2) + Math.Pow(anchor.Y - y, 2));
        var geometry = 1d - Math.Clamp(distance / maxDistance, 0d, 1d);
        var kind = (entry.FieldKind ?? "auto").Trim().ToLowerInvariant();
        var type = kind switch
        {
            "password" => candidate.IsPassword ? 1d : 0d,
            "username" or "email" or "text" => candidate.IsPassword ? .35d : 1d,
            _ => candidate.IsPassword == entry.Sensitive ? 1d : .7d
        };
        var row = Math.Abs(anchor.Y - (candidate.Bounds.Top + candidate.Bounds.Height / 2d)) <= Math.Max(28d, candidate.Bounds.Height * 1.8) ? .15d : 0d;
        return Math.Clamp(geometry * .65d + type * .25d + row, 0d, 1d);
    }
}
