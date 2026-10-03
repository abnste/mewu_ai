// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.InteropServices.WindowsRuntime;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;

namespace mewu_ai_Assistant.Recording;

internal sealed class PreparedVideoClip : IDisposable
{
    private readonly TempMediaLease _lease;
    internal PreparedVideoClip(TempMediaLease lease,VideoClipRange range,bool isOriginal)
    {_lease=lease;Range=range;IsOriginal=isOriginal;}
    internal string Path=>_lease.Path;
    internal TimeSpan Duration=>Range.Duration;
    internal VideoClipRange Range{get;}
    internal bool IsOriginal{get;}
    // WinRT may still release a native reader after render completion. Never
    // delete synchronously here; the ordinary unleased-media cleanup owns it.
    public void Dispose()=>_lease.Dispose();
}

internal static class VideoClipPreparationService
{
    internal static async Task<PreparedVideoClip> PrepareAsync(
        string sourcePath,VideoClipRange? range,CancellationToken token,TempFileService? temp=null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(sourcePath);
        token.ThrowIfCancellationRequested();
        if(range is { } requested&&(requested.Start<TimeSpan.Zero||requested.End<=requested.Start))throw new ArgumentOutOfRangeException(nameof(range));
        TempMediaLease? sourceLease=TempMediaRegistry.Shared.AcquireExistingFile(sourcePath);
        TempMediaLease? outputLease=null;
        MediaComposition? composition=null;
        try
        {
            using var deadline=CancellationTokenSource.CreateLinkedTokenSource(token);
            deadline.CancelAfter(TimeSpan.FromMinutes(10));
            var cancellation=deadline.Token;
            var source=await StorageFile.GetFileFromPathAsync(sourceLease.Path).AsTask(cancellation).ConfigureAwait(false);
            var clip=await MediaClip.CreateFromFileAsync(source).AsTask(cancellation).ConfigureAwait(false);
            composition=new MediaComposition();composition.Clips.Add(clip);
            var properties=clip.GetVideoEncodingProperties();
            if(clip.OriginalDuration<=TimeSpan.Zero||properties.Width==0||properties.Height==0)
                throw new InvalidDataException(LocalizationService.T("视频时长或尺寸无效，无法裁切。","The video duration or dimensions are invalid and cannot be trimmed."));
            var normalized=range is { } selected?VideoClipRange.Normalize(clip.OriginalDuration,selected.Start,selected.End):VideoClipRange.Full(clip.OriginalDuration);
            if(normalized.IsFull(clip.OriginalDuration))
            {
                cancellation.ThrowIfCancellationRequested();
                var original=new PreparedVideoClip(sourceLease,VideoClipRange.Full(clip.OriginalDuration),true);
                sourceLease=null;
                return original;
            }

            clip.TrimTimeFromStart=normalized.Start;
            clip.TrimTimeFromEnd=clip.OriginalDuration-normalized.End;
            MediaEncodingProfile sourceProfile;
            using(var input=await source.OpenReadAsync().AsTask(cancellation).ConfigureAwait(false))
                sourceProfile=await MediaEncodingProfile.CreateFromStreamAsync(input).AsTask(cancellation).ConfigureAwait(false);
            var profile=MediaEncodingProfile.CreateMp4(VideoEncodingQuality.Auto);
            profile.Video.Width=properties.Width;profile.Video.Height=properties.Height;
            if(properties.Bitrate>0)profile.Video.Bitrate=properties.Bitrate;
            if(properties.FrameRate.Numerator==0||properties.FrameRate.Denominator==0)
                throw new InvalidDataException(LocalizationService.T("视频帧率无效，无法裁切。","The video frame rate is invalid and cannot be trimmed."));
            profile.Video.FrameRate.Numerator=properties.FrameRate.Numerator;
            profile.Video.FrameRate.Denominator=properties.FrameRate.Denominator;
            if(properties.PixelAspectRatio.Numerator>0&&properties.PixelAspectRatio.Denominator>0)
            {
                profile.Video.PixelAspectRatio.Numerator=properties.PixelAspectRatio.Numerator;
                profile.Video.PixelAspectRatio.Denominator=properties.PixelAspectRatio.Denominator;
            }
            var hasAudio=clip.EmbeddedAudioTracks.Count>0;
            if(!hasAudio)profile.Audio=null;
            else
            {
                var audio=sourceProfile.Audio??throw new InvalidDataException("The source audio format is unavailable.");
                profile.Audio.ChannelCount=audio.ChannelCount;profile.Audio.SampleRate=audio.SampleRate;
                if(audio.Bitrate>0)profile.Audio.Bitrate=audio.Bitrate;
            }
            temp??=new TempFileService();
            var outputPath=temp.NewFile(".mp4");
            outputLease=TempMediaRegistry.Shared.Acquire(outputPath);
            var folder=await StorageFolder.GetFolderFromPathAsync(temp.DirectoryPath).AsTask(cancellation).ConfigureAwait(false);
            var output=await folder.CreateFileAsync(System.IO.Path.GetFileName(outputPath),CreationCollisionOption.FailIfExists).AsTask(cancellation).ConfigureAwait(false);
            var result=await composition.RenderToFileAsync(output,MediaTrimmingPreference.Precise,profile).AsTask(cancellation).ConfigureAwait(false);
            if(result!=TranscodeFailureReason.None)
                throw new InvalidOperationException(LocalizationService.T($"视频裁切失败：{result}",$"Video trimming failed: {result}"));
            cancellation.ThrowIfCancellationRequested();
            if(new FileInfo(outputPath).Length==0)throw new InvalidDataException("The trimmed video is empty.");
            var verified=await MediaClip.CreateFromFileAsync(output).AsTask(cancellation).ConfigureAwait(false);
            var actual=verified.GetVideoEncodingProperties();
            var frameSeconds=properties.FrameRate.Denominator/(double)properties.FrameRate.Numerator;
            var durationTolerance=TimeSpan.FromSeconds(Math.Max(.1,frameSeconds+.05));
            if(verified.OriginalDuration<=TimeSpan.Zero||
               (verified.OriginalDuration-normalized.Duration).Duration()>durationTolerance||
               actual.Width!=properties.Width||actual.Height!=properties.Height||
               actual.FrameRate.Numerator==0||actual.FrameRate.Denominator==0||
               Math.Abs(actual.FrameRate.Numerator/(double)actual.FrameRate.Denominator-1/frameSeconds)>.001||
               (verified.EmbeddedAudioTracks.Count>0)!=hasAudio)
                throw new InvalidDataException(LocalizationService.T("裁切视频的时长、画面或音轨校验失败。","The trimmed video's duration, picture, or audio track could not be verified."));
            cancellation.ThrowIfCancellationRequested();
            var prepared=new PreparedVideoClip(outputLease,normalized,false);
            outputLease=null;
            return prepared;
        }
        catch(OperationCanceledException)when(!token.IsCancellationRequested)
        {throw new TimeoutException(LocalizationService.T("视频裁切超过 10 分钟，请缩短范围后重试。","Video trimming exceeded 10 minutes. Select a shorter range and retry."));}
        finally
        {
            // Remove composition-held references without force-GC or COM
            // release tricks. Failed outputs also remain for normal cleanup.
            try{composition?.Clips.Clear();}
            finally{outputLease?.Dispose();sourceLease?.Dispose();}
        }
    }
}
