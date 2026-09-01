using System.Runtime.InteropServices.WindowsRuntime;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Windows.Foundation;
using Windows.Media.Editing;
using Windows.Media.MediaProperties;
using Windows.Media.Transcoding;
using Windows.Storage;

namespace mewu_ai_Assistant.Recording;

internal static class AnnotatedVideoExportService
{
    internal static async Task ExportAsync(string videoPath,string outputPath,BitmapSource? manualOverlay,IReadOnlyList<AiAnnotation> annotations,CancellationToken cancellationToken=default)
    {
        var stage="读取源视频";
        try
        {
        var sourcePath=Path.GetFullPath(videoPath);var destinationPath=Path.GetFullPath(outputPath);
        if(string.Equals(sourcePath,destinationPath,StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("带标注视频不能覆盖录屏源文件");
        using var sourceLease=TempMediaRegistry.Shared.AcquireExistingFile(sourcePath);
        var source=await StorageFile.GetFileFromPathAsync(sourcePath).AsTask(cancellationToken);
        var clip=await MediaClip.CreateFromFileAsync(source).AsTask(cancellationToken);var properties=clip.GetVideoEncodingProperties();if(properties.Width==0||properties.Height==0)throw new InvalidDataException("视频尺寸无效，无法合成标注");
        var composition=new MediaComposition();composition.Clips.Add(clip);var layer=new MediaOverlayLayer();var temporaryFiles=new List<(string Path,TempMediaLease Lease)>();
        try
        {
            stage="创建手工标注覆盖层";if(manualOverlay is not null)await AddOverlayAsync(manualOverlay,TimeSpan.Zero,composition.Duration);
            foreach(var frame in VideoAnnotationOverlayPlan.Create(annotations,composition.Duration))
            {
                stage="创建 AI 时间轴覆盖层";cancellationToken.ThrowIfCancellationRequested();var overlay=AnnotationOverlayRenderer.RenderAiOverlay((int)properties.Width,(int)properties.Height,annotations,frame.SampleTime.TotalSeconds);await AddOverlayAsync(overlay,frame.Start,frame.Duration);
            }
            if(layer.Overlays.Count==0){await Task.Run(()=>AtomicFileService.Copy(sourcePath,destinationPath),cancellationToken);return;}
            composition.OverlayLayers.Add(layer);
            var temporaryOutput=new TempFileService().NewFile(".mp4");using var outputLease=TempMediaRegistry.Shared.Acquire(temporaryOutput);
            try
            {
                stage="创建临时输出文件";var outputFolderPath=Path.GetDirectoryName(temporaryOutput)??throw new InvalidOperationException("临时视频目录无效");
                var outputFolder=await StorageFolder.GetFolderFromPathAsync(outputFolderPath).AsTask(cancellationToken);
                var output=await outputFolder.CreateFileAsync(Path.GetFileName(temporaryOutput),CreationCollisionOption.ReplaceExisting).AsTask(cancellationToken);
                stage="创建 MP4 编码配置";var quality=Math.Max(properties.Width,properties.Height)>=1920?VideoEncodingQuality.HD1080p:Math.Max(properties.Width,properties.Height)>=1280?VideoEncodingQuality.HD720p:VideoEncodingQuality.Wvga;var profile=MediaEncodingProfile.CreateMp4(quality);
                profile.Video.Width=properties.Width;
                profile.Video.Height=properties.Height;
                profile.Video.Bitrate=Math.Clamp(properties.Bitrate,750_000u,24_000_000u);
                if(properties.FrameRate.Numerator>0&&properties.FrameRate.Denominator>0){profile.Video.FrameRate.Numerator=properties.FrameRate.Numerator;profile.Video.FrameRate.Denominator=properties.FrameRate.Denominator;}
                stage="渲染带标注视频";var result=await composition.RenderToFileAsync(output,MediaTrimmingPreference.Precise,profile).AsTask(cancellationToken);
                if(result!=TranscodeFailureReason.None)throw new InvalidOperationException($"视频标注合成失败：{result}");
                cancellationToken.ThrowIfCancellationRequested();AtomicFileService.Copy(temporaryOutput,destinationPath);
            }
            finally{try{if(File.Exists(temporaryOutput))File.Delete(temporaryOutput);}catch{}}
        }
        finally
        {
            foreach(var temporary in temporaryFiles){temporary.Lease.Dispose();try{if(File.Exists(temporary.Path))File.Delete(temporary.Path);}catch{}}
        }

        async Task AddOverlayAsync(BitmapSource bitmap,TimeSpan delay,TimeSpan duration)
        {
            if(duration<=TimeSpan.Zero)return;var path=new TempFileService().NewFile(".png");var lease=TempMediaRegistry.Shared.Acquire(path);temporaryFiles.Add((path,lease));AnnotationOverlayRenderer.SavePng(bitmap,path);var file=await StorageFile.GetFileFromPathAsync(path).AsTask(cancellationToken);var imageClip=await MediaClip.CreateFromImageFileAsync(file,duration).AsTask(cancellationToken);layer.Overlays.Add(new MediaOverlay(imageClip){Delay=delay,Position=new Rect(0,0,properties.Width,properties.Height)});
        }
        }
        catch(System.Runtime.InteropServices.COMException ex){throw new InvalidOperationException($"{stage}失败（0x{ex.HResult:X8}）",ex);}
    }
}
