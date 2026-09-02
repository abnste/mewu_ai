using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class ScreenPixelSampler
{
    [ThreadStatic]private static byte[]? _pixelBuffer;

    internal static bool TrySample(BitmapSource source,int x,int y,out Color color)
    {
        color=default;
        if(x<0||y<0||x>=source.PixelWidth||y>=source.PixelHeight)return false;
        var format=source.Format;
        if(format!=PixelFormats.Bgr32&&format!=PixelFormats.Bgra32&&format!=PixelFormats.Pbgra32&&format!=PixelFormats.Bgr24&&format!=PixelFormats.Rgb24)return false;
        var bytesPerPixel=(format.BitsPerPixel+7)/8;var pixel=_pixelBuffer??=new byte[4];
        source.CopyPixels(new Int32Rect(x,y,1,1),pixel,bytesPerPixel,0);
        color=format==PixelFormats.Rgb24
            ?Color.FromRgb(pixel[0],pixel[1],pixel[2])
            :Color.FromRgb(pixel[2],pixel[1],pixel[0]);
        return true;
    }
}
