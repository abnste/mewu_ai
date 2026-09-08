// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.InteropServices.WindowsRuntime;
using System.Security.Cryptography;
using Windows.Graphics.Imaging;
using Windows.Media.Core;
using Windows.Media.Editing;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;

namespace mewu_ai_Assistant.Recording;

// MiniMax currently labels sampled video frames using the encoded frame rate.
// A separate analysis copy with matching encoded/sampling rates keeps its time
// labels in source seconds. The recording and its preview/export stay intact.
internal static class VideoAnalysisTimebaseService
{
    internal const int FramesPerSecond=2;
    private const long MaximumOutputBytes=48L*1024*1024;
    internal static async Task<VideoAnalysisCopy> CreateAsync(AiAttachment attachment,CancellationToken token)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(token);
        timeout.CancelAfter(TimeSpan.FromMinutes(3));
        var cancellation=timeout.Token;
        var temporary=new TempFileService();
        var inputPath=attachment.FilePath??temporary.NewFile(".mp4");
        using var inputLease=attachment.FilePath is null?TempMediaRegistry.Shared.Acquire(inputPath):TempMediaRegistry.Shared.AcquireExistingFile(inputPath);
        var outputPath=temporary.NewFile(".mp4");
        var outputLease=TempMediaRegistry.Shared.Acquire(outputPath);
        var stage="read-source";
        try
        {
            if(attachment.FilePath is null)await File.WriteAllBytesAsync(inputPath,attachment.Data??throw new InvalidDataException("视频内容为空"),cancellation).ConfigureAwait(false);
            var source=await StorageFile.GetFileFromPathAsync(inputPath).AsTask(cancellation).ConfigureAwait(false);
            using var sourceStream=await source.OpenReadAsync().AsTask(cancellation).ConfigureAwait(false);
            var original=await MediaEncodingProfile.CreateFromStreamAsync(sourceStream).AsTask(cancellation).ConfigureAwait(false);
            var metadata=await source.Properties.GetVideoPropertiesAsync().AsTask(cancellation).ConfigureAwait(false);
            var duration=metadata.Duration;
            if(original.Video is null||duration<=TimeSpan.Zero||duration>TimeSpan.FromMinutes(30))throw new InvalidDataException("视频时长无效或超过 30 分钟，请分段后发送");
            var encodedDuration=GetEncodedDuration(duration);
            var profile=MediaEncodingProfile.CreateMp4(VideoEncodingQuality.HD720p);
            var scale=Math.Min(1,1280d/Math.Max(original.Video.Width,original.Video.Height));
            profile.Video.Width=(uint)Math.Max(2,(int)(original.Video.Width*scale)&~1);
            profile.Video.Height=(uint)Math.Max(2,(int)(original.Video.Height*scale)&~1);
            profile.Video.FrameRate.Numerator=FramesPerSecond;profile.Video.FrameRate.Denominator=1;
            profile.Video.ProfileId=H264ProfileIds.Baseline;
            profile.Video.Bitrate=(uint)Math.Clamp(36_000_000d*8/duration.TotalSeconds,32_000,2_000_000);
            // M3's native video input is visual. The original audio remains in
            // the untouched recording used for preview, copy and export.
            profile.Audio=null;
            var folder=await StorageFolder.GetFolderFromPathAsync(temporary.DirectoryPath).AsTask(cancellation).ConfigureAwait(false);
            var output=await folder.CreateFileAsync(Path.GetFileName(outputPath),CreationCollisionOption.FailIfExists).AsTask(cancellation).ConfigureAwait(false);
            // Explicit end time prevents Media Foundation's low-FPS encoder
            // from padding the final GOP (27s otherwise became 41s locally).
            var transcoder=new MediaTranscoder{HardwareAccelerationEnabled=false,TrimStopTime=encodedDuration};
            // Feeding explicit source-time samples avoids the file transcoder's
            // frame-rate converter shifting scene changes in short videos.
            var clip=await MediaClip.CreateFromFileAsync(source).AsTask(cancellation).ConfigureAwait(false);
            var composition=new MediaComposition();composition.Clips.Add(clip);
            var sourceFrameDuration=original.Video.FrameRate.Numerator>0&&original.Video.FrameRate.Denominator>0
                ?TimeSpan.FromSeconds(original.Video.FrameRate.Denominator/(double)original.Video.FrameRate.Numerator)
                :TimeSpan.FromSeconds(1d/FramesPerSecond);
            using(var frames=new TimestampedFrames(composition,profile.Video.Width,profile.Video.Height,sourceFrameDuration,encodedDuration,cancellation))
            using(var destination=await output.OpenAsync(FileAccessMode.ReadWrite).AsTask(cancellation).ConfigureAwait(false))
            {
                stage="prepare-transcoder";
                var prepared=await transcoder.PrepareMediaStreamSourceTranscodeAsync(frames.Source,destination,profile).AsTask(cancellation).ConfigureAwait(false);
                if(!prepared.CanTranscode)throw new InvalidOperationException("无法准备视频时间轴校准，请换用 MP4 后重试");
                stage="transcode";
                try{await prepared.TranscodeAsync().AsTask(cancellation,new OutputBudget(outputPath,timeout)).ConfigureAwait(false);}
                catch(Exception)when(!cancellation.IsCancellationRequested&&frames.Failure is not null){throw new InvalidDataException($"视频采样失败（阶段 {frames.FailureStage}，错误码 0x{frames.Failure.HResult:X8}）",frames.Failure);}
                if(frames.Failure is { } failure)throw new InvalidDataException("无法提取视频时间采样帧",failure);
            }
            stage="verify-output";
            cancellation.ThrowIfCancellationRequested();
            var size=new FileInfo(outputPath).Length;
            if(size<=0||size>MaximumOutputBytes)throw new InvalidDataException("视频分析副本大小无效，请缩短视频后重试");
            using var encoded=await output.OpenReadAsync().AsTask(cancellation).ConfigureAwait(false);
            var actual=await MediaEncodingProfile.CreateFromStreamAsync(encoded).AsTask(cancellation).ConfigureAwait(false);
            var actualMetadata=await output.Properties.GetVideoPropertiesAsync().AsTask(cancellation).ConfigureAwait(false);
            EnsureTimebase(actual.Video?.FrameRate.Numerator??0,actual.Video?.FrameRate.Denominator??0,duration,actualMetadata.Duration);
            // Report source time, never the repeated transport-only tail.
            return new(outputLease,duration);
        }
        catch(OperationCanceledException)when(!token.IsCancellationRequested)
        {outputLease.Dispose();TryDelete(outputPath);throw new TimeoutException("视频时间轴校准超时或副本过大，请缩短视频后重试");}
        catch(System.Runtime.InteropServices.COMException ex)
        {outputLease.Dispose();TryDelete(outputPath);throw new InvalidDataException($"视频时间轴校准失败（阶段 {stage}，错误码 0x{ex.HResult:X8}）",ex);}
        catch{outputLease.Dispose();TryDelete(outputPath);throw;}
        finally{if(attachment.FilePath is null)TryDelete(inputPath);}
    }

    internal static TimeSpan GetEncodedDuration(TimeSpan sourceDuration)
    {
        if(sourceDuration<=TimeSpan.Zero||sourceDuration>TimeSpan.FromMinutes(30))throw new ArgumentOutOfRangeException(nameof(sourceDuration));
        // M3 uses temporal patches of two frames. Its API rejects odd sample
        // counts with 2013; a partial final sample can also be dropped by MF.
        // Complete the final pair with the last source frame, without moving
        // any earlier event or extending the user's original recording.
        const long frameTicks=TimeSpan.TicksPerSecond/FramesPerSecond;
        var frames=(sourceDuration.Ticks+frameTicks-1)/frameTicks;
        return TimeSpan.FromTicks(((frames+1)/2*2)*frameTicks);
    }

    internal static void EnsureTimebase(uint numerator,uint denominator,TimeSpan sourceDuration,TimeSpan actualDuration)
    {
        if(denominator==0||Math.Abs(numerator/(double)denominator-FramesPerSecond)>.001||actualDuration<=TimeSpan.Zero||Math.Abs((GetEncodedDuration(sourceDuration)-actualDuration).TotalSeconds)>.05)
            throw new InvalidDataException($"视频时间轴校准验证失败，已停止发送（帧率 {numerator}/{denominator}，时长 {sourceDuration.TotalSeconds:0.###}/{actualDuration.TotalSeconds:0.###} 秒）");
    }

    internal static TimeSpan GetSourceSampleTime(TimeSpan sampleTime,TimeSpan compositionDuration,TimeSpan sourceFrameDuration)
    {
        if(compositionDuration<=TimeSpan.Zero||sourceFrameDuration<=TimeSpan.Zero||sampleTime<TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(sampleTime));
        // Container duration may include fractional frame padding. Requesting
        // duration minus one tick can still round to EOF in NearestFrame mode
        // (E_INVALIDARG on Windows Server). Stay inside the last source frame
        // using the decoder's timeline, not the shell's container duration.
        var lastFrameStart=TimeSpan.FromTicks(Math.Max(0,compositionDuration.Ticks-sourceFrameDuration.Ticks));
        return sampleTime<lastFrameStart?sampleTime:lastFrameStart;
    }
    internal static void TryDelete(string path){try{File.Delete(path);}catch(IOException){}catch(UnauthorizedAccessException){}}

    private sealed class OutputBudget(string path,CancellationTokenSource timeout):IProgress<double>
    {
        public void Report(double value)
        {
            try{if(new FileInfo(path).Length>MaximumOutputBytes)timeout.Cancel();}
            catch(IOException){try{timeout.Cancel();}catch(ObjectDisposedException){}}
            catch(UnauthorizedAccessException){try{timeout.Cancel();}catch(ObjectDisposedException){}}
            catch(ObjectDisposedException){}
        }
    }

    private sealed class TimestampedFrames:IDisposable
    {
        private readonly MediaComposition _composition;
        private readonly uint _width,_height;
        private readonly TimeSpan _duration,_sourceFrameDuration;
        private readonly CancellationToken _token;
        private readonly object _gate=new();
        private readonly HashSet<byte[]> _buffers=[];
        private int _index;
        private bool _disposed;
        internal MediaStreamSource Source {get;}
        internal Exception? Failure {get;private set;}
        internal string FailureStage {get;private set;}=string.Empty;
        internal TimestampedFrames(MediaComposition composition,uint width,uint height,TimeSpan sourceFrameDuration,TimeSpan duration,CancellationToken token)
        {
            _composition=composition;_width=width;_height=height;_sourceFrameDuration=sourceFrameDuration;_duration=duration;_token=token;
            var raw=VideoEncodingProperties.CreateUncompressed(MediaEncodingSubtypes.Bgra8,width,height);
            raw.FrameRate.Numerator=FramesPerSecond;raw.FrameRate.Denominator=1;
            Source=new MediaStreamSource(new VideoStreamDescriptor(raw)){Duration=duration,BufferTime=TimeSpan.Zero,CanSeek=false};
            Source.Starting+=Starting;Source.SampleRequested+=SampleRequested;
        }
        private static void Starting(MediaStreamSource source,MediaStreamSourceStartingEventArgs args)=>args.Request.SetActualStartPosition(TimeSpan.Zero);
        private async void SampleRequested(MediaStreamSource source,MediaStreamSourceSampleRequestedEventArgs args)
        {
            var deferral=args.Request.GetDeferral();byte[]? bytes=null;
            var stage="request";
            try
            {
                _token.ThrowIfCancellationRequested();
                var time=TimeSpan.FromSeconds(Interlocked.Increment(ref _index)-1d)/FramesPerSecond;
                if(time>=_duration)return;
                var sourceTime=GetSourceSampleTime(time,_composition.Duration,_sourceFrameDuration);
                stage="thumbnail";
                using var thumbnail=await _composition.GetThumbnailAsync(sourceTime,(int)_width,(int)_height,VideoFramePrecision.NearestFrame).AsTask(_token).ConfigureAwait(false);
                stage="decode-thumbnail";
                var decoder=await BitmapDecoder.CreateAsync(thumbnail).AsTask(_token).ConfigureAwait(false);
                bytes=(await decoder.GetPixelDataAsync(BitmapPixelFormat.Bgra8,BitmapAlphaMode.Ignore,new BitmapTransform(),ExifOrientationMode.IgnoreExifOrientation,ColorManagementMode.DoNotColorManage).AsTask(_token).ConfigureAwait(false)).DetachPixelData();
                lock(_gate)
                {
                    _token.ThrowIfCancellationRequested();if(_disposed)return;
                    if(_buffers.Count>=16)throw new InvalidOperationException("视频编码缓冲超出限制");
                    _buffers.Add(bytes);
                    var owned=bytes;var sample=MediaStreamSample.CreateFromBuffer(owned.AsBuffer(),time);
                    sample.Duration=TimeSpan.FromSeconds(1d/FramesPerSecond);sample.KeyFrame=true;
                    sample.Processed+=(_,_)=>{lock(_gate){if(_buffers.Remove(owned))CryptographicOperations.ZeroMemory(owned);}};
                    stage="deliver-sample";args.Request.Sample=sample;bytes=null;
                }
            }
            catch(Exception ex){Failure=ex;FailureStage=stage;try{source.NotifyError(MediaStreamSourceErrorStatus.Other);}catch{}}
            finally{if(bytes is not null)CryptographicOperations.ZeroMemory(bytes);try{deferral.Complete();}catch(Exception ex){if(Failure is null){Failure=ex;FailureStage="complete-request";}}}
        }
        public void Dispose()
        {
            Source.SampleRequested-=SampleRequested;Source.Starting-=Starting;
            lock(_gate){_disposed=true;foreach(var bytes in _buffers)CryptographicOperations.ZeroMemory(bytes);_buffers.Clear();}
            _composition.Clips.Clear();
        }
    }
}

internal sealed class VideoAnalysisCopy(TempMediaLease lease,TimeSpan duration):IDisposable
{
    internal string Path=>lease.Path;
    internal TimeSpan Duration=>duration;
    public void Dispose(){VideoAnalysisTimebaseService.TryDelete(lease.Path);lease.Dispose();}
}
