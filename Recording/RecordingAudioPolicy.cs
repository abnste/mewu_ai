// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using ScreenRecorderLib;

namespace mewu_ai_Assistant.Recording;

internal static class RecordingAudioPolicy
{
    internal static AudioOptions Create(AppSettings settings)=>Create(settings,
        ()=>LoopbackAudioSource.Default,()=>CaptureAudioSource.Default);

    internal static AudioOptions Create(AppSettings settings,Func<LoopbackAudioSource?> output,Func<CaptureAudioSource?> input)
    {
        var sources=new List<AudioSourceBase>();
        if(settings.RecordSystemAudio)
            sources.Add(output()??throw new InvalidOperationException(LocalizationService.T(
                "未找到声音输出设备。请连接扬声器或耳机，或在录屏设置中关闭“录制电脑声音”。",
                "No audio output device was found. Connect speakers or headphones, or turn off computer audio in recording settings.")));
        if(settings.RecordMicrophone)
            sources.Add(input()??throw new InvalidOperationException(LocalizationService.T(
                "未找到麦克风。请连接并启用麦克风，或在录屏设置中关闭麦克风录音。",
                "No microphone was found. Connect and enable a microphone, or turn off microphone recording in settings.")));
        foreach(var source in sources){source.IsAudioCaptureEnabled=true;source.Volume=sources.Count>1?.5f:1f;}
        return new AudioOptions{IsAudioEnabled=sources.Count>0,AudioSources=sources,
            Channels=AudioChannels.Stereo,Bitrate=AudioBitrate.bitrate_192kbps,MasterVolume=1};
    }
}
