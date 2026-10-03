// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Xunit;

namespace MewuAI.Tests;

public sealed class CapturePixelGeometryTests
{
    [Fact]
    public void RegionSupportsNegativeDesktopOrigins()
    {
        var desktop = new ScreenRect(-1920, -1080, 5760, 3240);
        CapturePixelGeometry.ValidateRegion(new(-1920, -1080, 1920, 1080), desktop);
        CapturePixelGeometry.ValidateRegion(new(3839, 2159, 1, 1), desktop);
        Assert.Throws<ArgumentOutOfRangeException>(() => CapturePixelGeometry.ValidateRegion(new(3839, 2159, 2, 1), desktop));
    }

    [Fact]
    public void RegionRejectsWrappedRightEdge()
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => CapturePixelGeometry.ValidateRegion(new(int.MaxValue - 2, 0, 10, 1), new(0, 0, int.MaxValue, 10)));
    }

    [Fact]
    public void CropPreservesPixelsAndClipsNegativeOrigin()
    {
        var pixels = new byte[4 * 3 * 4];
        for (var i = 0; i < pixels.Length; i++) pixels[i] = (byte)i;
        var source = BitmapSource.Create(4, 3, 96, 96, PixelFormats.Bgra32, null, pixels, 16);
        source.Freeze();
        var result = ScreenCaptureService.Crop(source, new Int32Rect(-1, 1, 3, 3));
        Assert.Equal(2, result.PixelWidth);
        Assert.Equal(2, result.PixelHeight);
        Assert.True(result.IsFrozen);
        var output = new byte[16];
        result.CopyPixels(output, 8, 0);
        Assert.Equal(pixels[16..24].Concat(pixels[32..40]).ToArray(), output);
    }

    [Fact]
    public void CropRetainsDistinctInvalidSizeAndNoIntersectionErrors()
    {
        var source = BitmapSource.Create(1, 1, 96, 96, PixelFormats.Bgra32, null, new byte[4], 4);
        source.Freeze();
        Assert.Throws<ArgumentNullException>(() => ScreenCaptureService.Crop(null!, new Int32Rect(0, 0, 0, 0)));
        Assert.Throws<ArgumentOutOfRangeException>(() => ScreenCaptureService.Crop(new BitmapImage(), new Int32Rect(0, 0, 0, 1)));
        Assert.Throws<ArgumentOutOfRangeException>(() => ScreenCaptureService.Crop(source, new Int32Rect(0, 0, 0, 1)));
        Assert.Throws<ArgumentException>(() => ScreenCaptureService.Crop(source, new Int32Rect(1, 0, 1, 1)));
    }
}
