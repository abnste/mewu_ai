using System.Windows;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class PinnedWindowInteractionPolicyTests
{
    [Fact]
    public void OriginalPlacement_ShowsPinnedWindowWithoutActivation()
    {
        Assert.Equal(0x0040u,PinnedWindowInteractionPolicy.ShowWithoutActivationFlags&0x0040u);
        Assert.Equal(0x0010u,PinnedWindowInteractionPolicy.ShowWithoutActivationFlags&0x0010u);
    }

    [Theory]
    [InlineData(0,0,3.9,3.9,false)]
    [InlineData(0,0,4,0,true)]
    [InlineData(10,10,10,6,true)]
    [InlineData(10,10,5,10,true)]
    public void ShouldBeginDrag_UsesSystemStyleAxisThreshold(double startX,double startY,double currentX,double currentY,bool expected)
    {
        Assert.Equal(expected,PinnedWindowInteractionPolicy.ShouldBeginDrag(new Point(startX,startY),new Point(currentX,currentY),4,4));
    }

    [Fact]
    public void ZoomPolicyDoesNotUseTheOldSmallProductCap()
    {
        var maximum=PinnedWindowZoomPolicy.GetMaximumWidth(800,600,24);
        Assert.True(maximum>2_400);
        Assert.InRange(maximum,24, PinnedWindowZoomPolicy.MaxWindowDimension);
    }

    [Fact]
    public void ZoomPolicyPreservesAspectRatioAtItsSafeBound()
    {
        var padding=24d;var width=PinnedWindowZoomPolicy.GetMaximumWidth(2_000,1_000,padding);var height=PinnedWindowZoomPolicy.GetMaximumHeight(2_000,1_000,padding);
        Assert.InRange(width,2_000, PinnedWindowZoomPolicy.MaxWindowDimension);
        Assert.InRange(height,1_000, PinnedWindowZoomPolicy.MaxWindowDimension);
        Assert.True(width-padding>height-padding);
    }
}
