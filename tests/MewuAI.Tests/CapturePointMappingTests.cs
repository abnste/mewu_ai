// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;
using Xunit;
namespace MewuAI.Core.Tests;
public sealed class CapturePointMappingTests
{
    [Theory][InlineData(1)][InlineData(1.25)][InlineData(1.75)][InlineData(2)]
    public void PointerAtRightBottomStaysOnLastPixel(double scale)
    {
        var p=CapturePixelGeometry.ToScreenPixelPoint(1920/scale,1080/scale,1920/scale,1080/scale,1920,1080,-1920,-1080);
        Assert.Equal((-1,-1),p);
    }
    [Theory][InlineData(1)][InlineData(1.25)][InlineData(1.75)][InlineData(2)]
    public void ArrowStepIsExactlyOnePhysicalPixel(double scale)
    {
        var step=CapturePixelGeometry.PixelStepInSurface(1,1920/scale,1920);Assert.Equal(1/scale,step,10);
        var a=CapturePixelGeometry.ToScreenPixelPoint(100/scale,100/scale,1920/scale,1080/scale,1920,1080,-1920,-1080);
        var b=CapturePixelGeometry.ToScreenPixelPoint(100/scale+step,100/scale,1920/scale,1080/scale,1920,1080,-1920,-1080);
        Assert.Equal(a.X+1,b.X);Assert.Equal(a.Y,b.Y);
    }
    [Fact]public void PointerOutsideSurfaceClampsWithoutWrapping()
    {Assert.Equal((-1920,-1080),CapturePixelGeometry.ToScreenPixelPoint(-20,-20,1920,1080,1920,1080,-1920,-1080));Assert.Equal((-1,-1),CapturePixelGeometry.ToScreenPixelPoint(double.MaxValue,double.MaxValue,1920,1080,1920,1080,-1920,-1080));}
    [Fact]public void InvalidSurfaceFallsBackToOrigin()
    {Assert.Equal((-3,4),CapturePixelGeometry.ToScreenPixelPoint(0,0,0,10,100,100,-3,4));Assert.Equal((-3,4),CapturePixelGeometry.ToScreenPixelPoint(double.NaN,0,10,10,100,100,-3,4));}
}
