using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class ScrollingCaptureComposer
{
    internal const long MaxOutputPixels=80_000_000;
    internal static int EstimateVerticalShift(BitmapSource previous,BitmapSource current)
    {
        if(previous.PixelWidth!=current.PixelWidth||previous.PixelHeight!=current.PixelHeight)return 0;var width=previous.PixelWidth;var height=previous.PixelHeight;if(width<8||height<32)return 0;var a=Pixels(previous);var b=Pixels(current);var stride=width*4;var stepX=Math.Max(2,width/80);var stepY=Math.Max(2,height/100);if(AverageDifference(a,b,width,height,stride,stepX,stepY)<=2)return 0;double best=double.MaxValue;var bestShift=0;
        for(var shift=8;shift<=Math.Min(height-16,(int)(height*.88));shift+=4){long difference=0;long samples=0;var overlap=height-shift;var start=Math.Min(overlap/3,Math.Max(0,height/6));for(var y=start;y<overlap;y+=stepY)for(var x=width/20;x<width-width/20;x+=stepX){var first=(y+shift)*stride+x*4;var second=y*stride+x*4;difference+=Math.Abs(a[first]-b[second])+Math.Abs(a[first+1]-b[second+1])+Math.Abs(a[first+2]-b[second+2]);samples+=3;}if(samples==0)continue;var score=(double)difference/samples;if(score<best){best=score;bestShift=shift;}}
        return best<=22?bestShift:0;
    }
    private static double AverageDifference(byte[] first,byte[] second,int width,int height,int stride,int stepX,int stepY){long difference=0;long samples=0;for(var y=0;y<height;y+=stepY)for(var x=0;x<width;x+=stepX){var offset=y*stride+x*4;difference+=Math.Abs(first[offset]-second[offset])+Math.Abs(first[offset+1]-second[offset+1])+Math.Abs(first[offset+2]-second[offset+2]);samples+=3;}return samples==0?double.MaxValue:(double)difference/samples;}
    internal static BitmapSource Compose(IReadOnlyList<BitmapSource> frames)
    {
        ArgumentNullException.ThrowIfNull(frames);if(frames.Count==0)throw new ArgumentException("长截图至少需要一帧",nameof(frames));var width=frames[0].PixelWidth;var height=frames[0].PixelHeight;var shifts=new List<int>();long totalHeight=height;
        for(var index=1;index<frames.Count;index++){if(frames[index].PixelWidth!=width||frames[index].PixelHeight!=height)throw new ArgumentException("长截图帧尺寸不一致",nameof(frames));var shift=EstimateVerticalShift(frames[index-1],frames[index]);if(shift<=0)break;if(checked((totalHeight+shift)*width)>MaxOutputPixels)break;shifts.Add(shift);totalHeight+=shift;}
        var stride=checked(width*4);var output=new byte[checked(stride*(int)totalHeight)];var first=Pixels(frames[0]);Buffer.BlockCopy(first,0,output,0,first.Length);var destinationRow=height;
        for(var index=0;index<shifts.Count;index++){var shift=shifts[index];var pixels=Pixels(frames[index+1]);Buffer.BlockCopy(pixels,(height-shift)*stride,output,destinationRow*stride,shift*stride);destinationRow+=shift;}
        var result=BitmapSource.Create(width,destinationRow,frames[0].DpiX,frames[0].DpiY,PixelFormats.Bgra32,null,output,stride);result.Freeze();return result;
    }
    private static byte[] Pixels(BitmapSource source){var formatted=source.Format==PixelFormats.Bgra32?source:new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);var stride=formatted.PixelWidth*4;var result=new byte[stride*formatted.PixelHeight];formatted.CopyPixels(result,stride,0);return result;}
}
