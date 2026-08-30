using System.Runtime.InteropServices.WindowsRuntime;
using System.Security.Cryptography;
using System.Globalization;
using Windows.Graphics.Imaging;
using Windows.Media.Editing;
using Windows.Storage;

namespace MewuAI.ProviderSmoke;

internal static class VideoQaSampler
{
    private const int MaximumLongSide=1280;
    private const int SignatureColumns=32;
    private const int SignatureRows=18;
    private const int MeaningfulCellLumaDelta=12;
    private const double MinimumChangedCellRatio=.03;

    internal static async Task<string> HashFileAsync(string path,CancellationToken token)
    {
        await using var stream=new FileStream(path,FileMode.Open,FileAccess.Read,FileShare.ReadWrite|FileShare.Delete,1024*1024,FileOptions.Asynchronous|FileOptions.SequentialScan);
        return Convert.ToHexString(await SHA256.HashDataAsync(stream,token).ConfigureAwait(false));
    }

    internal static async Task<VideoQaEvidence> CaptureAsync(string videoPath,CancellationToken token)
    {
        var sourcePath=Path.GetFullPath(videoPath);
        var source=await StorageFile.GetFileFromPathAsync(sourcePath).AsTask(token).ConfigureAwait(false);
        var clip=await MediaClip.CreateFromFileAsync(source).AsTask(token).ConfigureAwait(false);
        var composition=new MediaComposition();composition.Clips.Add(clip);
        var duration=composition.Duration;
        if(duration<=TimeSpan.Zero)throw new InvalidDataException("录屏时长无效，无法抽取验收帧");

        var properties=clip.GetVideoEncodingProperties();
        var (scaledWidth,scaledHeight)=CalculateThumbnailSize(properties.Width,properties.Height);
        var frameRate=properties.FrameRate;
        var finalFrameMargin=frameRate.Numerator>0&&frameRate.Denominator>0
            ?TimeSpan.FromTicks(Math.Max(1,(long)Math.Ceiling(TimeSpan.TicksPerSecond*frameRate.Denominator/(double)frameRate.Numerator)))
            :TimeSpan.FromMilliseconds(100);
        var lastTicks=Math.Max(0,duration.Ticks-Math.Min(duration.Ticks,finalFrameMargin.Ticks));
        var points=new[]
        {
            (Label:"first",Timestamp:TimeSpan.Zero),
            (Label:"middle",Timestamp:TimeSpan.FromTicks(duration.Ticks/2)),
            (Label:"last",Timestamp:TimeSpan.FromTicks(lastTicks))
        };
        var sampleRoot=Path.Combine(Path.GetTempPath(),"MewuAI.ProviderSmoke");
        CleanupStaleSamples(sampleRoot);
        var directory=Path.Combine(sampleRoot,$"{DateTime.UtcNow:yyyyMMdd-HHmmss}-{Guid.NewGuid():N}");
        Directory.CreateDirectory(directory);
        var samples=new List<VideoFrameSample>(points.Length);

        try
        {
            foreach(var point in points)
            {
                token.ThrowIfCancellationRequested();
                using var thumbnail=await composition.GetThumbnailAsync(point.Timestamp,scaledWidth,scaledHeight,VideoFramePrecision.NearestFrame).AsTask(token).ConfigureAwait(false);
                var contentType=string.IsNullOrWhiteSpace(thumbnail.ContentType)?"image/jpeg":thumbnail.ContentType;
                var decoder=await BitmapDecoder.CreateAsync(thumbnail).AsTask(token).ConfigureAwait(false);
                var pixelProvider=await decoder.GetPixelDataAsync(
                    BitmapPixelFormat.Bgra8,
                    BitmapAlphaMode.Ignore,
                    new BitmapTransform(),
                    ExifOrientationMode.RespectExifOrientation,
                    ColorManagementMode.DoNotColorManage).AsTask(token).ConfigureAwait(false);
                var pixels=pixelProvider.DetachPixelData();
                string pixelHash;
                byte[] lumaSignature;
                try
                {
                    pixelHash=Convert.ToHexString(SHA256.HashData(pixels));
                    lumaSignature=CreateLumaSignature(pixels,(int)decoder.OrientedPixelWidth,(int)decoder.OrientedPixelHeight);
                }
                finally{CryptographicOperations.ZeroMemory(pixels);}
                var extension=contentType.Equals("image/png",StringComparison.OrdinalIgnoreCase)?".png":".jpg";
                var outputPath=Path.Combine(directory,$"{point.Label}{extension}");
                thumbnail.Seek(0);
                using(var input=thumbnail.AsStreamForRead())
                await using(var output=new FileStream(outputPath,FileMode.CreateNew,FileAccess.Write,FileShare.Read,81920,FileOptions.Asynchronous|FileOptions.SequentialScan))
                    await input.CopyToAsync(output,token).ConfigureAwait(false);
                samples.Add(new VideoFrameSample(point.Label,point.Timestamp,outputPath,new FileInfo(outputPath).Length,pixelHash,contentType,lumaSignature));
            }

            return new VideoQaEvidence(duration,directory,samples);
        }
        catch
        {
            try{if(Directory.Exists(directory))Directory.Delete(directory,true);}catch{}
            throw;
        }
    }

    internal static (int Width,int Height) CalculateThumbnailSize(uint sourceWidth,uint sourceHeight)
    {
        sourceWidth=Math.Max(1u,sourceWidth);sourceHeight=Math.Max(1u,sourceHeight);
        var scale=Math.Min(1d,MaximumLongSide/(double)Math.Max(sourceWidth,sourceHeight));
        return (Math.Max(1,(int)Math.Round(sourceWidth*scale)),Math.Max(1,(int)Math.Round(sourceHeight*scale)));
    }

    internal static IReadOnlyList<VideoFrameDifference> CompareFrames(IReadOnlyList<VideoFrameSample> samples)
    {
        var differences=new List<VideoFrameDifference>();
        for(var first=0;first<samples.Count;first++)
        for(var second=first+1;second<samples.Count;second++)
        {
            var left=samples[first];var right=samples[second];
            if(left.LumaSignature.Length!=right.LumaSignature.Length||left.LumaSignature.Length==0)continue;
            var changed=0;long totalDelta=0;
            for(var index=0;index<left.LumaSignature.Length;index++)
            {
                var delta=Math.Abs(left.LumaSignature[index]-right.LumaSignature[index]);
                totalDelta+=delta;if(delta>=MeaningfulCellLumaDelta)changed++;
            }
            var changedRatio=changed/(double)left.LumaSignature.Length;
            differences.Add(new VideoFrameDifference(left.Label,right.Label,changedRatio,totalDelta/(double)left.LumaSignature.Length,changedRatio>=MinimumChangedCellRatio));
        }
        return differences;
    }

    private static byte[] CreateLumaSignature(byte[] bgra,int width,int height)
    {
        if(width<=0||height<=0||bgra.LongLength<(long)width*height*4)throw new InvalidDataException("验收帧像素数据无效");
        var signature=new byte[SignatureColumns*SignatureRows];
        for(var row=0;row<SignatureRows;row++)
        {
            var top=row*height/SignatureRows;var bottom=Math.Max(top+1,(row+1)*height/SignatureRows);
            for(var column=0;column<SignatureColumns;column++)
            {
                var left=column*width/SignatureColumns;var right=Math.Max(left+1,(column+1)*width/SignatureColumns);long sum=0;var count=0;
                for(var y=top;y<bottom;y++)
                for(var x=left;x<right;x++)
                {
                    var offset=(y*width+x)*4;var blue=bgra[offset];var green=bgra[offset+1];var red=bgra[offset+2];
                    sum+=(54*red+183*green+19*blue)>>8;count++;
                }
                signature[row*SignatureColumns+column]=(byte)(sum/Math.Max(1,count));
            }
        }
        return signature;
    }

    private static void CleanupStaleSamples(string root)
    {
        if(!Directory.Exists(root))return;
        foreach(var path in Directory.EnumerateDirectories(root))
        {
            try
            {
                var directory=new DirectoryInfo(path);var name=directory.Name;
                if((directory.Attributes&System.IO.FileAttributes.ReparsePoint)!=0||name.Length!=48||name[15]!='-'||!Guid.TryParseExact(name[16..],"N",out _))continue;
                if(!DateTime.TryParseExact(name[..15],"yyyyMMdd-HHmmss",CultureInfo.InvariantCulture,DateTimeStyles.AssumeUniversal|DateTimeStyles.AdjustToUniversal,out var created)||DateTime.UtcNow-created<=TimeSpan.FromDays(2))continue;
                directory.Delete(true);
            }
            catch{}
        }
    }
}

internal sealed record VideoQaEvidence(TimeSpan Duration,string Directory,IReadOnlyList<VideoFrameSample> Samples);
internal sealed record VideoFrameSample(string Label,TimeSpan Timestamp,string Path,long Bytes,string Sha256,string ContentType,byte[] LumaSignature);
internal sealed record VideoFrameDifference(string First,string Second,double ChangedCellRatio,double MeanAbsoluteLumaDelta,bool Meaningful);
