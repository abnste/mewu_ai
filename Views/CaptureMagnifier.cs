// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Views;

/// <summary>A bounded physical-pixel sample of the clean capture, never of the overlay.</summary>
public sealed class CaptureMagnifier : FrameworkElement
{
    internal const int SampleSize = 25;
    private const int Radius = SampleSize / 2;
    private const int Stride = SampleSize * 4;
    private readonly byte[] _sourcePixels = new byte[Stride * SampleSize];
    private readonly byte[] _pixels = new byte[Stride * SampleSize];
    private readonly WriteableBitmap _sample = new(SampleSize, SampleSize, 96, 96, PixelFormats.Bgra32, null);
    private BitmapSource? _source;
    private int _x = -1, _y = -1;
    private bool _hasSample;
    private static readonly Brush Backdrop = FrozenBrush(Color.FromRgb(238, 243, 249));
    private static readonly Pen CrossOutline = FrozenPen(Color.FromArgb(210, 27, 38, 56), 2.5);
    private static readonly Pen CrossLine = FrozenPen(Colors.White, 1);
    private static readonly Pen PixelOutline = FrozenPen(Colors.White, 1.5);
    private static readonly Pen PixelLine = FrozenPen(Color.FromRgb(94, 110, 235), 1);

    internal BitmapSource Sample => _sample;

    public CaptureMagnifier()
    {
        IsHitTestVisible = false;
        Focusable = false;
        RenderOptions.SetBitmapScalingMode(this, BitmapScalingMode.NearestNeighbor);
    }

    internal bool SetSample(BitmapSource source, int x, int y)
    {
        if (x < 0 || y < 0 || x >= source.PixelWidth || y >= source.PixelHeight) return false;
        if (_hasSample && ReferenceEquals(_source, source) && _x == x && _y == y) return true;
        var format = source.Format;
        if (format != PixelFormats.Bgr32 && format != PixelFormats.Bgra32 && format != PixelFormats.Pbgra32 &&
            format != PixelFormats.Bgr24 && format != PixelFormats.Rgb24) return false;

        // Keep the addressed pixel at (12,12), even at a screen edge. Missing
        // neighbors remain transparent; moving the crop would move the target.
        var left = Math.Max(0, x - Radius);
        var top = Math.Max(0, y - Radius);
        var right = Math.Min(source.PixelWidth, x + Radius + 1);
        var bottom = Math.Min(source.PixelHeight, y + Radius + 1);
        var width = right - left;
        var height = bottom - top;
        var bytesPerPixel = format.BitsPerPixel / 8;
        source.CopyPixels(new Int32Rect(left, top, width, height), _sourcePixels, Stride, 0);
        Array.Clear(_pixels);
        for (var row = 0; row < height; row++)
        {
            for (var column = 0; column < width; column++)
            {
                var input = row * Stride + column * bytesPerPixel;
                var output = (row + top - y + Radius) * Stride + (column + left - x + Radius) * 4;
                var alpha = format == PixelFormats.Bgra32 || format == PixelFormats.Pbgra32 ? _sourcePixels[input + 3] : (byte)255;
                var red = _sourcePixels[input + (format == PixelFormats.Rgb24 ? 0 : 2)];
                var green = _sourcePixels[input + 1];
                var blue = _sourcePixels[input + (format == PixelFormats.Rgb24 ? 2 : 0)];
                if (format == PixelFormats.Pbgra32 && alpha is > 0 and < 255)
                {
                    red = (byte)Math.Min(255, (red * 255 + alpha / 2) / alpha);
                    green = (byte)Math.Min(255, (green * 255 + alpha / 2) / alpha);
                    blue = (byte)Math.Min(255, (blue * 255 + alpha / 2) / alpha);
                }
                _pixels[output] = blue;
                _pixels[output + 1] = green;
                _pixels[output + 2] = red;
                _pixels[output + 3] = alpha;
            }
        }
        _sample.WritePixels(new Int32Rect(0, 0, SampleSize, SampleSize), _pixels, Stride, 0);
        _source = source; _x = x; _y = y; _hasSample = true;
        InvalidateVisual();
        return true;
    }

    internal void ClearSample()
    {
        _source = null; _hasSample = false;
        Array.Clear(_sourcePixels); Array.Clear(_pixels);
        _sample.WritePixels(new Int32Rect(0, 0, SampleSize, SampleSize), _pixels, Stride, 0);
        InvalidateVisual();
    }

    protected override void OnRender(DrawingContext drawing)
    {
        base.OnRender(drawing);
        var bounds = new Rect(RenderSize);
        drawing.DrawRectangle(Backdrop, null, bounds);
        if (!_hasSample || bounds.IsEmpty) return;
        drawing.DrawImage(_sample, bounds);
        var center = new Point(bounds.Width / 2, bounds.Height / 2);
        var pixelWidth = bounds.Width / SampleSize;
        var pixelHeight = bounds.Height / SampleSize;
        var gapX = pixelWidth / 2 + 2;
        var gapY = pixelHeight / 2 + 2;
        Line(new Point(0, center.Y), new Point(center.X - gapX, center.Y));
        Line(new Point(center.X + gapX, center.Y), new Point(bounds.Width, center.Y));
        Line(new Point(center.X, 0), new Point(center.X, center.Y - gapY));
        Line(new Point(center.X, center.Y + gapY), new Point(center.X, bounds.Height));
        var pixel = new Rect(center.X - pixelWidth / 2, center.Y - pixelHeight / 2, pixelWidth, pixelHeight);
        drawing.DrawRectangle(null, PixelOutline, pixel);
        drawing.DrawRectangle(null, PixelLine, pixel);

        void Line(Point start, Point end)
        {
            drawing.DrawLine(CrossOutline, start, end);
            drawing.DrawLine(CrossLine, start, end);
        }
    }

    private static Brush FrozenBrush(Color color)
    {
        var brush = new SolidColorBrush(color); brush.Freeze(); return brush;
    }

    private static Pen FrozenPen(Color color, double width)
    {
        var pen = new Pen(FrozenBrush(color), width); pen.Freeze(); return pen;
    }
}
