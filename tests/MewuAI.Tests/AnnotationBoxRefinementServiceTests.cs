using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class AnnotationBoxRefinementServiceTests
{
    [Fact]
    public void SnapsCoarseBoxToStrongRectangularEdges()
    {
        const int width=120,height=100,stride=width*4;var pixels=new byte[stride*height];
        for(var y=25;y<75;y++)for(var x=30;x<90;x++){var offset=y*stride+x*4;pixels[offset]=pixels[offset+1]=pixels[offset+2]=220;pixels[offset+3]=255;}
        var coarse=new AiAnnotation(25d/width,20d/height,70d/width,60d/height,"目标");
        var refined=AnnotationBoxRefinementService.RefineBgra32(pixels,width,height,stride,coarse);
        Assert.InRange(refined.X*width,29,30);Assert.InRange(refined.Y*height,24,25);
        Assert.InRange(refined.Width*width,59,61);Assert.InRange(refined.Height*height,49,51);
    }

    [Fact]
    public void LeavesFlatImageAndVideoTimelineUnchanged()
    {
        const int width=80,height=60,stride=width*4;var pixels=new byte[stride*height];
        var image=new AiAnnotation(.2,.2,.4,.4,"目标");
        Assert.Equal(image,AnnotationBoxRefinementService.RefineBgra32(pixels,width,height,stride,image));
        var video=image with{StartTime=1,EndTime=1,Keyframes=[new(1,.2,.2,.4,.4)]};
        Assert.Equal(video,AnnotationBoxRefinementService.RefineBgra32(pixels,width,height,stride,video));
    }

    [Fact]
    public void DoesNotJumpToAWeakNeighbouringGlyphEdge()
    {
        const int width=160,height=90,stride=width*4;var pixels=new byte[stride*height];
        // The coarse box already has its left/right control borders.  A short
        // high-contrast glyph inside the search range must not move it.
        for(var y=20;y<70;y++)for(var x=40;x<120;x++){var offset=y*stride+x*4;pixels[offset]=pixels[offset+1]=pixels[offset+2]=220;pixels[offset+3]=255;}
        for(var y=38;y<52;y++){var offset=y*stride+48*4;pixels[offset]=pixels[offset+1]=pixels[offset+2]=0;pixels[offset+3]=255;}
        var coarse=new AiAnnotation(40d/width,20d/height,80d/width,50d/height,"输入框",Kind:AiAnnotationKind.Callout);
        var refined=AnnotationBoxRefinementService.RefineBgra32(pixels,width,height,stride,coarse);
        Assert.InRange(refined.X*width,39,41);Assert.InRange(refined.Width*width,79,81);
    }
}
