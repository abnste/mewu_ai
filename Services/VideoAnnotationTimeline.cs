using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class VideoAnnotationTimeline
{
    internal const int MaxAnswerActions=24;
    internal const double DurationOvershootToleranceSeconds=.35;

    internal static bool TryResolveTargetIndex(int regionIndex,bool isTimeline,IReadOnlyList<bool> videoTargets,out int targetIndex,out bool wasSingleVideoRemapped)
    {
        targetIndex=regionIndex;wasSingleVideoRemapped=false;
        if(videoTargets.Count==1&&videoTargets[0]&&isTimeline)
        {
            wasSingleVideoRemapped=regionIndex!=0;targetIndex=0;return true;
        }
        return regionIndex>=0&&regionIndex<videoTargets.Count&&videoTargets[regionIndex]==isTimeline;
    }

    internal static bool TryFitToDuration(AiAnnotation annotation,double durationSeconds,out AiAnnotation fitted,out bool wasClamped)
    {
        fitted=default!;wasClamped=false;
        if(!annotation.IsVideoTimeline||!double.IsFinite(durationSeconds)||durationSeconds<=0)return false;
        var start=annotation.StartTime!.Value;var end=annotation.EndTime!.Value;
        if(start>durationSeconds+DurationOvershootToleranceSeconds||end>durationSeconds+DurationOvershootToleranceSeconds)return false;
        var fittedStart=Math.Min(start,durationSeconds);var fittedEnd=Math.Min(end,durationSeconds);
        var frames=new List<VideoAnnotationKeyframe>();
        foreach(var frame in annotation.Keyframes!)
        {
            if(frame.Time>durationSeconds+DurationOvershootToleranceSeconds)return false;
            var time=Math.Min(frame.Time,durationSeconds);
            var next=frame with{Time=time};
            if(frames.Count>0&&Math.Abs(frames[^1].Time-time)<1e-9)frames[^1]=next;
            else frames.Add(next);
            wasClamped|=time!=frame.Time;
        }
        if(frames.Count==0||(fittedEnd>fittedStart&&frames.Count<2))return false;
        wasClamped|=fittedStart!=start||fittedEnd!=end;
        fitted=annotation with
        {
            X=frames[0].X,Y=frames[0].Y,Width=frames[0].Width,Height=frames[0].Height,
            StartTime=fittedStart,EndTime=fittedEnd,Keyframes=frames
        };
        return true;
    }

    internal static bool TryGetFirstMarker(IEnumerable<AiAnnotation> annotations,out AiAnnotation annotation,out VideoAnnotationKeyframe frame)
    {
        var candidate=annotations
            .Where(item=>item.IsVideoTimeline)
            .SelectMany(item=>item.Keyframes!.Select(keyframe=>(Annotation:item,Frame:keyframe)))
            .OrderBy(entry=>entry.Frame.Time)
            .FirstOrDefault();
        if(candidate.Annotation is null){annotation=null!;frame=null!;return false;}
        annotation=candidate.Annotation;frame=candidate.Frame;return true;
    }

    internal static IReadOnlyList<VideoAnnotationAnswerAction> CreateAnswerActions(IEnumerable<AiAnnotation> annotations)
    {
        var result=new List<VideoAnnotationAnswerAction>();
        foreach(var annotation in annotations.Where(item=>item.IsVideoTimeline).Take(6))
        {
            if(annotation.EndTime>annotation.StartTime)
                result.Add(new(VideoAnnotationAnswerActionKind.PlayRange,annotation,null));
            else result.Add(new(VideoAnnotationAnswerActionKind.JumpToFrame,annotation,annotation.Keyframes![0]));
            if(result.Count>=MaxAnswerActions)return result;
        }
        return result;
    }

    internal static bool TryInterpolate(AiAnnotation annotation,double time,out VideoAnnotationKeyframe frame)
    {
        frame=default!;
        if(!annotation.IsVideoTimeline||!double.IsFinite(time))return false;
        var frames=annotation.Keyframes!;
        if(frames.Count==1){frame=frames[0] with{Time=time};return true;}
        if(time<=frames[0].Time){frame=frames[0] with{Time=time};return true;}
        if(time>=frames[^1].Time){frame=frames[^1] with{Time=time};return true;}
        for(var index=1;index<frames.Count;index++)
        {
            var right=frames[index];
            if(time>right.Time)continue;
            var left=frames[index-1];
            var span=right.Time-left.Time;
            if(span<=0)return false;
            var amount=Math.Clamp((time-left.Time)/span,0,1);
            IReadOnlyList<AiAnnotationPoint>? points=null;
            if(left.Points is {Count:>0} leftPoints&&right.Points is {Count:>0} rightPoints&&leftPoints.Count==rightPoints.Count)
                points=leftPoints.Zip(rightPoints,(a,b)=>new AiAnnotationPoint(Lerp(a.X,b.X,amount),Lerp(a.Y,b.Y,amount))).ToArray();
            else points=amount<.5?left.Points:right.Points;
            frame=new VideoAnnotationKeyframe(
                time,
                Lerp(left.X,right.X,amount),
                Lerp(left.Y,right.Y,amount),
                Lerp(left.Width,right.Width,amount),
                Lerp(left.Height,right.Height,amount),points);
            return true;
        }
        return false;
    }

    private static double Lerp(double start,double end,double amount)=>start+(end-start)*amount;
}

internal enum VideoAnnotationAnswerActionKind{JumpToFrame,PlayRange}
internal sealed record VideoAnnotationAnswerAction(VideoAnnotationAnswerActionKind Kind,AiAnnotation Annotation,VideoAnnotationKeyframe? Frame);
