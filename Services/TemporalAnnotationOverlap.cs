// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Conservative comparison of original piecewise-linear annotation tracks.
/// A failed bound, invalid geometry or exhausted budget keeps both tracks.
/// This is a geometric test, not a claim that matching boxes identify the same object.
/// </summary>
internal static class TemporalAnnotationOverlap
{
    private const int MaximumSegments=512;
    private const int MaximumChecks=2048;
    private const int MaximumDepth=14;
    private const double RoundoffGuard=1e-7;

    internal static bool BoxesStayOverlapping(AiAnnotation first,AiAnnotation second,double from,double to,double threshold,int workBudget=MaximumChecks)
    {
        if(!double.IsFinite(threshold)||threshold<=0||threshold>1||!TryTimes(first,second,from,to,out var times))return false;
        var budget=Math.Clamp(workBudget,0,MaximumChecks);
        if(times.Length==1)return Frames(first,second,from,out var a,out var b)&&Valid(a)&&Valid(b)&&((Same(a,b)&&threshold+RoundoffGuard<1)||Iou(a,b)>=threshold+RoundoffGuard);
        for(var index=1;index<times.Length;index++)
        {
            if(!Frames(first,second,times[index-1],out var a0,out var b0)||!Frames(first,second,times[index],out var a1,out var b1)||
                !Certify(a0,a1,b0,b1,threshold,0,ref budget))return false;
        }
        return true;
    }

    internal static bool PathsStayClose(AiAnnotation first,AiAnnotation second,double from,double to,bool allowReverse)
    {
        if(!TryTimes(first,second,from,to,out var times))return false;
        var count=first.Keyframes![0].Points?.Count??0;
        if(count<2||count>4096||!FixedPoints(first,count)||!FixedPoints(second,count))return false;
        var forward=true;var reverse=allowReverse;
        foreach(var time in times)
        {
            if(!Frames(first,second,time,out var a,out var b)||a.Points?.Count!=count||b.Points?.Count!=count)return false;
            for(var index=0;index<count;index++)
            {
                forward&=Close(a.Points[index],b.Points[index]);
                if(reverse)reverse&=Close(a.Points[index],b.Points[count-index-1]);
            }
            if(!forward&&!reverse)return false;
        }
        // A single orientation holds on all endpoints. Corresponding vertices
        // interpolate linearly, and the Euclidean distance norm is convex.
        return forward||reverse;
    }

    private static bool FixedPoints(AiAnnotation annotation,int count)=>annotation.Keyframes!.All(frame=>frame.Points?.Count==count&&frame.Points.All(point=>double.IsFinite(point.X)&&double.IsFinite(point.Y)&&point.X>=0&&point.Y>=0&&point.X<=1.001&&point.Y<=1.001));
    private static bool Close(AiAnnotationPoint a,AiAnnotationPoint b)
    {var x=a.X-b.X;var y=a.Y-b.Y;return x*x+y*y<=.0125*.0125-RoundoffGuard;}

    private static bool TryTimes(AiAnnotation first,AiAnnotation second,double from,double to,out double[] times)
    {
        times=[];
        if(!double.IsFinite(from)||!double.IsFinite(to)||to<from||!TimelineValid(first)||!TimelineValid(second)||from<Math.Max(first.StartTime!.Value,second.StartTime!.Value)||to>Math.Min(first.EndTime!.Value,second.EndTime!.Value))return false;
        times=first.Keyframes!.Concat(second.Keyframes!).Select(frame=>frame.Time).Where(time=>time>from&&time<to)
            .Append(from).Append(to).Distinct().Order().ToArray();
        return times.Length<=MaximumSegments+1;
    }
    private static bool TimelineValid(AiAnnotation annotation)
    {
        if(!annotation.IsVideoTimeline||!double.IsFinite(annotation.StartTime!.Value)||!double.IsFinite(annotation.EndTime!.Value)||annotation.EndTime<annotation.StartTime||annotation.Keyframes!.Count>256)return false;
        var previous=double.NegativeInfinity;
        foreach(var frame in annotation.Keyframes)
        {if(!double.IsFinite(frame.Time)||frame.Time<=previous)return false;previous=frame.Time;}
        return true;
    }
    private static bool Frames(AiAnnotation a,AiAnnotation b,double time,out VideoAnnotationKeyframe left,out VideoAnnotationKeyframe right)
    {var one=VideoAnnotationTimeline.TryInterpolate(a,time,out left);var two=VideoAnnotationTimeline.TryInterpolate(b,time,out right);return one&&two;}
    private static bool Valid(VideoAnnotationKeyframe f)=>double.IsFinite(f.X)&&double.IsFinite(f.Y)&&double.IsFinite(f.Width)&&double.IsFinite(f.Height)&&f.X>=0&&f.Y>=0&&f.Width>=1e-6&&f.Height>=1e-6&&f.X+f.Width<=1.001&&f.Y+f.Height<=1.001;
    private static bool Same(VideoAnnotationKeyframe a,VideoAnnotationKeyframe b)=>a.X==b.X&&a.Y==b.Y&&a.Width==b.Width&&a.Height==b.Height;
    private static double OverlapWidth(VideoAnnotationKeyframe a,VideoAnnotationKeyframe b)
    {var dx=b.X-a.X;return Math.Max(0,Math.Min(a.Width,dx+b.Width)-Math.Max(0,dx));}
    private static double OverlapHeight(VideoAnnotationKeyframe a,VideoAnnotationKeyframe b)
    {var dy=b.Y-a.Y;return Math.Max(0,Math.Min(a.Height,dy+b.Height)-Math.Max(0,dy));}
    private static double Iou(VideoAnnotationKeyframe a,VideoAnnotationKeyframe b)
    {var intersection=OverlapWidth(a,b)*OverlapHeight(a,b);var union=a.Width*a.Height+b.Width*b.Height-intersection;return union>0?intersection/union:0;}
    private static VideoAnnotationKeyframe Mid(VideoAnnotationKeyframe a,VideoAnnotationKeyframe b)=>new(a.Time+(b.Time-a.Time)/2,a.X+(b.X-a.X)/2,a.Y+(b.Y-a.Y)/2,a.Width+(b.Width-a.Width)/2,a.Height+(b.Height-a.Height)/2);
    private static double MaxAreaSum(VideoAnnotationKeyframe a0,VideoAnnotationKeyframe a1,VideoAnnotationKeyframe b0,VideoAnnotationKeyframe b1)
    {
        var aw=a1.Width-a0.Width;var ah=a1.Height-a0.Height;var bw=b1.Width-b0.Width;var bh=b1.Height-b0.Height;
        var constant=a0.Width*a0.Height+b0.Width*b0.Height;
        var linear=a0.Width*ah+a0.Height*aw+b0.Width*bh+b0.Height*bw;
        var quadratic=aw*ah+bw*bh;
        var maximum=Math.Max(constant,a1.Width*a1.Height+b1.Width*b1.Height);
        if(quadratic<0)
        {var t=-linear/(2*quadratic);if(t>0&&t<1)maximum=Math.Max(maximum,constant+linear*t+quadratic*t*t);}
        return maximum;
    }
    private static bool Certify(VideoAnnotationKeyframe a0,VideoAnnotationKeyframe a1,VideoAnnotationKeyframe b0,VideoAnnotationKeyframe b1,double threshold,int depth,ref int budget)
    {
        if(--budget<0||!Valid(a0)||!Valid(a1)||!Valid(b0)||!Valid(b1))return false;
        if(Same(a0,b0)&&Same(a1,b1)&&threshold+RoundoffGuard<1)return true;
        if(Iou(a0,b0)<threshold+RoundoffGuard||Iou(a1,b1)<threshold+RoundoffGuard)return false;
        // In A-relative coordinates, raw overlap width and height are concave
        // piecewise-linear functions. Each endpoint minimum bounds its full
        // segment below, regardless of common global translation.
        var areaFloor=Math.Min(OverlapWidth(a0,b0),OverlapWidth(a1,b1))*Math.Min(OverlapHeight(a0,b0),OverlapHeight(a1,b1));
        var areaCeiling=MaxAreaSum(a0,a1,b0,b1);
        var denominator=areaCeiling-areaFloor;
        if(double.IsFinite(denominator)&&denominator>0&&areaFloor/denominator>=threshold+RoundoffGuard)return true;
        if(depth>=MaximumDepth)return false;
        var am=Mid(a0,a1);var bm=Mid(b0,b1);
        if(am.Time==a0.Time||am.Time==a1.Time||!Valid(am)||!Valid(bm)||Iou(am,bm)<threshold+RoundoffGuard)return false;
        return Certify(a0,am,b0,bm,threshold,depth+1,ref budget)&&Certify(am,a1,bm,b1,threshold,depth+1,ref budget);
    }
}
