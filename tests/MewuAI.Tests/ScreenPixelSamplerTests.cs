using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ScreenPixelSamplerTests
{
    [Fact]
    public void SamplesExactRgbFromFrozenDesktopBgr32Buffer()
    {
        var pixels=new byte[]{0x33,0x22,0x11,0,0xCC,0xBB,0xAA,0};
        var source=BitmapSource.Create(2,1,96,96,PixelFormats.Bgr32,null,pixels,8);
        Assert.True(ScreenPixelSampler.TrySample(source,1,0,out var color));
        Assert.Equal(Color.FromRgb(0xAA,0xBB,0xCC),color);
    }

    [Theory]
    [InlineData(-1,0)]
    [InlineData(2,0)]
    [InlineData(0,1)]
    public void RejectsPixelsOutsideCapturedFrame(int x,int y)
    {
        var source=BitmapSource.Create(2,1,96,96,PixelFormats.Bgr32,null,new byte[8],8);
        Assert.False(ScreenPixelSampler.TrySample(source,x,y,out _));
    }
}
