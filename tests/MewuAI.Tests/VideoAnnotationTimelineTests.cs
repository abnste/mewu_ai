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

    [Theory]
    [InlineData(4.09,true)]
    [InlineData(4.31,true)]
    [InlineData(4.33,false)]
    public void PointMarkerUsesBoundedLiveFrameTolerance(double presentedTime,bool expected)
    {
        var annotation=new AiAnnotation(.2,.3,.1,.1,"点",0,4.2,4.2,[new(4.2,.2,.3,.1,.1)]);
        var visible=VideoAnnotationTimeline.TryInterpolateForPresentation(annotation,presentedTime,VideoAnnotationTimeline.LiveFrameToleranceSeconds,out var frame);
        Assert.Equal(expected,visible);if(visible)Assert.Equal(4.2,frame.Time,8);
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

    [Fact] public void SlightPointOvershootIsClampedToActualDuration()
    {
        var annotation=new AiAnnotation(.2,.3,.1,.1,"片尾按钮",1,10.2,10.2,[new(10.2,.2,.3,.1,.1)]);
        Assert.True(VideoAnnotationTimeline.TryFitToDuration(annotation,10,out var fitted,out var clamped));
        Assert.True(clamped);Assert.Equal(10,fitted.StartTime);Assert.Equal(10,fitted.EndTime);Assert.Equal(10,Assert.Single(fitted.Keyframes!).Time);
    }

    [Fact] public void SlightRangeOvershootClampsEndAndLastKeyframeTogether()
    {
        var annotation=new AiAnnotation(.1,.2,.2,.2,"片尾动作",0,9.5,10.2,[new(9.5,.1,.2,.2,.2),new(10.2,.3,.4,.2,.2)]);
        Assert.True(VideoAnnotationTimeline.TryFitToDuration(annotation,10,out var fitted,out var clamped));
        Assert.True(clamped);Assert.Equal(10,fitted.EndTime);Assert.Equal(new[]{9.5,10},fitted.Keyframes!.Select(frame=>frame.Time));
    }

    [Fact] public void LargeDurationOvershootIsRejected()
    {
        var annotation=new AiAnnotation(.2,.3,.1,.1,"错误时间",0,10.5,10.5,[new(10.5,.2,.3,.1,.1)]);
        Assert.False(VideoAnnotationTimeline.TryFitToDuration(annotation,10,out _,out _));
    }

    [Fact] public void SingleVideoAcceptsTimelineAnnotationWithWrongModelIndex()
    {
        Assert.True(VideoAnnotationTimeline.TryResolveTargetIndex(1,true,[true],out var target,out var remapped));
        Assert.Equal(0,target);Assert.True(remapped);
    }

    [Fact] public void MultipleAttachmentsKeepStrictFullAttachmentIndexMapping()
    {
        Assert.False(VideoAnnotationTimeline.TryResolveTargetIndex(1,true,[true,false],out _,out _));
        Assert.True(VideoAnnotationTimeline.TryResolveTargetIndex(0,true,[true,false],out var videoTarget,out var remapped));Assert.Equal(0,videoTarget);Assert.False(remapped);
        Assert.True(VideoAnnotationTimeline.TryResolveTargetIndex(1,false,[true,false],out var imageTarget,out _));Assert.Equal(1,imageTarget);
    }
}
