// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class SeamlessEraseServiceTests
{
    [Theory]
    [InlineData(30, 24, 46, 34)]
    [InlineData(0, 0, 26, 22)]
    [InlineData(102, 0, 26, 22)]
    [InlineData(0, 74, 26, 22)]
    [InlineData(102, 74, 26, 22)]
    [InlineData(0, 30, 26, 22)]
    [InlineData(45, 74, 26, 22)]
    public void SolidBackgroundRemovesCoveredColorsAtInteriorAndImageEdges(int x, int y, int width, int height)
    {
        var region = new Int32Rect(x, y, width, height);
        var source = Source(region, false, false, out var original);
        var patch = SeamlessEraseService.CreatePatch(source, region);
        var pixels = Pixels(patch);
        AssertPatch(patch, region);
        for (var at = 0; at < pixels.Length; at += 4)
        {
            Assert.Equal(82, pixels[at]); Assert.Equal(147, pixels[at + 1]); Assert.Equal(213, pixels[at + 2]); Assert.Equal(255, pixels[at + 3]);
        }
        Assert.Equal(original, Pixels(source));
    }

    [Theory]
    [InlineData(30, 24, 46, 34)]
    [InlineData(0, 0, 26, 22)]
    [InlineData(102, 74, 26, 22)]
    public void LinearColorGradientIsReconstructedIncludingOneSidedBoundarySamples(int left, int top, int width, int height)
    {
        var region = new Int32Rect(left, top, width, height);
        var source = Source(region, true, false, out var original);
        var patch = SeamlessEraseService.CreatePatch(source, region); var actual = Pixels(patch);
        AssertPatch(patch, region);
        for (var y = 0; y < height; y++)
        for (var x = 0; x < width; x++)
        {
            var at = (y * width + x) * 4; var expected = Background(left + x, top + y, true);
            for (var channel = 0; channel < 3; channel++) Assert.InRange(Math.Abs(actual[at + channel] - expected[channel]), 0, 3);
            Assert.Equal(255, actual[at + 3]);
        }
        Assert.Equal(original, Pixels(source));
    }

    [Fact]
    public void CoveredForegroundDoesNotInfluenceBackgroundInference()
    {
        var region = new Int32Rect(30, 24, 46, 34);
        var first = Source(region, true, false, out _); var second = Source(region, true, true, out _);
        Assert.Equal(Pixels(SeamlessEraseService.CreatePatch(first, region)), Pixels(SeamlessEraseService.CreatePatch(second, region)));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void TwoColorBackgroundKeepsItsBoundaryInsteadOfBecomingAGradient(bool horizontal)
    {
        var region = new Int32Rect(30, 24, 46, 34);
        byte[] Expected(int x, int y) => (horizontal ? y < 48 : x < 64) ? [35, 90, 170] : [210, 155, 65];
        var source = Build(128, 96, (x, y) => Inside(region, x, y) ? [0, 0, 0] : Expected(x, y));
        var before = Pixels(source); var patch = SeamlessEraseService.CreatePatch(source, region); var actual = Pixels(patch);
        for (var y = 0; y < region.Height; y++) for (var x = 0; x < region.Width; x++)
        {
            var expected = Expected(region.X + x, region.Y + y); var at = (y * region.Width + x) * 4;
            for (var channel = 0; channel < 3; channel++) Assert.InRange(Math.Abs(actual[at + channel] - expected[channel]), 0, 3);
        }
        Assert.Equal(before, Pixels(source));
    }

    [Fact]
    public void WholeImageWithoutSurroundingBackgroundIsRejected()
    {
        var source = Build(32, 24, (x, y) => [(byte)(x * 7), (byte)(y * 9), 80]); var before = Pixels(source);
        var region = new Int32Rect(0, 0, 32, 24);
        Assert.Throws<InvalidOperationException>(() => SeamlessEraseService.CreatePatch(source, region));
        Assert.Throws<InvalidOperationException>(() => SeamlessEraseService.CreatePreviewPatch(source, region));
        Assert.Equal(before, Pixels(source));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void OnePixelWideBackgroundDonorPreservesItsConstantColor(bool horizontal)
    {
        var width = horizontal ? 31 : 2; var height = horizontal ? 2 : 31;
        var region = new Int32Rect(0, 0, horizontal ? width : 1, horizontal ? 1 : height);
        var source = Build(width, height, (x, y) => Inside(region, x, y) ? [0, 0, 0] : [82, 147, 213]);
        var patch = SeamlessEraseService.CreatePatch(source, region); var actual = Pixels(patch);
        for (var at = 0; at < actual.Length; at += 4)
        {
            Assert.Equal(82, actual[at]); Assert.Equal(147, actual[at + 1]); Assert.Equal(213, actual[at + 2]); Assert.Equal(255, actual[at + 3]);
        }
    }

    [Fact]
    public void BoundedPreviewAndFullPatchEvaluateTheSameSourceCoordinates()
    {
        var region = new Int32Rect(32, 32, 768, 384);
        var source = Build(832, 448, (x, y) => Inside(region, x, y) ? [0, 0, 0] : [(byte)(40 + x / 8), (byte)(50 + y / 4), (byte)(30 + (x + y) / 8)]);
        var before = Pixels(source); var full = SeamlessEraseService.CreatePatch(source, region); var preview = SeamlessEraseService.CreatePreviewPatch(source, region);
        Assert.Equal(256, preview.PixelWidth); Assert.Equal(128, preview.PixelHeight); Assert.True(preview.IsFrozen);
        Assert.Equal(source.DpiX, preview.DpiX); Assert.Equal(source.DpiY, preview.DpiY);
        var fullBytes = Pixels(full); var expected = new byte[preview.PixelWidth * preview.PixelHeight * 4];
        // With an exact 3:1 size ratio, the preview pixel centers map to integer
        // full-resolution source coordinates (3*x+1, 3*y+1), without interpolation.
        for (var y = 0; y < preview.PixelHeight; y++) for (var x = 0; x < preview.PixelWidth; x++)
            Array.Copy(fullBytes, ((3 * y + 1) * full.PixelWidth + 3 * x + 1) * 4, expected, (y * preview.PixelWidth + x) * 4, 4);
        Assert.Equal(expected, Pixels(preview)); Assert.Equal(before, Pixels(source));
    }

    private static bool Inside(Int32Rect region, int x, int y) => x >= region.X && x < region.X + region.Width && y >= region.Y && y < region.Y + region.Height;
    private static BitmapSource Build(int width, int height, Func<int, int, byte[]> color)
    {
        var bytes = new byte[width * height * 4];
        for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
        {
            var value = color(x, y); var at = (y * width + x) * 4;
            bytes[at] = value[0]; bytes[at + 1] = value[1]; bytes[at + 2] = value[2]; bytes[at + 3] = 255;
        }
        var source = BitmapSource.Create(width, height, 144, 144, PixelFormats.Bgra32, null, bytes, width * 4); source.Freeze(); return source;
    }

    private static BitmapSource Source(Int32Rect region, bool gradient, bool alternate, out byte[] pixels)
    {
        const int width = 128, height = 96; pixels = new byte[width * height * 4];
        for (var y = 0; y < height; y++)
        for (var x = 0; x < width; x++)
        {
            var color = Background(x, y, gradient); var at = (y * width + x) * 4;
            if (x >= region.X && x < region.X + region.Width && y >= region.Y && y < region.Y + region.Height)
                color = alternate ? [255, 0, 0] : ((x + y) % 2 == 0 ? [0, 0, 0] : [0, 0, 255]);
            pixels[at] = color[0]; pixels[at + 1] = color[1]; pixels[at + 2] = color[2]; pixels[at + 3] = 255;
        }
        var source = BitmapSource.Create(width, height, 144, 144, PixelFormats.Bgra32, null, pixels, width * 4); source.Freeze(); return source;
    }
    private static byte[] Background(int x, int y, bool gradient) => gradient ? [(byte)(30 + x), (byte)(50 + y), (byte)(20 + x + y)] : [82, 147, 213];
    private static byte[] Pixels(BitmapSource source) { var bytes = new byte[source.PixelWidth * source.PixelHeight * 4]; source.CopyPixels(bytes, source.PixelWidth * 4, 0); return bytes; }
    private static void AssertPatch(BitmapSource patch, Int32Rect region)
    {
        Assert.True(patch.IsFrozen); Assert.Equal(region.Width, patch.PixelWidth); Assert.Equal(region.Height, patch.PixelHeight);
        Assert.Equal(144, patch.DpiX); Assert.Equal(144, patch.DpiY);
    }
}
