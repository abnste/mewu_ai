// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class SeamlessLiftServiceTests
{
    [Theory]
    [InlineData(255, 255, 255, 0, 0, 0)]
    [InlineData(25, 35, 45, 255, 255, 255)]
    [InlineData(180, 210, 235, 25, 65, 220)]
    public void OpaqueContentHasTransparentSurroundingsAndKeepsItsColor(int b, int g, int r, int fb, int fg, int fr)
    {
        var backgroundColor = new byte[] { (byte)b, (byte)g, (byte)r };
        var foregroundColor = new byte[] { (byte)fb, (byte)fg, (byte)fr };
        var source = Build(24, 20, (x, y) => x >= 8 && x < 16 && y >= 7 && y < 13 ? foregroundColor : backgroundColor);
        var region = new Int32Rect(4, 3, 16, 14); var patch = Build(16, 14, (_, _) => backgroundColor);
        var before = Pixels(source); var patchBefore = Pixels(patch); var lifted = SeamlessEraseService.ExtractContent(source, region, patch); var bytes = Pixels(lifted);
        Assert.True(lifted.IsFrozen); Assert.Equal(PixelFormats.Bgra32, lifted.Format); Assert.Equal(144, lifted.DpiX); Assert.Equal(144, lifted.DpiY);
        Assert.Equal(16, lifted.PixelWidth); Assert.Equal(14, lifted.PixelHeight);
        for (var y = 0; y < 14; y++) for (var x = 0; x < 16; x++)
        {
            var at = (y * 16 + x) * 4; var content = x >= 4 && x < 12 && y >= 4 && y < 10;
            Assert.Equal(content ? 255 : 0, bytes[at + 3]);
            if (content) for (var c = 0; c < 3; c++) Assert.InRange(Math.Abs(bytes[at + c] - foregroundColor[c]), 0, 1);
        }
        Assert.Equal(before, Pixels(source)); Assert.Equal(patchBefore, Pixels(patch));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void KnownAntialiasedEdgeRecomposesAndDoesNotCarryTheOldBackground(bool premultiplied)
    {
        byte[] background = [230, 240, 250], foreground = [20, 45, 65], nextBackground = [180, 70, 30];
        double Alpha(int x, int y) => y < 4 || y >= 12 || x < 4 || x >= 12 ? 0 : x == 4 || x == 11 || y == 4 || y == 11 ? .5 : 1;
        var source = Build(16, 16, (x, y) => Enumerable.Range(0, 3).Select(c => (byte)Math.Round(Alpha(x, y) * foreground[c] + (1 - Alpha(x, y)) * background[c])).ToArray());
        BitmapSource input = source;
        if (premultiplied) { var converted = new FormatConvertedBitmap(source, PixelFormats.Pbgra32, null, 0); converted.Freeze(); input = converted; }
        var patch = Build(16, 16, (_, _) => background); var before = Pixels(input); var lifted = SeamlessEraseService.ExtractContent(input, new Int32Rect(0, 0, 16, 16), patch); var bytes = Pixels(lifted);
        var original = Pixels(source);
        for (var y = 0; y < 16; y++) for (var x = 0; x < 16; x++)
        {
            var at = (y * 16 + x) * 4; var alpha = bytes[at + 3] / 255d;
            for (var c = 0; c < 3; c++)
            {
                var recomposed = alpha * bytes[at + c] + (1 - alpha) * background[c];
                Assert.InRange(Math.Abs(recomposed - original[at + c]), 0, 3);
                var moved = alpha * bytes[at + c] + (1 - alpha) * nextBackground[c];
                var expected = Alpha(x, y) * foreground[c] + (1 - Alpha(x, y)) * nextBackground[c];
                Assert.InRange(Math.Abs(moved - expected), 0, 4);
            }
        }
        Assert.Equal(before, Pixels(input));
    }

    [Fact]
    public void ABackgroundOnlyRegionProducesNoOpaqueRectangle()
    {
        var source = Build(24, 20, (x, y) => [(byte)(40 + x), (byte)(60 + y), 180]);
        var region = new Int32Rect(4, 3, 16, 14); var patch = new CroppedBitmap(source, region); patch.Freeze();
        var lifted = SeamlessEraseService.ExtractContent(source, region, patch);
        Assert.All(Pixels(lifted).Where((_, index) => index % 4 == 3), alpha => Assert.Equal(0, alpha));
    }

    private static BitmapSource Build(int width, int height, Func<int, int, byte[]> color)
    {
        var bytes = new byte[width * height * 4];
        for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
        { var c = color(x, y); var at = (y * width + x) * 4; bytes[at] = c[0]; bytes[at + 1] = c[1]; bytes[at + 2] = c[2]; bytes[at + 3] = 255; }
        var bitmap = BitmapSource.Create(width, height, 144, 144, PixelFormats.Bgra32, null, bytes, width * 4); bitmap.Freeze(); return bitmap;
    }
    private static byte[] Pixels(BitmapSource source) { var bytes = new byte[source.PixelWidth * source.PixelHeight * 4]; source.CopyPixels(bytes, source.PixelWidth * 4, 0); return bytes; }
}
