using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class PinnedImageDropPlacementTests
{
    [Fact]
    public void Place_CentersNaturalSizeAtDropPoint()
    {
        var result=PinnedImageDropPlacement.Place(new ScreenRect(0,0,1920,1080),960,540,640,480);

        Assert.Equal(new ScreenRect(640,300,640,480),result);
    }

    [Fact]
    public void Place_FitsLargeImageAndPreservesAspectRatio()
    {
        var result=PinnedImageDropPlacement.Place(new ScreenRect(0,0,1920,1080),960,540,4000,2000);

        Assert.Equal(900,result.Width);
        Assert.Equal(450,result.Height);
        Assert.Equal(2d,result.Width/(double)result.Height,6);
    }

    [Fact]
    public void Place_ClampsToNegativeMonitorWorkingArea()
    {
        var workingArea=new ScreenRect(-1920,0,1920,1040);
        var result=PinnedImageDropPlacement.Place(workingArea,-1915,4,800,600);

        Assert.Equal(-1904,result.X);
        Assert.Equal(16,result.Y);
        Assert.True(result.Right<=workingArea.Right-16);
        Assert.True(result.Bottom<=workingArea.Bottom-16);
    }
}
