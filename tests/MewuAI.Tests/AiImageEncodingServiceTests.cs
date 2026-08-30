using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class AiImageEncodingServiceTests
{
    [Fact]
    public void HighEntropyScreenIsBoundedWithAnAcceptedReadableFallback()
    {
        const int width=2048,height=1152;var pixels=new byte[width*height*4];var random=new Random(1977);random.NextBytes(pixels);for(var index=3;index<pixels.Length;index+=4)pixels[index]=255;
        var image=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,width*4);image.Freeze();
        var result=AiImageEncodingService.Encode(image,700_000,new HashSet<string>(StringComparer.OrdinalIgnoreCase){"image/png","image/jpeg"},TestContext.Current.CancellationToken);
        Assert.InRange(result.Data.LongLength,1,700_000);Assert.Equal("image/jpeg",result.MimeType);Assert.InRange(result.PixelWidth,960,width);Assert.InRange(result.PixelHeight,1,height);
    }

    [Fact]
    public void SimpleScreenKeepsLosslessPngAtOriginalResolution()
    {
        const int width=320,height=180;var pixels=Enumerable.Repeat((byte)248,width*height*4).ToArray();var image=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,width*4);image.Freeze();
        var result=AiImageEncodingService.Encode(image,1_000_000,new HashSet<string>(StringComparer.OrdinalIgnoreCase){"image/png","image/jpeg"},TestContext.Current.CancellationToken);
        Assert.Equal("image/png",result.MimeType);Assert.Equal(width,result.PixelWidth);Assert.Equal(height,result.PixelHeight);
    }

    [Fact]
    public void SuccessfulJpegPayloadIsNotClearedByCandidateCleanup()
    {
        var pixels=new byte[32*16*4];
        for(var index=0;index<pixels.Length;index+=4)
        {
            pixels[index]=20;pixels[index+1]=80;pixels[index+2]=220;pixels[index+3]=255;
        }
        var image=BitmapSource.Create(32,16,96,96,PixelFormats.Bgra32,null,pixels,32*4);
        image.Freeze();

        var result=AiImageEncodingService.Encode(
            image,
            1_000_000,
            new HashSet<string>(StringComparer.OrdinalIgnoreCase){"image/jpeg"},
            TestContext.Current.CancellationToken);

        Assert.Equal("image/jpeg",result.MimeType);
        Assert.Contains(result.Data,value=>value!=0);
        using var stream=new MemoryStream(result.Data,false);
        var decoder=new JpegBitmapDecoder(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);
        Assert.Equal(32,decoder.Frames[0].PixelWidth);
        Assert.Equal(16,decoder.Frames[0].PixelHeight);
    }

    [Fact]
    public void CancellationStopsEncodingBeforeAllocatingCandidates()
    {
        var image=BitmapSource.Create(8,8,96,96,PixelFormats.Bgra32,null,new byte[8*8*4],8*4);image.Freeze();using var cancellation=new CancellationTokenSource();cancellation.Cancel();
        Assert.ThrowsAny<OperationCanceledException>(()=>AiImageEncodingService.Encode(image,1024,new HashSet<string>{"image/png"},cancellation.Token));
    }

    [Fact]
    public void SensitiveAttachmentBuffersAreZeroedWithoutChangingVideoPaths()
    {
        var imageData=new byte[]{1,2,3,4};var videoPath=Path.Combine(Path.GetTempPath(),"clip.mp4");
        var attachments=new[]{new AiAttachment(AiAttachmentType.Image,"image/png",imageData),new AiAttachment(AiAttachmentType.Video,"video/mp4",FilePath:videoPath)};

        AiImageEncodingService.ClearAttachmentBuffers(attachments);

        Assert.All(imageData,value=>Assert.Equal(0,value));Assert.Equal(videoPath,attachments[1].FilePath);Assert.Null(attachments[1].Data);
    }

    [Fact]
    public void BorrowedAttachmentBuffersAreNotZeroed()
    {
        var borrowed=new byte[]{4,3,2,1};
        AiImageEncodingService.ClearAttachmentBuffers([new AiAttachment(AiAttachmentType.Image,"image/png",borrowed,ProviderOwnsData:false)]);
        Assert.Equal(new byte[]{4,3,2,1},borrowed);
    }
}
