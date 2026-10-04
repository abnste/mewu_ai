// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class VideoClipTimelineTests
{
    private static VideoClipRange Range(double start,double end)=>new(TimeSpan.FromSeconds(start),TimeSpan.FromSeconds(end));
    private static AiAnnotation Tracked()=>new(.1,.2,.2,.1,"  用户原文 保存/设置\r\n  ",3,1,9,
        [new(1,.1,.2,.2,.1,[new(.1,.2)]),new(5,.5,.4,.4,.3,[new(.5,.4)]),new(9,.9,.6,.6,.5,[new(.9,.6)])],
        "source-user-reference",AiAnnotationKind.Pen,Style:new("#ABCDEF"),Number:7,
        Destination:new(4,"destination-user-reference",.2,.3,.4,.5));

    [Theory]
    [InlineData(-1,3)]
    [InlineData(0,3)]
    [InlineData(2.9,3)]
    [InlineData(3,3)]
    [InlineData(5,5)]
    [InlineData(7,7)]
    [InlineData(7.1,7)]
    [InlineData(10,7)]
    public void PlaybackPositionRemainsInsideRetainedSourceInterval(double requested,double expected)
    {
        Assert.Equal(TimeSpan.FromSeconds(expected),VideoClipTimeline.ClampPosition(TimeSpan.FromSeconds(requested),Range(3,7)));
    }

    [Fact]
    public void FractionalCutBoundariesAndAdjacentTicksRemainExact()
    {
        var range=new VideoClipRange(TimeSpan.FromTicks(12573344),TimeSpan.FromTicks(37820620));
        Assert.Equal(range.Start,VideoClipTimeline.ClampPosition(range.Start-TimeSpan.FromTicks(1),range));
        Assert.Equal(range.Start,VideoClipTimeline.ClampPosition(range.Start,range));
        Assert.Equal(range.End,VideoClipTimeline.ClampPosition(range.End,range));
        Assert.Equal(range.End,VideoClipTimeline.ClampPosition(range.End+TimeSpan.FromTicks(1),range));
        Assert.Equal(range.End-TimeSpan.FromTicks(1),VideoClipTimeline.ClampPosition(range.End-TimeSpan.FromTicks(1),range));
    }

    [Fact]
    public void MovingEitherCutInwardBringsAnExistingPlayheadBackInsideTheRange()
    {
        var position=TimeSpan.FromSeconds(5.5);
        Assert.Equal(TimeSpan.FromSeconds(4.75),VideoClipTimeline.ClampPosition(position,Range(1.25,4.75)));
        position=TimeSpan.FromSeconds(1.5);
        Assert.Equal(TimeSpan.FromSeconds(2.25),VideoClipTimeline.ClampPosition(position,Range(2.25,4.75)));
        Assert.Equal(position,VideoClipTimeline.ClampPosition(position,Range(0,6)));
    }

    [Fact]
    public void ShortSelectedRangeKeepsBothReachableEndpoints()
    {
        var range=new VideoClipRange(TimeSpan.FromTicks(1),TimeSpan.FromTicks(2));
        Assert.Equal(range.Start,VideoClipTimeline.ClampPosition(TimeSpan.Zero,range));
        Assert.Equal(range.End,VideoClipTimeline.ClampPosition(TimeSpan.FromSeconds(1),range));
    }

    [Fact]
    public void ExtremeRequestsClampWithoutArithmeticOverflow()
    {
        var range=Range(3,7);
        Assert.Equal(range.Start,VideoClipTimeline.ClampPosition(TimeSpan.MinValue,range));
        Assert.Equal(range.End,VideoClipTimeline.ClampPosition(TimeSpan.MaxValue,range));
        var tail=new VideoClipRange(TimeSpan.MaxValue-TimeSpan.FromTicks(2),TimeSpan.MaxValue);
        Assert.Equal(tail.Start,VideoClipTimeline.ClampPosition(TimeSpan.MinValue,tail));
        Assert.Equal(tail.End,VideoClipTimeline.ClampPosition(TimeSpan.MaxValue,tail));
    }

    [Theory]
    [InlineData(-1,3)]
    [InlineData(3,3)]
    [InlineData(4,3)]
    public void PlaybackPositionRejectsInvalidRetainedIntervals(double start,double end)
        =>Assert.Throws<ArgumentOutOfRangeException>(()=>VideoClipTimeline.ClampPosition(TimeSpan.Zero,Range(start,end)));

    [Fact]
    public void CrossingBothCutsInterpolatesBoundaryGeometryAndKeepsInteriorFrames()
    {
        var source=Tracked();var originalFrames=source.Keyframes!.ToArray();
        var note=Assert.Single(VideoClipTimeline.Project([source],Range(3,7)).Annotations);
        Assert.Equal(0,note.StartTime);Assert.Equal(4,note.EndTime);
        Assert.Equal(new[]{0d,2d,4d},note.Keyframes!.Select(frame=>frame.Time));
        Assert.Equal(.3,note.Keyframes![0].X,8);Assert.Equal(.7,note.Keyframes[^1].X,8);
        Assert.Equal(.3,Assert.Single(note.Keyframes[0].Points!).X,8);
        Assert.Equal(.7,Assert.Single(note.Keyframes[^1].Points!).X,8);
        Assert.Equal(note.Keyframes[0].X,note.X);Assert.Equal(note.Keyframes[0].Width,note.Width);
        Assert.Equal(originalFrames,source.Keyframes);Assert.Equal(1,source.StartTime);Assert.Equal(9,source.EndTime);
    }

    [Fact]
    public void RetainedAnnotationsKeepUserTextIdentifiersAndStylesVerbatim()
    {
        var source=Tracked();
        var note=Assert.Single(VideoClipTimeline.Project([source],Range(3,7)).Annotations);
        Assert.Equal(source.Text,note.Text);Assert.Equal(source.ReferenceHandle,note.ReferenceHandle);
        Assert.Equal(source.RegionIndex,note.RegionIndex);Assert.Equal(source.Kind,note.Kind);
        Assert.Same(source.Style,note.Style);Assert.Same(source.Destination,note.Destination);Assert.Equal(source.Number,note.Number);
    }

    [Fact]
    public void StaticAnnotationsKeepIdentityAndCardPositionsAreKeyedByProjectedObject()
    {
        var tracked=Tracked();var still=new AiAnnotation(.1,.2,.3,.4,"static");
        var outside=new AiAnnotation(.1,.2,.3,.4,"outside",0,20,21,[new(20,.1,.2,.3,.4),new(21,.2,.2,.3,.4)]);
        var cards=new Dictionary<AiAnnotation,Point>(ReferenceEqualityComparer.Instance)
        {[tracked]=new(.2,.7),[still]=new(.6,.8),[outside]=new(.1,.1)};
        var result=VideoClipTimeline.Project([tracked,still,outside],Range(3,7),cards);
        Assert.Equal(2,result.Annotations.Count);Assert.Same(still,result.Annotations[1]);
        Assert.NotSame(tracked,result.Annotations[0]);Assert.Equal(2,result.CardPositions.Count);
        Assert.False(result.CardPositions.ContainsKey(tracked));Assert.False(result.CardPositions.ContainsKey(outside));
        Assert.Equal(cards[tracked],result.CardPositions[result.Annotations[0]]);
        Assert.Equal(cards[still],result.CardPositions[still]);Assert.Equal(3,cards.Count);
    }

    [Theory]
    [InlineData(3,true)]
    [InlineData(5,true)]
    [InlineData(7,true)]
    [InlineData(2.999,false)]
    [InlineData(7.001,false)]
    public void ExplicitPointMarkersAtEitherBoundaryRemainSingleFrames(double time,bool expected)
    {
        var point=new AiAnnotation(.1,.2,.3,.4,"point",0,time,time,[new(time,.1,.2,.3,.4)]);
        var projected=VideoClipTimeline.Project([point],Range(3,7));
        if(!expected){Assert.Empty(projected.Annotations);return;}
        var note=Assert.Single(projected.Annotations);Assert.Equal(time-3,note.StartTime!.Value,8);
        Assert.Equal(note.StartTime,note.EndTime);Assert.Equal(time-3,Assert.Single(note.Keyframes!).Time,8);
    }

    [Fact]
    public void NonzeroRangesOnlyTouchingTheCutDoNotBecomePointMarkers()
    {
        var before=new AiAnnotation(.1,.2,.3,.4,"before",0,1,3,[new(1,.1,.2,.3,.4),new(3,.1,.2,.3,.4)]);
        var after=before with{Text="after",StartTime=7,EndTime=9,Keyframes=[new(7,.1,.2,.3,.4),new(9,.1,.2,.3,.4)]};
        Assert.Empty(VideoClipTimeline.Project([before,after],Range(3,7)).Annotations);
    }

    [Fact]
    public void CutExactlyOnExistingKeyframeDoesNotDuplicateEndpoints()
    {
        var note=Assert.Single(VideoClipTimeline.Project([Tracked()],Range(5,9)).Annotations);
        Assert.Equal(new[]{0d,4d},note.Keyframes!.Select(frame=>frame.Time));
    }

    [Fact]
    public void SharedStaticPathIsRetainedWhenKeyframesOnlyMoveTheBounds()
    {
        AiAnnotationPoint[] path=[new(.1,.2),new(.3,.4)];
        var source=new AiAnnotation(.1,.2,.3,.4,"path",0,1,9,
            [new(1,.1,.2,.3,.4),new(9,.2,.3,.4,.5)],Kind:AiAnnotationKind.Pen,Points:path);
        Assert.Same(path,Assert.Single(VideoClipTimeline.Project([source],Range(3,7)).Annotations).Points);
    }

    [Fact]
    public void ModelRelativeTimesReturnToSourceTimeWithoutMutatingTheResponse()
    {
        var relative=Assert.Single(VideoClipTimeline.Project([Tracked()],Range(3,7)).Annotations);
        var source=VideoClipTimeline.ToSourceTime(relative,Range(3,7));
        Assert.Equal(3,source.StartTime);Assert.Equal(7,source.EndTime);
        Assert.Equal(new[]{3d,5d,7d},source.Keyframes!.Select(frame=>frame.Time));
        Assert.Equal(0,relative.StartTime);Assert.Equal(new[]{0d,2d,4d},relative.Keyframes!.Select(frame=>frame.Time));
        Assert.Equal(relative.Text,source.Text);Assert.Equal(relative.ReferenceHandle,source.ReferenceHandle);
        Assert.Same(relative.Style,source.Style);
        var still=new AiAnnotation(.1,.2,.3,.4,"static");Assert.Same(still,VideoClipTimeline.ToSourceTime(still,Range(3,7)));
    }

    [Fact]
    public void InvalidRangesAndNonfiniteOrUnorderedTimelineTimesAreRejected()
    {
        Assert.Throws<ArgumentOutOfRangeException>(()=>VideoClipTimeline.Project([Tracked()],Range(3,3)));
        Assert.Throws<ArgumentOutOfRangeException>(()=>VideoClipTimeline.ToSourceTime(Tracked(),Range(-1,3)));
        Assert.Throws<ArgumentException>(()=>VideoClipTimeline.Project([Tracked() with{StartTime=double.NaN}],Range(3,7)));
        Assert.Throws<ArgumentException>(()=>VideoClipTimeline.ToSourceTime(Tracked() with{Keyframes=[new(5,.1,.2,.3,.4),new(1,.1,.2,.3,.4)]},Range(3,7)));
    }
}
