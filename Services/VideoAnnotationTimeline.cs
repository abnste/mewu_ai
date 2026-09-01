using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class VideoAnnotationTimeline
{
    internal const int MaxAnswerActions=24;

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
            frame=new VideoAnnotationKeyframe(
                time,
                Lerp(left.X,right.X,amount),
                Lerp(left.Y,right.Y,amount),
                Lerp(left.Width,right.Width,amount),
                Lerp(left.Height,right.Height,amount));
            return true;
        }
        return false;
    }

    private static double Lerp(double start,double end,double amount)=>start+(end-start)*amount;
}

internal enum VideoAnnotationAnswerActionKind{JumpToFrame,PlayRange}
internal sealed record VideoAnnotationAnswerAction(VideoAnnotationAnswerActionKind Kind,AiAnnotation Annotation,VideoAnnotationKeyframe? Frame);
