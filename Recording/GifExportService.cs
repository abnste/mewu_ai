using System.Runtime.InteropServices.WindowsRuntime;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Windows.Media.Editing;
using Windows.Storage;

namespace mewu_ai_Assistant.Recording;

public static class GifExportService
{
    internal const int MaximumFrameCount=240;
    internal const int MaximumDimension=720;
    internal const long MaximumDecodedPixelBudget=24_000_000;

    public static async Task<GifExportResult> ExportFromVideoAsync(
        string videoPath,
        string outputPath,
        int requestedFps,
        CancellationToken cancellationToken=default)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(videoPath);
        ArgumentException.ThrowIfNullOrWhiteSpace(outputPath);
        var sourcePath=Path.GetFullPath(videoPath);
        var destinationPath=Path.GetFullPath(outputPath);
        if(string.Equals(sourcePath,destinationPath,StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("GIF 输出路径不能覆盖源 MP4");
        using var sourceLease=TempMediaRegistry.Shared.AcquireExistingFile(sourcePath);

        var source=await StorageFile.GetFileFromPathAsync(sourcePath).AsTask(cancellationToken).ConfigureAwait(false);
        var clip=await MediaClip.CreateFromFileAsync(source).AsTask(cancellationToken).ConfigureAwait(false);
        var composition=new MediaComposition();composition.Clips.Add(clip);
        var plan=CreateFramePlan(composition.Duration,requestedFps);
        var properties=clip.GetVideoEncodingProperties();
        var thumbnailSize=CalculateThumbnailSize(properties.Width,properties.Height,plan.Timestamps.Count);
        var frames=new List<BitmapSource>(plan.Timestamps.Count);

        foreach(var timestamp in plan.Timestamps)
        {
            cancellationToken.ThrowIfCancellationRequested();
            using var thumbnail=await composition.GetThumbnailAsync(timestamp,thumbnailSize.Width,thumbnailSize.Height,VideoFramePrecision.NearestFrame).AsTask(cancellationToken).ConfigureAwait(false);
            using var stream=thumbnail.AsStreamForRead();
            var decoder=BitmapDecoder.Create(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);
            var bitmap=decoder.Frames[0];bitmap.Freeze();frames.Add(bitmap);
        }

        cancellationToken.ThrowIfCancellationRequested();
        var temporaryPath=new TempFileService().NewFile(".gif");
        using var temporaryLease=TempMediaRegistry.Shared.Acquire(temporaryPath);
        try
        {
            WriteGif(frames,temporaryPath,plan.Delays);
            AtomicFileService.Copy(temporaryPath,destinationPath);
        }
        finally
        {
            try{if(File.Exists(temporaryPath))File.Delete(temporaryPath);}catch{}
        }

        return new GifExportResult(plan.Timestamps.Count,plan.EffectiveFps,composition.Duration);
    }

    internal static GifFramePlan CreateFramePlan(TimeSpan duration,int requestedFps,int maximumFrames=MaximumFrameCount)
    {
        if(duration<=TimeSpan.Zero)throw new InvalidDataException("视频时长无效，无法导出 GIF");
        ArgumentOutOfRangeException.ThrowIfLessThan(maximumFrames,1);
        var fps=Math.Clamp(requestedFps,1,15);
        var desiredFrames=Math.Max(1L,(long)Math.Ceiling(duration.TotalSeconds*fps));
        var frameCount=(int)Math.Min(maximumFrames,desiredFrames);
        var intervalTicks=duration.Ticks/(double)frameCount;
        var timestamps=Enumerable.Range(0,frameCount)
            .Select(index=>TimeSpan.FromTicks(Math.Min(duration.Ticks-1,(long)Math.Round(index*intervalTicks))))
            .ToArray();

        var totalCentiseconds=Math.Max(frameCount*2L,(long)Math.Round(duration.TotalMilliseconds/10d,MidpointRounding.AwayFromZero));
        var baseDelay=totalCentiseconds/frameCount;var remainder=totalCentiseconds%frameCount;
        if(baseDelay+(remainder>0?1:0)>ushort.MaxValue)throw new InvalidDataException("视频过长，超出 GIF 单帧时长限制");
        var delays=Enumerable.Range(0,frameCount)
            .Select(index=>(ushort)(baseDelay+(index<remainder?1:0)))
            .ToArray();
        return new GifFramePlan(timestamps,delays,frameCount/duration.TotalSeconds);
    }

    internal static GifThumbnailSize CalculateThumbnailSize(
        uint sourceWidth,
        uint sourceHeight,
        int frameCount,
        int maximumDimension=MaximumDimension,
        long decodedPixelBudget=MaximumDecodedPixelBudget)
    {
        if(sourceWidth==0||sourceHeight==0)throw new InvalidDataException("视频尺寸无效，无法安全导出 GIF");
        ArgumentOutOfRangeException.ThrowIfLessThan(frameCount,1);
        ArgumentOutOfRangeException.ThrowIfLessThan(maximumDimension,1);
        if(decodedPixelBudget<frameCount)throw new ArgumentOutOfRangeException(nameof(decodedPixelBudget),"总解码像素预算至少需要每帧一个像素");

        var sourcePixels=(double)sourceWidth*sourceHeight;
        var dimensionScale=maximumDimension/(double)Math.Max(sourceWidth,sourceHeight);
        var budgetScale=Math.Sqrt(decodedPixelBudget/(double)frameCount/sourcePixels);
        var scale=Math.Min(1d,Math.Min(dimensionScale,budgetScale));
        var width=Math.Max(1,(int)Math.Floor(sourceWidth*scale));
        var height=Math.Max(1,(int)Math.Floor(sourceHeight*scale));
        return new GifThumbnailSize(width,height);
    }

    internal static void WriteGif(IReadOnlyList<BitmapSource> frames,string outputPath,IReadOnlyList<ushort> delays)
    {
        if(frames.Count==0)throw new InvalidOperationException("没有可导出的录屏帧");
        if(frames.Count!=delays.Count)throw new ArgumentException("GIF 帧数与延迟数量不一致",nameof(delays));
        var encoder=new GifBitmapEncoder();
        for(var index=0;index<frames.Count;index++)
        {
            var metadata=new BitmapMetadata("gif");metadata.SetQuery("/grctlext/Delay",delays[index]);metadata.SetQuery("/grctlext/Disposal",(byte)2);encoder.Frames.Add(BitmapFrame.Create(frames[index],null,metadata,null));
        }
        using(var output=File.Create(outputPath))encoder.Save(output);
        ApplyFrameDelays(outputPath,delays);
    }

    // .NET 10 WPF currently emits Graphic Control Extensions with a zero delay even when
    // BitmapMetadata contains /grctlext/Delay. Walk the GIF block structure and correct them.
    private static void ApplyFrameDelays(string path,IReadOnlyList<ushort> delays)
    {
        var source=File.ReadAllBytes(path);using var input=new MemoryStream(source,false);using var reader=new BinaryReader(input,System.Text.Encoding.ASCII,true);using var output=new MemoryStream(source.Length+delays.Count*8);using var writer=new BinaryWriter(output,System.Text.Encoding.ASCII,true);
        var header=reader.ReadBytes(6);if(header.Length!=6||new string(System.Text.Encoding.ASCII.GetChars(header)) is not ("GIF87a" or "GIF89a"))throw new InvalidDataException("GIF 文件头无效");writer.Write(header);var descriptor=reader.ReadBytes(7);if(descriptor.Length!=7)throw new EndOfStreamException();writer.Write(descriptor);if((descriptor[4]&0x80)!=0)CopyBytes(reader,writer,3*(1<<((descriptor[4]&7)+1)));
        var frameIndex=0;var hasGraphicControl=false;
        while(input.Position<input.Length)
        {
            var marker=reader.ReadByte();
            if(marker==0x3B){writer.Write(marker);break;}
            if(marker==0x21)
            {
                var label=reader.ReadByte();
                if(label==0xF9)
                {
                    if(frameIndex>=delays.Count)throw new InvalidDataException("GIF 图形控制扩展数量超过帧计划");if(reader.ReadByte()!=4)throw new InvalidDataException("GIF 图形控制扩展长度无效");var packed=reader.ReadByte();reader.ReadUInt16();var transparent=reader.ReadByte();if(reader.ReadByte()!=0)throw new InvalidDataException("GIF 图形控制扩展终止符无效");writer.Write((byte)0x21);writer.Write((byte)0xF9);writer.Write((byte)4);writer.Write(packed);writer.Write(delays[frameIndex]);writer.Write(transparent);writer.Write((byte)0);hasGraphicControl=true;
                }
                else{writer.Write(marker);writer.Write(label);CopySubBlocks(reader,writer);}
                continue;
            }
            if(marker!=0x2C)throw new InvalidDataException($"未知 GIF 块标记 0x{marker:X2}");
            if(frameIndex>=delays.Count)throw new InvalidDataException("GIF 图像帧数量超过帧计划");
            if(!hasGraphicControl){writer.Write((byte)0x21);writer.Write((byte)0xF9);writer.Write((byte)4);writer.Write((byte)8);writer.Write(delays[frameIndex]);writer.Write((byte)0);writer.Write((byte)0);}
            writer.Write(marker);var imageDescriptor=reader.ReadBytes(9);if(imageDescriptor.Length!=9)throw new EndOfStreamException();writer.Write(imageDescriptor);if((imageDescriptor[8]&0x80)!=0)CopyBytes(reader,writer,3*(1<<((imageDescriptor[8]&7)+1)));writer.Write(reader.ReadByte());CopySubBlocks(reader,writer);frameIndex++;hasGraphicControl=false;
        }
        if(frameIndex!=delays.Count)throw new InvalidDataException($"GIF 图像帧数量异常：期望 {delays.Count}，实际 {frameIndex}");writer.Flush();File.WriteAllBytes(path,output.ToArray());
    }

    private static void CopySubBlocks(BinaryReader reader,BinaryWriter writer){while(true){var size=reader.ReadByte();writer.Write(size);if(size==0)return;CopyBytes(reader,writer,size);}}
    private static void CopyBytes(BinaryReader reader,BinaryWriter writer,int count){var bytes=reader.ReadBytes(count);if(bytes.Length!=count)throw new EndOfStreamException();writer.Write(bytes);}
}

public sealed record GifExportResult(int FrameCount,double EffectiveFps,TimeSpan Duration);

internal sealed record GifFramePlan(IReadOnlyList<TimeSpan> Timestamps,IReadOnlyList<ushort> Delays,double EffectiveFps);
internal readonly record struct GifThumbnailSize(int Width,int Height);
