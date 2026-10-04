// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

public sealed class VideoClipRangeTests
{
    [Theory]
    [InlineData(-2,12,0,10)]
    [InlineData(2,8,2,8)]
    [InlineData(11,15,9.999,10)]
    [InlineData(8,2,8,8.001)]
    [InlineData(-2,-1,0,.001)]
    public void NormalizesWithoutEmptyOrOutOfSourceRanges(double start,double end,double expectedStart,double expectedEnd)
    {
        var range=VideoClipRange.Normalize(TimeSpan.FromSeconds(10),TimeSpan.FromSeconds(start),TimeSpan.FromSeconds(end));
        Assert.Equal(expectedStart,range.Start.TotalSeconds,6);
        Assert.Equal(expectedEnd,range.End.TotalSeconds,6);
        Assert.True(range.Duration>TimeSpan.Zero);
    }

    [Fact]
    public void ExplicitMinimumExtendsEndThenMovesStartOnlyAtSourceEnd()
    {
        var duration=TimeSpan.FromSeconds(10);var minimum=TimeSpan.FromSeconds(2);
        Assert.Equal(new VideoClipRange(TimeSpan.FromSeconds(3),TimeSpan.FromSeconds(5)),
            VideoClipRange.Normalize(duration,TimeSpan.FromSeconds(3),TimeSpan.FromSeconds(3.5),minimum));
        Assert.Equal(new VideoClipRange(TimeSpan.FromSeconds(8),duration),
            VideoClipRange.Normalize(duration,TimeSpan.FromSeconds(9),duration,minimum));
    }

    [Fact]
    public void SourceShorterThanMinimumKeepsAllAvailableTicks()
    {
        var duration=TimeSpan.FromTicks(4);
        Assert.Equal(VideoClipRange.Full(duration),VideoClipRange.Normalize(duration,duration,duration,TimeSpan.FromSeconds(1)));
    }

    [Fact]
    public void ExtremeTimeSpansNormalizeWithoutTickOverflow()
    {
        var range=VideoClipRange.Normalize(TimeSpan.MaxValue,TimeSpan.MaxValue,TimeSpan.MinValue);
        Assert.Equal(TimeSpan.MaxValue,range.End);
        Assert.Equal(TimeSpan.FromMilliseconds(1),range.Duration);
        Assert.Equal(VideoClipRange.Full(TimeSpan.MaxValue),
            VideoClipRange.Normalize(TimeSpan.MaxValue,TimeSpan.MinValue,TimeSpan.MaxValue));
    }

    [Theory]
    [InlineData(0)]
    [InlineData(-1)]
    public void NonPositiveSourceDurationIsRejected(int ticks)
    {
        Assert.Throws<ArgumentOutOfRangeException>(()=>VideoClipRange.Full(TimeSpan.FromTicks(ticks)));
        Assert.Throws<ArgumentOutOfRangeException>(()=>VideoClipRange.Normalize(TimeSpan.FromTicks(ticks),TimeSpan.Zero,TimeSpan.FromSeconds(1)));
    }

    [Theory]
    [InlineData(0)]
    [InlineData(-1)]
    public void NonPositiveMinimumIsRejected(int ticks)
        =>Assert.Throws<ArgumentOutOfRangeException>(()=>VideoClipRange.Normalize(TimeSpan.FromSeconds(1),TimeSpan.Zero,TimeSpan.FromSeconds(1),TimeSpan.FromTicks(ticks)));

    [Fact]
    public void FullDetectionUsesAtMostOneMillisecondAtEachEdge()
    {
        var duration=TimeSpan.FromSeconds(10);
        Assert.True(VideoClipRange.Full(duration).IsFull(duration));
        Assert.True(new VideoClipRange(TimeSpan.FromMilliseconds(1),duration-TimeSpan.FromMilliseconds(1)).IsFull(duration));
        Assert.False(new VideoClipRange(TimeSpan.FromTicks(TimeSpan.TicksPerMillisecond+1),duration).IsFull(duration));
        Assert.False(new VideoClipRange(TimeSpan.Zero,duration-TimeSpan.FromTicks(TimeSpan.TicksPerMillisecond+1)).IsFull(duration));
        Assert.False(new VideoClipRange(TimeSpan.FromTicks(-1),duration).IsFull(duration));
        Assert.False(default(VideoClipRange).IsFull(duration));
        Assert.False(VideoClipRange.Full(duration).IsFull(TimeSpan.Zero));
    }
}
