// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Recording;

internal enum VideoExportFormat { Mp4, Mp3, Gif }

internal static class VideoExportFormats
{
    internal static string Filter=>LocalizationService.T(
        "MP4 视频|*.mp4|MP3 音频|*.mp3|GIF 动图（无声）|*.gif",
        "MP4 video|*.mp4|MP3 audio|*.mp3|Animated GIF (silent)|*.gif");
    internal static VideoExportFormat FromFilterIndex(int index)=>index switch
    {1=>VideoExportFormat.Mp4,2=>VideoExportFormat.Mp3,3=>VideoExportFormat.Gif,_=>throw new ArgumentOutOfRangeException(nameof(index))};
    internal static string Extension(VideoExportFormat format)=>format switch
    {VideoExportFormat.Mp4=>".mp4",VideoExportFormat.Mp3=>".mp3",VideoExportFormat.Gif=>".gif",_=>throw new ArgumentOutOfRangeException(nameof(format))};
}
