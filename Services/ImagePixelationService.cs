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
        var clipped=new Int32Rect(
            Math.Clamp(region.X,0,source.PixelWidth-1),
            Math.Clamp(region.Y,0,source.PixelHeight-1),
            Math.Min(region.Width,source.PixelWidth-Math.Clamp(region.X,0,source.PixelWidth-1)),
            Math.Min(region.Height,source.PixelHeight-Math.Clamp(region.Y,0,source.PixelHeight-1)));
        var crop=ScreenCaptureService.Crop(source,clipped);
        var formatted=new FormatConvertedBitmap(crop,PixelFormats.Bgra32,null,0);formatted.Freeze();
        var stride=checked(formatted.PixelWidth*4);var pixels=new byte[checked(stride*formatted.PixelHeight)];formatted.CopyPixels(pixels,stride,0);
        for(var top=0;top<formatted.PixelHeight;top+=blockSize)
        for(var left=0;left<formatted.PixelWidth;left+=blockSize)
        {
            var right=Math.Min(formatted.PixelWidth,left+blockSize);var bottom=Math.Min(formatted.PixelHeight,top+blockSize);long blue=0,green=0,red=0,alpha=0;var count=(right-left)*(bottom-top);
            for(var y=top;y<bottom;y++)for(var x=left;x<right;x++){var offset=y*stride+x*4;blue+=pixels[offset];green+=pixels[offset+1];red+=pixels[offset+2];alpha+=pixels[offset+3];}
            var b=(byte)(blue/count);var g=(byte)(green/count);var r=(byte)(red/count);var a=(byte)(alpha/count);
            for(var y=top;y<bottom;y++)for(var x=left;x<right;x++){var offset=y*stride+x*4;pixels[offset]=b;pixels[offset+1]=g;pixels[offset+2]=r;pixels[offset+3]=a;}
        }
        var result=BitmapSource.Create(formatted.PixelWidth,formatted.PixelHeight,source.DpiX,source.DpiY,PixelFormats.Bgra32,null,pixels,stride);result.Freeze();return result;
    }
}
