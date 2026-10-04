// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using mewu_ai_Assistant.Views;
using MouseEventHandler = System.Windows.Input.MouseEventHandler;
using Point = System.Windows.Point;

/// <summary>
/// Replays capture-time reentrancy in the real eraser dispatcher. No global
/// input is injected; this supplements, but never replaces, the mouse test.
/// </summary>
internal static class DrawingEraserReplay
{
    internal static void Verify(CaptureOverlayWindow overlay, object item, Action<bool, string> require)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.DeclaredOnly;
        object? Invoke(string name, params object?[] args) =>
            typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, args);
        object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, flags)!.GetValue(overlay);
        var itemType = item.GetType();
        var canvas = (InkCanvas)itemType.GetProperty("Markup")!.GetValue(item)!;
        var order = (IList)itemType.GetProperty("DrawingOrder")!.GetValue(item)!;
        var marks = (IList)itemType.GetField("RegionMarks")!.GetValue(item)!;
        var point = new Point(100, 70);
        void Continue(Point at) => Invoke("ContinueEraserDrawingGesture", item, at);
        void Begin() => Invoke("BeginDrawingGesture", item, point, canvas, 1, canvas);
        void Up() => Invoke("MarkupUp", canvas, new MouseButtonEventArgs(Mouse.PrimaryDevice, Environment.TickCount, MouseButton.Left)
            { RoutedEvent = Mouse.PreviewMouseUpEvent });
        void Undo() => Invoke("DrawUndo", overlay, new RoutedEventArgs());
        void Redo() => Invoke("DrawRedo", overlay, new RoutedEventArgs());

        Invoke("DrawClear", overlay, new RoutedEventArgs());
        Invoke("DrawEraser", overlay, new RoutedEventArgs());
        Invoke("AddRegionMark", item, new Rect(40, 40, 180, 100));
        Invoke("AddRegionMark", item, new Rect(50, 45, 170, 100));
        var lowerMark = marks[0];
        var upperMark = marks[1];
        var stroke = new Stroke(new StylusPointCollection
        {
            new StylusPoint(60, 70), new StylusPoint(200, 70)
        }, new DrawingAttributes { Color = Colors.Red, Width = 6, Height = 6, FitToCurve = false });
        canvas.Strokes.Add(stroke);
        var before = order.Count;

        var captureCallbacks = 0;
        MouseEventHandler onCapture = (_, _) =>
        {
            captureCallbacks++;
            require(Equals(Field("_lastEraserPoint"), point), "eraser origin is initialized before WPF capture callbacks");
            // CaptureMouse may synchronously deliver MouseMove before returning
            // to the down handler. Replay that continuation at the same point.
            Continue(point);
            require(canvas.Strokes.Contains(stroke) && marks.Count == 2 && order.Count == before,
                "capture-time same-position move cannot erase before the down handler");
        };
        canvas.GotMouseCapture += onCapture;
        try { Begin(); }
        finally { canvas.GotMouseCapture -= onCapture; }
        require(captureCallbacks == 1 && canvas.IsMouseCaptured, "eraser regression observes the actual WPF capture transition");
        require(!canvas.Strokes.Contains(stroke) && marks.Count == 2 && order.Count == before + 1,
            "one eraser down removes only the top stroke and records exactly one action");
        Continue(point);
        Continue(new Point(point.X + 9, point.Y));
        require(marks.Count == 2 && order.Count == before + 1,
            "same-position and sub-threshold moves cannot penetrate to the lower marks");
        Up();
        Continue(new Point(point.X + 40, point.Y));
        require(!canvas.IsMouseCaptured && Field("_lastEraserPoint") is null && marks.Count == 2 && order.Count == before + 1,
            "eraser up and a later move do not erase or create another history action");
        Undo();
        require(canvas.Strokes.Contains(stroke) && marks.Count == 2 && order.Count == before,
            "one undo restores the single clicked stroke and leaves both marks intact");
        Redo();
        require(!canvas.Strokes.Contains(stroke) && marks.Count == 2 && order.Count == before + 1,
            "one redo removes only the same stroke");

        Begin();
        require(marks.Count == 1 && ReferenceEquals(marks[0], lowerMark) && order.Count == before + 2,
            "the next independent click erases only the upper overlapping region");
        Continue(new Point(point.X + 40, point.Y));
        require(marks.Count == 0 && order.Count == before + 3,
            "real displacement beyond the eraser threshold still supports continuous drag erasure");
        Up();
        Undo();
        require(marks.Count == 1 && ReferenceEquals(marks[0], lowerMark), "drag erase undo restores the lower region");
        Undo();
        require(marks.Count == 2 && ReferenceEquals(marks[0], lowerMark) && ReferenceEquals(marks[1], upperMark),
            "click erase undo restores the overlapping regions in their original order");
    }
}
