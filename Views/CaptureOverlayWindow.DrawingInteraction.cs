// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private double _drawStrokeWidth = 4;
    private double _drawHighlightWidth = 18;

    private Cursor DrawingToolCursor() => _drawTool switch
    {
        DrawTool.Select => Cursors.Arrow,
        DrawTool.Freehand => Cursors.Pen,
        DrawTool.Text => Cursors.IBeam,
        _ => Cursors.Cross
    };

    private void DrawSelectTool(object sender, RoutedEventArgs e)
    {
        SetDrawTool(DrawTool.Select);
        PromptStatus.Text = LocalizationService.T(
            "点击选择标注 · 拖动移动 · 双击编辑文字 · Delete 删除",
            "Click to select · drag to move · double-click to edit text · Delete to remove");
    }

    private void UpdateDrawingContextControls(SelectionItem item)
    {
        if (DrawingPropertiesRow is null) return;
        var selected = SelectedDrawingElement(item);
        var selectedStroke = _drawTool == DrawTool.Select && _selectedDrawingStroke is { } stroke &&
            item.Markup.Strokes.Contains(stroke) ? stroke : null;
        var text = _drawTool == DrawTool.Text || _drawTool == DrawTool.Select && selected is TextDrawingElement;
        var strokeTool = _drawTool is DrawTool.Freehand or DrawTool.Line or DrawTool.Arrow or DrawTool.Rectangle or DrawTool.Ellipse;
        var color = strokeTool || _drawTool is DrawTool.Text or DrawTool.Number or DrawTool.Mark ||
            _drawTool == DrawTool.Select && (SelectedDrawingRegionMark(item) is not null || selectedStroke is not null || selected is TextDrawingElement or NumberDrawingElement);
        DrawingTextControls.Visibility = text ? Visibility.Visible : Visibility.Collapsed;
        DrawingNumberControls.Visibility = _drawTool == DrawTool.Number ? Visibility.Visible : Visibility.Collapsed;
        DrawingColorControls.Visibility = color ? Visibility.Visible : Visibility.Collapsed;
        DrawingStrokeControls.Visibility = strokeTool || _drawTool == DrawTool.Heal || selectedStroke is not null ? Visibility.Visible : Visibility.Collapsed;
        UpdateDrawingToolbarPresentation();
        var wasSyncing = _syncingDrawingControls;
        _syncingDrawingControls = true;
        try
        {
            if (DrawingStrokeWidth.Items.Count == 0)
                foreach (var width in new[] { 2d, 4, 6, 8, 12, 18, 24, 32, 48, 64 }) DrawingStrokeWidth.Items.Add(width);
            var value = selectedStroke?.DrawingAttributes.Width ?? (_drawTool == DrawTool.Heal ? _drawHealWidth : _drawHighlighter ? _drawHighlightWidth : _drawStrokeWidth);
            if (!DrawingStrokeWidth.Items.Contains(value)) DrawingStrokeWidth.Items.Add(value);
            DrawingStrokeWidth.SelectedItem = value;
            EnsureDrawingControls(text);
            UpdateDrawingNumberControls(item);
        }
        finally { _syncingDrawingControls = wasSyncing; }
    }

    private void DrawingStrokeWidthChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_syncingDrawingControls || DrawingStrokeWidth.SelectedItem is not double width ||
            !double.IsFinite(width) || width < 1 || width > 64 || Active is not { } item) return;
        if (_drawTool == DrawTool.Select && _selectedDrawingStroke is { } stroke && item.Markup.Strokes.Contains(stroke))
        {
            if (stroke.DrawingAttributes.Width == width && stroke.DrawingAttributes.Height == width) return;
            var before = stroke.DrawingAttributes.Clone();
            stroke.DrawingAttributes.Width = stroke.DrawingAttributes.Height = width;
            item.DrawingOrder.Add(new StrokeStyleDrawingAction(stroke, before, stroke.DrawingAttributes.Clone()));
            item.DrawingRedo.Clear();MarkDrawingChanged(item);ShowDrawingObjectSelection(item);
            return;
        }
        if (_drawTool == DrawTool.Heal) { _drawHealWidth = width; return; }
        if (_drawHighlighter) _drawHighlightWidth = width;
        else _drawStrokeWidth = width;
        ApplyCurrentDrawingAttributes(item);
    }
}
