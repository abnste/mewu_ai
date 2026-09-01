using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class VideoAnnotationTimelineTests
{
    private static readonly AiAnnotation Tracked=new(.1,.2,.2,.1,"目标",0,10,12,
    [
        new(10,.1,.2,.2,.1),
        new(12,.5,.4,.4,.3)
    ]);

    [Fact] public void InterpolatesTrackedRectangleAtMidpoint()
    {
        Assert.True(VideoAnnotationTimeline.TryInterpolate(Tracked,11,out var frame));
        Assert.Equal(.3,frame.X,8);Assert.Equal(.3,frame.Y,8);Assert.Equal(.3,frame.Width,8);Assert.Equal(.2,frame.Height,8);
    }

    [Theory]
    [InlineData(9,.1)]
    [InlineData(10,.1)]
    [InlineData(12,.5)]
    [InlineData(13,.5)]
    public void ClampsBeforeAndAfterTimeline(double time,double expectedX)
    {
        Assert.True(VideoAnnotationTimeline.TryInterpolate(Tracked,time,out var frame));Assert.Equal(expectedX,frame.X,8);
    }

    [Fact] public void SinglePointFrameRemainsStable()
    {
        var annotation=new AiAnnotation(.2,.3,.1,.1,"点",0,4.2,4.2,[new(4.2,.2,.3,.1,.1)]);
        Assert.True(VideoAnnotationTimeline.TryInterpolate(annotation,4.2,out var frame));Assert.Equal(.2,frame.X,8);
    }
}
