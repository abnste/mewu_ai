// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

/// <summary>Shared frozen foreground coverage and detail protection in source pixels.</summary>
internal sealed class BackgroundHighlightSource(BitmapSource mask, BitmapSource foreground)
{
    internal BitmapSource Mask { get; } = mask;

    internal DrawingBrush CreateTintBrush(Color color, byte opacity, Rect sourceBounds)
    {
        var pixels = new Rect(0, 0, foreground.PixelWidth, foreground.PixelHeight);
        var drawing = new DrawingGroup();
        using (var context = drawing.Open())
        {
            // Tint the background contribution, then put the estimated
            // foreground back over it before applying highlight opacity.
            context.DrawRectangle(new SolidColorBrush(Color.FromRgb(color.R, color.G, color.B)), null, pixels);
            context.DrawImage(foreground, pixels);
        }
        drawing.Freeze();
        var brush = new DrawingBrush(drawing)
        {
            ViewportUnits = BrushMappingMode.Absolute, Viewport = sourceBounds,
            ViewboxUnits = BrushMappingMode.Absolute, Viewbox = pixels,
            Stretch = Stretch.Fill, TileMode = TileMode.None,
            Opacity = (byte)(opacity * color.A / 255) / 255d
        };
        brush.Freeze();
        return brush;
    }

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
        return new BackgroundHighlightSource(mask, mask);
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
        byte[] original = [], channel = [], first = [], second = [], scratch = [], protection = [], output = [], foreground = [];
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
            foreground = new byte[original.Length];
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
                    foreground[pixel * 4 + component] = background;
                }
            }
            // Antialias pixels contain both foreground and background. A binary
            // mask preserves their old background too, leaving pale outlines
            // when the surrounding background is highlighted. Estimate local
            // foreground coverage instead and separate its color contribution.
            // Merely fading a solid tint by coverage still forms colored rings
            // on repeated highlights. Compositing aF + (1-a)H at opacity t
            // retains aF and repeatedly tints only the background contribution.
            // Keep this neighborhood at one physical pixel: the wider radius
            // used to estimate background would mix unrelated dark text into
            // a nearby faint line and erase that line's protection.
            // https://www.w3.org/TR/compositing-1/#simplealphacompositing
            Morphology(protection, first, scratch, width, height, 1, maximum: true);
            for (var pixel = 0; pixel < count; pixel++)
            {
                var contrast = first[pixel];
                var offset = pixel * 4;
                // A one-level overshoot can come from premultiplied glyph
                // rounding; it must not make an otherwise solid core tintable.
                var coverage = contrast < 2 ? 0 : contrast - protection[pixel] <= 1 ? 255 :
                    (protection[pixel] * 255 + contrast / 2) / contrast;
                // Enforce the RGB feasibility bounds of P = aF + (1-a)B.
                // Increasing a when needed keeps the recovered foreground in
                // gamut rather than clipping its color or painting a halo.
                for (var component = 0; component < 3; component++)
                {
                    var value = original[offset + component];
                    var background = foreground[offset + component];
                    if (value < background)
                        coverage = Math.Max(coverage, ((background - value) * 255 + background - 1) / background);
                    else if (value > background)
                        coverage = Math.Max(coverage, ((value - background) * 255 + 254 - background) / (255 - background));
                }
                for (var component = 0; component < 3; component++)
                {
                    var premultiplied = (original[offset + component] * 255 -
                        (255 - coverage) * foreground[offset + component] + 127) / 255;
                    foreground[offset + component] = (byte)Math.Clamp(premultiplied, 0, coverage);
                }
                foreground[offset + 3] = (byte)coverage;
                // The source-over derivation assumes an opaque source. Keep
                // translucent pixels intact rather than increasing their alpha.
                var alpha = coverage < 255 && original[offset + 3] == 255 ? (byte)255 : (byte)0;
                output[offset] = output[offset + 1] = output[offset + 2] = output[offset + 3] = alpha;
            }
            var mask = BitmapSource.Create(width, height, source.DpiX, source.DpiY, PixelFormats.Pbgra32, null, output, stride);
            mask.Freeze();
            var detail = BitmapSource.Create(width, height, source.DpiX, source.DpiY, PixelFormats.Pbgra32, null, foreground, stride);
            detail.Freeze();
            return new BackgroundHighlightSource(mask, detail);
        }
        finally
        {
            Array.Clear(original); Array.Clear(channel); Array.Clear(first); Array.Clear(second);
            Array.Clear(scratch); Array.Clear(protection); Array.Clear(output); Array.Clear(foreground);
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
