// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Controls;
using System.Windows.Input;
using Point = System.Windows.Point;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private Point? _lastEraserPoint;

    private void BeginEraserDrawingGesture(SelectionItem item, Point point, InkCanvas canvas)
    {
        canvas.Focus();
        Keyboard.Focus(canvas);
        // WPF CaptureMouse synchronously resynchronizes the pointer and can
        // reenter MouseMove. Initialize first so that synthetic same-position
        // move cannot erase an object before this initial erase does.
        _lastEraserPoint = point;
        canvas.CaptureMouse();
        EraseDrawingObjectsAt(item, point);
    }

    private void ContinueEraserDrawingGesture(SelectionItem item, Point point)
    {
        // A missing origin means the gesture already ended/lost capture. Move
        // events must never start another erase, including capture resyncs.
        if (_lastEraserPoint is not { } previous || (point - previous).Length < 12) return;
        // Removing visuals can also resynchronize input. Advance before doing
        // that work, preserving continuous drag erasure without same-point
        // penetration through another annotation underneath.
        _lastEraserPoint = point;
        EraseDrawingObjectsAt(item, point);
    }
}
