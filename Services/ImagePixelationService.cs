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
        // Always preserve the source dimensions. Callers compose the returned
        // frame with other overlays and therefore cannot safely consume a crop
        // for a non-origin region.
        var left=Math.Clamp(region.X,0,source.PixelWidth-1);
        var top=Math.Clamp(region.Y,0,source.PixelHeight-1);
        var right=(int)Math.Clamp((long)region.X+region.Width,left+1,source.PixelWidth);
        var bottom=(int)Math.Clamp((long)region.Y+region.Height,top+1,source.PixelHeight);
        var formatted=source.Format==PixelFormats.Bgra32?source:new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);
        if(formatted is Freezable freezable&&!freezable.IsFrozen)freezable.Freeze();
        var stride=checked(source.PixelWidth*4);var pixels=new byte[checked(stride*source.PixelHeight)];formatted.CopyPixels(pixels,stride,0);
        for(var blockTop=top;blockTop<bottom;blockTop+=blockSize)
        for(var blockLeft=left;blockLeft<right;blockLeft+=blockSize)
        {
            var blockRight=Math.Min(right,blockLeft+blockSize);var blockBottom=Math.Min(bottom,blockTop+blockSize);long blue=0,green=0,red=0,alpha=0;var count=(blockRight-blockLeft)*(blockBottom-blockTop);
            for(var y=blockTop;y<blockBottom;y++)for(var x=blockLeft;x<blockRight;x++){var offset=y*stride+x*4;blue+=pixels[offset];green+=pixels[offset+1];red+=pixels[offset+2];alpha+=pixels[offset+3];}
            var b=(byte)(blue/count);var g=(byte)(green/count);var r=(byte)(red/count);var a=(byte)(alpha/count);
            for(var y=blockTop;y<blockBottom;y++)for(var x=blockLeft;x<blockRight;x++){var offset=y*stride+x*4;pixels[offset]=b;pixels[offset+1]=g;pixels[offset+2]=r;pixels[offset+3]=a;}
        }
        var result=BitmapSource.Create(source.PixelWidth,source.PixelHeight,source.DpiX,source.DpiY,PixelFormats.Bgra32,null,pixels,stride);result.Freeze();return result;
    }
}
