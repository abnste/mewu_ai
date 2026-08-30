using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using Xunit;

namespace MewuAI.Tests;

public sealed class RecordingLayoutTests
{
    [Fact]
    public void CreateSlices_ComposesRegionAcrossNegativeAndPrimaryDisplays()
    {
        var displays=new[]{new ScreenRect(-1920,0,1920,1080),new ScreenRect(0,0,1920,1080)};
        var slices=RecordingLayoutService.CreateSlices(new ScreenRect(-100,100,300,200),displays);
        Assert.Equal(2,slices.Count);
        Assert.Equal(new ScreenRect(-100,100,100,200),slices[0].Source);
        Assert.Equal(new ScreenRect(0,0,100,200),slices[0].Output);
        Assert.Equal(new ScreenRect(0,100,200,200),slices[1].Source);
        Assert.Equal(new ScreenRect(100,0,200,200),slices[1].Output);
    }

    [Fact]
    public void CreateSlices_DeduplicatesMirroredDisplayBounds()
    {
        var display=new ScreenRect(0,0,1920,1080);

        var slices=RecordingLayoutService.CreateSlices(
            new ScreenRect(100,100,640,360),
            new[]{display,display});

        var slice=Assert.Single(slices);
        Assert.Equal(new ScreenRect(100,100,640,360),slice.Source);
        Assert.Equal(new ScreenRect(0,0,640,360),slice.Output);
    }
}
