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

    [Fact] public void FirstMarkerUsesEarliestKeyframeAcrossAnnotations()
    {
        var later=new AiAnnotation(.1,.1,.2,.2,"later",0,8,10,[new(8,.1,.1,.2,.2),new(10,.2,.2,.2,.2)]);
        var earlier=new AiAnnotation(.2,.2,.2,.2,"earlier",0,2,4,[new(2,.2,.2,.2,.2),new(4,.3,.3,.2,.2)]);
        Assert.True(VideoAnnotationTimeline.TryGetFirstMarker([later,earlier],out var annotation,out var frame));Assert.Same(earlier,annotation);Assert.Equal(2,frame.Time);
    }

    [Fact] public void RangeAnnotationCreatesOnePlaybackActionWithoutDuplicateEndpointLinks()
    {
        var actions=VideoAnnotationTimeline.CreateAnswerActions([Tracked]);
        var action=Assert.Single(actions);Assert.Equal(VideoAnnotationAnswerActionKind.PlayRange,action.Kind);Assert.Null(action.Frame);
    }

    [Fact] public void DistinctPointAnnotationsCreateDistinctJumpActions()
    {
        var first=new AiAnnotation(.1,.1,.2,.2,"red circle",0,1,1,[new(1,.1,.1,.2,.2)]);var second=new AiAnnotation(.4,.4,.2,.2,"phone",0,5,5,[new(5,.4,.4,.2,.2)]);
        var actions=VideoAnnotationTimeline.CreateAnswerActions([first,second]);Assert.Equal(2,actions.Count);Assert.All(actions,action=>Assert.Equal(VideoAnnotationAnswerActionKind.JumpToFrame,action.Kind));Assert.Equal(new[]{1d,5d},actions.Select(action=>action.Frame!.Time));
    }
}
