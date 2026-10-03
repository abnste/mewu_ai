// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class MaskedInpaintingServiceTests
{
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void CurvedMaskWithHoleAndDisconnectedIslandRepairsOnlyCoveredPixels(bool gradient)
    {
        var region = new Int32Rect(12, 10, 64, 48);
        var mask = Mask(region.Width, region.Height, (x, y) =>
        {
            var radius = Math.Pow(x - 25, 2) + Math.Pow(y - 23, 2);
            return radius >= 70 && radius <= 170 ? (byte)255 : x >= 51 && x < 57 && y >= 7 && y < 13 ? (byte)128 : (byte)0;
        });
        byte[] Background(int x, int y) => gradient ? [(byte)(30 + x), (byte)(60 + y), (byte)(20 + x + y)] : [70, 130, 205];
        var source = Source(96, 72, (x, y) => Covered(region, mask, x, y) ? [0, 0, 255] : Background(x, y));
        var original = Pixels(source); var originalMask = (byte[])mask.Clone();
        var patch = MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken); var pixels = Pixels(patch);
        Assert.True(patch.IsFrozen); Assert.Equal(PixelFormats.Bgra32, patch.Format); Assert.Equal(144, patch.DpiX); Assert.Equal(120, patch.DpiY);
        for (var y = 0; y < region.Height; y++) for (var x = 0; x < region.Width; x++)
        {
            var index = y * region.Width + x; var at = index * 4;
            if (mask[index] == 0) { Assert.Equal(new byte[4], pixels[at..(at + 4)]); continue; }
            var expected = Background(region.X + x, region.Y + y);
            for (var channel = 0; channel < 3; channel++) Assert.InRange(Math.Abs(pixels[at + channel] - expected[channel]), 0, 3);
            Assert.Equal(mask[index], pixels[at + 3]);
        }
        Assert.Equal(original, Pixels(source)); Assert.Equal(originalMask, mask);
    }

    [Theory]
    [InlineData(0, 0, 32, 24)]
    [InlineData(64, 48, 32, 24)]
    [InlineData(20, 20, 32, 24)]
    public void FullyMaskedRoiUsesUnmaskedContextOutsideItsBounds(int x, int y, int width, int height)
    {
        var region = new Int32Rect(x, y, width, height); var mask = Enumerable.Repeat((byte)255, width * height).ToArray();
        var source = Source(96, 72, (px, py) => Covered(region, mask, px, py) ? [250, 0, 170] : [65, 140, 210]);
        var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        for (var at = 0; at < patch.Length; at += 4) Assert.Equal(new byte[] { 65, 140, 210, 255 }, patch[at..(at + 4)]);
    }

    [Fact]
    public void NonPlanarTextureFallbackIsDeterministicAndIgnoresAllMaskedOriginalColors()
    {
        var region = new Int32Rect(8, 8, 64, 48);
        var mask = Mask(region.Width, region.Height, (x, y) => Math.Abs(y - (12 + x % 23)) < 4 || x is > 40 and < 49 && y is > 26 and < 39 ? (byte)(x % 2 == 0 ? 255 : 128) : (byte)0);
        byte[] Texture(int x, int y) => [(byte)((x * 67 + y * 31) % 256), (byte)((x / 3 + y / 3) % 2 == 0 ? 220 : 35), (byte)((x * x + y * 17) % 256)];
        var black = Source(80, 64, (x, y) => Covered(region, mask, x, y) ? [0, 0, 0] : Texture(x, y));
        var red = Source(80, 64, (x, y) => Covered(region, mask, x, y) ? [0, 0, 255] : Texture(x, y));
        var blackBefore = Pixels(black); var redBefore = Pixels(red); var maskBefore = (byte[])mask.Clone();
        var first = Pixels(MaskedInpaintingService.CreatePatch(black, region, mask, TestContext.Current.CancellationToken));
        Assert.Equal(first, Pixels(MaskedInpaintingService.CreatePatch(red, region, mask, TestContext.Current.CancellationToken)));
        Assert.Equal(first, Pixels(MaskedInpaintingService.CreatePatch(black, region, mask, TestContext.Current.CancellationToken)));
        for (var i = 0; i < mask.Length; i++)
        {
            Assert.Equal(mask[i], first[i * 4 + 3]);
            if (mask[i] == 0) Assert.Equal(new byte[4], first[(i * 4)..(i * 4 + 4)]);
        }
        Assert.Equal(blackBefore, Pixels(black)); Assert.Equal(redBefore, Pixels(red)); Assert.Equal(maskBefore, mask);
    }

    [Fact]
    public void TransparentBackgroundAlphaIsMultipliedByMaskCoverage()
    {
        var region = new Int32Rect(4, 4, 8, 8); var mask = Enumerable.Repeat((byte)128, 64).ToArray();
        var source = Source(16, 16, (_, _) => [75, 130, 210], 128);
        var result = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        for (var at = 0; at < result.Length; at += 4) Assert.Equal(64, result[at + 3]);
    }

    [Fact]
    public void EmptyMaskReturnsTransparentPatchAndWholeSourceMaskRejectsMissingDonors()
    {
        var source = Source(20, 16, (_, _) => [75, 130, 210]); var original = Pixels(source); var region = new Int32Rect(0, 0, 20, 16);
        Assert.All(Pixels(MaskedInpaintingService.CreatePatch(source, region, new byte[320], TestContext.Current.CancellationToken)), value => Assert.Equal(0, value));
        var error = Assert.Throws<InvalidOperationException>(() => MaskedInpaintingService.CreatePatch(source, region, Enumerable.Repeat((byte)255, 320).ToArray(), TestContext.Current.CancellationToken));
        Assert.Equal("Masked repair requires surrounding background pixels.", error.Message); Assert.Equal(original, Pixels(source));
    }

    [Fact]
    public void InvalidMaskAndCancelledRequestLeaveInputsUnchanged()
    {
        var source = Source(32, 24, (_, _) => [75, 130, 210]); var original = Pixels(source); var region = new Int32Rect(4, 4, 16, 12); var mask = Enumerable.Repeat((byte)255, 192).ToArray();
        Assert.Throws<ArgumentException>(() => MaskedInpaintingService.CreatePatch(source, region, new byte[191], TestContext.Current.CancellationToken));
        using var cancellation = new CancellationTokenSource(); cancellation.Cancel();
        Assert.Throws<OperationCanceledException>(() => MaskedInpaintingService.CreatePatch(source, region, mask, cancellation.Token));
        Assert.Equal(original, Pixels(source)); Assert.All(mask, value => Assert.Equal(255, value));
    }

    private static bool Covered(Int32Rect r, byte[] mask, int x, int y) => x >= r.X && y >= r.Y && x < r.X + r.Width && y < r.Y + r.Height && mask[(y - r.Y) * r.Width + x - r.X] != 0;
    private static byte[] Mask(int width, int height, Func<int, int, byte> coverage)
    { var values = new byte[width * height]; for (var y = 0; y < height; y++) for (var x = 0; x < width; x++) values[y * width + x] = coverage(x, y); return values; }
    private static BitmapSource Source(int width, int height, Func<int, int, byte[]> color, byte alpha = 255)
    {
        var bytes = new byte[width * height * 4];
        for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
        { var c = color(x, y); var at = (y * width + x) * 4; bytes[at] = c[0]; bytes[at + 1] = c[1]; bytes[at + 2] = c[2]; bytes[at + 3] = alpha; }
        var source = BitmapSource.Create(width, height, 144, 120, PixelFormats.Bgra32, null, bytes, width * 4); source.Freeze(); return source;
    }
    private static byte[] Pixels(BitmapSource source) { var bytes = new byte[source.PixelWidth * source.PixelHeight * 4]; source.CopyPixels(bytes, source.PixelWidth * 4, 0); return bytes; }
}
