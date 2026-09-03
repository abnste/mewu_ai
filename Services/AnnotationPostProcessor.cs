using System.Globalization;
using System.Text;
using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal readonly record struct AnnotationPostProcessStats(int QualityRejected,int DuplicatesRemoved,int KeyframesRemoved);

/// <summary>
/// Deterministic, provider-independent quality gate for visual annotations.
/// It applies conservative NMS-style deduplication to image boxes and to
/// spatio-temporally overlapping video tracks, and removes only timeline
/// keyframes that are already represented by linear interpolation.
/// </summary>
internal static class AnnotationPostProcessor
{
    private const double DuplicateIou=.72;
    private const double TimelineSimplificationTolerance=.0025;

    internal static IReadOnlyList<AiAnnotation> Process(IReadOnlyList<AiAnnotation> annotations,bool isVideo,out AnnotationPostProcessStats stats)
    {
        ArgumentNullException.ThrowIfNull(annotations);
        var qualityRejected=0;var duplicates=0;var keyframesRemoved=0;
        var candidates=new List<(int Index,AiAnnotation Annotation)>();
        foreach(var (annotation,index) in annotations.Take(VisualAnnotationProtocol.MaximumAnnotations).Select((value,index)=>(value,index)))
        {
            if(annotation.IsVideoTimeline!=isVideo)continue;
            var removed=0;var normalized=isVideo?SimplifyTimeline(annotation,out removed):annotation;keyframesRemoved+=removed;
            if(!isVideo&&IsImplausiblyBroadLocalCallout(normalized)){qualityRejected++;continue;}
            candidates.Add((index,normalized));
        }

        // Prefer semantically richer callouts over duplicate primitive boxes,
        // while restoring source order after suppression for stable rendering.
        var kept=new List<(int Index,AiAnnotation Annotation)>();
        foreach(var candidate in candidates.OrderByDescending(item=>Priority(item.Annotation)).ThenBy(item=>item.Index))
        {
            if(kept.Any(existing=>AreDuplicates(candidate.Annotation,existing.Annotation,isVideo))){duplicates++;continue;}
            kept.Add(candidate);
        }
        var ordered=kept.OrderBy(item=>item.Index).Select(item=>item.Annotation).ToArray();var calloutCount=0;var bounded=new List<AiAnnotation>(ordered.Length);
        foreach(var annotation in ordered){if(annotation.Kind==AiAnnotationKind.Callout&&calloutCount++>=VisualAnnotationProtocol.MaximumCallouts){qualityRejected++;continue;}bounded.Add(annotation);}
        stats=new(qualityRejected,duplicates,keyframesRemoved);return bounded;
    }

    private static int Priority(AiAnnotation annotation)=>annotation.Kind switch
    {
        AiAnnotationKind.Callout=>100,
        AiAnnotationKind.Mosaic=>90,
        AiAnnotationKind.Text=>80,
        AiAnnotationKind.Number=>75,
        AiAnnotationKind.Arrow=>70,
        AiAnnotationKind.Highlighter=>65,
        AiAnnotationKind.Pen=>60,
        AiAnnotationKind.Rectangle=>55,
        AiAnnotationKind.Ellipse=>50,
        _=>0
    };

    private static bool AreDuplicates(AiAnnotation first,AiAnnotation second,bool isVideo)
    {
        if(first.RegionIndex!=second.RegionIndex||!string.Equals(first.ReferenceHandle,second.ReferenceHandle,StringComparison.Ordinal))return false;
        var targetKinds=IsTargetMarker(first.Kind)&&IsTargetMarker(second.Kind);
        if(!targetKinds&&first.Kind!=second.Kind)return false;
        var textSimilarity=TextSimilarity(first.Text,second.Text);
        if(!targetKinds&&textSimilarity<.82)return false;
        if(isVideo)
        {
            if(IsClosePointEvent(first,second))
            {
                var firstFrame=first.Keyframes![0];var secondFrame=second.Keyframes![0];
                var pointIou=AnnotationGeometryService.IntersectionOverUnion(AnnotationGeometryService.ToNormalizedRect(firstFrame),AnnotationGeometryService.ToNormalizedRect(secondFrame));
                return pointIou>=DuplicateIou&&(first.Kind!=AiAnnotationKind.Callout||second.Kind!=AiAnnotationKind.Callout||textSimilarity>=.48||pointIou>=.9);
            }
            if(!TimelineOverlap(first,second,out var from,out var to))return false;
            var samples=from==to?[from]:new[]{from,(from+to)/2,to};var iou=0d;
            foreach(var sample in samples)
            {
                if(!VideoAnnotationTimeline.TryInterpolate(first,sample,out var a)||!VideoAnnotationTimeline.TryInterpolate(second,sample,out var b))return false;
                iou+=AnnotationGeometryService.IntersectionOverUnion(AnnotationGeometryService.ToNormalizedRect(a),AnnotationGeometryService.ToNormalizedRect(b));
            }
            iou/=samples.Length;
            return iou>=DuplicateIou&&(first.Kind!=AiAnnotationKind.Callout||second.Kind!=AiAnnotationKind.Callout||textSimilarity>=.48||iou>=.9);
        }
        var imageIou=AnnotationGeometryService.IntersectionOverUnion(AnnotationGeometryService.ToNormalizedRect(first),AnnotationGeometryService.ToNormalizedRect(second));
        return imageIou>=DuplicateIou&&(first.Kind!=AiAnnotationKind.Callout||second.Kind!=AiAnnotationKind.Callout||textSimilarity>=.48||imageIou>=.9);
    }

    private static bool TimelineOverlap(AiAnnotation first,AiAnnotation second,out double from,out double to)
    {
        var firstStart=first.StartTime!.Value;var firstEnd=first.EndTime!.Value;var secondStart=second.StartTime!.Value;var secondEnd=second.EndTime!.Value;var firstDuration=firstEnd-firstStart;var secondDuration=secondEnd-secondStart;
        from=Math.Max(firstStart,secondStart);to=Math.Min(firstEnd,secondEnd);if(to<from)return false;
        var overlap=Math.Max(0,to-from);var shorter=Math.Max(.001,Math.Min(firstDuration,secondDuration));return overlap/shorter>=.7;
    }

    private static bool IsClosePointEvent(AiAnnotation first,AiAnnotation second)=>first.EndTime!.Value-first.StartTime!.Value<=.05&&second.EndTime!.Value-second.StartTime!.Value<=.05&&Math.Abs(first.StartTime.Value-second.StartTime.Value)<=.12;

    private static bool IsTargetMarker(AiAnnotationKind kind)=>kind is AiAnnotationKind.Callout or AiAnnotationKind.Rectangle or AiAnnotationKind.Ellipse;

    private static bool IsImplausiblyBroadLocalCallout(AiAnnotation annotation)
    {
        if(annotation.Kind!=AiAnnotationKind.Callout)return false;if(annotation.Width<.012||annotation.Height<.012||annotation.Width>.92||annotation.Height>.92)return true;if(IsCompositeTargetLabel(annotation.Text))return false;
        var area=annotation.Width*annotation.Height;
        return area>.34||annotation.Width>.58&&annotation.Height>.16||annotation.Height>.48&&annotation.Width>.2;
    }

    private static bool IsCompositeTargetLabel(string text)
    {
        var value=text.ToLowerInvariant();
        string[] hints=["整个","整体","整张","全屏","全部","区域","模块","段落","表格","代码块","流程","whole","entire","full screen","region","section","panel","paragraph","table","block","flow"];
        return hints.Any(value.Contains);
    }

    private static AiAnnotation SimplifyTimeline(AiAnnotation annotation,out int removed)
    {
        removed=0;var frames=annotation.Keyframes!;
        if(frames.Count<=2||frames.Any(frame=>frame.Points is {Count:>0}))return annotation;
        var keep=new SortedSet<int>{0,frames.Count-1};
        SimplifyRange(frames,0,frames.Count-1,keep);
        if(annotation.StartTime==annotation.EndTime)keep.Add(0);
        var simplified=keep.Select(index=>frames[index]).ToArray();removed=frames.Count-simplified.Length;
        return removed==0?annotation:annotation with{Keyframes=simplified,X=simplified[0].X,Y=simplified[0].Y,Width=simplified[0].Width,Height=simplified[0].Height};
    }

    private static void SimplifyRange(IReadOnlyList<VideoAnnotationKeyframe> frames,int start,int end,ISet<int> keep)
    {
        if(end-start<=1)return;
        var left=frames[start];var right=frames[end];var span=right.Time-left.Time;if(span<=0)return;
        var maximum=0d;var selected=-1;
        for(var index=start+1;index<end;index++)
        {
            var amount=(frames[index].Time-left.Time)/span;var expected=new VideoAnnotationKeyframe(frames[index].Time,Lerp(left.X,right.X,amount),Lerp(left.Y,right.Y,amount),Lerp(left.Width,right.Width,amount),Lerp(left.Height,right.Height,amount));
            var error=GeometryError(frames[index],expected);if(error<=maximum)continue;maximum=error;selected=index;
        }
        if(selected<0||maximum<=TimelineSimplificationTolerance)return;
        keep.Add(selected);SimplifyRange(frames,start,selected,keep);SimplifyRange(frames,selected,end,keep);
    }

    private static double GeometryError(VideoAnnotationKeyframe actual,VideoAnnotationKeyframe expected)=>Math.Max(Math.Max(Math.Abs(actual.X-expected.X),Math.Abs(actual.Y-expected.Y)),Math.Max(Math.Abs(actual.Width-expected.Width),Math.Abs(actual.Height-expected.Height)));
    private static double Lerp(double start,double end,double amount)=>start+(end-start)*amount;

    private static double TextSimilarity(string first,string second)
    {
        var a=Normalize(first);var b=Normalize(second);if(a.Length==0||b.Length==0)return 0;if(string.Equals(a,b,StringComparison.Ordinal))return 1;
        var left=Bigrams(a);var right=Bigrams(b);if(left.Count==0||right.Count==0)return (a.Contains(b,StringComparison.Ordinal)||b.Contains(a,StringComparison.Ordinal)) ? .8 : 0;
        var common=left.Count(right.Contains);return 2d*common/(left.Count+right.Count);
    }

    private static HashSet<string> Bigrams(string value){var result=new HashSet<string>(StringComparer.Ordinal);for(var index=0;index<value.Length-1;index++)result.Add(value.Substring(index,2));return result;}
    private static string Normalize(string value){var builder=new StringBuilder(Math.Min(value.Length,160));foreach(var rune in value.EnumerateRunes()){var category=Rune.GetUnicodeCategory(rune);if(category is UnicodeCategory.UppercaseLetter or UnicodeCategory.LowercaseLetter or UnicodeCategory.TitlecaseLetter or UnicodeCategory.ModifierLetter or UnicodeCategory.OtherLetter or UnicodeCategory.DecimalDigitNumber)builder.Append(rune.ToString().ToUpperInvariant());if(builder.Length>=160)break;}return builder.ToString();}
}
