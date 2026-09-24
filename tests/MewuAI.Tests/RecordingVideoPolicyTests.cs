// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Recording;
using ScreenRecorderLib;
using Xunit;

namespace MewuAI.Tests;

public sealed class RecordingVideoPolicyTests
{
    [Fact]
    public void DesktopRecordingUsesHardwareFixedCadenceAndHighProfileVbr()
    {
        var options=RecordingVideoPolicy.Create(1920,1080,30,75);

        Assert.True(options.IsHardwareEncodingEnabled);
        Assert.True(options.IsFixedFramerate);
        Assert.False(options.IsLowLatencyEnabled);
        Assert.False(options.IsThrottlingDisabled);
        Assert.True(options.IsMp4FastStartEnabled);
        Assert.Equal(30,options.Framerate);
        Assert.Equal(75,options.Quality);
        var encoder=Assert.IsType<H264VideoEncoder>(options.Encoder);
        Assert.Equal(H264BitrateControlMode.UnconstrainedVBR,encoder.BitrateMode);
        Assert.Equal(H264Profile.High,encoder.EncoderProfile);
    }

    [Theory]
    [InlineData(360,240,30,75,4_000_000)]
    [InlineData(1920,1080,30,75,10_886_400)]
    [InlineData(3840,2160,60,100,50_000_000)]
    public void BitrateTracksCaptureLoadWithinSafeBounds(int width,int height,int fps,int quality,int expected)
    {
        Assert.Equal(expected,RecordingVideoPolicy.CalculateBitrate(width,height,fps,quality));
    }

    [Fact]
    public void EncoderInputsAreClamped()
    {
        var options=RecordingVideoPolicy.Create(1,1,200,-20);

        Assert.Equal(60,options.Framerate);
        Assert.Equal(20,options.Quality);
        Assert.Equal(4_000_000,options.Bitrate);
    }
}
