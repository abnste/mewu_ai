// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class BackgroundHighlightServiceTests
{
    [Fact]
    public void OversizedMaskIsRejectedBeforeAllocatingPixelsAndFallbackProtectsEverything()
    {
        BackgroundHighlightService.EnsureSupportedDimensions(4096, 4096);
        Assert.Throws<InvalidOperationException>(() => BackgroundHighlightService.EnsureSupportedDimensions(4097, 4096));
        var fallback = BackgroundHighlightService.TransparentSource;
        Assert.True(fallback.Mask.IsFrozen); Assert.Equal(0, Pixel(fallback.Mask, 0, 0)[3]);
        Assert.Same(fallback, BackgroundHighlightService.TransparentSource);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void SpacesAndCountersBetweenThinGlyphEdgesRemainTintable(bool darkBackground)
    {
        var background = darkBackground ? (byte)20 : (byte)240; var text = darkBackground ? (byte)245 : (byte)0;
        var bytes = new byte[64 * 36 * 4];
        for (var y = 0; y < 36; y++) for (var x = 0; x < 64; x++)
        {
            bool Glyph(int left) => x >= left && x <= left + 10 && y >= 8 && y <= 27 && (x <= left + 1 || x >= left + 9 || y <= 9 || y >= 26);
            var value = Glyph(16) || Glyph(31) ? text : background; var at = (y * 64 + x) * 4;
            bytes[at] = bytes[at + 1] = bytes[at + 2] = value; bytes[at + 3] = 255;
        }
        var image = BitmapSource.Create(64, 36, 96, 96, PixelFormats.Bgra32, null, bytes, 64 * 4); image.Freeze();
        var mask = BackgroundHighlightService.CreateSource(image).Mask;
        Assert.Equal(0, Pixel(mask, 16, 17)[3]);
        Assert.Equal(255, Pixel(mask, 21, 17)[3]);
        Assert.Equal(255, Pixel(mask, 28, 17)[3]);
    }

    [Theory]
    [InlineData(255, 255, 255, 0, 0, 0)]
    [InlineData(20, 30, 40, 255, 255, 255)]
    [InlineData(100, 100, 100, 200, 91, 50)]
    public void ProtectsTextCoresWhileWeakAntialiasEdgesRemainTintable(int b, int g, int r, int tb, int tg, int tr)
    {
        byte[] background = [(byte)b, (byte)g, (byte)r], text = [(byte)tb, (byte)tg, (byte)tr];
        var source = Source(background, text); var before = Pixels(source); var mask = BackgroundHighlightService.CreateSource(source);
        Assert.True(mask.Mask.IsFrozen); Assert.Equal(PixelFormats.Pbgra32, mask.Mask.Format); Assert.Equal(source.DpiX, mask.Mask.DpiX);
        for (var y = 10; y < 24; y++) for (var x = 21; x <= 25; x++)
        {
            var actualDifference = Pixel(source, x, y).Take(3).Where((channel, index) => channel != background[index]).Any();
            var opacity = Pixel(mask.Mask, x, y)[3];
            if (x is >= 22 and <= 24) Assert.Equal(0, opacity);
            else if (actualDifference) Assert.InRange(opacity, 1, 254);
            else Assert.Equal(255, opacity);
        }
        Assert.Equal(255, Pixel(mask.Mask, 8, 18)[3]); Assert.Equal(before, Pixels(source));
        var brush = mask.CreateOpacityBrush(new Rect(0, 0, 64, 36)); Assert.True(brush.IsFrozen);
    }

    [Fact]
    public void RepeatedRealStrokeRenderingTintsBackgroundButLeavesProtectedTextUnchanged() => Sta(() =>
    {
        var source = Source([255, 255, 255], [0, 0, 0]); var protection = BackgroundHighlightService.CreateSource(source);
        var stroke = Stroke(protection); var once = Render(source, stroke); var twice = Render(source, stroke, stroke);
        Assert.False(stroke.DrawingAttributes.IsHighlighter);
        for (var y = 10; y < 24; y++) for (var x = 22; x <= 24; x++)
        { Assert.Equal(Pixel(source, x, y), Pixel(once, x, y)); Assert.Equal(Pixel(source, x, y), Pixel(twice, x, y)); }
        Assert.NotEqual(Pixel(source, 21, 18), Pixel(once, 21, 18));
        Assert.NotEqual(Pixel(once, 21, 18), Pixel(twice, 21, 18));
        Assert.NotEqual(Pixel(source, 10, 18), Pixel(once, 10, 18));
        Assert.NotEqual(Pixel(once, 10, 18), Pixel(twice, 10, 18));
        Assert.Equal(Pixel(source, 10, 1), Pixel(twice, 10, 1));
    });

    [Theory]
    [InlineData(2)]
    [InlineData(4)]
    [InlineData(8)]
    public void IndependentFaintLineBesideBlackLineKeepsItsOwnCore(int distance)
    {
        var bytes = Enumerable.Repeat((byte)255, 64 * 36 * 4).ToArray();
        for (var y = 8; y < 28; y++)
        {
            foreach (var (x, value) in new[] { (24, (byte)0), (24 + distance, (byte)235) })
            { var at = (y * 64 + x) * 4; bytes[at] = bytes[at + 1] = bytes[at + 2] = value; }
        }
        var source = BitmapSource.Create(64, 36, 96, 96, PixelFormats.Bgra32, null, bytes, 256); source.Freeze();
        var mask = BackgroundHighlightService.CreateSource(source).Mask;
        Assert.Equal(0, Pixel(mask, 24 + distance, 18)[3]);
        Assert.Equal(0, Pixel(mask, 24, 18)[3]);
        Assert.Equal(255, Pixel(mask, 25, 18)[3]);
        Assert.Equal(bytes, Pixels(source));
    }

    [Fact]
    public void ContinuousMaskNeverExceedsSourceAlphaOrMutatesTransparentInput()
    {
        var pixels = Pixels(Source([240, 240, 240], [0, 0, 0]));
        for (var pixel = 0; pixel < 64 * 36; pixel++) pixels[pixel * 4 + 3] = pixel % 2 == 0 ? (byte)128 : (byte)0;
        var source = BitmapSource.Create(64, 36, 96, 96, PixelFormats.Bgra32, null, pixels, 256); source.Freeze();
        var mask = Pixels(BackgroundHighlightService.CreateSource(source).Mask);
        for (var pixel = 0; pixel < 64 * 36; pixel++) Assert.InRange(mask[pixel * 4 + 3], 0, pixels[pixel * 4 + 3]);
        Assert.Equal(pixels, Pixels(source));
    }

    [Theory]
    [InlineData(220, 220, 220, 0, 0, 0)]
    [InlineData(170, 210, 230, 0, 0, 0)]
    [InlineData(20, 30, 40, 255, 255, 255)]
    [InlineData(240, 240, 240, 200, 55, 30)]
    public void KnownAntialiasCoverageHasNoBrightOrDarkRingAfterRepeatedRedHighlight(int b, int g, int r, int tb, int tg, int tr) => Sta(() =>
    {
        byte[] background = [(byte)b, (byte)g, (byte)r], foreground = [(byte)tb, (byte)tg, (byte)tr];
        byte[] coverage = [8, 32, 64, 128, 192, 240];
        var bytes = new byte[64 * 36 * 4];
        for (var y = 0; y < 36; y++) for (var x = 0; x < 64; x++)
        {
            var alpha = 0;
            for (var i = 0; i < coverage.Length; i++)
                if (y >= 10 && y < 24) { if (x == 10 + i * 8) alpha = 255; else if (x == 11 + i * 8) alpha = coverage[i]; }
            var at = (y * 64 + x) * 4;
            for (var c = 0; c < 3; c++) bytes[at + c] = (byte)((foreground[c] * alpha + background[c] * (255 - alpha) + 127) / 255);
            bytes[at + 3] = 255;
        }
        var source = BitmapSource.Create(64, 36, 96, 96, PixelFormats.Bgra32, null, bytes, 256); source.Freeze();
        var protection = BackgroundHighlightService.CreateSource(source);
        var stroke = Stroke(protection); stroke.DrawingAttributes.Color = Colors.Red;
        foreach (var rendered in new[] { Render(source, stroke), Render(source, stroke, stroke) })
        {
            var tintedBackground = Pixel(rendered, 7, 18);
            var previous = tintedBackground;
            for (var i = 0; i < coverage.Length; i++)
            {
                var edge = Pixel(rendered, 11 + i * 8, 18);
                Assert.Equal(Pixel(source, 10 + i * 8, 18), Pixel(rendered, 10 + i * 8, 18));
                Assert.NotEqual(Pixel(source, 11 + i * 8, 18), edge);
                for (var c = 0; c < 3; c++)
                {
                    Assert.InRange((int)edge[c], Math.Min(tintedBackground[c], foreground[c]) - 1, Math.Max(tintedBackground[c], foreground[c]) + 1);
                    if (foreground[c] > tintedBackground[c]) Assert.True(edge[c] >= previous[c] - 1, "Light glyph edge must progress toward its core.");
                    else Assert.True(edge[c] <= previous[c] + 1, "Dark glyph edge must progress toward its core without a bright halo.");
                }
                previous = edge;
            }
        }
        Assert.Equal(bytes, Pixels(source));
    });

    [Fact]
    public void CloneTransformRebaseAndRefreshKeepIndependentGeometryAndCorrectMaskAlignment() => Sta(() =>
    {
        var source = Source([255, 255, 255], [0, 0, 0]); var protection = BackgroundHighlightService.CreateSource(source); var original = Stroke(protection);
        var originalPixels = Pixels(Render(null, original));
        var clone = Assert.IsType<BackgroundHighlightStroke>(original.Clone());
        clone.Transform(new Matrix(1, 0, 0, 1, 8, 0), false); clone.RebaseSource(new Vector(8, 0));
        var translated = Render(null, clone);
        Assert.Equal(0, Pixel(translated, 31, 18)[3]); Assert.True(Pixel(translated, 18, 18)[3] > 0);
        Assert.Equal(originalPixels, Pixels(Render(null, original)));
        var plain = BitmapSource.Create(64, 36, 96, 96, PixelFormats.Bgra32, null, Enumerable.Repeat((byte)255, 64 * 36 * 4).ToArray(), 64 * 4); plain.Freeze();
        clone.UpdateSource(BackgroundHighlightService.CreateSource(plain), new Rect(8, 0, 64, 36));
        Assert.True(Pixel(Render(null, clone), 31, 18)[3] > 0);
        Assert.Equal(0, Pixel(Render(null, original), 23, 18)[3]);
    });

    private static BackgroundHighlightStroke Stroke(BackgroundHighlightSource source) => new(
        new StylusPointCollection { new StylusPoint(6, 18), new StylusPoint(58, 18) },
        new DrawingAttributes { Color = Colors.Yellow, Width = 18, Height = 18, FitToCurve = false, IsHighlighter = true }, source, new Rect(0, 0, 64, 36));
    private static BitmapSource Source(byte[] background, byte[] text)
    {
        var pixels = new byte[64 * 36 * 4];
        for (var y = 0; y < 36; y++) for (var x = 0; x < 64; x++)
        {
            var alpha = y >= 10 && y < 24 ? x >= 22 && x <= 24 ? 1d : x is 21 or 25 ? .005 : 0 : 0;
            var at = (y * 64 + x) * 4;
            for (var c = 0; c < 3; c++) pixels[at + c] = (byte)Math.Round(background[c] * (1 - alpha) + text[c] * alpha);
            pixels[at + 3] = 255;
        }
        var image = BitmapSource.Create(64, 36, 96, 96, PixelFormats.Bgra32, null, pixels, 64 * 4); image.Freeze(); return image;
    }
    private static BitmapSource Render(BitmapSource? background, params Stroke[] strokes)
    {
        var visual = new DrawingVisual(); using (var context = visual.RenderOpen())
        { if (background is not null) context.DrawImage(background, new Rect(0, 0, 64, 36)); foreach (var stroke in strokes) stroke.Draw(context); }
        var bitmap = new RenderTargetBitmap(80, 36, 96, 96, PixelFormats.Pbgra32); bitmap.Render(visual); bitmap.Freeze(); return bitmap;
    }
    private static byte[] Pixel(BitmapSource source, int x, int y) { var bytes = new byte[4]; source.CopyPixels(new Int32Rect(x, y, 1, 1), bytes, 4, 0); return bytes; }
    private static byte[] Pixels(BitmapSource source) { var bytes = new byte[source.PixelWidth * source.PixelHeight * 4]; source.CopyPixels(bytes, source.PixelWidth * 4, 0); return bytes; }
    private static void Sta(Action action)
    {
        Exception? failure = null; var thread = new Thread(() => { try { action(); } catch (Exception error) { failure = error; } }) { IsBackground = true };
        thread.SetApartmentState(ApartmentState.STA); thread.Start(); Assert.True(thread.Join(TimeSpan.FromSeconds(10)), "WPF mask rendering exceeded its deadline.");
        if (failure is not null) ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
