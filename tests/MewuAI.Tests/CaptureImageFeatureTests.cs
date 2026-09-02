using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class CaptureImageFeatureTests
{
    [Fact]
    public void PinnedImageRotationSwapsDimensionsAndIsReversible()
    {
        var source=Frame(30,20,0);var rotated=PinnedImageTransform.RotateQuarterTurns(source,1);Assert.Equal(20,rotated.PixelWidth);Assert.Equal(30,rotated.PixelHeight);var restored=PinnedImageTransform.RotateQuarterTurns(source,4);Assert.Same(source,restored);
    }

    [Fact]
    public void PixelationAveragesEveryMosaicBlock()
    {
        var pixels=new byte[4*4*4];for(var y=0;y<4;y++)for(var x=0;x<4;x++){var offset=(y*4+x)*4;pixels[offset]=(byte)(x*50+y*7);pixels[offset+3]=255;}var source=BitmapSource.Create(4,4,96,96,PixelFormats.Bgra32,null,pixels,16);var result=ImagePixelationService.Pixelate(source,new Int32Rect(0,0,4,4),2);var output=new byte[64];result.CopyPixels(output,16,0);Assert.Equal(output[0],output[4]);Assert.Equal(output[0],output[16]);Assert.NotEqual(output[0],output[8]);
    }

    [Fact]
    public void ScrollingFramesDetectOverlapAndComposeNovelRows()
    {
        var first=Frame(64,100,0);var second=Frame(64,100,40);Assert.Equal(40,ScrollingCaptureComposer.EstimateVerticalShift(first,second));var result=ScrollingCaptureComposer.Compose([first,second]);Assert.Equal(64,result.PixelWidth);Assert.Equal(140,result.PixelHeight);
    }

    [Fact]
    public void ScrollingCaptureStopsWhenThePageNoLongerMoves()
    {
        var frame=Frame(64,100,0);Assert.Equal(0,ScrollingCaptureComposer.EstimateVerticalShift(frame,frame));Assert.Equal(100,ScrollingCaptureComposer.Compose([frame,frame]).PixelHeight);
    }

    [Fact]
    public void ResizeHandleSnapsOnlyTheActiveEdges()
    {
        var result=SelectionSnapPolicy.SnapResize(new Rect(98,80,202,220),"NW",new Rect(100,100,400,300),5);Assert.Equal(100,result.Left);Assert.Equal(80,result.Top);Assert.Equal(300,result.Right);
    }

    private static BitmapSource Frame(int width,int height,int globalTop)
    {
        var stride=width*4;var pixels=new byte[stride*height];for(var y=0;y<height;y++)for(var x=0;x<width;x++){var globalY=y+globalTop;var offset=y*stride+x*4;pixels[offset]=(byte)((globalY*17+x*3)%251);pixels[offset+1]=(byte)((globalY*7+x*11)%253);pixels[offset+2]=(byte)((globalY*13+x*5)%247);pixels[offset+3]=255;}var result=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,stride);result.Freeze();return result;
    }
}
