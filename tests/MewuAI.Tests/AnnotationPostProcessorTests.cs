using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class AnnotationPostProcessorTests
{
    [Fact]
    public void PrefersCalloutOverOverlappingPrimitiveTarget()
    {
        var rectangle=Image(.20,.30,.25,.10,"矩形",AiAnnotationKind.Rectangle);
        var callout=Image(.205,.302,.248,.098,"保存按钮：保存",AiAnnotationKind.Callout);

        var result=AnnotationPostProcessor.Process([rectangle,callout],false,out var stats);

        Assert.Same(callout,Assert.Single(result));Assert.Equal(1,stats.DuplicatesRemoved);
    }

    [Fact]
    public void RejectsHugeLocalCalloutButAllowsWholeRegionDescription()
    {
        var bad=Image(.1,.1,.72,.55,"保存按钮",AiAnnotationKind.Callout);
        var whole=Image(.1,.1,.72,.55,"整个设置区域",AiAnnotationKind.Callout);

        var result=AnnotationPostProcessor.Process([bad,whole],false,out var stats);

        Assert.Same(whole,Assert.Single(result));Assert.Equal(1,stats.QualityRejected);
    }

    [Fact]
    public void RemovesDuplicateVideoTrackAcrossOverlappingTimeRange()
    {
        var first=Timeline("手机",AiAnnotationKind.Callout,0);
        var duplicate=Timeline("黑色手机",AiAnnotationKind.Rectangle,.004);

        var result=AnnotationPostProcessor.Process([duplicate,first],true,out var stats);

        Assert.Equal(AiAnnotationKind.Callout,Assert.Single(result).Kind);Assert.Equal(1,stats.DuplicatesRemoved);
    }

    [Fact]
    public void RemovesDuplicatePointEventsWithinPlaybackTolerance()
    {
        var first=PointEvent("确认按钮",AiAnnotationKind.Callout,3,.2);
        var duplicate=PointEvent("确认",AiAnnotationKind.Rectangle,3.08,.204);

        var result=AnnotationPostProcessor.Process([duplicate,first],true,out var stats);

        Assert.Equal(AiAnnotationKind.Callout,Assert.Single(result).Kind);Assert.Equal(1,stats.DuplicatesRemoved);
    }

    [Fact]
    public void SimplifiesOnlyLinearlyRedundantTimelineKeyframes()
    {
        var linear=new AiAnnotation(.1,.1,.2,.2,"移动",0,0,2,[new(0,.1,.1,.2,.2),new(1,.2,.2,.2,.2),new(2,.3,.3,.2,.2)],"ref-video");
        var curved=new AiAnnotation(.1,.1,.2,.2,"转向",0,0,2,[new(0,.1,.1,.2,.2),new(1,.5,.2,.2,.2),new(2,.3,.3,.2,.2)],"ref-video");

        var result=AnnotationPostProcessor.Process([linear,curved],true,out var stats);

        Assert.Equal(2,result[0].Keyframes!.Count);Assert.Equal(3,result[1].Keyframes!.Count);Assert.Equal(1,stats.KeyframesRemoved);
    }

    [Fact]
    public void ComputesStableNormalizedIntersectionOverUnion()
    {
        var first=new System.Windows.Rect(.1,.1,.4,.4);var second=new System.Windows.Rect(.3,.1,.4,.4);
        Assert.Equal(1d/3,AnnotationGeometryService.IntersectionOverUnion(first,second),8);
    }

    [Fact]
    public void BoundsVisibleCalloutCountWithoutDroppingOtherDrawingTools()
    {
        var callouts=Enumerable.Range(0,VisualAnnotationProtocol.MaximumCallouts+3).Select(index=>Image(index*.02,0,.015,.02,$"目标{index}",AiAnnotationKind.Callout)).ToList();callouts.Add(Image(.1,.8,.2,.1,"高亮",AiAnnotationKind.Highlighter));
        var result=AnnotationPostProcessor.Process(callouts,false,out var stats);
        Assert.Equal(VisualAnnotationProtocol.MaximumCallouts,result.Count(note=>note.Kind==AiAnnotationKind.Callout));Assert.Contains(result,note=>note.Kind==AiAnnotationKind.Highlighter);Assert.Equal(3,stats.QualityRejected);
    }

    [Fact]
    public void PreservesBothIntersectingPenStrokesThatFormACross()
    {
        var down=Path("错误",[new(.2,.2),new(.6,.6)]);var up=Path("错误",[new(.2,.6),new(.6,.2)]);
        var result=AnnotationPostProcessor.Process([down,up],false,out var stats);
        Assert.Equal(2,result.Count);Assert.Equal(0,stats.DuplicatesRemoved);
    }

    [Fact]
    public void RemovesTheSamePenStrokeReturnedInReverseOrder()
    {
        var forward=Path("错误",[new(.2,.2),new(.6,.6)]);var reverse=Path("错误",[new(.6,.6),new(.2,.2)]);
        var result=AnnotationPostProcessor.Process([forward,reverse],false,out var stats);
        Assert.Single(result);Assert.Equal(1,stats.DuplicatesRemoved);
    }

    [Fact]
    public void PreservesOppositeArrowDirectionsOnTheSamePath()
    {
        var forward=Path("方向",[new(.2,.2),new(.6,.6)],AiAnnotationKind.Arrow);var reverse=Path("方向",[new(.6,.6),new(.2,.2)],AiAnnotationKind.Arrow);
        var result=AnnotationPostProcessor.Process([forward,reverse],false,out var stats);
        Assert.Equal(2,result.Count);Assert.Equal(0,stats.DuplicatesRemoved);
    }

    private static AiAnnotation Image(double x,double y,double width,double height,string text,AiAnnotationKind kind)=>new(x,y,width,height,text,0,ReferenceHandle:"ref-image",Kind:kind);
    private static AiAnnotation Timeline(string text,AiAnnotationKind kind,double offset)=>new(.1+offset,.1,.2,.2,text,0,1,2,[new(1,.1+offset,.1,.2,.2),new(2,.3+offset,.3,.2,.2)],"ref-video",kind);
    private static AiAnnotation PointEvent(string text,AiAnnotationKind kind,double time,double x)=>new(x,.1,.2,.2,text,0,time,time,[new(time,x,.1,.2,.2)],"ref-video",kind);
    private static AiAnnotation Path(string text,IReadOnlyList<AiAnnotationPoint> points,AiAnnotationKind kind=AiAnnotationKind.Pen)=>new(points.Min(point=>point.X),points.Min(point=>point.Y),points.Max(point=>point.X)-points.Min(point=>point.X),points.Max(point=>point.Y)-points.Min(point=>point.Y),text,0,ReferenceHandle:"ref-image",Kind:kind,Points:points);
}
