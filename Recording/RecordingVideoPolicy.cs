// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using ScreenRecorderLib;

namespace mewu_ai_Assistant.Recording;

internal static class RecordingVideoPolicy
{
    private const int MinimumBitrate=4_000_000;
    private const int MaximumBitrate=50_000_000;

    internal static VideoEncoderOptions Create(int width,int height,int requestedFps,int requestedQuality)
    {
        var fps=Math.Clamp(requestedFps,10,60);
        var quality=Math.Clamp(requestedQuality,20,100);
        return new VideoEncoderOptions
        {
            Encoder=new H264VideoEncoder
            {
                BitrateMode=H264BitrateControlMode.UnconstrainedVBR,
                EncoderProfile=H264Profile.High
            },
            Framerate=fps,
            Quality=quality,
            Bitrate=CalculateBitrate(width,height,fps,quality),
            // ScreenRecorderLib and the Windows capture stack default to the
            // GPU path. It keeps capture/encode work off the UI and CPU while
            // fixed cadence fills unchanged desktop frames when necessary.
            IsHardwareEncodingEnabled=true,
            IsFixedFramerate=true,
            IsLowLatencyEnabled=false,
            IsThrottlingDisabled=false,
            IsMp4FastStartEnabled=true
        };
    }

    internal static int CalculateBitrate(int width,int height,int fps,int quality)
    {
        var safeWidth=Math.Max(2,width);
        var safeHeight=Math.Max(2,height);
        var safeFps=Math.Clamp(fps,10,60);
        var safeQuality=Math.Clamp(quality,20,100);
        // Text and thin UI edges need more bits than camera footage. Scale the
        // VBR target with pixels, cadence and the visible quality setting while
        // bounding both tiny selections and 4K/60 recordings.
        // Keep this arithmetic integral.  A floating-point ceiling turns
        // exact targets such as 1920×1080×30×0.175 into 10,886,401 on some
        // runtimes because the intermediate value is a tiny fraction above
        // the mathematical result.
        var numerator=(long)safeWidth*safeHeight*safeFps*(100L+safeQuality);
        var calculated=(numerator+999L)/1000L;
        return (int)Math.Clamp(calculated,MinimumBitrate,MaximumBitrate);
    }
}
