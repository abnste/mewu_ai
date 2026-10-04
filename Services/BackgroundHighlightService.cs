// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

/// <summary>A shared, immutable background-only alpha mask in source pixel coordinates.</summary>
internal sealed class BackgroundHighlightSource(BitmapSource mask)
{
    internal BitmapSource Mask { get; } = mask;

    internal ImageBrush CreateOpacityBrush(Rect sourceBounds)
    {
        var brush = new ImageBrush(Mask)
        {
            ViewportUnits = BrushMappingMode.Absolute, Viewport = sourceBounds,
            ViewboxUnits = BrushMappingMode.RelativeToBoundingBox, Viewbox = new Rect(0, 0, 1, 1),
            Stretch = Stretch.Fill, TileMode = TileMode.None
        };
        brush.Freeze();
        return brush;
    }
}

internal static class BackgroundHighlightService
{
    internal static BackgroundHighlightSource TransparentSource { get; } = CreateTransparentSource();

    internal static void EnsureSupportedDimensions(int width, int height)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(width, 1);
        ArgumentOutOfRangeException.ThrowIfLessThan(height, 1);
        if ((long)width * height > 16L * 1024 * 1024)
            throw new InvalidOperationException("The background highlight region is too large.");
    }

    private static BackgroundHighlightSource CreateTransparentSource()
    {
        var mask = BitmapSource.Create(1, 1, 96, 96, PixelFormats.Pbgra32, null, new byte[4], 4);
        mask.Freeze();
        return new BackgroundHighlightSource(mask);
    }

    // Choose opening/closing from the wider mean's polarity against local extrema.
    // Combining both residuals would also protect background gaps inside text.
    // Per-channel comparison protects colored text with little luminance contrast.
    // https://docs.opencv.org/4.x/d9/d61/tutorial_py_morphological_ops.html
    internal static BackgroundHighlightSource CreateSource(BitmapSource source, double pixelsPerDip = 1)
    {
        ArgumentNullException.ThrowIfNull(source);
        var width = source.PixelWidth;
        var height = source.PixelHeight;
        // Check before allocating any full-image working buffer. Long captures
        // can be much larger than an ordinary desktop screenshot.
        EnsureSupportedDimensions(width, height);
        var count = checked(width * height);
        var stride = checked(width * 4);
        byte[] original = [], channel = [], first = [], second = [], scratch = [], protection = [], output = [];
        var radius = Math.Clamp((int)Math.Ceiling(6 * (double.IsFinite(pixelsPerDip) ? pixelsPerDip : 1)), 6, 24);
        try
        {
            original = new byte[checked(count * 4)];
            channel = new byte[count];
            first = new byte[count];
            second = new byte[count];
            scratch = new byte[count];
            protection = new byte[count];
            output = new byte[original.Length];
            var formatted = source.Format == PixelFormats.Bgra32 ? source : new FormatConvertedBitmap(source, PixelFormats.Bgra32, null, 0);
            formatted.CopyPixels(original, stride, 0);
            for (var component = 0; component < 3; component++)
            {
                for (var pixel = 0; pixel < count; pixel++) channel[pixel] = original[pixel * 4 + component];
                BoxMean(channel, scratch, output, 0, width, height, radius * 2);
                Morphology(channel, first, scratch, width, height, radius, maximum: false);
                for (var pixel = 0; pixel < count; pixel++) output[pixel * 4 + 1] = first[pixel];
                Morphology(first, second, scratch, width, height, radius, maximum: true);
                // The final output is not written until all channels finish;
                // reuse its alpha byte for this channel's opening candidate.
                for (var pixel = 0; pixel < count; pixel++) output[pixel * 4 + 3] = second[pixel];
                Morphology(channel, first, scratch, width, height, radius, maximum: true);
                for (var pixel = 0; pixel < count; pixel++) output[pixel * 4 + 2] = first[pixel];
                Morphology(first, second, scratch, width, height, radius, maximum: false);
                for (var pixel = 0; pixel < count; pixel++)
                {
                    var mean = output[pixel * 4];
                    var minimum = output[pixel * 4 + 1];
                    var maximum = output[pixel * 4 + 2];
                    var opening = output[pixel * 4 + 3];
                    var closing = second[pixel];
                    // Near an antialias edge both candidates can differ by only
                    // one level. Comparing the text-biased mean to those two
                    // would select the edge itself as background (254 vs 255).
                    // Raw extrema retain the actual foreground/background split.
                    var background = Math.Abs(mean - minimum) < Math.Abs(mean - maximum) ? opening : closing;
                    protection[pixel] = (byte)Math.Max(protection[pixel], Math.Abs(channel[pixel] - background));
                }
            }
            // Antialias pixels contain both foreground and background. A binary
            // mask preserves their old background too, leaving pale outlines
            // when the surrounding background is highlighted. Estimate local
            // foreground coverage instead, keeping the detail core protected
            // while allowing its background-heavy edge to receive the tint.
            // Keep this neighborhood at one physical pixel: the wider radius
            // used to estimate background would mix unrelated dark text into
            // a nearby faint line and erase that line's protection.
            // https://www.w3.org/TR/compositing-1/#simplealphacompositing
            Morphology(protection, first, scratch, width, height, 1, maximum: true);
            for (var pixel = 0; pixel < count; pixel++)
            {
                var contrast = first[pixel];
                var sourceAlpha = original[pixel * 4 + 3];
                var alpha = contrast < 2 ? sourceAlpha :
                    (byte)((sourceAlpha * (contrast - protection[pixel]) + contrast / 2) / contrast);
                var offset = pixel * 4;
                output[offset] = output[offset + 1] = output[offset + 2] = output[offset + 3] = alpha;
            }
            var mask = BitmapSource.Create(width, height, source.DpiX, source.DpiY, PixelFormats.Pbgra32, null, output, stride);
            mask.Freeze();
            return new BackgroundHighlightSource(mask);
        }
        finally
        {
            Array.Clear(original); Array.Clear(channel); Array.Clear(first); Array.Clear(second);
            Array.Clear(scratch); Array.Clear(protection); Array.Clear(output);
        }
    }

    // A separable sliding box mean needs no extra image-sized buffer. Horizontal
    // means occupy scratch; vertical means use an unused output color channel.
    private static void BoxMean(byte[] source, byte[] scratch, byte[] output, int component, int width, int height, int radius)
    {
        for (var y = 0; y < height; y++)
        {
            var row = y * width;
            var sum = 0;
            for (var x = 0; x <= Math.Min(radius, width - 1); x++) sum += source[row + x];
            for (var x = 0; x < width; x++)
            {
                var samples = Math.Min(width - 1, x + radius) - Math.Max(0, x - radius) + 1;
                scratch[row + x] = (byte)((sum + samples / 2) / samples);
                if (x - radius >= 0) sum -= source[row + x - radius];
                if (x + radius + 1 < width) sum += source[row + x + radius + 1];
            }
        }
        for (var x = 0; x < width; x++)
        {
            var sum = 0;
            for (var y = 0; y <= Math.Min(radius, height - 1); y++) sum += scratch[y * width + x];
            for (var y = 0; y < height; y++)
            {
                var samples = Math.Min(height - 1, y + radius) - Math.Max(0, y - radius) + 1;
                output[(y * width + x) * 4 + component] = (byte)((sum + samples / 2) / samples);
                if (y - radius >= 0) sum -= scratch[(y - radius) * width + x];
                if (y + radius + 1 < height) sum += scratch[(y + radius + 1) * width + x];
            }
        }
    }

    // Separable monotonic windows are O(width*height), independent of radius.
    // Clipped edge windows are equivalent to repeating the edge for min/max.
    private static void Morphology(byte[] source, byte[] target, byte[] scratch, int width, int height, int radius, bool maximum)
    {
        var deque = new int[Math.Max(width, height)];
        for (var y = 0; y < height; y++) FilterLine(source, scratch, y * width, 1, width, radius, maximum, deque);
        for (var x = 0; x < width; x++) FilterLine(scratch, target, x, width, height, radius, maximum, deque);
    }

    private static void FilterLine(byte[] source, byte[] target, int start, int step, int length, int radius, bool maximum, int[] deque)
    {
        var head = 0;
        var tail = 0;
        var next = 0;
        for (var position = 0; position < length; position++)
        {
            var end = Math.Min(length - 1, position + radius);
            while (next <= end)
            {
                var value = source[start + next * step];
                while (tail > head && (maximum ? source[start + deque[tail - 1] * step] <= value : source[start + deque[tail - 1] * step] >= value)) tail--;
                deque[tail++] = next++;
            }
            while (head < tail && deque[head] < position - radius) head++;
            target[start + position * step] = source[start + deque[head] * step];
        }
    }
}
