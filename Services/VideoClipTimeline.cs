// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal sealed record VideoClipTimelineProjection(
    IReadOnlyList<AiAnnotation> Annotations,
    IReadOnlyDictionary<AiAnnotation,Point> CardPositions);

internal static class VideoClipTimeline
{
    /// <summary>Keep a source-time playhead inside both inclusive retained boundaries.</summary>
    internal static TimeSpan ClampPosition(TimeSpan position,VideoClipRange range)
    {
        ValidateRange(range);
        return TimeSpan.FromTicks(Math.Clamp(position.Ticks,range.Start.Ticks,range.End.Ticks));
    }

    internal static VideoClipTimelineProjection Project(
        IReadOnlyList<AiAnnotation> annotations,VideoClipRange range,
        IReadOnlyDictionary<AiAnnotation,Point>? cardPositions=null)
    {
        ArgumentNullException.ThrowIfNull(annotations);
        ValidateRange(range);
        var result=new List<AiAnnotation>(annotations.Count);
        var cards=new Dictionary<AiAnnotation,Point>(ReferenceEqualityComparer.Instance);
        var offset=range.Start.TotalSeconds;
        var limit=range.End.TotalSeconds;
        foreach(var original in annotations)
        {
            AiAnnotation projected;
            if(!original.IsVideoTimeline)projected=original;
            else
            {
                ValidateTimeline(original);
                var start=Math.Max(offset,original.StartTime!.Value);
                var end=Math.Min(limit,original.EndTime!.Value);
                if(end<start)continue;
                // A nonzero interval merely touching a cut boundary has no
                // visible duration. An explicit point marker is retained.
                if(end==start&&original.EndTime>original.StartTime)continue;
                if(!VideoAnnotationTimeline.TryInterpolate(original,start,out var first)||
                   !VideoAnnotationTimeline.TryInterpolate(original,end,out var last))continue;
                var frames=new List<VideoAnnotationKeyframe>{first with{Time=start-offset}};
                foreach(var frame in original.Keyframes!)
                    if(frame.Time>start&&frame.Time<end)
                        frames.Add(frame with{Time=frame.Time-offset});
                if(end>start)frames.Add(last with{Time=end-offset});
                projected=original with
                {
                    X=first.X,Y=first.Y,Width=first.Width,Height=first.Height,Points=first.Points??original.Points,
                    StartTime=start-offset,EndTime=end-offset,Keyframes=frames.ToArray()
                };
            }
            result.Add(projected);
            if(cardPositions is not null&&cardPositions.TryGetValue(original,out var position))cards[projected]=position;
        }
        return new(result.ToArray(),cards);
    }

    internal static AiAnnotation ToSourceTime(AiAnnotation relative,VideoClipRange range)
    {
        ArgumentNullException.ThrowIfNull(relative);
        ValidateRange(range);
        if(!relative.IsVideoTimeline)return relative;
        ValidateTimeline(relative);
        var offset=range.Start.TotalSeconds;
        return relative with
        {
            StartTime=relative.StartTime!.Value+offset,
            EndTime=relative.EndTime!.Value+offset,
            Keyframes=relative.Keyframes!.Select(frame=>frame with{Time=frame.Time+offset}).ToArray()
        };
    }

    private static void ValidateRange(VideoClipRange range)
    {
        if(range.Start<TimeSpan.Zero||range.End<=range.Start)throw new ArgumentOutOfRangeException(nameof(range));
    }

    private static void ValidateTimeline(AiAnnotation annotation)
    {
        if(!double.IsFinite(annotation.StartTime!.Value)||!double.IsFinite(annotation.EndTime!.Value)||
           annotation.StartTime<0||annotation.EndTime<annotation.StartTime)
            throw new ArgumentException("Invalid video annotation interval.",nameof(annotation));
        var previous=double.NegativeInfinity;
        foreach(var frame in annotation.Keyframes!)
        {
            if(!double.IsFinite(frame.Time)||frame.Time<0||frame.Time<previous)
                throw new ArgumentException("Invalid video annotation keyframe time.",nameof(annotation));
            previous=frame.Time;
        }
    }
}
