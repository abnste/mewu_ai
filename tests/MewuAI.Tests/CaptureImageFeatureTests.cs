using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Drawing;
using DrawingColor=System.Drawing.Color;
using DrawingPixelFormat=System.Drawing.Imaging.PixelFormat;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

public sealed class CaptureImageFeatureTests
{
    [Fact]
    public void LongCaptureQueuePreservesFramesAndDirectionWhileConsumerIsBusy()
    {
        var buffer=new LongCaptureSampleBuffer();
        for(var index=0;index<8;index++)Assert.True(buffer.TryEnqueue(new(Frame(32,40,index),null,index<4?1:-1,false)));
        Assert.False(buffer.TryEnqueue(new(Frame(32,40,9),null,1,false)));
        for(var index=0;index<8;index++)
        {
            Assert.True(buffer.TryDequeue(out var sample));
            Assert.Equal(index<4?1:-1,sample.Direction);
            var actual=new byte[32*40*4];var expected=new byte[actual.Length];
            sample.Image.CopyPixels(actual,32*4,0);Frame(32,40,index).CopyPixels(expected,32*4,0);
            Assert.Equal(expected,actual);
        }
        Assert.False(buffer.TryDequeue(out _));
    }

    [Fact]
    public void LongCaptureQueueClearReleasesCapacityAndRejectsOversize()
    {
        var buffer=new LongCaptureSampleBuffer();
        Assert.False(buffer.HasCapacity(8000,8000));
        for(var i=0;i<8;i++)Assert.True(buffer.TryEnqueue(new(Frame(32,40,i),null,1,false)));
        buffer.Clear();Assert.Equal(0,buffer.Count);Assert.True(buffer.HasCapacity(32,40));
    }

    [Fact]
    public void LongCaptureBudgetIncludesBothDirectionsWithoutCountingRetracedAreaTwice()
    {
        Assert.True(ScrollingCaptureComposer.FitsOutputBudget(8000,5000,[4000,-4000]));
        Assert.False(ScrollingCaptureComposer.FitsOutputBudget(8000,5000,[4000,-4000,-2000]));
        Assert.False(ScrollingCaptureComposer.FitsOutputBudget(8000,5000,[int.MinValue]));
    }

    [Fact]
    public void ExportNamesUseReadableLocalTimestamp()
    {
        var timestamp=new DateTime(2026,9,3,18,42,7,DateTimeKind.Local);
        Assert.Equal("截图_20260903_184207",ExportFileNameService.Screenshot(timestamp));
        Assert.Equal("录屏_20260903_184207",ExportFileNameService.Recording(timestamp));
    }

    [Fact]
    public void SemanticButtonWinsOverSmallerFocusableTextChild()
    {
        var text=new WindowSnapTarget(new IntPtr(1),new ScreenRect(120,110,42,18),1);
        var button=new WindowSnapTarget(new IntPtr(1),new ScreenRect(100,100,100,40),2);
        Assert.Equal(button,NativeWindowSnapService.PreferTarget(text,button));
        Assert.Equal(button,NativeWindowSnapService.PreferTarget(button,text));
    }

    [Fact]
    public void ConfirmedButtonPreviewStaysStableAcrossItsBackground()
    {
        var stable=new Rect(100,100,100,40);
        var fullWindow=new Rect(0,0,1920,1080);
        Assert.Equal(stable,SelectionSnapPolicy.PreferStablePreview(new System.Windows.Point(185,120),stable,fullWindow));
        Assert.Equal(fullWindow,SelectionSnapPolicy.PreferStablePreview(new System.Windows.Point(240,120),stable,fullWindow));
    }

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
    public void PixelationKeepsFullFrameAndLeavesOutsideRegionUntouched()
    {
        var pixels=new byte[6*4*4];for(var y=0;y<4;y++)for(var x=0;x<6;x++){var offset=(y*6+x)*4;pixels[offset]=(byte)(x*31+y*7);pixels[offset+1]=(byte)(x*17+y*11);pixels[offset+2]=(byte)(x*13+y*19);pixels[offset+3]=255;}
        var source=BitmapSource.Create(6,4,96,96,PixelFormats.Bgra32,null,pixels,24);var result=ImagePixelationService.Pixelate(source,new Int32Rect(2,1,2,2),2);var output=new byte[pixels.Length];result.CopyPixels(output,24,0);
        Assert.Equal(6,result.PixelWidth);Assert.Equal(4,result.PixelHeight);Assert.Equal(pixels[(0*6+0)*4],output[(0*6+0)*4]);Assert.Equal(pixels[(3*6+5)*4+1],output[(3*6+5)*4+1]);Assert.Equal(output[(1*6+2)*4],output[(1*6+3)*4]);Assert.Equal(output[(2*6+2)*4+1],output[(2*6+3)*4+1]);
    }

    [Fact]
    public void MultipleMosaicsShareOneCleanSourcePass()
    {
        var pixels=new byte[8*4*4];for(var index=0;index<pixels.Length;index+=4){pixels[index]=(byte)(index/4*7);pixels[index+1]=(byte)(index/4*3);pixels[index+3]=255;}var source=BitmapSource.Create(8,4,96,96,PixelFormats.Bgra32,null,pixels,32);
        var result=ImagePixelationService.PixelateMany(source,[new Int32Rect(0,0,2,2),new Int32Rect(6,2,2,2)],2);var output=new byte[pixels.Length];result.CopyPixels(output,32,0);
        Assert.Equal(output[0],output[4]);Assert.Equal(output[(2*8+6)*4],output[(2*8+7)*4]);Assert.Equal(pixels[(1*8+4)*4],output[(1*8+4)*4]);
    }

    [Fact]
    public void ScrollingFramesDetectOverlapAndComposeNovelRows()
    {
        var first=Frame(64,100,0);var second=Frame(64,100,40);Assert.Equal(40,ScrollingCaptureComposer.EstimateVerticalShift(first,second));var result=ScrollingCaptureComposer.Compose([first,second]);Assert.Equal(64,result.PixelWidth);Assert.Equal(140,result.PixelHeight);
    }

    [Fact]
    public void ScrollingFramesSupportUpwardCaptureAndPrependNovelRows()
    {
        var first=Frame(64,100,80);var second=Frame(64,100,0);Assert.Equal(-80,ScrollingCaptureComposer.EstimateVerticalShift(first,second));var result=ScrollingCaptureComposer.Compose([first,second]);Assert.Equal(64,result.PixelWidth);Assert.Equal(180,result.PixelHeight);AssertPixelEquals(second,0,result,0);AssertPixelEquals(first,0,result,80);
    }

    [Fact]
    public void ReversingScrollDirectionDoesNotDuplicatePreviouslyCapturedRows()
    {
        var first=Frame(64,100,40);var second=Frame(64,100,80);var third=Frame(64,100,40);var result=ScrollingCaptureComposer.Compose([first,second,third],[40,-40]);Assert.Equal(140,result.PixelHeight);
    }

    [Theory]
    [InlineData(1)]
    [InlineData(-1)]
    public void LargeScrollKeepsNarrowButValidOverlap(int direction)
    {
        var first=Frame(512,720,direction<0?630:0);var second=Frame(512,720,direction<0?0:630);
        Assert.Equal(direction*630,ScrollingCaptureComposer.EstimateVerticalShift(first,second,out _,null,direction));
    }

    [Theory]
    [InlineData(7)]
    [InlineData(31)]
    [InlineData(147)]
    [InlineData(289)]
    public void SparseCandidateSearchStillResolvesExactPixelDisplacements(int shift)
    {
        Assert.Equal(shift,ScrollingCaptureComposer.EstimateVerticalShift(Frame(512,720,0),Frame(512,720,shift)));
    }

    [Theory]
    [InlineData(163)]
    [InlineData(506)]
    [InlineData(630)]
    public void RepeatedCardLayoutsUseColorToDisambiguateTheScroll(int shift)
    {
        var first=ColoredCardsFrame(720);var second=ColoredCardsFrame(720+shift);
        Assert.Equal(shift,ScrollingCaptureComposer.EstimateVerticalShift(first,second,out _,null,1));
        Assert.Equal(-shift,ScrollingCaptureComposer.EstimateVerticalShift(second,first,out _,null,-1));
    }

    private static BitmapSource ColoredCardsFrame(int top)
    {
        const int width=512,height=720;var pixels=new byte[width*height*4];
        for(var y=0;y<height;y++)for(var x=0;x<width;x++)
        {
            var card=(y+top)/137;var row=(y+top)%137;var p=(y*width+x)*4;
            byte red=245,green=245,blue=245;
            if(row is >28 and <95&&x is >20 and <390)
            {
                red=(byte)(45+card*73%170);green=(byte)(45+card*47%170);blue=(byte)(45+card*109%170);
            }
            else if(row is >10 and <17&&x is >20 and <180)red=green=blue=20;
            pixels[p]=blue;pixels[p+1]=green;pixels[p+2]=red;pixels[p+3]=255;
        }
        var image=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,width*4);image.Freeze();return image;
    }

    [Fact]
    public void ScrollingCaptureStopsWhenThePageNoLongerMoves()
    {
        var frame=Frame(64,100,0);Assert.Equal(0,ScrollingCaptureComposer.EstimateVerticalShift(frame,frame));Assert.Equal(100,ScrollingCaptureComposer.Compose([frame,frame]).PixelHeight);
    }

    [Fact]
    public void SparseDocumentTextIsNotHiddenByBlankBackground()
    {
        var first=SparseDocumentFrame(960,520,0);var second=SparseDocumentFrame(960,520,120);
        Assert.Equal(120,ScrollingCaptureComposer.EstimateVerticalShift(first,second));
        Assert.Equal(640,ScrollingCaptureComposer.Compose([first,second]).PixelHeight);
    }

    [Fact]
    public void TransientPointerAreaDoesNotCreateDuplicateScrollingFrame()
    {
        var ignored=new Int32Rect(380,180,200,160);var first=SparseDocumentFrame(960,520,0);var second=SparseDocumentFrame(960,520,0,ignored,96);
        Assert.Equal(0,ScrollingCaptureComposer.EstimateVerticalShift(first,second,out _,ignored));
    }

    [Fact]
    public void TransientPointerAreaDoesNotHideRealDocumentScroll()
    {
        var ignored=new Int32Rect(380,180,200,160);var first=SparseDocumentFrame(960,520,0);var second=SparseDocumentFrame(960,520,120,ignored,96);
        Assert.Equal(120,ScrollingCaptureComposer.EstimateVerticalShift(first,second,out _,ignored,1));
    }

    [Theory]
    [InlineData(120)]
    [InlineData(-120)]
    [InlineData(720)]
    [InlineData(-720)]
    public void ForwardedWheelPreservesSignedDelta(int delta)
    {
        var packed=MouseWheelInputService.PackWheelWParam(delta).ToInt64();
        Assert.Equal((short)delta,unchecked((short)((ulong)packed>>16)));
    }

    [Theory]
    [InlineData(0x00780000u,120)]
    [InlineData(0xFF880000u,-120)]
    public void LowLevelMouseWheelMonitorDecodesSignedWheelDelta(uint mouseData,int expected)
    {
        Assert.Equal(expected,LowLevelMouseWheelMonitor.DecodeWheelDelta(mouseData));
    }

    [Fact]
    public void LowLevelMouseWheelMonitorDistinguishesPhysicalAndInjectedInput()
    {
        Assert.False(LowLevelMouseWheelMonitor.IsInjected(0));
        Assert.True(LowLevelMouseWheelMonitor.IsInjected(1));
    }

    [Fact]
    public void ExternalInjectedWheelsAreObservedButOwnForwardingIsNot()
    {
        Assert.False(LowLevelMouseWheelMonitor.IsOwnForwardedInput(1,UIntPtr.Zero));
        Assert.False(LowLevelMouseWheelMonitor.IsOwnForwardedInput(1,new UIntPtr(1234)));
        Assert.True(LowLevelMouseWheelMonitor.IsOwnForwardedInput(1,new UIntPtr(MouseWheelInputService.ForwardedWheelMarker)));
        Assert.False(LowLevelMouseWheelMonitor.IsOwnForwardedInput(0,new UIntPtr(MouseWheelInputService.ForwardedWheelMarker)));
    }

    [Fact]
    public void CaptureRegionKeepsRequestedPhysicalPixelSize()
    {
        var desktop=System.Windows.Forms.SystemInformation.VirtualScreen;
        var frame=new ScreenCaptureService().CaptureRegion(new ScreenRect(desktop.Left+8,desktop.Top+8,160,120));
        Assert.Equal(160,frame.PixelWidth);Assert.Equal(120,frame.PixelHeight);Assert.True(frame.IsFrozen);
    }

    [Fact]
    public void CaptureRegionRejectsEmptyOrOffscreenRectangles()
    {
        var capture=new ScreenCaptureService();
        Assert.Throws<ArgumentOutOfRangeException>(()=>capture.CaptureRegion(default));
        Assert.Throws<ArgumentOutOfRangeException>(()=>capture.CaptureRegion(new ScreenRect(int.MinValue,0,20,20)));
        Assert.Throws<ArgumentOutOfRangeException>(()=>capture.CaptureRegion(new ScreenRect(0,0,10000,10000)));
    }

    [Theory]
    [InlineData(640,480)]
    [InlineData(-1920,240)]
    [InlineData(320,-1080)]
    public void ForwardedWheelPreservesSignedVirtualDesktopCoordinates(int x,int y)
    {
        var packed=MouseWheelInputService.PackScreenPointLParam(x,y).ToInt64();
        Assert.Equal((short)x,unchecked((short)packed));
        Assert.Equal((short)y,unchecked((short)((ulong)packed>>16)));
    }

    [Fact]
    public void ResizeHandleSnapsOnlyTheActiveEdges()
    {
        var result=SelectionSnapPolicy.SnapResize(new Rect(98,80,202,220),"NW",new Rect(100,100,400,300),5);Assert.Equal(100,result.Left);Assert.Equal(80,result.Top);Assert.Equal(300,result.Right);
    }

    [Fact]
    public void ExplicitScreenshotCompositesVisiblePinnedImageBackIntoFrame()
    {
        var source=BitmapSource.Create(2,2,96,96,PixelFormats.Bgra32,null,new byte[]
        {
            0,0,255,255,0,0,255,255,
            0,0,255,255,0,0,255,255
        },8);source.Freeze();
        using var registration=PinnedImageCaptureRegistry.Register(()=>new PinnedImageCaptureSnapshot(new ScreenRect(1,1,2,2),source,1));
        using var destination=new Bitmap(4,4,DrawingPixelFormat.Format32bppPArgb);
        using(var graphics=Graphics.FromImage(destination))graphics.Clear(DrawingColor.Black);
        PinnedImageCaptureRegistry.CompositeInto(destination,new ScreenRect(0,0,4,4));
        var pixel=destination.GetPixel(1,1);
        Assert.True(pixel.R>200);Assert.True(pixel.G<20);Assert.True(pixel.B<20);
    }

    [Fact]
    public void NewerPinnedImageIsCompositedAboveOlderImage()
    {
        var older=BitmapSource.Create(1,1,96,96,PixelFormats.Bgra32,null,new byte[]{255,0,0,255},4);older.Freeze();
        var newer=BitmapSource.Create(1,1,96,96,PixelFormats.Bgra32,null,new byte[]{0,255,0,255},4);newer.Freeze();
        using var first=PinnedImageCaptureRegistry.Register(()=>new PinnedImageCaptureSnapshot(new ScreenRect(0,0,1,1),older,1));
        using var second=PinnedImageCaptureRegistry.Register(()=>new PinnedImageCaptureSnapshot(new ScreenRect(0,0,1,1),newer,1));
        using var destination=new Bitmap(1,1,DrawingPixelFormat.Format32bppPArgb);
        PinnedImageCaptureRegistry.CompositeInto(destination,new ScreenRect(0,0,1,1));
        var pixel=destination.GetPixel(0,0);
        Assert.True(pixel.G>200);Assert.True(pixel.R<20);
    }

    private static BitmapSource Frame(int width,int height,int globalTop)
    {
        var stride=width*4;var pixels=new byte[stride*height];for(var y=0;y<height;y++)for(var x=0;x<width;x++){var globalY=y+globalTop;var offset=y*stride+x*4;pixels[offset]=(byte)((globalY*17+x*3)%251);pixels[offset+1]=(byte)((globalY*7+x*11)%253);pixels[offset+2]=(byte)((globalY*13+x*5)%247);pixels[offset+3]=255;}var result=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,stride);result.Freeze();return result;
    }

    private static void AssertPixelEquals(BitmapSource expected,int expectedY,BitmapSource actual,int actualY)
    {
        var expectedPixel=new byte[4];var actualPixel=new byte[4];expected.CopyPixels(new Int32Rect(0,expectedY,1,1),expectedPixel,4,0);actual.CopyPixels(new Int32Rect(0,actualY,1,1),actualPixel,4,0);Assert.Equal(expectedPixel,actualPixel);
    }

    private static BitmapSource SparseDocumentFrame(int width,int height,int globalTop,Int32Rect? patch=null,byte patchValue=0)
    {
        var stride=width*4;var pixels=new byte[stride*height];
        for(var y=0;y<height;y++)for(var x=0;x<width;x++)
        {
            var globalY=y+globalTop;var line=globalY/20;var withinLine=globalY%20;var text=x<240&&withinLine is >=4 and <=13&&((x/3+line*7)%19)<8;var inPatch=patch is { } area&&x>=area.X&&x<area.X+area.Width&&y>=area.Y&&y<area.Y+area.Height;var value=inPatch?patchValue:(byte)(text?24:250);var offset=y*stride+x*4;
            pixels[offset]=pixels[offset+1]=pixels[offset+2]=value;pixels[offset+3]=255;
        }
        var result=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,stride);result.Freeze();return result;
    }
}
