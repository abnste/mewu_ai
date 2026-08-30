using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Security.Cryptography;
using mewu_ai_Assistant.Recording;
using Xunit;

namespace MewuAI.Tests;

public sealed class GifExportTests
{
    [Fact]
    public void Export_WritesAnimatedFramesWithDelayMetadata()
    {
        var root = Path.Combine(Path.GetTempPath(), "MewuAI.Tests", Guid.NewGuid().ToString("N"));
        var output = Path.Combine(root, "result.gif");
        Directory.CreateDirectory(root);
        try
        {
            GifExportService.WriteGif([CreateBitmap(Colors.Red),CreateBitmap(Colors.Blue)],output,[10,15]);
            using var stream = File.OpenRead(output);
            var decoder = new GifBitmapDecoder(stream, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
            Assert.Equal(2, decoder.Frames.Count);
            var first = Assert.IsType<BitmapMetadata>(decoder.Frames[0].Metadata);
            var second = Assert.IsType<BitmapMetadata>(decoder.Frames[1].Metadata);
            Assert.Equal((ushort)10, first.GetQuery("/grctlext/Delay"));
            Assert.Equal((ushort)15, second.GetQuery("/grctlext/Delay"));
        }
        finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    }

    [Fact]
    public void CreateFramePlan_CapsFramesWhilePreservingFullDuration()
    {
        var plan=GifExportService.CreateFramePlan(TimeSpan.FromSeconds(60),15);

        Assert.Equal(GifExportService.MaximumFrameCount,plan.Timestamps.Count);
        Assert.Equal(plan.Timestamps.Count,plan.Delays.Count);
        Assert.Equal(6000,plan.Delays.Sum(delay=>(int)delay));
        Assert.Equal(4,plan.EffectiveFps,3);
        Assert.True(plan.Timestamps.Zip(plan.Timestamps.Skip(1)).All(pair=>pair.First<pair.Second));
    }

    [Fact]
    public void CreateFramePlan_UsesRequestedRateForShortVideos()
    {
        var plan=GifExportService.CreateFramePlan(TimeSpan.FromSeconds(2),10);

        Assert.Equal(20,plan.Timestamps.Count);
        Assert.Equal(10,plan.EffectiveFps,3);
        Assert.All(plan.Delays,delay=>Assert.Equal((ushort)10,delay));
    }

    [Fact]
    public async Task Export_RejectsOverwritingSourceVideoBeforeDecodingIt()
    {
        var root=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));
        var source=Path.Combine(root,"clip.mp4");
        Directory.CreateDirectory(root);
        try
        {
            await File.WriteAllBytesAsync(source,[1,2,3,4],TestContext.Current.CancellationToken);
            var originalHash=SHA256.HashData(await File.ReadAllBytesAsync(source,TestContext.Current.CancellationToken));

            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>GifExportService.ExportFromVideoAsync(source,Path.Combine(root,".","clip.mp4"),5,TestContext.Current.CancellationToken));

            Assert.Contains("不能覆盖源 MP4",error.Message);
            Assert.Equal(originalHash,SHA256.HashData(await File.ReadAllBytesAsync(source,TestContext.Current.CancellationToken)));
        }
        finally{if(Directory.Exists(root))Directory.Delete(root,true);}
    }

    [Fact]
    public void ThumbnailPlan_BoundsWorstCaseDecodedPixelsWithoutDroppingFrames()
    {
        var frames=GifExportService.MaximumFrameCount;

        var size=GifExportService.CalculateThumbnailSize(3840,2160,frames);

        Assert.True((long)size.Width*size.Height*frames<=GifExportService.MaximumDecodedPixelBudget);
        Assert.True(size.Width<GifExportService.MaximumDimension);
        Assert.Equal(16d/9,size.Width/(double)size.Height,2);
    }

    [Fact]
    public void ThumbnailPlan_KeepsNormalShortGifAtTheQualityCap()
    {
        var size=GifExportService.CalculateThumbnailSize(1920,1080,20);

        Assert.Equal(720,size.Width);
        Assert.Equal(405,size.Height);
        Assert.True((long)size.Width*size.Height*20<=GifExportService.MaximumDecodedPixelBudget);
    }

    private static BitmapSource CreateBitmap(Color color)
    {
        var pixels = Enumerable.Repeat(new[] { color.B, color.G, color.R, color.A }, 16).SelectMany(x => x).ToArray();
        var bitmap = BitmapSource.Create(4, 4, 96, 96, PixelFormats.Bgra32, null, pixels, 16);
        bitmap.Freeze();
        return bitmap;
    }
}
