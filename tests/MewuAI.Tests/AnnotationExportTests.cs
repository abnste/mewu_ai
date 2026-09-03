using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class AnnotationExportTests
{
    [Fact]
    public void UnifiedDrawingPrimitivesAndMosaicRenderIntoImage()
    {
        var source=SolidBitmap(200,120,Colors.White);var annotations=new AiAnnotation[]{
            new(.1,.1,.3,.25,"框",Kind:AiAnnotationKind.Rectangle,Style:new("#FF0000",.01,1,true)),
            new(.1,.55,.5,.001,"笔",Kind:AiAnnotationKind.Pen,Points:[new(.1,.55),new(.6,.7)],Style:new("#0055FF",.02)),
            new(.65,.1,.2,.3,"隐私",Kind:AiAnnotationKind.Mosaic)};
        var result=AnnotationOverlayRenderer.ApplyAiAnnotations(source,annotations);Assert.True(HasNonWhitePixel(result));
    }
    [Fact]
    public void VideoOverlayPlan_PointEventGetsReadableBoundedDuration()
    {
        var note=Timeline(1.25,1.25);var plan=VideoAnnotationOverlayPlan.Create([note],TimeSpan.FromSeconds(2));var frame=Assert.Single(plan);Assert.Equal(TimeSpan.FromSeconds(1.25),frame.Start);Assert.Equal(TimeSpan.FromMilliseconds(750),frame.Duration);
    }

    [Fact]
    public void VideoOverlayPlan_LongMotionIsBoundedAndCoversItsInterval()
    {
        var plan=VideoAnnotationOverlayPlan.Create([Timeline(2,80)],TimeSpan.FromSeconds(90),10,40);Assert.InRange(plan.Count,2,40);Assert.Equal(TimeSpan.FromSeconds(2),plan[0].Start);Assert.True(plan[^1].Start+plan[^1].Duration>=TimeSpan.FromSeconds(80)-TimeSpan.FromMilliseconds(2));for(var index=1;index<plan.Count;index++)Assert.True(plan[index-1].Start+plan[index-1].Duration>=plan[index].Start);
    }

    [Fact]
    public void AiOverlayAndCompositeContainRenderedPixels()
    {
        var note=new AiAnnotation(.1,.1,.25,.2,"按钮");var overlay=AnnotationOverlayRenderer.RenderAiOverlay(320,180,[note]);var source=SolidBitmap(320,180,Colors.White);var composite=AnnotationOverlayRenderer.Composite(source,overlay);Assert.True(HasNonWhitePixel(composite));
    }

    [Fact]
    public void CalloutRendersOneRedTargetOutlineAtItsProtocolGeometry()
    {
        var note=new AiAnnotation(.25,.30,.28,.18,"新对话",0,ReferenceHandle:"ref-1",Kind:AiAnnotationKind.Callout);
        var overlay=AnnotationOverlayRenderer.RenderAiOverlay(200,120,[note]);
        Assert.True(ContainsRedPixel(overlay,48,34,56,42));
    }

    [Fact]
    public void ExportUsesTheUsersDraggedCalloutPosition()
    {
        var note=new AiAnnotation(.7,.7,.15,.12,"右下目标",ReferenceHandle:"ref-1",Kind:AiAnnotationKind.Callout);var positions=new Dictionary<AiAnnotation,System.Windows.Point>{{note,new(.05,.05)}};
        var overlay=AnnotationOverlayRenderer.RenderAiOverlay(300,180,[note],null,positions);
        Assert.True(ContainsBluePixel(overlay,10,5,105,15));
    }

    [Fact]
    public void CrossSurvivesPostProcessingAndRendersBothDiagonalStrokes()
    {
        var style=new AiAnnotationStyle("#FF0000",.025);var down=new AiAnnotation(.2,.2,.4,.4,"错误",ReferenceHandle:"ref",Kind:AiAnnotationKind.Pen,Points:[new(.2,.2),new(.6,.6)],Style:style);var up=new AiAnnotation(.2,.2,.4,.4,"错误",ReferenceHandle:"ref",Kind:AiAnnotationKind.Pen,Points:[new(.2,.6),new(.6,.2)],Style:style);
        var notes=AnnotationPostProcessor.Process([down,up],false,out _);var overlay=AnnotationOverlayRenderer.RenderAiOverlay(200,200,notes);
        Assert.Equal(2,notes.Count);Assert.True(ContainsRedPixel(overlay,36,36,44,44));Assert.True(ContainsRedPixel(overlay,36,116,44,124));
    }

    private static AiAnnotation Timeline(double start,double end)=>new(.1,.2,.2,.2,"目标",0,start,end,[new VideoAnnotationKeyframe(start,.1,.2,.2,.2),new VideoAnnotationKeyframe(end,.2,.25,.2,.2)]);
    private static BitmapSource SolidBitmap(int width,int height,Color color){var pixels=new byte[width*height*4];for(var i=0;i<pixels.Length;i+=4){pixels[i]=color.B;pixels[i+1]=color.G;pixels[i+2]=color.R;pixels[i+3]=color.A;}var bitmap=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,width*4);bitmap.Freeze();return bitmap;}
    private static bool HasNonWhitePixel(BitmapSource image){var pixels=new byte[image.PixelWidth*image.PixelHeight*4];image.CopyPixels(pixels,image.PixelWidth*4,0);for(var i=0;i<pixels.Length;i+=4)if(pixels[i]<245||pixels[i+1]<245||pixels[i+2]<245)return true;return false;}
    private static bool ContainsRedPixel(BitmapSource image,int left,int top,int right,int bottom){var pixels=new byte[image.PixelWidth*image.PixelHeight*4];image.CopyPixels(pixels,image.PixelWidth*4,0);for(var y=top;y<=bottom;y++)for(var x=left;x<=right;x++){var offset=(y*image.PixelWidth+x)*4;if(pixels[offset+2]>180&&pixels[offset+1]<100&&pixels[offset]<100)return true;}return false;}
    private static bool ContainsBluePixel(BitmapSource image,int left,int top,int right,int bottom){var pixels=new byte[image.PixelWidth*image.PixelHeight*4];image.CopyPixels(pixels,image.PixelWidth*4,0);for(var y=top;y<=bottom;y++)for(var x=left;x<=right;x++){var offset=(y*image.PixelWidth+x)*4;if(pixels[offset]>150&&pixels[offset+1]>100&&pixels[offset+2]<150)return true;}return false;}
}
