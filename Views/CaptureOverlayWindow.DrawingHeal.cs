// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Point = System.Windows.Point;
using Path = System.Windows.Shapes.Path;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private double _drawHealWidth = 32;
    private HealingDrawingPreview? _drawingHealPreview;
    private CancellationTokenSource? _drawingHealRequest;

    private sealed class HealingDrawingPreview(SelectionItem item, BitmapSource source, Stroke stroke)
    {
        internal SelectionItem Item { get; } = item;
        internal BitmapSource Source { get; } = source;
        internal Stroke Stroke { get; } = stroke;
        internal Path Visual { get; } = new()
        { Fill = new SolidColorBrush(Color.FromArgb(90, 49, 140, 255)), IsHitTestVisible = false };
    }

    private void BeginHealingDrawingPreview(SelectionItem item, Point point)
    {
        CancelMosaicDrawingPreview();
        if (item.VideoPath is not null)
        {
            PromptStatus.Text = LocalizationService.T("涂抹消除用于截图，请先截取视频画面。", "Use the healing brush on a captured image.");
            return;
        }
        try
        {
            var source = RenderSeamlessLiftSource(item);
            if (!source.IsFrozen) source.Freeze();
            point = ClampHealingPoint(item, point);
            var stroke = new Stroke(new StylusPointCollection { new StylusPoint(point.X, point.Y) },
                new DrawingAttributes { Color = Colors.White, Width = _drawHealWidth, Height = _drawHealWidth, FitToCurve = false });
            var preview = new HealingDrawingPreview(item, source, stroke);
            preview.Visual.Data = stroke.GetGeometry();
            _drawingHealPreview = preview;
            item.Markup.Children.Add(preview.Visual);
            item.Markup.CaptureMouse();
            if (!item.Markup.IsMouseCaptured) CancelHealingDrawingPreview();
        }
        catch (Exception error) { HealingDrawingFailed(error); }
    }

    private static Point ClampHealingPoint(SelectionItem item, Point point) =>
        new(Math.Clamp(point.X, 0, Math.Max(0, item.Bounds.Width)), Math.Clamp(point.Y, 0, Math.Max(0, item.Bounds.Height)));

    private void UpdateHealingDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingHealPreview is not { } preview || !ReferenceEquals(preview.Item, item)) return;
        point = ClampHealingPoint(item, point);
        var previous = preview.Stroke.StylusPoints[^1];
        if ((new Point(previous.X, previous.Y) - point).Length < .5) return;
        preview.Stroke.StylusPoints.Add(new StylusPoint(point.X, point.Y));
        preview.Visual.Data = preview.Stroke.GetGeometry();
    }

    private async void CommitHealingDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingHealPreview is not { } preview || !ReferenceEquals(preview.Item, item)) return;
        UpdateHealingDrawingPreview(item, point);
        CancelHealingDrawingPreview();
        var request = new CancellationTokenSource();
        _drawingHealRequest = request;
        DrawingToolbar.IsEnabled = false;
        item.Markup.Cursor = Cursors.Wait;
        byte[]? mask = null;
        try
        {
            var scaleX = preview.Source.PixelWidth / Math.Max(1, item.Bounds.Width);
            var scaleY = preview.Source.PixelHeight / Math.Max(1, item.Bounds.Height);
            var bounds = Rect.Intersect(preview.Stroke.GetBounds(), new Rect(0, 0, item.Bounds.Width, item.Bounds.Height));
            if (bounds.IsEmpty || bounds.Width <= 0 || bounds.Height <= 0) return;
            var region = MosaicPixelBounds(bounds, scaleX, scaleY, preview.Source.PixelWidth, preview.Source.PixelHeight);
            if ((long)region.Width * region.Height > 16L * 1024 * 1024)
                throw new InvalidOperationException("Healing stroke area is too large.");
            mask = RenderHealingMask(preview.Stroke, region, scaleX, scaleY);
            var bitmap = await Task.Run(() => MaskedInpaintingService.CreatePatch(preview.Source, region, mask, request.Token), request.Token);
            if (request.IsCancellationRequested || !ReferenceEquals(_drawingHealRequest, request) || _closed ||
                !_selections.Contains(item) || !_drawingMode) return;
            var element = new MosaicDrawingElement(Guid.NewGuid(), region.X / scaleX, region.Y / scaleY,
                region.Width / scaleX, region.Height / scaleY) { Pixels = bitmap, SeamlessErase = true };
            item.DrawingElements.Add(element);
            item.DrawingOrder.Add(new ElementDrawingAction(element));
            item.DrawingRedo.Clear();
            item.RasterLayer.Children.Add(CreateMosaicVisual(item, element));
            MarkDrawingChanged(item);
            RenderRegionMarks(item);
            PromptStatus.Text = LocalizationService.T("已修补涂抹区域 · 可继续涂抹或撤回", "Painted area repaired · continue brushing or undo");
        }
        catch (OperationCanceledException) when (request.IsCancellationRequested) { }
        catch (Exception error) { if (ReferenceEquals(_drawingHealRequest, request) && !_closed) HealingDrawingFailed(error); }
        finally
        {
            if (mask is not null) Array.Clear(mask);
            if (ReferenceEquals(_drawingHealRequest, request))
            {
                _drawingHealRequest = null;
                DrawingToolbar.IsEnabled = true;
                item.Markup.Cursor = DrawingToolCursor();
                ResumeAnnotatedImageCopy();
            }
            request.Dispose();
        }
    }

    private static byte[] RenderHealingMask(Stroke stroke, Int32Rect region, double scaleX, double scaleY)
    {
        var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen())
        {
            drawing.PushTransform(new MatrixTransform(scaleX, 0, 0, scaleY, -region.X, -region.Y));
            drawing.DrawGeometry(Brushes.White, null, stroke.GetGeometry());
            drawing.Pop();
        }
        var bitmap = new RenderTargetBitmap(region.Width, region.Height, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(visual);
        var mask = new byte[checked(region.Width * region.Height)];
        var row = new byte[checked(region.Width * 4)];
        try
        {
            for (var y = 0; y < region.Height; y++)
            {
                bitmap.CopyPixels(new Int32Rect(0, y, region.Width, 1), row, row.Length, 0);
                for (var x = 0; x < region.Width; x++) mask[y * region.Width + x] = row[x * 4 + 3];
            }
            return mask;
        }
        finally { Array.Clear(row); }
    }

    private void CancelHealingDrawingPreview()
    {
        var preview = _drawingHealPreview;
        _drawingHealPreview = null;
        if (preview is not null) preview.Item.Markup.Children.Remove(preview.Visual);
    }

    private void CancelHealingOperation()
    {
        CancelHealingDrawingPreview();
        var request = _drawingHealRequest;
        _drawingHealRequest = null;
        request?.Cancel();
        if (DrawingToolbar is not null) DrawingToolbar.IsEnabled = true;
        if (Active is { } item) item.Markup.Cursor = DrawingToolCursor();
    }

    private void HealingDrawingFailed(Exception error)
    {
        CancelHealingDrawingPreview();
        SeamlessEraseFailed(error, DrawingHealButton,
            LocalizationService.T("无法修补这一笔，请缩小涂抹范围并留出周围背景后重试。", "Unable to repair this stroke. Use a smaller area with surrounding background and retry."));
    }
}
