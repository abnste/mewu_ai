// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;

namespace mewu_ai_Assistant.Services;

internal sealed class BackgroundHighlightStroke : Stroke
{
    private BackgroundHighlightSource _source;
    private Rect _sourceBounds;

    internal BackgroundHighlightStroke(StylusPointCollection points, DrawingAttributes attributes,
        BackgroundHighlightSource source, Rect sourceBounds) : base(points, attributes.Clone())
    {
        _source = source;
        _sourceBounds = sourceBounds;
        // The custom draw owns opacity; native highlighter grouping must not
        // apply a second opacity to the protected pixels.
        DrawingAttributes.IsHighlighter = false;
    }

    internal void UpdateSource(BackgroundHighlightSource source, Rect sourceBounds)
    {
        if (ReferenceEquals(_source, source) && _sourceBounds == sourceBounds) return;
        _source = source;
        _sourceBounds = sourceBounds;
        OnInvalidated(EventArgs.Empty);
    }

    internal void RebaseSource(Vector offset)
    {
        _sourceBounds.Offset(offset);
        OnInvalidated(EventArgs.Empty);
    }

    // History snapshots retain geometry, not a full-size derived source mask.
    // RebuildDrawingElements refreshes this from the restored raster on apply.
    internal void ReleaseDerivedSource() => _source = BackgroundHighlightService.TransparentSource;

    public override Stroke Clone() => base.Clone();

    protected override void DrawCore(DrawingContext drawingContext, DrawingAttributes drawingAttributes)
    {
        if (_sourceBounds.IsEmpty || _sourceBounds.Width <= 0 || _sourceBounds.Height <= 0) return;
        var brush = _source.CreateTintBrush(drawingAttributes.Color, 84, _sourceBounds);
        drawingContext.PushOpacityMask(_source.CreateOpacityBrush(_sourceBounds));
        drawingContext.DrawGeometry(brush, null, GetGeometry(drawingAttributes));
        drawingContext.Pop();
    }
}
