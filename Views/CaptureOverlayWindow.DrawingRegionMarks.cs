// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using mewu_ai_Assistant.Services;
using Point = System.Windows.Point;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private RegionMark? _selectedDrawingRegionMark;
    private RegionMark? _drawingMoveOriginalRegionMark;
    private sealed record RegionMarkGeometryDrawingAction(RegionMark Mark, RegionMark Before, RegionMark After) : DrawingAction;
    private sealed record RegionMarkColorDrawingAction(RegionMark Mark, Color Before, Color After) : DrawingAction;

    private RegionMark? SelectedDrawingRegionMark(SelectionItem item) =>
        _selectedDrawingRegionMark is { } mark && item.RegionMarks.Contains(mark) ? mark : null;

    private bool BeginRegionMarkSelection(SelectionItem item, Point point, InkCanvas canvas)
    {
        var mark = item.RegionMarks.LastOrDefault(candidate => candidate.OnScreen && candidate.Bounds.Contains(point));
        if (mark is null) return false;
        InvalidateRegionMarkMatching(item);
        _selectedDrawingRegionMark = mark;
        _drawingMoveOriginalRegionMark = CloneRegionMark(mark);
        _drawingMovePointerStart = point;
        SyncDrawingPropertyControls(item);
        ShowDrawingObjectSelection(item);
        canvas.EditingMode = InkCanvasEditingMode.None;
        canvas.CaptureMouse();
        PromptStatus.Text = LocalizationService.T("重点高亮已选中 · 拖动移动、拖角调整 · 可改色或删除", "Highlight selected · drag to move or resize · change color or delete");
        return true;
    }

    private bool MoveSelectedRegionMark(SelectionItem item, Point point, InkCanvas canvas)
    {
        if (_drawingMoveOriginalRegionMark is not { } before || SelectedDrawingRegionMark(item) is not { } mark) return false;
        var requested = point - _drawingMovePointerStart;
        // Reframing can leave part of a highlight outside the crop. Preserve
        // that existing overflow instead of snapping an untouched axis inward.
        var leftDelta = -before.Bounds.Left;
        var rightDelta = canvas.ActualWidth - before.Bounds.Right;
        var topDelta = -before.Bounds.Top;
        var bottomDelta = canvas.ActualHeight - before.Bounds.Bottom;
        var delta = new Vector(
            Math.Clamp(requested.X, Math.Min(0, Math.Min(leftDelta, rightDelta)), Math.Max(0, Math.Max(leftDelta, rightDelta))),
            Math.Clamp(requested.Y, Math.Min(0, Math.Min(topDelta, bottomDelta)), Math.Max(0, Math.Max(topDelta, bottomDelta))));
        var bounds = before.Bounds;
        bounds.Offset(delta);
        mark.Bounds = bounds;
        RenderRegionMarks(item);
        return true;
    }

    private bool BeginRegionMarkResize(SelectionItem item)
    {
        if (SelectedDrawingRegionMark(item) is not { } mark) return false;
        InvalidateRegionMarkMatching(item);
        _drawingMoveOriginalRegionMark = CloneRegionMark(mark);
        _drawingResizeOriginalBounds = mark.Bounds;
        return true;
    }

    private bool ResizeSelectedRegionMark(SelectionItem item, Rect bounds)
    {
        if (_drawingMoveOriginalRegionMark is null || SelectedDrawingRegionMark(item) is not { } mark) return false;
        mark.Bounds = bounds;
        RenderRegionMarks(item);
        return true;
    }

    private static Rect ResizeRegionMarkBounds(Rect original, int corner, Point pointer, Size canvas, bool constrain)
    {
        if (!constrain) return DrawingAnnotationGeometry.ResizeCorner(original, corner, pointer, canvas);
        // The fixed opposite corner may be outside the current crop. Include
        // the existing extent when applying the shared square constraint.
        var origin = new Vector(Math.Min(0, original.Left), Math.Min(0, original.Top));
        var expanded = new Size(Math.Max(canvas.Width, original.Right) - origin.X,
            Math.Max(canvas.Height, original.Bottom) - origin.Y);
        original.Offset(-origin);
        var resized = DrawingAnnotationGeometry.ResizeSquareCorner(original, corner, pointer - origin, expanded);
        resized.Offset(origin);
        return resized;
    }

    private bool CommitSelectedRegionMarkMove(SelectionItem item)
    {
        if (_drawingMoveOriginalRegionMark is not { } before || SelectedDrawingRegionMark(item) is not { } mark) return false;
        if (mark.Bounds == before.Bounds) return true;
        InvalidateRegionMarkMatching(item);
        // A deliberate edit anchors to the pixels at its new bounds. Undo must
        // restore the old template, not silently sample the current frame.
        mark.OnScreen = true;
        mark.Template = null;
        mark.TemplateWidth = mark.TemplateHeight = 0;
        mark.TemplateStepX = mark.TemplateStepY = 1;
        mark.TemplateOffsetX = mark.TemplateOffsetY = 0;
        try { CaptureRegionMarkTemplate(RenderSelectionImage(item, false, false, false), item, mark); }
        catch (Exception error) { new PrivacyLogger().Error("RegionMarkTemplate", error); }
        item.DrawingOrder.Add(new RegionMarkGeometryDrawingAction(mark, before, CloneRegionMark(mark)));
        item.DrawingRedo.Clear();
        MarkDrawingChanged(item);
        SyncDrawingPropertyControls(item);
        ShowDrawingObjectSelection(item);
        PromptStatus.Text = LocalizationService.T("重点高亮已调整 · Ctrl+Z 撤销", "Highlight adjusted · Ctrl+Z to undo");
        return true;
    }

    private bool ChangeSelectedRegionMarkColor(SelectionItem item)
    {
        if (_drawTool != DrawTool.Select || SelectedDrawingRegionMark(item) is not { } mark) return false;
        if (mark.Color == _drawColor) return true;
        InvalidateRegionMarkMatching(item);
        var before = mark.Color;
        mark.Color = _drawColor;
        item.DrawingOrder.Add(new RegionMarkColorDrawingAction(mark, before, mark.Color));
        item.DrawingRedo.Clear();
        RenderRegionMarks(item);
        MarkDrawingChanged(item);
        ShowDrawingObjectSelection(item);
        return true;
    }

    private bool DeleteSelectedRegionMark(SelectionItem item)
    {
        if (SelectedDrawingRegionMark(item) is not { } mark) return false;
        InvalidateRegionMarkMatching(item);
        var index = item.RegionMarks.IndexOf(mark);
        item.RegionMarks.Remove(mark);
        item.DrawingOrder.Add(new RegionMarkDrawingAction(mark, true, index));
        item.DrawingRedo.Clear();
        ClearDrawingObjectSelection();
        RenderRegionMarks(item);
        MarkDrawingChanged(item);
        PromptStatus.Text = LocalizationService.T("已删除重点高亮", "Highlight deleted");
        return true;
    }

    private static void ApplyRegionMarkGeometry(RegionMark mark, RegionMark geometry)
    {
        mark.Bounds = geometry.Bounds;
        mark.OnScreen = geometry.OnScreen;
        mark.Template = geometry.Template?.ToArray();
        mark.TemplateWidth = geometry.TemplateWidth;
        mark.TemplateHeight = geometry.TemplateHeight;
        mark.TemplateStepX = geometry.TemplateStepX;
        mark.TemplateStepY = geometry.TemplateStepY;
        mark.TemplateOffsetX = geometry.TemplateOffsetX;
        mark.TemplateOffsetY = geometry.TemplateOffsetY;
    }
}
