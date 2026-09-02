using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Conservatively snaps a model-provided coarse image box to strong local edges.
/// Both opposing sides of an axis must be independently convincing, so textured
/// content inside an object cannot pull just one side of the box out of shape.
/// </summary>
internal static class AnnotationBoxRefinementService
{
    private const byte EdgeCoverageThreshold=24;

    internal static AiAnnotation Refine(BitmapSource source,AiAnnotation annotation)
    {
        if(annotation.IsVideoTimeline||source.PixelWidth<12||source.PixelHeight<12)return annotation;
        var converted=source.Format==PixelFormats.Bgra32?source:new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);
        var stride=checked(converted.PixelWidth*4);var pixels=new byte[checked(stride*converted.PixelHeight)];
        converted.CopyPixels(pixels,stride,0);
        return RefineBgra32(pixels,converted.PixelWidth,converted.PixelHeight,stride,annotation);
    }

    internal static AiAnnotation RefineBgra32(byte[] pixels,int width,int height,int stride,AiAnnotation annotation)
    {
        if(annotation.IsVideoTimeline||width<12||height<12||stride<width*4||pixels.Length<stride*height)return annotation;
        var left=Math.Clamp((int)Math.Round(annotation.X*width),1,width-2);
        var top=Math.Clamp((int)Math.Round(annotation.Y*height),1,height-2);
        var right=Math.Clamp((int)Math.Round((annotation.X+annotation.Width)*width),left+1,width-2);
        var bottom=Math.Clamp((int)Math.Round((annotation.Y+annotation.Height)*height),top+1,height-2);
        var originalWidth=right-left;var originalHeight=bottom-top;
        if(originalWidth<6||originalHeight<6)return annotation;

        var horizontalRadius=Math.Clamp((int)Math.Round(originalWidth*.14),3,36);
        var verticalRadius=Math.Clamp((int)Math.Round(originalHeight*.14),3,36);
        var refinedLeft=FindVerticalEdge(pixels,width,height,stride,left,horizontalRadius,top,bottom);
        var refinedRight=FindVerticalEdge(pixels,width,height,stride,right,horizontalRadius,top,bottom);
        var refinedTop=FindHorizontalEdge(pixels,width,height,stride,top,verticalRadius,left,right);
        var refinedBottom=FindHorizontalEdge(pixels,width,height,stride,bottom,verticalRadius,left,right);

        var newLeft=left;var newRight=right;var newTop=top;var newBottom=bottom;
        if(refinedLeft.HasValue&&refinedRight.HasValue&&IsPlausibleSpan(refinedRight.Value-refinedLeft.Value,originalWidth))
        {newLeft=refinedLeft.Value;newRight=refinedRight.Value;}
        if(refinedTop.HasValue&&refinedBottom.HasValue&&IsPlausibleSpan(refinedBottom.Value-refinedTop.Value,originalHeight))
        {newTop=refinedTop.Value;newBottom=refinedBottom.Value;}
        if(newLeft==left&&newRight==right&&newTop==top&&newBottom==bottom)return annotation;

        return annotation with
        {
            X=(double)newLeft/width,
            Y=(double)newTop/height,
            Width=(double)(newRight-newLeft)/width,
            Height=(double)(newBottom-newTop)/height
        };
    }

    private static bool IsPlausibleSpan(int candidate,int original)=>candidate>=6&&candidate>=original*.72&&candidate<=original*1.28;

    private static int? FindVerticalEdge(byte[] pixels,int width,int height,int stride,int center,int radius,int top,int bottom)
    {
        var from=Math.Max(1,center-radius);var to=Math.Min(width-2,center+radius);
        var inset=Math.Max(1,(bottom-top)/12);var sampleTop=Math.Clamp(top+inset,1,height-2);var sampleBottom=Math.Clamp(bottom-inset,sampleTop+1,height-1);
        return FindConfidentEdge(from,to,x=>ScoreVertical(pixels,stride,x,sampleTop,sampleBottom));
    }

    private static int? FindHorizontalEdge(byte[] pixels,int width,int height,int stride,int center,int radius,int left,int right)
    {
        var from=Math.Max(1,center-radius);var to=Math.Min(height-2,center+radius);
        var inset=Math.Max(1,(right-left)/12);var sampleLeft=Math.Clamp(left+inset,1,width-2);var sampleRight=Math.Clamp(right-inset,sampleLeft+1,width-1);
        return FindConfidentEdge(from,to,y=>ScoreHorizontal(pixels,stride,y,sampleLeft,sampleRight));
    }

    private static int? FindConfidentEdge(int from,int to,Func<int,(double Average,double Coverage)> score)
    {
        if(to<from)return null;
        var candidates=new List<(int Position,double Average,double Coverage)>();
        for(var position=from;position<=to;position++)
        {
            var value=score(position);candidates.Add((position,value.Average,value.Coverage));
        }
        var best=candidates.OrderByDescending(item=>item.Average).ThenByDescending(item=>item.Coverage).First();
        var averages=candidates.Select(item=>item.Average).Order().ToArray();var median=averages[averages.Length/2];
        var mean=averages.Average();var deviation=Math.Sqrt(averages.Sum(value=>(value-mean)*(value-mean))/averages.Length);
        return best.Coverage>=.28&&best.Average>=median+10&&best.Average>=mean+deviation*1.15?best.Position:null;
    }

    private static (double Average,double Coverage) ScoreVertical(byte[] pixels,int stride,int x,int top,int bottom)
    {
        long total=0;var covered=0;var count=Math.Max(1,bottom-top);
        for(var y=top;y<bottom;y++){var value=Math.Abs(Luma(pixels,stride,x+1,y)-Luma(pixels,stride,x-1,y));total+=value;if(value>=EdgeCoverageThreshold)covered++;}
        return ((double)total/count,(double)covered/count);
    }

    private static (double Average,double Coverage) ScoreHorizontal(byte[] pixels,int stride,int y,int left,int right)
    {
        long total=0;var covered=0;var count=Math.Max(1,right-left);
        for(var x=left;x<right;x++){var value=Math.Abs(Luma(pixels,stride,x,y+1)-Luma(pixels,stride,x,y-1));total+=value;if(value>=EdgeCoverageThreshold)covered++;}
        return ((double)total/count,(double)covered/count);
    }

    private static int Luma(byte[] pixels,int stride,int x,int y)
    {
        var offset=y*stride+x*4;return (pixels[offset]*29+pixels[offset+1]*150+pixels[offset+2]*77)>>8;
    }
}
