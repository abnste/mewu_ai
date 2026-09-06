// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;

namespace mewu_ai_Assistant.Recording;

internal static class Mp3ExportService
{
    internal static async Task ExportAsync(string videoPath,string outputPath,CancellationToken cancellationToken=default)
    {
        var sourcePath=Path.GetFullPath(videoPath);
        var destinationPath=Path.GetFullPath(outputPath);
        if(string.Equals(sourcePath,destinationPath,StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException(LocalizationService.T("音频导出不能覆盖原视频。","Audio export cannot overwrite the source video."));
        using var sourceLease=TempMediaRegistry.Shared.AcquireExistingFile(sourcePath);
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromMinutes(5));
        var token=timeout.Token;
        var temporaryPath=new TempFileService().NewFile(".mp3");
        using var temporaryLease=TempMediaRegistry.Shared.Acquire(temporaryPath);
        try
        {
            var source=await StorageFile.GetFileFromPathAsync(sourcePath).AsTask(token).ConfigureAwait(false);
            var clip=await MediaClip.CreateFromFileAsync(source).AsTask(token).ConfigureAwait(false);
            if(clip.EmbeddedAudioTracks.Count==0)
                throw new InvalidOperationException(LocalizationService.T("这个视频没有音轨，无法导出 MP3。请开启录屏声音后重新录制。","This video has no audio track. Enable recording audio and record again to export MP3."));
            var folder=await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(temporaryPath)!).AsTask(token).ConfigureAwait(false);
            var output=await folder.CreateFileAsync(Path.GetFileName(temporaryPath),CreationCollisionOption.FailIfExists).AsTask(token).ConfigureAwait(false);
            var profile=MediaEncodingProfile.CreateMp3(AudioEncodingQuality.High);
            profile.Audio.Bitrate=192_000;
            var transcoder=new MediaTranscoder();
            using(var inputStream=await source.OpenAsync(FileAccessMode.Read).AsTask(token).ConfigureAwait(false))
            using(var outputStream=await output.OpenAsync(FileAccessMode.ReadWrite).AsTask(token).ConfigureAwait(false))
            {
                var prepared=await transcoder.PrepareStreamTranscodeAsync(inputStream,outputStream,profile).AsTask(token).ConfigureAwait(false);
                if(!prepared.CanTranscode)throw new InvalidOperationException(LocalizationService.T($"无法编码 MP3：{prepared.FailureReason}",$"Unable to encode MP3: {prepared.FailureReason}"));
                await prepared.TranscodeAsync().AsTask(token).ConfigureAwait(false);
            }
            token.ThrowIfCancellationRequested();
            if(new FileInfo(temporaryPath).Length==0)throw new InvalidDataException("MP3 encoder produced an empty file.");
            await Task.Run(()=>{token.ThrowIfCancellationRequested();AtomicFileService.Copy(temporaryPath,destinationPath);},token).ConfigureAwait(false);
        }
        catch(OperationCanceledException) when(!cancellationToken.IsCancellationRequested)
        {throw new TimeoutException(LocalizationService.T("MP3 导出超时，请缩短视频后重试。","MP3 export timed out. Try a shorter video."));}
        finally{try{File.Delete(temporaryPath);}catch(IOException){}catch(UnauthorizedAccessException){}}
    }
}
