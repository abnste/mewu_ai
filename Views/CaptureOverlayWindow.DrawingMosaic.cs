// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Point = System.Windows.Point;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private MosaicDrawingPreview? _drawingMosaicPreview;

    private sealed record MosaicDrawingPreview(SelectionItem Item, MosaicPixelGrid Pixels, Image Image,
        RectangleGeometry Clip, double ScaleX, double ScaleY);

    private void BeginMosaicDrawingPreview(SelectionItem item)
    {
        CancelMosaicDrawingPreview();
        try
        {
            // Read the unmodified screenshot once. Mouse moves only change the clip;
            // neither the preview nor previously committed mosaics are sampled again.
            var source = RenderSelectionImage(item, false, false, false);
            var scaleX = source.PixelWidth / Math.Max(1, item.Bounds.Width);
            var scaleY = source.PixelHeight / Math.Max(1, item.Bounds.Height);
            var blockSize = Math.Clamp((int)Math.Round(12 * Math.Max(scaleX, scaleY)), 6, 40);
            var pixels = MosaicPixelGrid.Create(source, new Int32Rect(0, 0, source.PixelWidth, source.PixelHeight), blockSize);
            var clip = new RectangleGeometry(new Rect(0, 0, 0, 0));
            var image = new Image
            {
                Source = pixels.Image,
                Width = pixels.Columns * (double)pixels.BlockSize / scaleX,
                Height = pixels.Rows * (double)pixels.BlockSize / scaleY,
                Stretch = Stretch.Fill,
                Clip = clip,
                IsHitTestVisible = false
            };
            RenderOptions.SetBitmapScalingMode(image, BitmapScalingMode.NearestNeighbor);
            InkCanvas.SetLeft(image, 0);
            InkCanvas.SetTop(image, 0);
            _drawingMosaicPreview = new MosaicDrawingPreview(item, pixels, image, clip, scaleX, scaleY);
            item.RasterPreviewLayer.Children.Add(image);
        }
        catch (Exception error)
        {
            CancelMosaicDrawingPreview();
            new PrivacyLogger().Error("DrawingMosaicPreview", error);
            PromptStatus.Text = LocalizationService.T("马赛克预览失败，请重新框选后重试。", "Mosaic preview failed. Select the area again and retry.");
        }
    }

    private void UpdateMosaicDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingMosaicPreview is not { } preview || !ReferenceEquals(preview.Item, item)) return;
        preview.Clip.Rect = MosaicDrawingBounds(item, _drawStart, point);
    }

    private void CommitMosaicDrawingPreview(SelectionItem item, Point point)
    {
        if (_drawingMosaicPreview is not { } preview || !ReferenceEquals(preview.Item, item))
        {
            CancelMosaicDrawingPreview();
            return;
        }
        try
        {
            var bounds = MosaicDrawingBounds(item, _drawStart, point);
            if (bounds.Width < 3 || bounds.Height < 3) return;
            var region = MosaicPixelBounds(bounds, preview.ScaleX, preview.ScaleY,
                preview.Pixels.SourceWidth, preview.Pixels.SourceHeight);
            var bitmap = preview.Pixels.RenderCrop(region);
            if (!bitmap.IsFrozen) bitmap.Freeze();
            var element = new MosaicDrawingElement(Guid.NewGuid(), bounds.X, bounds.Y, bounds.Width, bounds.Height) { Pixels = bitmap };
            var visual = new Image
            {
                Tag = element.Id, Source = bitmap, Width = bounds.Width, Height = bounds.Height,
                Stretch = Stretch.Fill, IsHitTestVisible = false
            };
            RenderOptions.SetBitmapScalingMode(visual, BitmapScalingMode.NearestNeighbor);
            InkCanvas.SetLeft(visual, bounds.X);
            InkCanvas.SetTop(visual, bounds.Y);
            CancelMosaicDrawingPreview();
            item.RasterLayer.Children.Add(visual);
            item.DrawingElements.Add(element);
            item.DrawingOrder.Add(new ElementDrawingAction(element));
            item.DrawingRedo.Clear();
            MarkDrawingChanged(item);
        }
        finally { CancelMosaicDrawingPreview(); }
    }

    private void CancelMosaicDrawingPreview()
    {
        CancelBackgroundHighlightPreview();
        CancelHealingOperation();
        DismissSeamlessEraseFailure();
        CancelMosaicDrawingPreviewContent();
    }

    private void CancelMosaicDrawingPreviewContent()
    {
        CancelHealingDrawingPreview();
        CancelSeamlessEraseDrawingPreview();
        var preview = _drawingMosaicPreview;
        _drawingMosaicPreview = null;
        if (preview is null) return;
        preview.Item.RasterPreviewLayer.Children.Remove(preview.Image);
        preview.Image.Source = null;
        preview.Pixels.Dispose();
    }

    private static Rect MosaicDrawingBounds(SelectionItem item, Point start, Point end)
    {
        var width = Math.Max(0, item.Bounds.Width);
        var height = Math.Max(0, item.Bounds.Height);
        return new Rect(new Point(Math.Clamp(start.X, 0, width), Math.Clamp(start.Y, 0, height)),
            new Point(Math.Clamp(end.X, 0, width), Math.Clamp(end.Y, 0, height)));
    }

    private static Int32Rect MosaicPixelBounds(Rect bounds, double scaleX, double scaleY, int width, int height)
    {
        var left = Math.Clamp((int)Math.Floor(bounds.Left * scaleX), 0, width - 1);
        var top = Math.Clamp((int)Math.Floor(bounds.Top * scaleY), 0, height - 1);
        var right = Math.Clamp((int)Math.Ceiling(bounds.Right * scaleX), left + 1, width);
        var bottom = Math.Clamp((int)Math.Ceiling(bounds.Bottom * scaleY), top + 1, height);
        return new Int32Rect(left, top, right - left, bottom - top);
    }

    private static BitmapSource CreateMosaicPixels(BitmapSource source, Int32Rect region, int blockSize)
    {
        // Rebuilds after undo/redo or object movement use the same screenshot-aligned
        // grid as the live preview, but only read the blocks touching this rectangle.
        using var pixels = MosaicPixelGrid.Create(source, region, blockSize);
        return pixels.RenderCrop(region);
    }

    private BitmapSource CaptureMosaicPixels(SelectionItem item, MosaicDrawingElement element)
    {
        var source = RenderSelectionImage(item, false, false, false);
        var scaleX = source.PixelWidth / Math.Max(1, item.Bounds.Width);
        var scaleY = source.PixelHeight / Math.Max(1, item.Bounds.Height);
        var region = MosaicPixelBounds(new Rect(element.X, element.Y, element.Width, element.Height),
            scaleX, scaleY, source.PixelWidth, source.PixelHeight);
        var bitmap = element.SeamlessErase ? SeamlessEraseService.CreatePatch(source, region)
            : CreateMosaicPixels(source, region, Math.Clamp((int)Math.Round(12 * Math.Max(scaleX, scaleY)), 6, 40));
        if (!bitmap.IsFrozen) bitmap.Freeze();
        return bitmap;
    }

    private sealed class MosaicPixelGrid : IDisposable
    {
        private readonly byte[] _colors;
        private readonly int _left, _top;
        private readonly double _dpiX, _dpiY;
        internal BitmapSource Image { get; }
        internal int BlockSize { get; }
        internal int Columns { get; }
        internal int Rows { get; }
        internal int SourceWidth { get; }
        internal int SourceHeight { get; }

        private MosaicPixelGrid(BitmapSource source, int left, int top, int blockSize, int columns, int rows, byte[] colors)
        {
            _colors = colors;
            _left = left;
            _top = top;
            _dpiX = source.DpiX;
            _dpiY = source.DpiY;
            SourceWidth = source.PixelWidth;
            SourceHeight = source.PixelHeight;
            BlockSize = blockSize;
            Columns = columns;
            Rows = rows;
            Image = BitmapSource.Create(columns, rows, 96, 96, PixelFormats.Bgra32, null, colors, checked(columns * 4));
            Image.Freeze();
        }

        internal static MosaicPixelGrid Create(BitmapSource source, Int32Rect region, int blockSize)
        {
            ArgumentNullException.ThrowIfNull(source);
            if (blockSize is < 2 or > 128) throw new ArgumentOutOfRangeException(nameof(blockSize));
            if (region.X < 0 || region.Y < 0 || region.Width <= 0 || region.Height <= 0 ||
                (long)region.X + region.Width > source.PixelWidth || (long)region.Y + region.Height > source.PixelHeight)
                throw new ArgumentOutOfRangeException(nameof(region));
            var left = region.X / blockSize * blockSize;
            var top = region.Y / blockSize * blockSize;
            var columns = checked((region.X + region.Width - left + blockSize - 1) / blockSize);
            var rows = checked((region.Y + region.Height - top + blockSize - 1) / blockSize);
            var stripWidth = (int)Math.Min((long)columns * blockSize, source.PixelWidth - left);
            var stripStride = checked(stripWidth * 4);
            var strip = new byte[checked(stripStride * Math.Min(blockSize, source.PixelHeight - top))];
            var colors = new byte[checked(columns * rows * 4)];
            try
            {
                var formatted = source.Format == PixelFormats.Bgra32 ? source : new FormatConvertedBitmap(source, PixelFormats.Bgra32, null, 0);
                for (var row = 0; row < rows; row++)
                {
                    var stripTop = checked(top + row * blockSize);
                    var stripHeight = Math.Min(blockSize, source.PixelHeight - stripTop);
                    // Only a small strip of original pixels is resident in managed memory.
                    formatted.CopyPixels(new Int32Rect(left, stripTop, stripWidth, stripHeight), strip, stripStride, 0);
                    for (var column = 0; column < columns; column++)
                    {
                        var blockLeft = column * blockSize;
                        var blockWidth = Math.Min(blockSize, stripWidth - blockLeft);
                        long blue = 0, green = 0, red = 0, alpha = 0;
                        for (var y = 0; y < stripHeight; y++)
                        for (var x = blockLeft; x < blockLeft + blockWidth; x++)
                        {
                            var offset = y * stripStride + x * 4;
                            blue += strip[offset]; green += strip[offset + 1]; red += strip[offset + 2]; alpha += strip[offset + 3];
                        }
                        var count = blockWidth * stripHeight;
                        var target = (row * columns + column) * 4;
                        colors[target] = (byte)(blue / count); colors[target + 1] = (byte)(green / count);
                        colors[target + 2] = (byte)(red / count); colors[target + 3] = (byte)(alpha / count);
                    }
                }
                return new MosaicPixelGrid(source, left, top, blockSize, columns, rows, colors);
            }
            catch { Array.Clear(colors); throw; }
            finally { Array.Clear(strip); }
        }

        internal BitmapSource RenderCrop(Int32Rect region)
        {
            if (region.X < _left || region.Y < _top || region.Width <= 0 || region.Height <= 0 ||
                (long)region.X + region.Width > Math.Min(_left + (long)Columns * BlockSize, SourceWidth) ||
                (long)region.Y + region.Height > Math.Min(_top + (long)Rows * BlockSize, SourceHeight))
                throw new ArgumentOutOfRangeException(nameof(region));
            var stride = checked(region.Width * 4);
            var output = new byte[checked(stride * region.Height)];
            try
            {
                for (var y = 0; y < region.Height; y++)
                {
                    var row = (region.Y + y - _top) / BlockSize;
                    for (var x = 0; x < region.Width; x++)
                    {
                        var column = (region.X + x - _left) / BlockSize;
                        var color = (row * Columns + column) * 4;
                        var pixel = y * stride + x * 4;
                        output[pixel] = _colors[color]; output[pixel + 1] = _colors[color + 1];
                        output[pixel + 2] = _colors[color + 2]; output[pixel + 3] = _colors[color + 3];
                    }
                }
                var result = BitmapSource.Create(region.Width, region.Height, _dpiX, _dpiY, PixelFormats.Bgra32, null, output, stride);
                result.Freeze();
                return result;
            }
            finally { Array.Clear(output); }
        }

        public void Dispose() => Array.Clear(_colors);
    }
}
