// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using Point = System.Windows.Point;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private sealed class SeamlessEraseDrawingPreview(SelectionItem item, BitmapSource source, double scaleX, double scaleY)
    {
        internal SelectionItem Item { get; } = item;
        internal BitmapSource Source { get; } = source;
        internal double ScaleX { get; } = scaleX;
        internal double ScaleY { get; } = scaleY;
        internal Border Outline { get; } = new()
        {
            BorderBrush = new SolidColorBrush(Color.FromRgb(49, 140, 255)), BorderThickness = new Thickness(1),
            Background = new SolidColorBrush(Color.FromArgb(20, 49, 140, 255)),
            IsHitTestVisible = false, Visibility = Visibility.Collapsed
        };
        internal Int32Rect LastRegion { get; set; }
    }

    private SeamlessEraseDrawingPreview? _drawingSeamlessErasePreview;
    private ToolTip? _seamlessEraseFailureTip;
    private DispatcherTimer? _seamlessEraseFailureTimer;

    private void BeginSeamlessEraseDrawingPreview(SelectionItem item)
    {
        CancelMosaicDrawingPreview();
        try
        {
            var source = RenderSeamlessLiftSource(item);
            if (!source.IsFrozen) source.Freeze();
            var preview = new SeamlessEraseDrawingPreview(item, source,
                source.PixelWidth / Math.Max(1, item.Bounds.Width), source.PixelHeight / Math.Max(1, item.Bounds.Height));
            _drawingSeamlessErasePreview = preview;
            item.Markup.Children.Add(preview.Outline);
        }
        catch (Exception error) { SeamlessEraseFailed(error); }
    }

    private BitmapSource RenderSeamlessLiftSource(SelectionItem item)
    {
        var source = RenderSelectionImage(item, false, false, false);
        var elements = item.DrawingElements.OfType<MosaicDrawingElement>().ToArray();
        if (elements.Length == 0) return source;
        // Lift from the current raster state: an earlier repaired area stays
        // empty and content already moved elsewhere can be selected there.
        // Editable ink/text and transient selection adorners remain separate.
        var size = new Size(Math.Max(1, item.Bounds.Width), Math.Max(1, item.Bounds.Height));
        var content = new InkCanvas { Width = size.Width, Height = size.Height, Background = Brushes.Transparent };
        foreach (var element in elements) content.Children.Add(CreateMosaicVisual(item, element));
        content.Measure(size); content.Arrange(new Rect(size)); content.UpdateLayout();
        var brush = new VisualBrush(content)
        { ViewboxUnits = BrushMappingMode.Absolute, Viewbox = new Rect(size), Stretch = Stretch.Fill };
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.PushTransform(new ScaleTransform(source.PixelWidth / size.Width, source.PixelHeight / size.Height));
            drawing.DrawRectangle(brush, null, new Rect(size));
            drawing.Pop();
        }
        var bitmap = new RenderTargetBitmap(source.PixelWidth, source.PixelHeight, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(visual); bitmap.Freeze();
        return AnnotationOverlayRenderer.Composite(source, bitmap);
    }

    private void UpdateSeamlessEraseDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingSeamlessErasePreview is not { } preview || !ReferenceEquals(preview.Item, item)) return;
        var bounds = MosaicDrawingBounds(item, _drawStart, point);
        if (bounds.Width < 3 || bounds.Height < 3) { preview.Outline.Visibility = Visibility.Collapsed; preview.LastRegion = default; return; }
        preview.LastRegion = MosaicPixelBounds(bounds, preview.ScaleX, preview.ScaleY, preview.Source.PixelWidth, preview.Source.PixelHeight);
        preview.Outline.Visibility = Visibility.Visible;
        preview.Outline.Width = bounds.Width;
        preview.Outline.Height = bounds.Height;
        InkCanvas.SetLeft(preview.Outline, bounds.X);
        InkCanvas.SetTop(preview.Outline, bounds.Y);
    }

    private void CommitSeamlessEraseDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingSeamlessErasePreview is not { } preview || !ReferenceEquals(preview.Item, item))
        { CancelSeamlessEraseDrawingPreview(); return; }
        Exception? failure = null;
        try
        {
            var bounds = MosaicDrawingBounds(item, _drawStart, point);
            if (bounds.Width < 3 || bounds.Height < 3) return;
            var region = MosaicPixelBounds(bounds, preview.ScaleX, preview.ScaleY, preview.Source.PixelWidth, preview.Source.PixelHeight);
            var bitmap = SeamlessEraseService.CreatePatch(preview.Source, region);
            var pixels = SeamlessEraseService.ExtractContent(preview.Source, region, bitmap);
            if (!HasLiftedContent(pixels)) return;
            // Both layers exactly cover the sampled physical pixels, even at
            // fractional monitor scales. Their caches never sample a new destination.
            bounds = new Rect(region.X / preview.ScaleX, region.Y / preview.ScaleY,
                region.Width / preview.ScaleX, region.Height / preview.ScaleY);
            var background = new MosaicDrawingElement(Guid.NewGuid(), bounds.X, bounds.Y, bounds.Width, bounds.Height)
            { Pixels = bitmap, SeamlessErase = true };
            var content = new MosaicDrawingElement(Guid.NewGuid(), bounds.X, bounds.Y, bounds.Width, bounds.Height)
            { Pixels = pixels, IsLiftedContent = true };
            var backgroundVisual = CreateMosaicVisual(item, background);
            var contentVisual = CreateMosaicVisual(item, content);
            CancelSeamlessEraseDrawingPreview();
            item.DrawingElements.Add(background);
            item.DrawingElements.Add(content);
            item.DrawingOrder.Add(new LiftDrawingAction(background, content));
            item.DrawingRedo.Clear();
            item.RasterLayer.Children.Add(backgroundVisual);
            item.RasterLayer.Children.Add(contentVisual);
            MarkDrawingChanged(item);
            SetDrawTool(DrawTool.Select);
            _selectedDrawingElementId = content.Id;
            SyncDrawingPropertyControls(item);
            ShowDrawingObjectSelection(item);
        }
        catch (Exception error) { failure = error; }
        finally { CancelSeamlessEraseDrawingPreview(); }
        if (failure is not null) SeamlessEraseFailed(failure);
    }

    private void CancelSeamlessEraseDrawingPreview()
    {
        var preview = _drawingSeamlessErasePreview;
        _drawingSeamlessErasePreview = null;
        if (preview is null) return;
        preview.Item.Markup.Children.Remove(preview.Outline);
    }

    private static bool HasLiftedContent(BitmapSource bitmap)
    {
        var row = new byte[checked(bitmap.PixelWidth * 4)];
        try
        {
            for (var y = 0; y < bitmap.PixelHeight; y++)
            {
                bitmap.CopyPixels(new Int32Rect(0, y, bitmap.PixelWidth, 1), row, row.Length, 0);
                for (var offset = 3; offset < row.Length; offset += 4)
                    if (row[offset] != 0) return true;
            }
            return false;
        }
        finally { Array.Clear(row); }
    }

    private void DismissSeamlessEraseFailure()
    {
        _seamlessEraseFailureTimer?.Stop();
        _seamlessEraseFailureTimer = null;
        if (_seamlessEraseFailureTip is { } tip) tip.IsOpen = false;
        _seamlessEraseFailureTip = null;
    }

    private void SeamlessEraseFailed(Exception error, Button? anchor = null, string? message = null)
    {
        DismissSeamlessEraseFailure();
        CancelSeamlessEraseDrawingPreview();
        if (error is not InvalidOperationException) new PrivacyLogger().Error("DrawingSeamlessErase", error);
        PromptStatus.Text = message ?? LocalizationService.T("无法补齐背景，请缩小提取范围，在四周留出一些背景后重试。",
            "Unable to reconstruct the background. Reduce the area and leave some surrounding background, then retry.");
        // The prompt bar is hidden during drawing. Only a failed action needs a
        // transient message; the second row stays free of placeholder text.
        anchor ??= DrawingSeamlessEraseButton;
        if (!IsVisible || !anchor.IsVisible) return;
        _seamlessEraseFailureTip = new ToolTip
        {
            Content = new TextBlock { Text = PromptStatus.Text, MaxWidth = 300, TextWrapping = TextWrapping.Wrap },
            PlacementTarget = anchor, Placement = PlacementMode.Top,
            StaysOpen = true, IsOpen = true
        };
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(3) };
        _seamlessEraseFailureTimer = timer;
        timer.Tick += (_, _) => { timer.Stop(); if (ReferenceEquals(_seamlessEraseFailureTimer, timer)) DismissSeamlessEraseFailure(); };
        timer.Start();
    }
}
