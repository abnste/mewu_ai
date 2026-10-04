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
    public void SmallHolesInLowContrastRgbTextureDoNotCreateColorsOutsideTheBackgroundRange()
    {
        // Four alternating cross-shaped texture details: this is deliberately
        // non-planar, but every clean channel lies in the narrow 116..140 range.
        var centers = new[] { (24, 24), (48, 24), (24, 48), (48, 48) };
        var region = new Int32Rect(16, 16, 40, 40);
        var mask = Mask(region.Width, region.Height, (x, y) =>
            centers.Contains((x + region.X, y + region.Y)) ? (byte)255 : (byte)0);
        byte[] Background(int x, int y)
        {
            foreach (var (cx, cy) in centers)
            {
                var dx = Math.Abs(x - cx); var dy = Math.Abs(y - cy);
                if (dx > 4 || dy > 4) continue;
                var cross = dx + dy <= 1;
                var colors = new byte[3];
                for (var channel = 0; channel < 3; channel++)
                    colors[channel] = cross == ((cx / 24 + cy / 24 + channel) % 2 == 0) ? (byte)140 : (byte)116;
                return colors;
            }
            return [128, 128, 128];
        }
        var source = Source(80, 80, (x, y) => Covered(region, mask, x, y) ? [0, 255, 0] : Background(x, y));
        var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        foreach (var (x, y) in centers)
        {
            var at = ((y - region.Y) * region.Width + x - region.X) * 4;
            for (var channel = 0; channel < 3; channel++) Assert.InRange((int)patch[at + channel], 116, 140);
            Assert.Equal(255, patch[at + 3]);
        }
    }

    [Fact]
    public void HealingNonPlanarGrayscaleNeverIntroducesAColorCast()
    {
        var region = new Int32Rect(12, 10, 64, 48);
        var mask = Mask(region.Width, region.Height, (x, y) =>
            Math.Abs(y - (12 + x / 3)) <= 7 ? (byte)255 : (byte)0);
        var source = Source(96, 72, (x, y) =>
        {
            if (Covered(region, mask, x, y)) return [0, 0, 255];
            var gray = (byte)(80 + ((x / 3 + y / 5) % 2) * 30 + (x * 17 + y * 11) % 7);
            return [gray, gray, gray];
        });
        var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        for (var i = 0; i < mask.Length; i++)
        {
            if (mask[i] == 0) continue;
            Assert.Equal(patch[i * 4], patch[i * 4 + 1]);
            Assert.Equal(patch[i * 4], patch[i * 4 + 2]);
            Assert.Equal(255, patch[i * 4 + 3]);
        }
    }

    [Theory]
    [InlineData(37)]
    [InlineData(187)]
    public void InvisibleSaturatedDonorsCannotTintVisibleNeutralBackground(byte gray)
    {
        var region = new Int32Rect(16, 12, 32, 24);
        var mask = Enumerable.Repeat((byte)255, region.Width * region.Height).ToArray();
        var bytes = new byte[64 * 48 * 4];
        for (var y = 0; y < 48; y++) for (var x = 0; x < 64; x++)
        {
            var at = (y * 64 + x) * 4;
            var hidden = ((x / 3 + y / 3) % 2) == 0;
            bytes[at] = bytes[at + 2] = hidden ? (byte)255 : gray;
            bytes[at + 1] = hidden ? (byte)0 : gray;
            bytes[at + 3] = hidden ? (byte)0 : (byte)255;
            if (Covered(region, mask, x, y))
            { bytes[at] = 0; bytes[at + 1] = 255; bytes[at + 2] = 0; bytes[at + 3] = 255; }
        }
        var source = FromPixels(64, 48, bytes);
        var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        var visible = 0;
        for (var at = 0; at < patch.Length; at += 4)
        {
            var alpha = patch[at + 3];
            if (alpha == 0) continue;
            visible++;
            // Compare the visible result over its known neutral background,
            // rather than requiring a particular repair algorithm or opacity.
            for (var channel = 0; channel < 3; channel++)
            {
                var composited = (patch[at + channel] * alpha + gray * (255 - alpha) + 127) / 255;
                Assert.InRange(Math.Abs(composited - gray), 0, 1);
            }
        }
        Assert.True(visible > mask.Length / 2, "Repair must retain visible background, not pass by returning a transparent patch.");
        Assert.Equal(bytes, Pixels(source));
    }

    [Fact]
    public void RepeatedOverlappingBrushRepairsDoNotAmplifyLowContrastColorEdges()
    {
        var region = new Int32Rect(16, 12, 64, 48);
        var source = Source(96, 72, (x, y) =>
        [
            (byte)(116 + ((x / 3 + y / 5) % 2) * 24),
            (byte)(116 + ((x / 5 + y / 3) % 2) * 24),
            (byte)(116 + ((x / 4 + y / 4) % 2) * 24)
        ]);
        for (var pass = 0; pass < 8; pass++)
        {
            var cx = 18 + pass * 3; var cy = 24 + (pass % 3 - 1) * 3;
            var mask = Mask(region.Width, region.Height, (x, y) =>
                (x - cx) * (x - cx) + (y - cy) * (y - cy) <= 81 ? (byte)255 : (byte)0);
            var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
            var composited = Pixels(source);
            for (var y = 0; y < region.Height; y++) for (var x = 0; x < region.Width; x++)
            {
                var index = y * region.Width + x;
                if (mask[index] == 0) continue;
                var at = index * 4;
                for (var channel = 0; channel < 3; channel++) Assert.InRange((int)patch[at + channel], 116, 140);
                Assert.Equal(255, patch[at + 3]);
                var target = ((region.Y + y) * source.PixelWidth + region.X + x) * 4;
                Array.Copy(patch, at, composited, target, 4);
            }
            // The next stroke sees the committed previous repair, just as users
            // brushing repeatedly over partially overlapping areas do.
            source = FromPixels(source.PixelWidth, source.PixelHeight, composited);
        }
    }

    [Theory]
    [InlineData(1)]
    [InlineData(3)]
    public void ShortColoredEdgesBesideTheMaskDoNotPolluteTheSurroundingGrayBackground(int thickness)
    {
        const byte gray = 92;
        var region = new Int32Rect(24, 20, 32, 24);
        var mask = Enumerable.Repeat((byte)255, region.Width * region.Height).ToArray();
        var source = Source(80, 64, (x, y) =>
        {
            if (Covered(region, mask, x, y)) return [0, 255, 0];
            if (x >= region.X - thickness && x < region.X && y >= region.Y + 4 && y < region.Y + 17)
                return ((y - region.Y) % 3) switch { 0 => [225, 20, 20], 1 => [20, 225, 20], _ => [20, 20, 225] };
            return [gray, gray, gray];
        });
        var before = Pixels(source);
        var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        for (var at = 0; at < patch.Length; at += 4)
        {
            for (var channel = 0; channel < 3; channel++) Assert.InRange(Math.Abs(patch[at + channel] - gray), 0, 2);
            Assert.Equal(255, patch[at + 3]);
        }
        Assert.Equal(before, Pixels(source));
    }

    [Fact]
    public void ARealWideTwoColorBoundaryIsNotReplacedByOneBackgroundColor()
    {
        var region = new Int32Rect(24, 20, 48, 36);
        var mask = Enumerable.Repeat((byte)255, region.Width * region.Height).ToArray();
        byte[] left = [60, 110, 170]; byte[] right = [180, 130, 75];
        var source = Source(96, 80, (x, y) => Covered(region, mask, x, y) ? [0, 255, 0] : x < 48 ? left : right);
        var before = Pixels(source);
        var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        // These points are well inside the mask and far from the genuine edge.
        foreach (var x in new[] { 3, 4, region.Width - 5, region.Width - 4 })
        for (var y = region.Height / 2 - 2; y <= region.Height / 2 + 2; y++)
        {
            var expected = x < region.Width / 2 ? left : right;
            var at = (y * region.Width + x) * 4;
            for (var channel = 0; channel < 3; channel++) Assert.InRange(Math.Abs(patch[at + channel] - expected[channel]), 0, 8);
            Assert.Equal(255, patch[at + 3]);
        }
        Assert.Equal(before, Pixels(source));
    }

    [Fact]
    public void AThinLineEnteringOppositeMaskEdgesIsNotEntirelyClassifiedAsBackgroundNoise()
    {
        const byte gray = 128;
        var region = new Int32Rect(24, 20, 48, 36);
        var mask = Enumerable.Repeat((byte)255, region.Width * region.Height).ToArray();
        var source = Source(96, 80, (x, y) => Covered(region, mask, x, y) ? [0, 255, 0] : x == 48 ? [25, 45, 220] : [gray, gray, gray]);
        var before = Pixels(source);
        var patch = Pixels(MaskedInpaintingService.CreatePatch(source, region, mask, TestContext.Current.CancellationToken));
        foreach (var firstRow in new[] { 0, region.Height - 4 })
        {
            var strongestDifference = 0;
            for (var y = firstRow; y < firstRow + 4; y++)
            for (var x = 23; x <= 25; x++)
            for (var channel = 0; channel < 3; channel++)
                strongestDifference = Math.Max(strongestDifference, Math.Abs(patch[(y * region.Width + x) * 4 + channel] - gray));
            Assert.True(strongestDifference >= 16, "A continuous line must remain visible where it enters each edge; precise reconstruction of its full length is not required.");
        }
        Assert.Equal(before, Pixels(source));
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
    private static BitmapSource FromPixels(int width, int height, byte[] pixels)
    {
        var source = BitmapSource.Create(width, height, 144, 120, PixelFormats.Bgra32, null, pixels, width * 4);
        source.Freeze(); return source;
    }
}
