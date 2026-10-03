// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;
namespace MewuAI.Core.Tests;
public sealed class TemporalAnnotationOverlapTests
{
    private static VideoAnnotationKeyframe F(double t,double x=.125,double y=.125,double w=.125,double h=.125)=>new(t,x,y,w,h);
    private static AiAnnotation Track(params VideoAnnotationKeyframe[] f)=>new(f[0].X,f[0].Y,f[0].Width,f[0].Height,"target",0,f[0].Time,f[^1].Time,f,"ref-video",AiAnnotationKind.Rectangle);
    private static bool Compare(AiAnnotation a,AiAnnotation b,double threshold=.72)=>TemporalAnnotationOverlap.BoxesStayOverlapping(a,b,Math.Max(a.StartTime!.Value,b.StartTime!.Value),Math.Min(a.EndTime!.Value,b.EndTime!.Value),threshold);
    [Fact]public void KeepsMissedExcursionInEitherOrder()
    {var a=Track(F(0),F(4));var b=Track(F(0),F(1,.625),F(2),F(4));Assert.False(Compare(a,b));Assert.False(Compare(b,a));}
    [Fact]public void PreservesTinyExcursionThatSimplificationWouldErase()
    {var a=Track(F(0,w:.0001,h:.0001),F(4,w:.0001,h:.0001));var b=Track(F(0,w:.0001,h:.0001),F(1,.126,w:.0001,h:.0001),F(2,w:.0001,h:.0001),F(4,w:.0001,h:.0001));Assert.False(Compare(a,b));}
    [Fact]public void KeepsNearCoincidentUnionTimesDistinct()
    {var a=Track(F(0),F(4));var b=Track(F(0),F(1),F(1+1e-10,.625),F(1+2e-10),F(4));Assert.False(Compare(a,b));}
    [Fact]public void RetainsExistingOffsetMovingDuplicates()
    {var a=Track(F(1,.1,.1,.2,.2),F(2,.3,.3,.2,.2));var b=Track(F(1,.104,.1,.2,.2),F(2,.304,.3,.2,.2));Assert.True(Compare(a,b));}
    [Fact]public void IdenticalPathsAllowDifferentKeyframePartitions()
    {var a=Track(F(0,.125),F(4,.625));var b=Track(F(0,.125),F(2,.375),F(4,.625));Assert.True(Compare(a,b));}
    [Fact]public void DeformingDuplicateBoxesStillCertify()
    {var a=Track(F(0,.1,.1,.1,.6),F(4,.1,.1,.6,.1));var b=Track(F(0,.101,.1,.1,.6),F(4,.101,.1,.6,.1));Assert.True(Compare(a,b));}
    [Fact]public void StricterCalloutThresholdKeepsLooseOverlap()
    {var a=Track(F(0),F(4));var b=Track(F(0,.137),F(4,.137));Assert.True(Compare(a,b,.72));Assert.False(Compare(a,b,.9));}
    [Fact]public void InvalidGeometryTimesAndBudgetRejectConservatively()
    {
        var a=Track(F(0),F(4));
        foreach(var b in new[]{Track(F(0,w:0),F(4)),Track(F(0,x:double.NaN),F(4)),Track(F(0),F(0),F(4)),Track(F(0),F(3),F(2),F(4)),Track(F(0,h:1e-300),F(4))})Assert.False(Compare(a,b));
        Assert.False(Compare(a,a,0));Assert.False(Compare(a,a,double.NaN));
        var many=Track(Enumerable.Range(0,300).Select(i=>F(i)).ToArray());Assert.False(Compare(many,many));
    }
    [Fact]public void AdjacentRepresentableTimesDoNotLoop()
    {var t=1d;var end=Math.BitIncrement(t);var a=Track(F(t,.1,.1,.1,.6),F(end,.1,.1,.6,.1));var b=Track(F(t,.101,.1,.1,.6),F(end,.101,.1,.6,.1));Assert.False(Compare(a,b));}
    [Fact]public void CertifiedRandomPairsHaveNoDenseSampleViolations()
    {
        var random=new Random(1739);var accepted=0;
        for(var test=0;test<500;test++)
        {
            VideoAnnotationKeyframe Next(double t)=>F(t,random.NextDouble()*.4,random.NextDouble()*.4,.02+random.NextDouble()*.5,.02+random.NextDouble()*.5);
            var a=Track(Next(0),Next(4));var b=Track(a.Keyframes!.Select(f=>f with{X=f.X+(random.NextDouble()-.5)*.03,Y=f.Y+(random.NextDouble()-.5)*.03,Width=f.Width*(.9+random.NextDouble()*.2),Height=f.Height*(.9+random.NextDouble()*.2)}).ToArray());
            if(!Compare(a,b))continue;accepted++;
            for(var i=0;i<=1000;i++){VideoAnnotationTimeline.TryInterpolate(a,4d*i/1000,out var af);VideoAnnotationTimeline.TryInterpolate(b,4d*i/1000,out var bf);Assert.True(Iou(af,bf)>=.72-1e-10);}
        }
        Assert.True(accepted>100);
    }
    [Fact]public void PathsRequireFixedCorrespondenceAndDirection()
    {
        var p=new[]{new AiAnnotationPoint(.1,.1),new AiAnnotationPoint(.3,.3)};
        var a=Track(F(0) with{Points=p},F(4) with{Points=p});
        var reverse=Track(F(0) with{Points=p.Reverse().ToArray()},F(4) with{Points=p.Reverse().ToArray()});
        Assert.True(TemporalAnnotationOverlap.PathsStayClose(a,reverse,0,4,true));Assert.False(TemporalAnnotationOverlap.PathsStayClose(a,reverse,0,4,false));
        var swapping=Track(F(0) with{Points=p},F(4) with{Points=p.Reverse().ToArray()});Assert.False(TemporalAnnotationOverlap.PathsStayClose(a,swapping,0,4,true));
        var changedCount=Track(F(0) with{Points=p},F(2) with{Points=[..p,new(.7,.7)]},F(4) with{Points=p});Assert.False(TemporalAnnotationOverlap.PathsStayClose(a,changedCount,0,4,true));
        var excursion=Track(F(0) with{Points=p},F(1) with{Points=p.Select(q=>q with{X=q.X+.5}).ToArray()},F(2) with{Points=p},F(4) with{Points=p});Assert.False(TemporalAnnotationOverlap.PathsStayClose(a,excursion,0,4,true));
    }
    [Fact]public void PathologicalCoordinatePrecisionCannotCertify()
    {
        var x=999999d;var ulp=Math.BitIncrement(x)-x;
        var a=Track(F(0,x,w:1e-12,h:1e-12),F(1,x+10*ulp,w:1e-12,h:1e-12));
        var b=Track(F(.01,x,w:1e-12,h:1e-12),F(.11,x+ulp,w:1e-12,h:1e-12));
        Assert.False(Compare(a,b));
    }
    [Fact]public void ExhaustedSharedSubdivisionBudgetKeepsCandidate()
    {
        var a=Track(F(0,.1,.1,.1,.6),F(4,.1,.1,.6,.1));var b=Track(F(0,.101,.1,.1,.6),F(4,.101,.1,.6,.1));
        Assert.True(Compare(a,b));
        Assert.False(TemporalAnnotationOverlap.BoxesStayOverlapping(a,b,0,4,.72,1));
        Assert.False(TemporalAnnotationOverlap.BoxesStayOverlapping(a,b,0,4,.72,0));
    }
    private static double Iou(VideoAnnotationKeyframe a,VideoAnnotationKeyframe b)
    {var w=Math.Max(0,Math.Min(a.X+a.Width,b.X+b.Width)-Math.Max(a.X,b.X));var h=Math.Max(0,Math.Min(a.Y+a.Height,b.Y+b.Height)-Math.Max(a.Y,b.Y));var intersection=w*h;return intersection/(a.Width*a.Height+b.Width*b.Height-intersection);}
}
