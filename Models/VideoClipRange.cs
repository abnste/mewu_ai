// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Models;

/// <summary>A non-destructive range in the original video's time coordinates.</summary>
internal readonly record struct VideoClipRange(TimeSpan Start,TimeSpan End)
{
    internal TimeSpan Duration=>End-Start;

    internal static VideoClipRange Full(TimeSpan duration)
    {
        if(duration<=TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(duration));
        return new(TimeSpan.Zero,duration);
    }

    internal static VideoClipRange Normalize(TimeSpan duration,TimeSpan start,TimeSpan end,TimeSpan? minDuration=null)
    {
        if(duration<=TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(duration));
        var minimum=minDuration??TimeSpan.FromMilliseconds(1);
        if(minimum<=TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(minDuration));
        var minimumTicks=Math.Min(minimum.Ticks,duration.Ticks);
        var startTicks=Math.Clamp(start.Ticks,0,duration.Ticks);
        var endTicks=Math.Clamp(end.Ticks,startTicks,duration.Ticks);
        if(endTicks-startTicks<minimumTicks)
        {
            // Preserve the requested start where possible, then move it back
            // only when the source's end leaves too little room.
            startTicks=Math.Min(startTicks,duration.Ticks-minimumTicks);
            endTicks=startTicks+minimumTicks;
        }
        return new(TimeSpan.FromTicks(startTicks),TimeSpan.FromTicks(endTicks));
    }

    internal bool IsFull(TimeSpan duration)
        =>duration>TimeSpan.Zero&&Start>=TimeSpan.Zero&&End>Start&&
          Start.Ticks<=TimeSpan.TicksPerMillisecond&&
          Math.Abs(End.Ticks-duration.Ticks)<=TimeSpan.TicksPerMillisecond;
}
