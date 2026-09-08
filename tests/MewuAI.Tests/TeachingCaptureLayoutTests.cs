// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;
public sealed class TeachingCaptureLayoutTests
{
    [Theory]
    [InlineData(0,0)]
    [InlineData(-1920,-200)]
    public void CaptureControlsStayOutsidePixelsAtEveryMonitorEdge(double x,double y)
    {
        var monitor=new Rect(x,y,1920,1080);
        foreach(var capture in new[]{new Rect(x,y,700,500),new Rect(x+100,y+200,700,500),new Rect(x+1200,y+580,720,500),new Rect(x+400,y,1000,1080)})
        {
            var space=CaptureOverlayPolicy.FindCaptureControlSpace(monitor,capture,300,50);
            Assert.False(space.IsEmpty);Assert.True(monitor.Contains(space));Assert.False(capture.IntersectsWith(space));
        }
    }
    [Fact]
    public void ScrollingPreviewDoesNotCoverFinishControls()
    {
        var monitor=new Rect(0,0,1920,1080);var capture=new Rect(100,300,500,400);
        var controls=CaptureOverlayPolicy.FindCaptureControlSpace(monitor,capture,300,50);
        var preview=CaptureOverlayPolicy.FindCaptureControlSpace(monitor,capture,200,260,controls);
        Assert.False(preview.IsEmpty);Assert.False(preview.IntersectsWith(controls));Assert.False(preview.IntersectsWith(capture));
    }
    [Fact]
    public void FullScreenAndOversizedControlsUseKeyboardInsteadOfCoveringCapture()
    {
        var monitor=new Rect(-1920,0,1920,1080);
        Assert.True(CaptureOverlayPolicy.FindCaptureControlSpace(monitor,monitor,300,50).IsEmpty);
        Assert.True(CaptureOverlayPolicy.FindCaptureControlSpace(monitor,new Rect(-1800,100,300,200),2000,50).IsEmpty);
        Assert.True(CaptureOverlayPolicy.FindCaptureControlSpace(monitor,monitor,double.NaN,50).IsEmpty);
    }
}
