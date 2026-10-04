// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Point = System.Windows.Point;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private sealed class BackgroundHighlightCache
    {
        internal BitmapSource? Frame;
        internal BitmapSource? Replacement;
        internal Rect Bounds;
        internal Rect SourceBounds;
        internal MosaicDrawingElement[] Elements = [];
        internal BackgroundHighlightSource? Source;
        internal BackgroundHighlightSource? LastAppliedSource;
        internal Rect LastAppliedSourceBounds;
        internal bool Failed;
    }

    private readonly ConditionalWeakTable<SelectionItem, BackgroundHighlightCache> _backgroundHighlightSources = new();
    private (SelectionItem Item, BackgroundHighlightStroke Stroke)? _backgroundHighlightPreview;

    private BackgroundHighlightSource GetBackgroundHighlightSource(SelectionItem item)
    {
        var cache = _backgroundHighlightSources.GetValue(item, static _ => new BackgroundHighlightCache());
        // Reframing is a geometry preview. Keep its existing source pixels
        // anchored while the pointer moves; recompute once after the gesture.
        if (cache.Source is not null && ((_moving && ReferenceEquals(item, Active)) || ReferenceEquals(_resizeTarget, item)))
            return cache.Source;
        var elements = BackgroundHighlightRasterElements(item);
        if (cache.Source is not null && ReferenceEquals(cache.Frame, _frame.Image) &&
            ReferenceEquals(cache.Replacement, item.CapturedImageOverride) && cache.Bounds == item.Bounds &&
            cache.Elements.Length == elements.Length && cache.Elements.Zip(elements).All(pair => ReferenceEquals(pair.First, pair.Second))) return cache.Source;
        cache.Frame = _frame.Image; cache.Replacement = item.CapturedImageOverride; cache.Bounds = item.Bounds;
        cache.Elements = elements;
        cache.SourceBounds = new Rect(0, 0, Math.Max(1, item.Bounds.Width), Math.Max(1, item.Bounds.Height));
        try
        {
            // Reject oversized sources before raster composition allocates them.
            var pixels = ToPixelRect(item.Bounds);
            BackgroundHighlightService.EnsureSupportedDimensions(
                item.CapturedImageOverride?.PixelWidth ?? pixels.Width,
                item.CapturedImageOverride?.PixelHeight ?? pixels.Height);
            var raster = RenderSeamlessLiftSource(item);
            cache.Source = BackgroundHighlightService.CreateSource(raster,
                Math.Max(raster.PixelWidth / Math.Max(1, item.Bounds.Width), raster.PixelHeight / Math.Max(1, item.Bounds.Height)));
            cache.Failed = false;
        }
        catch (Exception)
        {
            // A derived mask must never make a geometry/history operation fail.
            // Cache this failure fingerprint so pointer updates do not retry it.
            cache.Source = BackgroundHighlightService.TransparentSource;
            cache.Failed = true;
            ShowBackgroundHighlightFailure();
        }
        return cache.Source;
    }

    private Rect BackgroundHighlightSourceBounds(SelectionItem item)
    {
        GetBackgroundHighlightSource(item);
        return _backgroundHighlightSources.GetValue(item, static _ => new BackgroundHighlightCache()).SourceBounds;
    }

    private void RebaseBackgroundHighlightSource(SelectionItem item, Vector offset)
    {
        if (_backgroundHighlightSources.TryGetValue(item, out var cache) && cache.Source is not null)
            cache.SourceBounds.Offset(offset);
    }

    private void ShowBackgroundHighlightFailure() => SeamlessEraseFailed(new InvalidOperationException(),
        _drawTool == DrawTool.Mark ? DrawingMarkButton : DrawingHighlightButton,
        LocalizationService.T("截图范围过大或无法生成高亮，请缩小范围重试。",
            "The capture is too large or highlighting is unavailable. Select a smaller area and try again."));

    private bool CanBeginBackgroundHighlight(SelectionItem item)
    {
        GetBackgroundHighlightSource(item);
        if (!_backgroundHighlightSources.GetValue(item, static _ => new BackgroundHighlightCache()).Failed) return true;
        ShowBackgroundHighlightFailure();
        return false;
    }

    private void RefreshBackgroundHighlightSources(SelectionItem item)
    {
        var strokes = item.Markup.Strokes.OfType<BackgroundHighlightStroke>().ToArray();
        if (strokes.Length == 0 && item.RegionMarks.Count == 0) return;
        var source = GetBackgroundHighlightSource(item);
        var cache = _backgroundHighlightSources.GetValue(item, static _ => new BackgroundHighlightCache());
        foreach (var stroke in strokes) stroke.UpdateSource(source, cache.SourceBounds);
        if (!ReferenceEquals(cache.LastAppliedSource, source) || cache.LastAppliedSourceBounds != cache.SourceBounds)
        {
            cache.LastAppliedSource = source;
            cache.LastAppliedSourceBounds = cache.SourceBounds;
            if (item.RegionMarks.Count > 0) RenderRegionMarks(item);
        }
    }

    private void ReleaseInactiveBackgroundHighlightSources(SelectionItem item)
    {
        var live = item.Markup.Strokes.OfType<BackgroundHighlightStroke>().ToHashSet();
        foreach (var action in item.DrawingOrder.Concat(item.DrawingRedo))
        {
            switch (action)
            {
                case StrokeDrawingAction value: Release(value.Stroke); break;
                case StrokeRemovalDrawingAction value: Release(value.Stroke); break;
                case StrokeMoveDrawingAction value: Release(value.Stroke); break;
                case StrokeStyleDrawingAction value: Release(value.Stroke); break;
                case ClearDrawingAction value:
                    foreach (var stroke in value.Strokes) Release(stroke);
                    break;
            }
        }
        if (live.Count == 0 && item.RegionMarks.Count == 0) _backgroundHighlightSources.Remove(item);

        void Release(Stroke stroke)
        {
            if (stroke is BackgroundHighlightStroke highlight && !live.Contains(highlight)) highlight.ReleaseDerivedSource();
        }
    }

    private bool BeginBackgroundHighlightPreview(SelectionItem item, Point point)
    {
        CancelBackgroundHighlightPreview();
        if (!CanBeginBackgroundHighlight(item)) return true;
        var source = GetBackgroundHighlightSource(item);
        var attributes = item.Markup.DefaultDrawingAttributes.Clone();
        attributes.Width = attributes.Height = _drawHighlightWidth;
        var stroke = new BackgroundHighlightStroke(new StylusPointCollection { new StylusPoint(point.X, point.Y) }, attributes,
            source, BackgroundHighlightSourceBounds(item));
        _backgroundHighlightPreview = (item, stroke);
        item.Markup.EditingMode = InkCanvasEditingMode.None;
        item.Markup.Strokes.Add(stroke);
        if (!item.Markup.CaptureMouse()) CancelBackgroundHighlightPreview();
        return true;
    }

    private void UpdateBackgroundHighlightPreview(SelectionItem item, Point point)
    {
        if (_backgroundHighlightPreview is not { } preview || !ReferenceEquals(preview.Item, item)) return;
        point = new Point(Math.Clamp(point.X, 0, Math.Max(0, item.Bounds.Width)), Math.Clamp(point.Y, 0, Math.Max(0, item.Bounds.Height)));
        var previous = preview.Stroke.StylusPoints[^1];
        if ((point - new Point(previous.X, previous.Y)).Length < .5) return;
        preview.Stroke.StylusPoints.Add(new StylusPoint(point.X, point.Y));
    }

    private void CommitBackgroundHighlightPreview(SelectionItem item, Point point)
    {
        UpdateBackgroundHighlightPreview(item, point);
        if (_backgroundHighlightPreview is not { } preview || !ReferenceEquals(preview.Item, item)) return;
        CommitBackgroundHighlightPreviewOnCaptureLoss();
    }

    private void CommitBackgroundHighlightPreviewOnCaptureLoss()
    {
        if (_backgroundHighlightPreview is not { } preview) return;
        _backgroundHighlightPreview = null;
        if (!preview.Item.Markup.Strokes.Contains(preview.Stroke)) return;
        preview.Item.DrawingOrder.Add(new StrokeDrawingAction(preview.Stroke));
        preview.Item.DrawingRedo.Clear();
        MarkDrawingChanged(preview.Item);
    }

    private void CancelBackgroundHighlightPreview()
    {
        if (_backgroundHighlightPreview is not { } preview) return;
        _backgroundHighlightPreview = null;
        preview.Item.Markup.Strokes.Remove(preview.Stroke);
    }
}
