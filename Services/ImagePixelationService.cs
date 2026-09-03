using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class ImagePixelationService
{
    internal static BitmapSource Pixelate(BitmapSource source,Int32Rect region,int blockSize=12)
    {
        ArgumentNullException.ThrowIfNull(source);
        if(region.Width<=0||region.Height<=0)throw new ArgumentOutOfRangeException(nameof(region));
        if(blockSize<2||blockSize>128)throw new ArgumentOutOfRangeException(nameof(blockSize));
        return PixelateMany(source,[region],blockSize);
    }

    internal static BitmapSource PixelateMany(BitmapSource source,IReadOnlyList<Int32Rect> regions,int blockSize=12)
    {
        ArgumentNullException.ThrowIfNull(source);ArgumentNullException.ThrowIfNull(regions);
        if(blockSize<2||blockSize>128)throw new ArgumentOutOfRangeException(nameof(blockSize));if(regions.Count==0)return source;
        var formatted=source.Format==PixelFormats.Bgra32?source:new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);
        if(formatted is Freezable freezable&&!freezable.IsFrozen)freezable.Freeze();
        var stride=checked(source.PixelWidth*4);var clean=new byte[checked(stride*source.PixelHeight)];formatted.CopyPixels(clean,stride,0);var output=(byte[])clean.Clone();
        foreach(var region in regions.Take(32))
        {
            if(region.Width<=0||region.Height<=0)continue;var left=Math.Clamp(region.X,0,source.PixelWidth-1);var top=Math.Clamp(region.Y,0,source.PixelHeight-1);var right=(int)Math.Clamp((long)region.X+region.Width,left+1,source.PixelWidth);var bottom=(int)Math.Clamp((long)region.Y+region.Height,top+1,source.PixelHeight);
            for(var blockTop=top;blockTop<bottom;blockTop+=blockSize)
            for(var blockLeft=left;blockLeft<right;blockLeft+=blockSize)
            {
                var blockRight=Math.Min(right,blockLeft+blockSize);var blockBottom=Math.Min(bottom,blockTop+blockSize);long blue=0,green=0,red=0,alpha=0;var count=(blockRight-blockLeft)*(blockBottom-blockTop);
                for(var y=blockTop;y<blockBottom;y++)for(var x=blockLeft;x<blockRight;x++){var offset=y*stride+x*4;blue+=clean[offset];green+=clean[offset+1];red+=clean[offset+2];alpha+=clean[offset+3];}
                var b=(byte)(blue/count);var g=(byte)(green/count);var r=(byte)(red/count);var a=(byte)(alpha/count);
                for(var y=blockTop;y<blockBottom;y++)for(var x=blockLeft;x<blockRight;x++){var offset=y*stride+x*4;output[offset]=b;output[offset+1]=g;output[offset+2]=r;output[offset+3]=a;}
            }
        }
        Array.Clear(clean);var result=BitmapSource.Create(source.PixelWidth,source.PixelHeight,source.DpiX,source.DpiY,PixelFormats.Bgra32,null,output,stride);result.Freeze();Array.Clear(output);return result;
    }
}
